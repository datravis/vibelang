use crate::ast::*;
use crate::memory::{self, EscapeInfo};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module as LLVMModule;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, StructType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue, IntValue};
use inkwell::OptimizationLevel;
use inkwell::passes::PassBuilderOptions;
use inkwell::{AddressSpace, IntPredicate, FloatPredicate};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodegenError {
    #[error("codegen error: {0}")]
    General(String),
    #[error("LLVM error: {0}")]
    Llvm(String),
    #[error("undefined variable: {0}")]
    UndefinedVar(String),
    #[error("unsupported feature: {0}")]
    Unsupported(String),
}

pub fn emit_ir(
    module: &crate::ast::Module,
    output: &Path,
    target: &str,
    opt_level: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = Context::create();
    let mut gen = Codegen::new(&context, &module.name.join("."), target)?;
    gen.compile_module(module)?;
    if opt_level > 0 {
        gen.run_optimization_passes(opt_level)?;
    }
    gen.llvm_module.print_to_file(output).map_err(|e| {
        CodegenError::Llvm(e.to_string())
    })?;
    Ok(())
}

pub fn emit_object(
    module: &crate::ast::Module,
    output: &Path,
    target: &str,
    opt_level: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = Context::create();
    let mut gen = Codegen::new(&context, &module.name.join("."), target)?;
    gen.compile_module(module)?;
    if opt_level > 0 {
        gen.run_optimization_passes(opt_level)?;
    }

    let target_machine = gen.create_target_machine()?;
    target_machine
        .write_to_file(&gen.llvm_module, FileType::Object, output)
        .map_err(|e| CodegenError::Llvm(e.to_string()))?;

    Ok(())
}

pub fn jit_run(module: &crate::ast::Module, opt_level: u8) -> Result<(), Box<dyn std::error::Error>> {
    let context = Context::create();
    let mut gen = Codegen::new(&context, &module.name.join("."), "native")?;
    gen.compile_module(module)?;
    if opt_level > 0 {
        gen.run_optimization_passes(opt_level)?;
    }

    let engine = gen
        .llvm_module
        .create_jit_execution_engine(OptimizationLevel::Default)
        .map_err(|e| CodegenError::Llvm(e.to_string()))?;

    unsafe {
        let main_fn = engine
            .get_function::<unsafe extern "C" fn() -> i64>("main")
            .map_err(|e| CodegenError::General(format!("no main function: {e}")))?;
        let result = main_fn.call();
        if result != 0 {
            eprintln!("Program exited with code: {result}");
        }
    }

    Ok(())
}

struct Codegen<'ctx> {
    context: &'ctx Context,
    llvm_module: LLVMModule<'ctx>,
    builder: Builder<'ctx>,
    target_triple: String,
    variables: Vec<HashMap<String, BasicValueEnum<'ctx>>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    // TCO state: when compiling a tail-recursive function, these are set
    tco_loop_header: Option<inkwell::basic_block::BasicBlock<'ctx>>,
    tco_param_allocs: Vec<PointerValue<'ctx>>,
    tco_fn_name: Option<String>,

    // --- Memory management ---
    /// Named struct types for records: type_name -> (llvm_struct_type, field_names)
    record_types: HashMap<String, (StructType<'ctx>, Vec<String>)>,
    /// Variant type info: variant_type_name -> [(constructor_name, field_count, tag)]
    variant_types: HashMap<String, Vec<(String, usize, u64)>>,
    /// Constructor -> (variant_type_name, tag, field_count)
    constructor_info: HashMap<String, (String, u64, usize)>,
    /// Region stack: each scope can have a region for arena allocation.
    /// Stores the region pointer (to a linked list of allocations).
    region_stack: Vec<Option<PointerValue<'ctx>>>,
    /// Current escape info for the function being compiled
    current_escape_info: Option<EscapeInfo>,
}

impl<'ctx> Codegen<'ctx> {
    fn new(
        context: &'ctx Context,
        module_name: &str,
        target: &str,
    ) -> Result<Self, CodegenError> {
        let llvm_module = context.create_module(module_name);
        let builder = context.create_builder();

        if target != "native" {
            llvm_module.set_triple(&TargetTriple::create(target));
        }

        Ok(Self {
            context,
            llvm_module,
            builder,
            target_triple: target.to_string(),
            variables: vec![HashMap::new()],
            functions: HashMap::new(),
            tco_loop_header: None,
            tco_param_allocs: Vec::new(),
            tco_fn_name: None,
            record_types: HashMap::new(),
            variant_types: HashMap::new(),
            constructor_info: HashMap::new(),
            region_stack: Vec::new(),
            current_escape_info: None,
        })
    }

    fn create_target_machine(&self) -> Result<TargetMachine, CodegenError> {
        Target::initialize_all(&InitializationConfig::default());

        let triple = if self.target_triple == "native" {
            TargetMachine::get_default_triple()
        } else {
            TargetTriple::create(&self.target_triple)
        };

        let target = Target::from_triple(&triple)
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;

        let cpu = if self.target_triple.contains("aarch64-apple") {
            "apple-m1"
        } else if self.target_triple.contains("x86_64") {
            "x86-64"
        } else {
            "generic"
        };

        let features = if self.target_triple.contains("aarch64") {
            "+neon,+fp-armv8,+v8.5a"
        } else if self.target_triple.contains("x86_64") {
            "+sse2,+cx8"
        } else {
            ""
        };

        target
            .create_target_machine(
                &triple,
                cpu,
                features,
                OptimizationLevel::Default,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| CodegenError::Llvm("failed to create target machine".into()))
    }

    fn run_optimization_passes(&self, opt_level: u8) -> Result<(), CodegenError> {
        let target_machine = self.create_target_machine()?;
        let passes = match opt_level {
            1 => "default<O1>",
            2 => "default<O2>",
            _ => "default<O3>",
        };
        let options = PassBuilderOptions::create();
        options.set_loop_vectorization(opt_level >= 2);
        options.set_loop_slp_vectorization(opt_level >= 2);
        options.set_loop_unrolling(opt_level >= 2);
        options.set_loop_interleaving(opt_level >= 2);
        options.set_merge_functions(opt_level >= 2);

        self.llvm_module
            .run_passes(passes, &target_machine, options)
            .map_err(|e| CodegenError::Llvm(e.to_string()))
    }

    fn push_scope(&mut self) {
        self.variables.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.variables.pop();
    }

    fn set_var(&mut self, name: String, val: BasicValueEnum<'ctx>) {
        if let Some(scope) = self.variables.last_mut() {
            scope.insert(name, val);
        }
    }

    fn get_var(&self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        for scope in self.variables.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Some(*val);
            }
        }
        None
    }

    // ---- Compilation ----

    fn compile_module(&mut self, module: &crate::ast::Module) -> Result<(), CodegenError> {
        self.declare_external_functions();

        // Pass 0: register type definitions (records, variants)
        for decl in &module.declarations {
            if let Decl::TypeDef(td) = decl {
                self.register_type_def(td)?;
            }
        }

        // First pass: declare all functions
        for decl in &module.declarations {
            if let Decl::Function(f) = decl {
                self.declare_function(f)?;
            }
        }

        // Second pass: compile function bodies
        for decl in &module.declarations {
            if let Decl::Function(f) = decl {
                self.compile_function(f)?;
            }
        }

        self.llvm_module.verify().map_err(|e| {
            CodegenError::Llvm(format!("module verification failed: {}", e.to_string()))
        })?;

        Ok(())
    }

    /// Register a type definition so codegen knows about record field layouts and variant tags.
    fn register_type_def(&mut self, td: &TypeDef) -> Result<(), CodegenError> {
        match &td.body {
            TypeBody::Record(fields) => {
                // Create an LLVM struct type for this record.
                // All fields are i64 for now (we use pointer-sized values for everything).
                let field_types: Vec<BasicTypeEnum> = fields
                    .iter()
                    .map(|_| self.context.i64_type().into())
                    .collect();
                let struct_ty = self.context.struct_type(&field_types, false);
                let field_names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
                self.record_types
                    .insert(td.name.clone(), (struct_ty, field_names));
            }
            TypeBody::Variants(variants) => {
                let mut variant_info = Vec::new();
                for (i, v) in variants.iter().enumerate() {
                    let tag = i as u64;
                    variant_info.push((v.name.clone(), v.fields.len(), tag));
                    self.constructor_info
                        .insert(v.name.clone(), (td.name.clone(), tag, v.fields.len()));
                }
                self.variant_types.insert(td.name.clone(), variant_info);
            }
            TypeBody::Alias(_) => {
                // Type aliases don't need codegen registration
            }
        }
        Ok(())
    }

    fn declare_external_functions(&mut self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let i32_ty = self.context.i32_type();
        let void_ty = self.context.void_type();

        // printf for IO
        let printf_type = i32_ty.fn_type(
            &[BasicMetadataTypeEnum::PointerType(ptr_ty)],
            true,
        );
        let printf = self.llvm_module.add_function("printf", printf_type, None);
        self.functions.insert("printf".into(), printf);

        // puts
        let puts_type = i32_ty.fn_type(
            &[BasicMetadataTypeEnum::PointerType(ptr_ty)],
            false,
        );
        let puts = self.llvm_module.add_function("puts", puts_type, None);
        self.functions.insert("puts".into(), puts);

        // --- Memory management runtime ---

        // malloc(size) -> ptr
        let malloc_type = ptr_ty.fn_type(
            &[BasicMetadataTypeEnum::IntType(i64_ty)],
            false,
        );
        let malloc = self.llvm_module.add_function("malloc", malloc_type, None);
        self.functions.insert("malloc".into(), malloc);

        // free(ptr) -> void
        let free_type = void_ty.fn_type(
            &[BasicMetadataTypeEnum::PointerType(ptr_ty)],
            false,
        );
        let free = self.llvm_module.add_function("free", free_type, None);
        self.functions.insert("free".into(), free);

        // memcpy(dest, src, n) -> ptr
        let memcpy_type = ptr_ty.fn_type(
            &[
                BasicMetadataTypeEnum::PointerType(ptr_ty),
                BasicMetadataTypeEnum::PointerType(ptr_ty),
                BasicMetadataTypeEnum::IntType(i64_ty),
            ],
            false,
        );
        let memcpy = self.llvm_module.add_function("memcpy", memcpy_type, None);
        self.functions.insert("memcpy".into(), memcpy);

        // strlen(s) -> i64
        let strlen_type = i64_ty.fn_type(
            &[BasicMetadataTypeEnum::PointerType(ptr_ty)],
            false,
        );
        let strlen = self.llvm_module.add_function("strlen", strlen_type, None);
        self.functions.insert("strlen".into(), strlen);

        // Declare region management functions (compiled inline as LLVM IR)
        self.declare_region_runtime();
        self.declare_refcount_runtime();
    }

    /// Declare region allocation runtime functions.
    ///
    /// A region is a simple arena: a linked list of allocated blocks.
    /// When the region is destroyed, all blocks are freed at once.
    ///
    /// Region node layout: { next: ptr, data: [payload...] }
    fn declare_region_runtime(&mut self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let void_ty = self.context.void_type();

        // vibe_region_alloc(region_head_ptr: ptr, size: i64) -> ptr
        // Allocates `size` bytes in the region, prepending to the linked list.
        let region_alloc_ty = ptr_ty.fn_type(
            &[
                BasicMetadataTypeEnum::PointerType(ptr_ty),
                BasicMetadataTypeEnum::IntType(i64_ty),
            ],
            false,
        );
        let region_alloc_fn = self.llvm_module.add_function(
            "vibe_region_alloc",
            region_alloc_ty,
            None,
        );
        self.functions.insert("vibe_region_alloc".into(), region_alloc_fn);

        // Build the body: allocate node = malloc(8 + size), set node->next = *head, *head = node, return node+8
        {
            let entry = self.context.append_basic_block(region_alloc_fn, "entry");
            self.builder.position_at_end(entry);

            let head_ptr = region_alloc_fn.get_nth_param(0).unwrap().into_pointer_value();
            let size = region_alloc_fn.get_nth_param(1).unwrap().into_int_value();

            // total = size + 8 (for next pointer)
            let eight = i64_ty.const_int(8, false);
            let total = self.builder.build_int_add(size, eight, "total").unwrap();

            // node = malloc(total)
            let malloc_fn = self.functions["malloc"];
            let node = self.builder.build_call(malloc_fn, &[total.into()], "node").unwrap()
                .try_as_basic_value().left().unwrap().into_pointer_value();

            // node->next = *head_ptr
            let old_head = self.builder.build_load(ptr_ty, head_ptr, "old_head").unwrap();
            self.builder.build_store(node, old_head).unwrap();

            // *head_ptr = node
            self.builder.build_store(head_ptr, node).unwrap();

            // return node + 8 (skip the next pointer to get to data)
            let data_ptr = unsafe {
                self.builder.build_gep(self.context.i8_type(), node, &[eight], "data").unwrap()
            };
            self.builder.build_return(Some(&data_ptr)).unwrap();
        }

        // vibe_region_destroy(region_head_ptr: ptr) -> void
        // Walks the linked list and frees all nodes.
        let region_destroy_ty = void_ty.fn_type(
            &[BasicMetadataTypeEnum::PointerType(ptr_ty)],
            false,
        );
        let region_destroy_fn = self.llvm_module.add_function(
            "vibe_region_destroy",
            region_destroy_ty,
            None,
        );
        self.functions.insert("vibe_region_destroy".into(), region_destroy_fn);

        {
            let entry = self.context.append_basic_block(region_destroy_fn, "entry");
            let loop_bb = self.context.append_basic_block(region_destroy_fn, "loop");
            let done_bb = self.context.append_basic_block(region_destroy_fn, "done");

            self.builder.position_at_end(entry);
            let head_ptr = region_destroy_fn.get_nth_param(0).unwrap().into_pointer_value();
            let current = self.builder.build_load(ptr_ty, head_ptr, "current").unwrap().into_pointer_value();
            let null = ptr_ty.const_null();

            // Store null to head so region is empty
            self.builder.build_store(head_ptr, null).unwrap();

            let is_null = self.builder.build_is_null(current, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done_bb, loop_bb).unwrap();

            // Loop: free current, advance to next
            self.builder.position_at_end(loop_bb);
            let phi = self.builder.build_phi(ptr_ty, "node").unwrap();
            phi.add_incoming(&[(&current, entry)]);
            let node = phi.as_basic_value().into_pointer_value();

            // next = node->next (first field)
            let next = self.builder.build_load(ptr_ty, node, "next").unwrap().into_pointer_value();

            // free(node)
            let free_fn = self.functions["free"];
            self.builder.build_call(free_fn, &[node.into()], "").unwrap();

            let next_null = self.builder.build_is_null(next, "next_null").unwrap();
            phi.add_incoming(&[(&next, loop_bb)]);
            self.builder.build_conditional_branch(next_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(done_bb);
            self.builder.build_return(None).unwrap();
        }
    }

    /// Declare reference counting runtime functions.
    ///
    /// Refcounted objects have a header: { refcount: i64, tag: i64, region_id: i64 }
    /// followed by the payload.
    fn declare_refcount_runtime(&mut self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let void_ty = self.context.void_type();

        // vibe_rc_alloc(size: i64, tag: i64) -> ptr
        // Allocates a refcounted object: header (24 bytes) + payload (size bytes).
        // Returns pointer to the payload (past the header).
        let rc_alloc_ty = ptr_ty.fn_type(
            &[
                BasicMetadataTypeEnum::IntType(i64_ty),
                BasicMetadataTypeEnum::IntType(i64_ty),
            ],
            false,
        );
        let rc_alloc_fn = self.llvm_module.add_function("vibe_rc_alloc", rc_alloc_ty, None);
        self.functions.insert("vibe_rc_alloc".into(), rc_alloc_fn);

        {
            let entry = self.context.append_basic_block(rc_alloc_fn, "entry");
            self.builder.position_at_end(entry);

            let size = rc_alloc_fn.get_nth_param(0).unwrap().into_int_value();
            let tag = rc_alloc_fn.get_nth_param(1).unwrap().into_int_value();

            // header_size = 24 (3 * i64)
            let header_size = i64_ty.const_int(24, false);
            let total = self.builder.build_int_add(size, header_size, "total").unwrap();

            let malloc_fn = self.functions["malloc"];
            let raw = self.builder.build_call(malloc_fn, &[total.into()], "raw").unwrap()
                .try_as_basic_value().left().unwrap().into_pointer_value();

            // Write refcount = 1
            let one = i64_ty.const_int(1, false);
            self.builder.build_store(raw, one).unwrap();

            // Write tag at offset 8
            let tag_ptr = unsafe {
                self.builder.build_gep(i64_ty, raw, &[i64_ty.const_int(1, false)], "tag_ptr").unwrap()
            };
            self.builder.build_store(tag_ptr, tag).unwrap();

            // Write region_id = 0 at offset 16
            let region_ptr = unsafe {
                self.builder.build_gep(i64_ty, raw, &[i64_ty.const_int(2, false)], "region_ptr").unwrap()
            };
            self.builder.build_store(region_ptr, i64_ty.const_int(0, false)).unwrap();

            // Return pointer past header
            let payload = unsafe {
                self.builder.build_gep(self.context.i8_type(), raw, &[header_size], "payload").unwrap()
            };
            self.builder.build_return(Some(&payload)).unwrap();
        }

        // vibe_retain(ptr) -> void
        // Increments refcount. If ptr is null, no-op.
        let retain_ty = void_ty.fn_type(
            &[BasicMetadataTypeEnum::PointerType(ptr_ty)],
            false,
        );
        let retain_fn = self.llvm_module.add_function("vibe_retain", retain_ty, None);
        self.functions.insert("vibe_retain".into(), retain_fn);

        {
            let entry = self.context.append_basic_block(retain_fn, "entry");
            let do_retain = self.context.append_basic_block(retain_fn, "do_retain");
            let done = self.context.append_basic_block(retain_fn, "done");

            self.builder.position_at_end(entry);
            let payload_ptr = retain_fn.get_nth_param(0).unwrap().into_pointer_value();
            let is_null = self.builder.build_is_null(payload_ptr, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done, do_retain).unwrap();

            self.builder.position_at_end(do_retain);
            // header = payload - 24
            let header = unsafe {
                self.builder.build_gep(
                    self.context.i8_type(), payload_ptr,
                    &[i64_ty.const_int((-24i64) as u64, true)], "header",
                ).unwrap()
            };
            let rc = self.builder.build_load(i64_ty, header, "rc").unwrap().into_int_value();
            let new_rc = self.builder.build_int_add(rc, i64_ty.const_int(1, false), "new_rc").unwrap();
            self.builder.build_store(header, new_rc).unwrap();
            self.builder.build_unconditional_branch(done).unwrap();

            self.builder.position_at_end(done);
            self.builder.build_return(None).unwrap();
        }

        // vibe_release(ptr) -> void
        // Decrements refcount. If it reaches 0, frees the allocation.
        let release_ty = void_ty.fn_type(
            &[BasicMetadataTypeEnum::PointerType(ptr_ty)],
            false,
        );
        let release_fn = self.llvm_module.add_function("vibe_release", release_ty, None);
        self.functions.insert("vibe_release".into(), release_fn);

        {
            let entry = self.context.append_basic_block(release_fn, "entry");
            let do_release = self.context.append_basic_block(release_fn, "do_release");
            let do_free = self.context.append_basic_block(release_fn, "do_free");
            let done = self.context.append_basic_block(release_fn, "done");

            self.builder.position_at_end(entry);
            let payload_ptr = release_fn.get_nth_param(0).unwrap().into_pointer_value();
            let is_null = self.builder.build_is_null(payload_ptr, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done, do_release).unwrap();

            self.builder.position_at_end(do_release);
            let header = unsafe {
                self.builder.build_gep(
                    self.context.i8_type(), payload_ptr,
                    &[i64_ty.const_int((-24i64) as u64, true)], "header",
                ).unwrap()
            };
            let rc = self.builder.build_load(i64_ty, header, "rc").unwrap().into_int_value();
            let new_rc = self.builder.build_int_sub(rc, i64_ty.const_int(1, false), "new_rc").unwrap();
            self.builder.build_store(header, new_rc).unwrap();
            let is_zero = self.builder.build_int_compare(
                IntPredicate::EQ, new_rc, i64_ty.const_int(0, false), "is_zero",
            ).unwrap();
            self.builder.build_conditional_branch(is_zero, do_free, done).unwrap();

            self.builder.position_at_end(do_free);
            let free_fn = self.functions["free"];
            self.builder.build_call(free_fn, &[header.into()], "").unwrap();
            self.builder.build_unconditional_branch(done).unwrap();

            self.builder.position_at_end(done);
            self.builder.build_return(None).unwrap();
        }
    }

    fn declare_function(&mut self, decl: &FnDecl) -> Result<(), CodegenError> {
        let param_types: Vec<BasicMetadataTypeEnum<'ctx>> = decl
            .params
            .iter()
            .map(|p| self.resolve_param_type(p))
            .collect();

        let ret_type = decl
            .return_type
            .as_ref()
            .map(|t| self.resolve_llvm_type(t))
            .unwrap_or(LLVMType::I64);

        let fn_type = match ret_type {
            LLVMType::I64 => self.context.i64_type().fn_type(&param_types, false),
            LLVMType::F64 => self.context.f64_type().fn_type(&param_types, false),
            LLVMType::I1 => self.context.bool_type().fn_type(&param_types, false),
            LLVMType::I8 => self.context.i8_type().fn_type(&param_types, false),
            LLVMType::Void => self.context.void_type().fn_type(&param_types, false),
            LLVMType::Ptr => self.context.ptr_type(AddressSpace::default()).fn_type(&param_types, false),
        };

        let function = self.llvm_module.add_function(&decl.name, fn_type, None);
        self.functions.insert(decl.name.clone(), function);
        Ok(())
    }

    fn has_tail_self_call(name: &str, expr: &Expr) -> bool {
        match expr {
            Expr::Call(func, _, _) => {
                if let Expr::Ident(fn_name, _) = func.as_ref() {
                    fn_name == name
                } else {
                    false
                }
            }
            Expr::If(_, then_br, else_br, _) => {
                Self::has_tail_self_call(name, then_br)
                    || else_br
                        .as_ref()
                        .map(|e| Self::has_tail_self_call(name, e))
                        .unwrap_or(false)
            }
            Expr::DoBlock(exprs, _) => exprs
                .last()
                .map(|e| Self::has_tail_self_call(name, e))
                .unwrap_or(false),
            _ => false,
        }
    }

    fn compile_function(&mut self, decl: &FnDecl) -> Result<(), CodegenError> {
        let function = *self
            .functions
            .get(&decl.name)
            .ok_or_else(|| CodegenError::UndefinedVar(decl.name.clone()))?;

        let is_tail_recursive =
            !decl.params.is_empty() && Self::has_tail_self_call(&decl.name, &decl.body);

        // Run escape analysis for this function
        let escape_info = memory::analyze_function(decl);
        self.current_escape_info = Some(escape_info);

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        self.push_scope();

        // Set up a region for this function scope
        let region_ptr = self.create_region(function)?;

        if is_tail_recursive {
            // TCO: allocate stack slots for params, create loop header
            let mut allocs = Vec::new();
            for (i, param) in decl.params.iter().enumerate() {
                let val = function.get_nth_param(i as u32).unwrap();
                let alloca = self.builder.build_alloca(self.context.i64_type(), &param.name)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let store_val = if val.is_int_value() && val.into_int_value().get_type().get_bit_width() != 64 {
                    self.builder.build_int_z_extend(val.into_int_value(), self.context.i64_type(), "zext")
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?.into()
                } else {
                    val
                };
                self.builder.build_store(alloca, store_val)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                allocs.push(alloca);
            }

            let loop_header = self.context.append_basic_block(function, "tco_loop");
            self.builder.build_unconditional_branch(loop_header)
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            self.builder.position_at_end(loop_header);

            // Load params from allocas
            for (i, param) in decl.params.iter().enumerate() {
                let loaded = self.builder.build_load(self.context.i64_type(), allocs[i], &param.name)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                self.set_var(param.name.clone(), loaded);
            }

            // Set TCO state
            self.tco_loop_header = Some(loop_header);
            self.tco_param_allocs = allocs;
            self.tco_fn_name = Some(decl.name.clone());

            let result = self.compile_expr(&decl.body, function)?;

            // Clear TCO state
            self.tco_loop_header = None;
            self.tco_param_allocs.clear();
            self.tco_fn_name = None;

            // Return (only reached for non-tail-call branches)
            self.destroy_region(region_ptr)?;
            match result {
                Some(val) => {
                    self.builder.build_return(Some(&val))
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                }
                None => {
                    let zero = self.context.i64_type().const_int(0, false);
                    self.builder.build_return(Some(&zero))
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                }
            }
        } else {
            // Non-tail-recursive: normal compilation
            for (i, param) in decl.params.iter().enumerate() {
                let val = function.get_nth_param(i as u32).unwrap();
                val.set_name(&param.name);
                self.set_var(param.name.clone(), val);
            }

            let result = self.compile_expr(&decl.body, function)?;

            self.destroy_region(region_ptr)?;
            match result {
                Some(val) => {
                    self.builder.build_return(Some(&val))
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                }
                None => {
                    let zero = self.context.i64_type().const_int(0, false);
                    self.builder.build_return(Some(&zero))
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                }
            }
        }

        self.pop_scope();
        self.current_escape_info = None;
        Ok(())
    }

    fn compile_expr(
        &mut self,
        expr: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        match expr {
            Expr::IntLit(n, _) => {
                let val = self.context.i64_type().const_int(*n as u64, true);
                Ok(Some(val.into()))
            }

            Expr::FloatLit(n, _) => {
                let val = self.context.f64_type().const_float(*n);
                Ok(Some(val.into()))
            }

            Expr::BoolLit(b, _) => {
                let val = self.context.bool_type().const_int(*b as u64, false);
                Ok(Some(val.into()))
            }

            Expr::StringLit(s, _) => {
                let global = self.builder.build_global_string_ptr(s, "str")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                Ok(Some(global.as_pointer_value().into()))
            }

            Expr::CharLit(c, _) => {
                let val = self.context.i8_type().const_int(*c as u64, false);
                Ok(Some(val.into()))
            }

            Expr::UnitLit(_) => {
                let val = self.context.i64_type().const_int(0, false);
                Ok(Some(val.into()))
            }

            Expr::Ident(name, _) => {
                if let Some(val) = self.get_var(name) {
                    Ok(Some(val))
                } else if let Some(func) = self.functions.get(name) {
                    Ok(Some(func.as_global_value().as_pointer_value().into()))
                } else {
                    Err(CodegenError::UndefinedVar(name.clone()))
                }
            }

            Expr::BinOp(lhs, op, rhs, _) => {
                let l = self.compile_expr(lhs, function)?.unwrap();
                let r = self.compile_expr(rhs, function)?.unwrap();

                let result = if l.is_int_value() && r.is_int_value() {
                    let lv = l.into_int_value();
                    let rv = r.into_int_value();
                    match op {
                        BinOp::Add => self.builder.build_int_add(lv, rv, "add")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Sub => self.builder.build_int_sub(lv, rv, "sub")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Mul => self.builder.build_int_mul(lv, rv, "mul")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Div => self.builder.build_int_signed_div(lv, rv, "div")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Mod => self.builder.build_int_signed_rem(lv, rv, "rem")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Eq => self.builder.build_int_compare(IntPredicate::EQ, lv, rv, "eq")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Neq => self.builder.build_int_compare(IntPredicate::NE, lv, rv, "ne")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Lt => self.builder.build_int_compare(IntPredicate::SLT, lv, rv, "lt")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Gt => self.builder.build_int_compare(IntPredicate::SGT, lv, rv, "gt")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Lte => self.builder.build_int_compare(IntPredicate::SLE, lv, rv, "le")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Gte => self.builder.build_int_compare(IntPredicate::SGE, lv, rv, "ge")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::And => self.builder.build_and(lv, rv, "and")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Or => self.builder.build_or(lv, rv, "or")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::BitAnd => self.builder.build_and(lv, rv, "band")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::BitOr => self.builder.build_or(lv, rv, "bor")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::BitXor => self.builder.build_xor(lv, rv, "bxor")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Shl => self.builder.build_left_shift(lv, rv, "shl")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Shr => self.builder.build_right_shift(lv, rv, true, "shr")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Concat => {
                            // For now, treat concat on ints as addition
                            self.builder.build_int_add(lv, rv, "concat")
                                .map_err(|e| CodegenError::Llvm(e.to_string()))?.into()
                        }
                    }
                } else if l.is_float_value() && r.is_float_value() {
                    let lv = l.into_float_value();
                    let rv = r.into_float_value();
                    match op {
                        BinOp::Add => self.builder.build_float_add(lv, rv, "fadd")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Sub => self.builder.build_float_sub(lv, rv, "fsub")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Mul => self.builder.build_float_mul(lv, rv, "fmul")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Div => self.builder.build_float_div(lv, rv, "fdiv")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Mod => self.builder.build_float_rem(lv, rv, "frem")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Eq => self.builder.build_float_compare(FloatPredicate::OEQ, lv, rv, "feq")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Neq => self.builder.build_float_compare(FloatPredicate::ONE, lv, rv, "fne")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Lt => self.builder.build_float_compare(FloatPredicate::OLT, lv, rv, "flt")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Gt => self.builder.build_float_compare(FloatPredicate::OGT, lv, rv, "fgt")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Lte => self.builder.build_float_compare(FloatPredicate::OLE, lv, rv, "fle")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        BinOp::Gte => self.builder.build_float_compare(FloatPredicate::OGE, lv, rv, "fge")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into(),
                        _ => return Err(CodegenError::Unsupported(format!("float op {op:?}"))),
                    }
                } else {
                    return Err(CodegenError::Unsupported("mixed-type binary op".into()));
                };

                Ok(Some(result))
            }

            Expr::UnaryOp(op, inner, _) => {
                let val = self.compile_expr(inner, function)?.unwrap();
                let result = match op {
                    UnaryOp::Neg => {
                        if val.is_int_value() {
                            self.builder.build_int_neg(val.into_int_value(), "neg")
                                .map_err(|e| CodegenError::Llvm(e.to_string()))?.into()
                        } else {
                            self.builder.build_float_neg(val.into_float_value(), "fneg")
                                .map_err(|e| CodegenError::Llvm(e.to_string()))?.into()
                        }
                    }
                    UnaryOp::Not => {
                        self.builder.build_not(val.into_int_value(), "not")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into()
                    }
                    UnaryOp::BitNot => {
                        self.builder.build_not(val.into_int_value(), "bnot")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?.into()
                    }
                };
                Ok(Some(result))
            }

            Expr::Call(func_expr, args, _) => {
                // Check if it's a type constructor call like Some(42)
                if let Expr::TypeConstructor(name, _) = func_expr.as_ref() {
                    return self.compile_type_constructor(name, args, function);
                }

                // Check if it's a direct function call
                if let Expr::Ident(name, _) = func_expr.as_ref() {
                    if name == "print" {
                        return self.compile_print(args, function);
                    }

                    // TCO: if this is a tail-recursive self-call, emit a loop jump
                    if self.tco_fn_name.as_deref() == Some(name.as_str()) {
                        if let Some(loop_header) = self.tco_loop_header {
                            let allocs = self.tco_param_allocs.clone();
                            // Evaluate all args first (before storing)
                            let mut compiled_args = Vec::new();
                            for a in args {
                                let val = self.compile_expr(a, function)?.unwrap();
                                compiled_args.push(val);
                            }
                            // Store new values into param allocas
                            for (i, val) in compiled_args.iter().enumerate() {
                                let store_val = self.ensure_i64(*val);
                                self.builder.build_store(allocs[i], store_val)
                                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                            }
                            // Branch back to loop header
                            self.builder.build_unconditional_branch(loop_header)
                                .map_err(|e| CodegenError::Llvm(e.to_string()))?;

                            // Create an unreachable block for subsequent code
                            let dead_bb = self.context.append_basic_block(function, "tco.dead");
                            self.builder.position_at_end(dead_bb);

                            // Return a dummy value (this block is unreachable)
                            let dummy = self.context.i64_type().const_int(0, false);
                            return Ok(Some(dummy.into()));
                        }
                    }

                    if let Some(func) = self.functions.get(name).copied() {
                        let compiled_args: Result<Vec<BasicMetadataValueEnum>, _> = args
                            .iter()
                            .map(|a| {
                                self.compile_expr(a, function)
                                    .map(|v| v.unwrap().into())
                            })
                            .collect();
                        let result = self
                            .builder
                            .build_call(func, &compiled_args?, "call")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        return Ok(result.try_as_basic_value().left());
                    }
                }

                // Indirect call via function pointer
                let callee = self.compile_expr(func_expr, function)?.unwrap();
                let compiled_args: Result<Vec<BasicMetadataValueEnum>, _> = args
                    .iter()
                    .map(|a| self.compile_expr(a, function).map(|v| v.unwrap().into()))
                    .collect();

                // For indirect calls, we need to construct the function type
                let param_types: Vec<BasicMetadataTypeEnum> = args
                    .iter()
                    .map(|_| BasicMetadataTypeEnum::IntType(self.context.i64_type()))
                    .collect();
                let fn_type = self.context.i64_type().fn_type(&param_types, false);

                let result = self
                    .builder
                    .build_indirect_call(fn_type, callee.into_pointer_value(), &compiled_args?, "icall")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                Ok(result.try_as_basic_value().left())
            }

            Expr::If(cond, then_br, else_br, _) => {
                let cond_val = self.compile_expr(cond, function)?.unwrap();

                // Convert to i1 if needed
                let cond_bool = if cond_val.is_int_value() {
                    let iv = cond_val.into_int_value();
                    if iv.get_type().get_bit_width() == 1 {
                        iv
                    } else {
                        let zero = iv.get_type().const_int(0, false);
                        self.builder.build_int_compare(IntPredicate::NE, iv, zero, "tobool")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?
                    }
                } else {
                    return Err(CodegenError::Unsupported("non-integer condition".into()));
                };

                let then_bb = self.context.append_basic_block(function, "then");
                let else_bb = self.context.append_basic_block(function, "else");
                let merge_bb = self.context.append_basic_block(function, "merge");

                self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;

                // Then branch
                self.builder.position_at_end(then_bb);
                let then_val = self.compile_expr(then_br, function)?;
                let then_val = then_val.unwrap_or_else(|| self.context.i64_type().const_int(0, false).into());
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let then_bb_end = self.builder.get_insert_block().unwrap();

                // Else branch
                self.builder.position_at_end(else_bb);
                let else_val = if let Some(else_expr) = else_br {
                    self.compile_expr(else_expr, function)?
                        .unwrap_or_else(|| self.context.i64_type().const_int(0, false).into())
                } else {
                    self.context.i64_type().const_int(0, false).into()
                };
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let else_bb_end = self.builder.get_insert_block().unwrap();

                // Merge
                self.builder.position_at_end(merge_bb);
                let phi = self.builder.build_phi(self.context.i64_type(), "ifresult")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;

                // Ensure types match for phi: cast to i64 if needed
                let then_i64 = self.coerce_to_i64(then_val, function)?;
                let else_i64 = self.coerce_to_i64(else_val, function)?;

                // Need to reposition after coercion since we might have added casts
                // Actually, coercion happens inline, phi incoming should reference the correct blocks
                phi.add_incoming(&[(&then_i64, then_bb_end), (&else_i64, else_bb_end)]);

                Ok(Some(phi.as_basic_value()))
            }

            Expr::Match(scrutinee, arms, _) => {
                if arms.is_empty() {
                    return Ok(Some(self.context.i64_type().const_int(0, false).into()));
                }

                let scrut_val = self.compile_expr(scrutinee, function)?.unwrap();
                let i64_ty = self.context.i64_type();
                let merge_bb = self.context.append_basic_block(function, "match.end");
                let default_bb = self.context.append_basic_block(function, "match.default");

                // Determine if we're matching on constructors (pointer-based, check tag)
                let has_constructors = arms.iter().any(|a| matches!(&a.pattern, Pattern::Constructor(_, _, _)));

                // Get the actual integer to switch on
                let switch_val = if has_constructors && scrut_val.is_pointer_value() {
                    // Load the tag from the first field of the struct
                    let tag = self.builder.build_load(i64_ty, scrut_val.into_pointer_value(), "tag")
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    tag.into_int_value()
                } else if scrut_val.is_int_value() {
                    let iv = scrut_val.into_int_value();
                    if iv.get_type().get_bit_width() != 64 {
                        self.builder.build_int_z_extend(iv, i64_ty, "zext_match")
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?
                    } else {
                        iv
                    }
                } else {
                    // Non-int, non-ptr: fall through to default
                    self.builder.build_unconditional_branch(default_bb)
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;

                    self.builder.position_at_end(default_bb);
                    if let Some(arm) = arms.first() {
                        self.push_scope();
                        let _val = self.compile_expr(&arm.body, function)?;
                        self.pop_scope();
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    } else {
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    }
                    self.builder.position_at_end(merge_bb);
                    return Ok(Some(i64_ty.const_int(0, false).into()));
                };

                // Build switch cases
                let mut switch_cases: Vec<(u64, inkwell::basic_block::BasicBlock<'ctx>, &MatchArm)> = Vec::new();

                for arm in arms {
                    match &arm.pattern {
                        Pattern::IntLit(n, _) => {
                            let bb = self.context.append_basic_block(function, "match.int");
                            switch_cases.push((*n as u64, bb, arm));
                        }
                        Pattern::BoolLit(b, _) => {
                            let bb = self.context.append_basic_block(function, "match.bool");
                            switch_cases.push((*b as u64, bb, arm));
                        }
                        Pattern::Constructor(name, _, _) => {
                            let bb = self.context.append_basic_block(function, &format!("match.{name}"));
                            let tag = if let Some((_, tag, _)) = self.constructor_info.get(name.as_str()).cloned() {
                                tag
                            } else {
                                name.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
                            };
                            switch_cases.push((tag, bb, arm));
                        }
                        _ => {} // Wildcard/Ident handled as default
                    }
                }

                let default_arm = arms.iter().find(|a| {
                    matches!(&a.pattern, Pattern::Wildcard(_) | Pattern::Ident(_, _))
                });

                self.builder.build_switch(
                    switch_val,
                    default_bb,
                    &switch_cases.iter().map(|(n, bb, _)| (i64_ty.const_int(*n, true), *bb)).collect::<Vec<_>>(),
                ).map_err(|e| CodegenError::Llvm(e.to_string()))?;

                let mut phi_incoming: Vec<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();

                for (_, bb, arm) in &switch_cases {
                    self.builder.position_at_end(*bb);
                    self.push_scope();
                    // Bind pattern variables
                    self.bind_pattern_val(&arm.pattern, scrut_val);
                    let val = self.compile_expr(&arm.body, function)?
                        .unwrap_or_else(|| i64_ty.const_int(0, false).into());
                    self.pop_scope();
                    self.builder.build_unconditional_branch(merge_bb)
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    phi_incoming.push((val, self.builder.get_insert_block().unwrap()));
                }

                // Default block
                self.builder.position_at_end(default_bb);
                if let Some(arm) = default_arm {
                    self.push_scope();
                    if let Pattern::Ident(name, _) = &arm.pattern {
                        self.set_var(name.clone(), scrut_val);
                    }
                    let val = self.compile_expr(&arm.body, function)?
                        .unwrap_or_else(|| i64_ty.const_int(0, false).into());
                    self.pop_scope();
                    self.builder.build_unconditional_branch(merge_bb)
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    phi_incoming.push((val, self.builder.get_insert_block().unwrap()));
                } else {
                    self.builder.build_unconditional_branch(merge_bb)
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    phi_incoming.push((i64_ty.const_int(0, false).into(), self.builder.get_insert_block().unwrap()));
                }

                self.builder.position_at_end(merge_bb);
                if phi_incoming.is_empty() {
                    return Ok(Some(i64_ty.const_int(0, false).into()));
                }

                // Determine phi type from first value
                let first_val = phi_incoming[0].0;
                let phi_type: BasicTypeEnum = if first_val.is_pointer_value() {
                    self.context.ptr_type(AddressSpace::default()).into()
                } else if first_val.is_float_value() {
                    self.context.f64_type().into()
                } else {
                    i64_ty.into()
                };

                let phi = self.builder.build_phi(phi_type, "match.result")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;

                for (val, bb) in &phi_incoming {
                    if first_val.is_pointer_value() {
                        // All values must be pointers
                        phi.add_incoming(&[(&*val, *bb)]);
                    } else {
                        let coerced = self.ensure_i64(*val);
                        phi.add_incoming(&[(&coerced, *bb)]);
                    }
                }
                Ok(Some(phi.as_basic_value()))
            }

            Expr::DoBlock(exprs, _) => {
                self.push_scope();
                let mut last = None;
                for expr in exprs {
                    last = self.compile_expr(expr, function)?;
                }
                self.pop_scope();
                Ok(last.or(Some(self.context.i64_type().const_int(0, false).into())))
            }

            Expr::Let(pattern, _, value, body, _) => {
                let val = self.compile_expr(value, function)?.unwrap();
                self.push_scope();
                self.bind_pattern_val(&pattern, val);
                let result = self.compile_expr(body, function)?;
                self.pop_scope();
                Ok(result)
            }

            Expr::LetBind(pattern, _, value, _) => {
                let val = self.compile_expr(value, function)?.unwrap();
                self.bind_pattern_val(&pattern, val);
                Ok(Some(self.context.i64_type().const_int(0, false).into()))
            }

            Expr::Pipe(lhs, rhs, _) => {
                let input = self.compile_expr(lhs, function)?;
                // Pipe: rhs should be a function, call it with lhs
                if let Expr::Ident(name, _) = rhs.as_ref() {
                    if let Some(func) = self.functions.get(name).copied() {
                        if let Some(input_val) = input {
                            let result = self
                                .builder
                                .build_call(func, &[input_val.into()], "pipe")
                                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                            return Ok(result.try_as_basic_value().left());
                        }
                    }
                }
                // Fallback: just evaluate rhs
                self.compile_expr(rhs, function)
            }

            Expr::Lambda(params, body, _) => {
                // Compile lambda as a separate function
                let param_types: Vec<BasicMetadataTypeEnum> = params
                    .iter()
                    .map(|_| BasicMetadataTypeEnum::IntType(self.context.i64_type()))
                    .collect();
                let fn_type = self.context.i64_type().fn_type(&param_types, false);
                let lambda_fn = self.llvm_module.add_function("lambda", fn_type, None);

                let prev_bb = self.builder.get_insert_block();
                let entry = self.context.append_basic_block(lambda_fn, "entry");
                self.builder.position_at_end(entry);

                self.push_scope();
                for (i, p) in params.iter().enumerate() {
                    let val = lambda_fn.get_nth_param(i as u32).unwrap();
                    self.set_var(p.name.clone(), val);
                }

                let result = self.compile_expr(body, lambda_fn)?;
                let ret_val = result.unwrap_or_else(|| self.context.i64_type().const_int(0, false).into());
                self.builder.build_return(Some(&ret_val))
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                self.pop_scope();

                if let Some(bb) = prev_bb {
                    self.builder.position_at_end(bb);
                }

                Ok(Some(lambda_fn.as_global_value().as_pointer_value().into()))
            }

            Expr::List(elems, _) => {
                self.compile_list(elems, function)
            }

            Expr::Tuple(elems, _) => {
                self.compile_tuple(elems, function)
            }

            Expr::Record(fields, _) => {
                self.compile_record(fields, function)
            }

            Expr::RecordUpdate(base, updates, _) => {
                self.compile_record_update(base, updates, function)
            }

            Expr::FieldAccess(base, field, _) => {
                self.compile_field_access(base, field, function)
            }

            Expr::TypeConstructor(name, _) => {
                self.compile_type_constructor(name, &[], function)
            }

            Expr::Handle(expr, _, _) => self.compile_expr(expr, function),

            Expr::Resume(expr, _) => self.compile_expr(expr, function),
        }
    }

    // ---- Compound type compilation ----

    /// Compile a tuple expression: heap-allocate a struct { elem0, elem1, ... }
    fn compile_tuple(
        &mut self,
        elems: &[Expr],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        if elems.is_empty() {
            return Ok(Some(self.context.i64_type().const_int(0, false).into()));
        }

        let i64_ty = self.context.i64_type();

        // Compile all elements
        let mut compiled = Vec::new();
        for e in elems {
            let val = self.compile_expr(e, function)?.unwrap();
            compiled.push(val);
        }

        // Create struct type: { i64, i64, ... } for all elements
        let field_types: Vec<BasicTypeEnum> = compiled.iter().map(|v| {
            if v.is_float_value() {
                self.context.f64_type().into()
            } else {
                i64_ty.into()
            }
        }).collect();
        let struct_ty = self.context.struct_type(&field_types, false);

        // Allocate in region
        let size = i64_ty.const_int((elems.len() * 8) as u64, false);
        let ptr = self.region_alloc(size, function)?;

        // Store each element
        for (i, val) in compiled.iter().enumerate() {
            let elem_ptr = self.builder.build_struct_gep(struct_ty, ptr, i as u32, &format!("tuple.elem.{i}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            let store_val = self.ensure_i64_or_f64(*val);
            self.builder.build_store(elem_ptr, store_val)
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        }

        Ok(Some(ptr.into()))
    }

    /// Compile a list expression: linked list of cons cells { value: i64, next: ptr }
    fn compile_list(
        &mut self,
        elems: &[Expr],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        // cons cell: { value: i64, next: ptr }
        let cons_ty = self.context.struct_type(
            &[i64_ty.into(), ptr_ty.into()],
            false,
        );
        let cell_size = i64_ty.const_int(16, false); // 8 + 8

        if elems.is_empty() {
            return Ok(Some(ptr_ty.const_null().into()));
        }

        // Build list in reverse (last element first) so we can link them
        let mut compiled = Vec::new();
        for e in elems {
            let val = self.compile_expr(e, function)?.unwrap();
            compiled.push(val);
        }

        // Start with null tail
        let mut current: BasicValueEnum = ptr_ty.const_null().into();

        for val in compiled.iter().rev() {
            let cell_ptr = self.region_alloc(cell_size, function)?;

            // Store value
            let val_ptr = self.builder.build_struct_gep(cons_ty, cell_ptr, 0, "cons.val")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            let store_val = self.ensure_i64(*val);
            self.builder.build_store(val_ptr, store_val)
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;

            // Store next pointer
            let next_ptr = self.builder.build_struct_gep(cons_ty, cell_ptr, 1, "cons.next")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            self.builder.build_store(next_ptr, current)
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;

            current = cell_ptr.into();
        }

        Ok(Some(current))
    }

    /// Compile a record expression: { field1: val1, field2: val2 }
    fn compile_record(
        &mut self,
        fields: &[(String, Expr)],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();

        if fields.is_empty() {
            return Ok(Some(self.context.ptr_type(AddressSpace::default()).const_null().into()));
        }

        // Compile all field values
        let mut compiled = Vec::new();
        for (_, expr) in fields {
            let val = self.compile_expr(expr, function)?.unwrap();
            compiled.push(val);
        }

        // Create anonymous struct type
        let field_types: Vec<BasicTypeEnum> = (0..fields.len())
            .map(|_| i64_ty.into())
            .collect();
        let struct_ty = self.context.struct_type(&field_types, false);

        // Allocate
        let size = i64_ty.const_int((fields.len() * 8) as u64, false);
        let ptr = self.region_alloc(size, function)?;

        // Store each field
        for (i, val) in compiled.iter().enumerate() {
            let field_ptr = self.builder.build_struct_gep(struct_ty, ptr, i as u32, &format!("rec.{}", fields[i].0))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            let store_val = self.ensure_i64(*val);
            self.builder.build_store(field_ptr, store_val)
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        }

        Ok(Some(ptr.into()))
    }

    /// Compile a record update: { base | field1: val1 }
    fn compile_record_update(
        &mut self,
        base: &Expr,
        updates: &[(String, Expr)],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let base_val = self.compile_expr(base, function)?.unwrap();

        // Try to find a matching record type to know field layout
        // For now, if we can't find one, fall through to copying
        for (_type_name, (struct_ty, field_names)) in &self.record_types.clone() {
            let num_fields = field_names.len();
            let size = i64_ty.const_int((num_fields * 8) as u64, false);
            let new_ptr = self.region_alloc(size, function)?;

            // Copy all fields from base
            let base_ptr = base_val.into_pointer_value();
            for i in 0..num_fields {
                let src = self.builder.build_struct_gep(*struct_ty, base_ptr, i as u32, "src")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let dst = self.builder.build_struct_gep(*struct_ty, new_ptr, i as u32, "dst")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let val = self.builder.build_load(i64_ty, src, "copy")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                self.builder.build_store(dst, val)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            }

            // Apply updates
            for (field_name, expr) in updates {
                if let Some(idx) = field_names.iter().position(|n| n == field_name) {
                    let val = self.compile_expr(expr, function)?.unwrap();
                    let dst = self.builder.build_struct_gep(*struct_ty, new_ptr, idx as u32, "upd")
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    let store_val = self.ensure_i64(val);
                    self.builder.build_store(dst, store_val)
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                }
            }

            return Ok(Some(new_ptr.into()));
        }

        // Fallback: just return base
        Ok(Some(base_val))
    }

    /// Compile field access: base.field_name
    fn compile_field_access(
        &mut self,
        base: &Expr,
        field: &str,
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let base_val = self.compile_expr(base, function)?.unwrap();

        if !base_val.is_pointer_value() {
            return Ok(Some(base_val));
        }

        let base_ptr = base_val.into_pointer_value();

        // Try to match against known record types
        for (_type_name, (struct_ty, field_names)) in &self.record_types.clone() {
            if let Some(idx) = field_names.iter().position(|n| n == field) {
                let field_ptr = self.builder.build_struct_gep(*struct_ty, base_ptr, idx as u32, &format!("field.{field}"))
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let val = self.builder.build_load(i64_ty, field_ptr, field)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                return Ok(Some(val));
            }
        }

        // Fallback: try numeric index for tuples (field names like "0", "1", ...)
        if let Ok(idx) = field.parse::<u32>() {
            let struct_ty = self.context.struct_type(
                &vec![i64_ty.into(); (idx + 1) as usize],
                false,
            );
            let field_ptr = self.builder.build_struct_gep(struct_ty, base_ptr, idx, &format!("tuple.{idx}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            let val = self.builder.build_load(i64_ty, field_ptr, "elem")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            return Ok(Some(val));
        }

        // Last resort: treat as first field
        let val = self.builder.build_load(i64_ty, base_ptr, "field")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        Ok(Some(val))
    }

    /// Compile a type constructor, optionally with arguments.
    /// e.g., `None` (no args), `Some(42)` (one arg)
    ///
    /// Layout: heap struct { tag: i64, field0: i64, field1: i64, ... }
    fn compile_type_constructor(
        &mut self,
        name: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();

        let (tag, expected_fields) = if let Some((_type_name, tag, field_count)) = self.constructor_info.get(name).cloned() {
            (tag, field_count)
        } else {
            // Unknown constructor: use hash-based tag, no fields
            let tag = name.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            (tag, args.len())
        };

        if expected_fields == 0 && args.is_empty() {
            // Nullary constructor: just return the tag as an i64
            return Ok(Some(i64_ty.const_int(tag, false).into()));
        }

        // Compile arguments
        let mut compiled_args = Vec::new();
        for a in args {
            let val = self.compile_expr(a, function)?.unwrap();
            compiled_args.push(val);
        }

        // Struct: { tag: i64, field0: i64, ... }
        let num_fields = 1 + compiled_args.len();
        let field_types: Vec<BasicTypeEnum> = (0..num_fields)
            .map(|_| i64_ty.into())
            .collect();
        let struct_ty = self.context.struct_type(&field_types, false);

        let size = i64_ty.const_int((num_fields * 8) as u64, false);
        let ptr = self.region_alloc(size, function)?;

        // Store tag
        let tag_ptr = self.builder.build_struct_gep(struct_ty, ptr, 0, "ctor.tag")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        self.builder.build_store(tag_ptr, i64_ty.const_int(tag, false))
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;

        // Store fields
        for (i, val) in compiled_args.iter().enumerate() {
            let field_ptr = self.builder.build_struct_gep(struct_ty, ptr, (i + 1) as u32, &format!("ctor.field.{i}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            let store_val = self.ensure_i64(*val);
            self.builder.build_store(field_ptr, store_val)
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        }

        Ok(Some(ptr.into()))
    }

    // ---- Memory management helpers ----

    /// Create a region for the current scope. Returns a pointer to the region head.
    fn create_region(&mut self, _function: FunctionValue<'ctx>) -> Result<PointerValue<'ctx>, CodegenError> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let region_head = self.builder.build_alloca(ptr_ty, "region")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        self.builder.build_store(region_head, ptr_ty.const_null())
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        self.region_stack.push(Some(region_head));
        Ok(region_head)
    }

    /// Destroy the region, freeing all allocations.
    fn destroy_region(&mut self, region_ptr: PointerValue<'ctx>) -> Result<(), CodegenError> {
        let destroy_fn = self.functions["vibe_region_destroy"];
        self.builder.build_call(destroy_fn, &[region_ptr.into()], "")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        self.region_stack.pop();
        Ok(())
    }

    /// Allocate memory in the current region.
    fn region_alloc(&mut self, size: IntValue<'ctx>, _function: FunctionValue<'ctx>) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(Some(region_ptr)) = self.region_stack.last().copied() {
            let alloc_fn = self.functions["vibe_region_alloc"];
            let ptr = self.builder.build_call(alloc_fn, &[region_ptr.into(), size.into()], "ralloc")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();
            Ok(ptr)
        } else {
            // No region: fall back to malloc
            let malloc_fn = self.functions["malloc"];
            let ptr = self.builder.build_call(malloc_fn, &[size.into()], "malloc")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();
            Ok(ptr)
        }
    }

    /// Allocate a refcounted object (for values that escape their region).
    fn rc_alloc(&mut self, size: IntValue<'ctx>, tag: u64) -> Result<PointerValue<'ctx>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let rc_alloc_fn = self.functions["vibe_rc_alloc"];
        let ptr = self.builder.build_call(
            rc_alloc_fn,
            &[size.into(), i64_ty.const_int(tag, false).into()],
            "rc_alloc",
        ).map_err(|e| CodegenError::Llvm(e.to_string()))?
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();
        Ok(ptr)
    }

    /// Emit a retain call for a pointer value.
    fn emit_retain(&mut self, ptr: PointerValue<'ctx>) -> Result<(), CodegenError> {
        let retain_fn = self.functions["vibe_retain"];
        self.builder.build_call(retain_fn, &[ptr.into()], "")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        Ok(())
    }

    /// Emit a release call for a pointer value.
    fn emit_release(&mut self, ptr: PointerValue<'ctx>) -> Result<(), CodegenError> {
        let release_fn = self.functions["vibe_release"];
        self.builder.build_call(release_fn, &[ptr.into()], "")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        Ok(())
    }

    /// Ensure a value is i64 or f64 (passthrough for floats).
    fn ensure_i64_or_f64(&self, val: BasicValueEnum<'ctx>) -> BasicValueEnum<'ctx> {
        if val.is_float_value() {
            val
        } else {
            self.ensure_i64(val)
        }
    }

    fn compile_print(
        &mut self,
        args: &[Expr],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let puts_fn = self.functions.get("puts").copied()
            .ok_or_else(|| CodegenError::UndefinedVar("puts".into()))?;

        if let Some(arg) = args.first() {
            let val = self.compile_expr(arg, function)?.unwrap();
            if val.is_pointer_value() {
                self.builder
                    .build_call(puts_fn, &[val.into()], "print")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            } else {
                // Print integer: use printf with %lld format
                let printf_fn = self.functions.get("printf").copied()
                    .ok_or_else(|| CodegenError::UndefinedVar("printf".into()))?;
                let fmt = self.builder.build_global_string_ptr("%lld\n", "intfmt")
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                self.builder
                    .build_call(
                        printf_fn,
                        &[fmt.as_pointer_value().into(), val.into()],
                        "printf",
                    )
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            }
        }
        Ok(Some(self.context.i64_type().const_int(0, false).into()))
    }

    fn bind_pattern_val(&mut self, pattern: &Pattern, val: BasicValueEnum<'ctx>) {
        match pattern {
            Pattern::Ident(name, _) => {
                self.set_var(name.clone(), val);
            }
            Pattern::Wildcard(_) => {}
            Pattern::Tuple(pats, _) => {
                if val.is_pointer_value() {
                    let ptr = val.into_pointer_value();
                    let i64_ty = self.context.i64_type();
                    let field_types: Vec<BasicTypeEnum> = (0..pats.len())
                        .map(|_| i64_ty.into())
                        .collect();
                    let struct_ty = self.context.struct_type(&field_types, false);
                    for (i, pat) in pats.iter().enumerate() {
                        if let Ok(field_ptr) = self.builder.build_struct_gep(struct_ty, ptr, i as u32, &format!("tup.{i}")) {
                            if let Ok(field_val) = self.builder.build_load(i64_ty, field_ptr, &format!("tup_val.{i}")) {
                                self.bind_pattern_val(pat, field_val);
                            }
                        }
                    }
                }
            }
            Pattern::Constructor(_name, pats, _) => {
                if val.is_pointer_value() && !pats.is_empty() {
                    let ptr = val.into_pointer_value();
                    let i64_ty = self.context.i64_type();
                    // Constructor layout: { tag: i64, field0: i64, ... }
                    let num_fields = 1 + pats.len();
                    let field_types: Vec<BasicTypeEnum> = (0..num_fields)
                        .map(|_| i64_ty.into())
                        .collect();
                    let struct_ty = self.context.struct_type(&field_types, false);
                    for (i, pat) in pats.iter().enumerate() {
                        if let Ok(field_ptr) = self.builder.build_struct_gep(struct_ty, ptr, (i + 1) as u32, &format!("ctor.{i}")) {
                            if let Ok(field_val) = self.builder.build_load(i64_ty, field_ptr, &format!("ctor_val.{i}")) {
                                self.bind_pattern_val(pat, field_val);
                            }
                        }
                    }
                }
            }
            Pattern::Record(fields, _) => {
                if val.is_pointer_value() {
                    let ptr = val.into_pointer_value();
                    let i64_ty = self.context.i64_type();
                    // Try to find matching record type
                    for (_type_name, (struct_ty, field_names)) in &self.record_types.clone() {
                        for (field_name, pat) in fields {
                            if let Some(idx) = field_names.iter().position(|n| n == field_name) {
                                if let Ok(field_ptr) = self.builder.build_struct_gep(*struct_ty, ptr, idx as u32, field_name) {
                                    if let Ok(field_val) = self.builder.build_load(i64_ty, field_ptr, &format!("{field_name}_val")) {
                                        self.bind_pattern_val(pat, field_val);
                                    }
                                }
                            }
                        }
                        break; // Use first matching type
                    }
                }
            }
            _ => {} // Literal patterns are handled by the match switch
        }
    }

    fn coerce_to_i64(
        &self,
        val: BasicValueEnum<'ctx>,
        _function: FunctionValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        Ok(self.ensure_i64(val))
    }

    fn ensure_i64(&self, val: BasicValueEnum<'ctx>) -> BasicValueEnum<'ctx> {
        if val.is_int_value() {
            let iv = val.into_int_value();
            if iv.get_type().get_bit_width() == 64 {
                val
            } else {
                self.builder
                    .build_int_z_extend(iv, self.context.i64_type(), "zext")
                    .unwrap()
                    .into()
            }
        } else if val.is_pointer_value() {
            self.builder
                .build_ptr_to_int(val.into_pointer_value(), self.context.i64_type(), "ptoi")
                .unwrap()
                .into()
        } else {
            val
        }
    }

    fn resolve_param_type(&self, param: &Param) -> BasicMetadataTypeEnum<'ctx> {
        if let Some(ty) = &param.type_ann {
            match self.resolve_llvm_type(ty) {
                LLVMType::F64 => BasicMetadataTypeEnum::FloatType(self.context.f64_type()),
                LLVMType::I1 => BasicMetadataTypeEnum::IntType(self.context.bool_type()),
                LLVMType::I8 => BasicMetadataTypeEnum::IntType(self.context.i8_type()),
                LLVMType::Ptr => BasicMetadataTypeEnum::PointerType(
                    self.context.ptr_type(AddressSpace::default()),
                ),
                _ => BasicMetadataTypeEnum::IntType(self.context.i64_type()),
            }
        } else {
            BasicMetadataTypeEnum::IntType(self.context.i64_type())
        }
    }

    fn resolve_llvm_type(&self, ty: &TypeExpr) -> LLVMType {
        match ty {
            TypeExpr::Named(name, _) => match name.as_str() {
                "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "Int128" => LLVMType::I64,
                "UInt" | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" | "Byte" => {
                    LLVMType::I64
                }
                "Float" | "Float32" | "Float64" => LLVMType::F64,
                "Bool" => LLVMType::I1,
                "Char" => LLVMType::I8,
                "String" => LLVMType::Ptr,
                "Unit" => LLVMType::I64,
                "Never" => LLVMType::Void,
                _ => {
                    // Check if this is a known record or variant type -> Ptr
                    if self.record_types.contains_key(name)
                        || self.variant_types.contains_key(name)
                    {
                        LLVMType::Ptr
                    } else {
                        LLVMType::I64
                    }
                }
            },
            TypeExpr::Unit => LLVMType::I64,
            TypeExpr::Tuple(_) => LLVMType::Ptr,
            TypeExpr::Function(_, _, _) => LLVMType::Ptr,
            TypeExpr::Record(_, _) => LLVMType::Ptr,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum LLVMType {
    I64,
    F64,
    I1,
    I8,
    Ptr,
    Void,
}
