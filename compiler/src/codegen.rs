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

    // --- Closures ---
    /// Counter for generating unique lambda function names
    lambda_counter: usize,

    // --- Effect system ---
    /// Effect definitions: effect_name -> [(op_name, param_count)]
    effect_defs: HashMap<String, Vec<(String, usize)>>,
    /// Effect operation -> (effect_name, op_index, param_count)
    effect_ops: HashMap<String, (String, usize, usize)>,
    /// Global handler stack pointer
    handler_stack_global: Option<PointerValue<'ctx>>,
    /// Global handler stack top counter
    handler_top_global: Option<PointerValue<'ctx>>,
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
            lambda_counter: 0,
            effect_defs: HashMap::new(),
            effect_ops: HashMap::new(),
            handler_stack_global: None,
            handler_top_global: None,
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
        self.declare_effect_runtime();
        self.declare_concurrency_runtime();
        self.declare_pipeline_runtime();

        // Pass 0: register type definitions (records, variants)
        for decl in &module.declarations {
            match decl {
                Decl::TypeDef(td) => self.register_type_def(td)?,
                Decl::EffectDef(ed) => self.register_effect_def(ed)?,
                _ => {}
            }
        }

        // First pass: declare all functions (including vibe declarations)
        for decl in &module.declarations {
            match decl {
                Decl::Function(f) => self.declare_function(f)?,
                Decl::VibeDecl(v) => {
                    let fn_decl = FnDecl {
                        public: false,
                        name: v.name.clone(),
                        params: v.params.clone(),
                        return_type: v.return_type.clone(),
                        effects: Vec::new(),
                        body: v.body.clone(),
                        span: v.span,
                    };
                    self.declare_function(&fn_decl)?;
                }
                _ => {}
            }
        }

        // Second pass: compile function bodies
        for decl in &module.declarations {
            match decl {
                Decl::Function(f) => self.compile_function(f)?,
                Decl::VibeDecl(v) => {
                    let fn_decl = FnDecl {
                        public: false,
                        name: v.name.clone(),
                        params: v.params.clone(),
                        return_type: v.return_type.clone(),
                        effects: Vec::new(),
                        body: v.body.clone(),
                        span: v.span,
                    };
                    self.compile_function(&fn_decl)?;
                }
                _ => {}
            }
        }

        self.llvm_module.verify().map_err(|e| {
            CodegenError::Llvm(format!("module verification failed: {}", e.to_string()))
        })?;

        Ok(())
    }

    fn register_effect_def(&mut self, ed: &EffectDef) -> Result<(), CodegenError> {
        let mut ops = Vec::new();
        for (i, op) in ed.operations.iter().enumerate() {
            ops.push((op.name.clone(), op.params.len()));
            self.effect_ops.insert(
                op.name.clone(),
                (ed.name.clone(), i, op.params.len()),
            );
        }
        self.effect_defs.insert(ed.name.clone(), ops);
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

                    // Check if this is an effect operation call
                    if let Some((effect_name, _op_idx, _param_count)) = self.effect_ops.get(name).cloned() {
                        return self.compile_perform(&effect_name, name, args, function);
                    }

                    // Check if this is a pipeline operation (source, map, filter, etc.)
                    if let Some(result) = self.try_compile_pipeline_call(name, args, function)? {
                        return Ok(Some(result));
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

                // Indirect call — the callee is a closure struct {fn_ptr, env_ptr}
                let callee = self.compile_expr(func_expr, function)?.unwrap();
                let compiled_args: Result<Vec<BasicValueEnum>, _> = args
                    .iter()
                    .map(|a| self.compile_expr(a, function).map(|v| v.unwrap()))
                    .collect();
                let compiled_args = compiled_args?;

                self.compile_closure_call(callee, &compiled_args, function)
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

                // Check if rhs is a pipeline operation like map(f), filter(f), fold(init, f), etc.
                if let Some(input_val) = input {
                    if let Expr::Call(func_expr, args, _) = rhs.as_ref() {
                        if let Expr::Ident(name, _) = func_expr.as_ref() {
                            let ptr_ty = self.context.ptr_type(AddressSpace::default());
                            let region_ptr = self.region_stack.last().copied().flatten()
                                .unwrap_or(ptr_ty.const_null());

                            match name.as_str() {
                                "map" if !args.is_empty() && input_val.is_pointer_value() => {
                                    let func_val = self.compile_expr(&args[0], function)?.unwrap();
                                    if func_val.is_pointer_value() {
                                        let map_fn = self.functions["vibe_list_map"];
                                        let result = self.builder.build_call(
                                            map_fn,
                                            &[input_val.into(), func_val.into(), region_ptr.into()],
                                            "pipe_map",
                                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                                        return Ok(result.try_as_basic_value().left());
                                    }
                                }
                                "filter" if !args.is_empty() && input_val.is_pointer_value() => {
                                    let func_val = self.compile_expr(&args[0], function)?.unwrap();
                                    if func_val.is_pointer_value() {
                                        let filter_fn = self.functions["vibe_list_filter"];
                                        let result = self.builder.build_call(
                                            filter_fn,
                                            &[input_val.into(), func_val.into(), region_ptr.into()],
                                            "pipe_filter",
                                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                                        return Ok(result.try_as_basic_value().left());
                                    }
                                }
                                "fold" if args.len() >= 2 && input_val.is_pointer_value() => {
                                    let init_val = self.compile_expr(&args[0], function)?.unwrap();
                                    let func_val = self.compile_expr(&args[1], function)?.unwrap();
                                    let fold_fn = self.functions["vibe_list_fold"];
                                    let init_i64 = self.ensure_i64(init_val);
                                    let result = self.builder.build_call(
                                        fold_fn,
                                        &[input_val.into(), init_i64.into(), func_val.into()],
                                        "pipe_fold",
                                    ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                                    return Ok(result.try_as_basic_value().left());
                                }
                                "for_each" if !args.is_empty() && input_val.is_pointer_value() => {
                                    let func_val = self.compile_expr(&args[0], function)?.unwrap();
                                    if func_val.is_pointer_value() {
                                        let foreach_fn = self.functions["vibe_list_for_each"];
                                        self.builder.build_call(
                                            foreach_fn,
                                            &[input_val.into(), func_val.into()],
                                            "",
                                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                                        let i64_ty = self.context.i64_type();
                                        return Ok(Some(i64_ty.const_int(0, false).into()));
                                    }
                                }
                                "collect" | "collect_vec" => {
                                    return Ok(Some(input_val));
                                }
                                "count" => {
                                    if input_val.is_pointer_value() {
                                        let len_fn = self.functions["vibe_list_length"];
                                        let result = self.builder.build_call(
                                            len_fn, &[input_val.into()], "pipe_count",
                                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                                        return Ok(result.try_as_basic_value().left());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    // Handle pipe to identifiers: collect, count, first, last
                    if let Expr::Ident(name, _) = rhs.as_ref() {
                        match name.as_str() {
                            "collect" | "collect_vec" => {
                                return Ok(Some(input_val));
                            }
                            "count" => {
                                if input_val.is_pointer_value() {
                                    let len_fn = self.functions["vibe_list_length"];
                                    let result = self.builder.build_call(
                                        len_fn, &[input_val.into()], "pipe_count",
                                    ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                                    return Ok(result.try_as_basic_value().left());
                                }
                            }
                            _ => {}
                        }

                        if let Some(func) = self.functions.get(name).copied() {
                            let result = self
                                .builder
                                .build_call(func, &[input_val.into()], "pipe")
                                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                            return Ok(result.try_as_basic_value().left());
                        }

                        // Could be a variable holding a closure
                        if let Some(closure_val) = self.get_var(name) {
                            if closure_val.is_pointer_value() {
                                return self.compile_closure_call(closure_val, &[input_val], function);
                            }
                        }
                    }

                    // Fallback: evaluate rhs as a closure and call it with input
                    let rhs_val = self.compile_expr(rhs, function)?;
                    if let Some(rhs_v) = rhs_val {
                        if rhs_v.is_pointer_value() {
                            return self.compile_closure_call(rhs_v, &[input_val], function);
                        }
                    }
                }

                // Last resort fallback: just evaluate rhs
                self.compile_expr(rhs, function)
            }

            Expr::Lambda(params, body, _) => {
                self.compile_lambda(params, body, function)
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

            Expr::Handle(body_expr, handlers, _) => {
                self.compile_handle(body_expr, handlers, function)
            }

            Expr::Resume(expr, _) => {
                // Resume returns the value from the handler — just evaluate and return
                self.compile_expr(expr, function)
            }

            Expr::Perform(effect_name, op_name, args, _) => {
                self.compile_perform(effect_name, op_name, args, function)
            }

            Expr::Par(exprs, _) => {
                self.compile_par(exprs, function)
            }

            Expr::Pmap(collection, func, _) => {
                self.compile_pmap(collection, func, function)
            }

            Expr::VibePipeline(source, stages, _) => {
                self.compile_vibe_pipeline(source, stages, function)
            }
        }
    }

    // ---- Closure compilation ----

    /// Collect free variables in an expression that are not bound by lambda params or local lets.
    fn collect_free_vars(expr: &Expr, bound: &mut Vec<String>, free: &mut Vec<String>) {
        match expr {
            Expr::Ident(name, _) => {
                if !bound.contains(name) && !free.contains(name) {
                    free.push(name.clone());
                }
            }
            Expr::IntLit(_, _) | Expr::FloatLit(_, _) | Expr::StringLit(_, _)
            | Expr::CharLit(_, _) | Expr::BoolLit(_, _) | Expr::UnitLit(_)
            | Expr::TypeConstructor(_, _) => {}

            Expr::List(elems, _) | Expr::Tuple(elems, _) | Expr::DoBlock(elems, _) => {
                for e in elems {
                    Self::collect_free_vars(e, bound, free);
                }
            }
            Expr::Record(fields, _) => {
                for (_, e) in fields {
                    Self::collect_free_vars(e, bound, free);
                }
            }
            Expr::RecordUpdate(base, fields, _) => {
                Self::collect_free_vars(base, bound, free);
                for (_, e) in fields {
                    Self::collect_free_vars(e, bound, free);
                }
            }
            Expr::FieldAccess(base, _, _) => {
                Self::collect_free_vars(base, bound, free);
            }
            Expr::BinOp(l, _, r, _) | Expr::Pipe(l, r, _) | Expr::Pmap(l, r, _) => {
                Self::collect_free_vars(l, bound, free);
                Self::collect_free_vars(r, bound, free);
            }
            Expr::UnaryOp(_, inner, _) | Expr::Resume(inner, _) => {
                Self::collect_free_vars(inner, bound, free);
            }
            Expr::Call(func, args, _) => {
                Self::collect_free_vars(func, bound, free);
                for a in args {
                    Self::collect_free_vars(a, bound, free);
                }
            }
            Expr::Lambda(params, body, _) => {
                let mut inner_bound = bound.clone();
                for p in params {
                    inner_bound.push(p.name.clone());
                }
                Self::collect_free_vars(body, &mut inner_bound, free);
            }
            Expr::If(cond, then_br, else_br, _) => {
                Self::collect_free_vars(cond, bound, free);
                Self::collect_free_vars(then_br, bound, free);
                if let Some(e) = else_br {
                    Self::collect_free_vars(e, bound, free);
                }
            }
            Expr::Match(scrut, arms, _) => {
                Self::collect_free_vars(scrut, bound, free);
                for arm in arms {
                    let mut arm_bound = bound.clone();
                    Self::collect_pattern_bindings(&arm.pattern, &mut arm_bound);
                    if let Some(g) = &arm.guard {
                        Self::collect_free_vars(g, &mut arm_bound, free);
                    }
                    Self::collect_free_vars(&arm.body, &mut arm_bound, free);
                }
            }
            Expr::Let(pat, _, val, body, _) => {
                Self::collect_free_vars(val, bound, free);
                let mut inner_bound = bound.clone();
                Self::collect_pattern_bindings(pat, &mut inner_bound);
                Self::collect_free_vars(body, &mut inner_bound, free);
            }
            Expr::LetBind(pat, _, val, _) => {
                Self::collect_free_vars(val, bound, free);
                Self::collect_pattern_bindings(pat, bound);
            }
            Expr::Handle(body, handlers, _) => {
                Self::collect_free_vars(body, bound, free);
                for h in handlers {
                    let mut h_bound = bound.clone();
                    for p in &h.params {
                        h_bound.push(p.clone());
                    }
                    Self::collect_free_vars(&h.body, &mut h_bound, free);
                }
            }
            Expr::Perform(_, _, args, _) => {
                for a in args {
                    Self::collect_free_vars(a, bound, free);
                }
            }
            Expr::Par(exprs, _) => {
                for e in exprs {
                    Self::collect_free_vars(e, bound, free);
                }
            }
            Expr::VibePipeline(source, stages, _) => {
                Self::collect_free_vars(source, bound, free);
                for stage in stages {
                    match stage {
                        PipelineStage::Map(e) | PipelineStage::Filter(e)
                        | PipelineStage::FlatMap(e) | PipelineStage::FilterMap(e)
                        | PipelineStage::Take(e) | PipelineStage::Drop(e)
                        | PipelineStage::TakeWhile(e) | PipelineStage::DropWhile(e)
                        | PipelineStage::ForEach(e) | PipelineStage::SortBy(e)
                        | PipelineStage::GroupBy(e) | PipelineStage::Chunk(e)
                        | PipelineStage::Any(e) | PipelineStage::All(e)
                        | PipelineStage::Reduce(e) | PipelineStage::Inspect(e) => {
                            Self::collect_free_vars(e, bound, free);
                        }
                        PipelineStage::Fold(a, b) | PipelineStage::Scan(a, b) => {
                            Self::collect_free_vars(a, bound, free);
                            Self::collect_free_vars(b, bound, free);
                        }
                        PipelineStage::Collect | PipelineStage::Count
                        | PipelineStage::First | PipelineStage::Last
                        | PipelineStage::Distinct => {}
                    }
                }
            }
        }
    }

    /// Extract variable names introduced by a pattern.
    fn collect_pattern_bindings(pat: &Pattern, bound: &mut Vec<String>) {
        match pat {
            Pattern::Ident(name, _) => bound.push(name.clone()),
            Pattern::Constructor(_, pats, _) | Pattern::Tuple(pats, _) => {
                for p in pats {
                    Self::collect_pattern_bindings(p, bound);
                }
            }
            Pattern::Record(fields, _) => {
                for (_, p) in fields {
                    Self::collect_pattern_bindings(p, bound);
                }
            }
            _ => {} // Wildcard, literals — no bindings
        }
    }

    /// Compile a lambda expression into a closure: a heap-allocated `{fn_ptr, env_ptr}` pair.
    ///
    /// Closure layout (2 pointers):
    ///   slot 0: fn_ptr  — pointer to the lifted function (env_ptr as first arg, then params)
    ///   slot 1: env_ptr — pointer to captured environment struct, or null if no captures
    ///
    /// The lifted function signature: `fn(env: ptr, p0: i64, p1: i64, ...) -> i64`
    /// For lambdas with no captures, env is still passed but ignored.
    fn compile_lambda(
        &mut self,
        params: &[Param],
        body: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        // Generate unique name
        let lambda_id = self.lambda_counter;
        self.lambda_counter += 1;
        let lambda_name = format!("__lambda_{}", lambda_id);

        // Collect free variables (variables captured from the enclosing scope)
        let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let mut bound = param_names.clone();
        // Also treat known top-level functions as bound (not captured)
        for fname in self.functions.keys() {
            bound.push(fname.clone());
        }
        let mut free_vars = Vec::new();
        Self::collect_free_vars(body, &mut bound, &mut free_vars);

        // Filter to only variables actually available in the current scope
        let captures: Vec<(String, BasicValueEnum<'ctx>)> = free_vars
            .iter()
            .filter_map(|name| {
                self.get_var(name).map(|val| (name.clone(), val))
            })
            .collect();

        // Build the lifted function type: fn(env_ptr, param0, param1, ...) -> i64
        let mut lifted_param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        lifted_param_types.push(BasicMetadataTypeEnum::PointerType(ptr_ty)); // env ptr
        for p in params {
            lifted_param_types.push(self.resolve_param_type(p));
        }
        let fn_type = i64_ty.fn_type(&lifted_param_types, false);
        let lambda_fn = self.llvm_module.add_function(&lambda_name, fn_type, None);

        // Save current builder position
        let prev_bb = self.builder.get_insert_block();

        // Compile the lambda body in the new function
        let entry = self.context.append_basic_block(lambda_fn, "entry");
        self.builder.position_at_end(entry);

        self.push_scope();

        // Load captured variables from env struct
        let env_ptr = lambda_fn.get_nth_param(0).unwrap().into_pointer_value();
        if !captures.is_empty() {
            let env_struct_ty = self.context.struct_type(
                &vec![i64_ty.into(); captures.len()],
                false,
            );
            for (i, (name, _)) in captures.iter().enumerate() {
                let field_ptr = self.builder
                    .build_struct_gep(env_struct_ty, env_ptr, i as u32, &format!("env.{}", name))
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let loaded = self.builder
                    .build_load(i64_ty, field_ptr, &name)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                self.set_var(name.clone(), loaded);
            }
        }

        // Bind lambda parameters (starting at index 1 because 0 is env)
        for (i, p) in params.iter().enumerate() {
            let val = lambda_fn.get_nth_param((i + 1) as u32).unwrap();
            val.set_name(&p.name);
            self.set_var(p.name.clone(), val);
        }

        let result = self.compile_expr(body, lambda_fn)?;
        let ret_val = result.unwrap_or_else(|| i64_ty.const_int(0, false).into());
        let ret_val = self.ensure_i64(ret_val);
        self.builder.build_return(Some(&ret_val))
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;

        self.pop_scope();

        // Restore builder position
        if let Some(bb) = prev_bb {
            self.builder.position_at_end(bb);
        }

        // Allocate the environment struct on the heap (if captures exist)
        let env_alloc = if captures.is_empty() {
            ptr_ty.const_null()
        } else {
            let env_size = i64_ty.const_int((captures.len() * 8) as u64, false);
            let malloc_fn = self.functions["malloc"];
            let env_raw = self.builder
                .build_call(malloc_fn, &[env_size.into()], "env_alloc")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();

            // Store captured values into the env struct
            let env_struct_ty = self.context.struct_type(
                &vec![i64_ty.into(); captures.len()],
                false,
            );
            for (i, (_, val)) in captures.iter().enumerate() {
                let field_ptr = self.builder
                    .build_struct_gep(env_struct_ty, env_raw, i as u32, &format!("env_store_{}", i))
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                let store_val = self.ensure_i64(*val);
                self.builder.build_store(field_ptr, store_val)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            }
            env_raw
        };

        // Allocate the closure struct: { fn_ptr, env_ptr }
        let closure_struct_ty = self.context.struct_type(
            &[ptr_ty.into(), ptr_ty.into()],
            false,
        );
        let closure_size = i64_ty.const_int(16, false); // 2 pointers
        let closure_ptr = self.builder
            .build_call(self.functions["malloc"], &[closure_size.into()], "closure_alloc")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Store fn_ptr
        let fn_ptr_slot = self.builder
            .build_struct_gep(closure_struct_ty, closure_ptr, 0, "closure.fn")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        let fn_as_ptr = lambda_fn.as_global_value().as_pointer_value();
        self.builder.build_store(fn_ptr_slot, fn_as_ptr)
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;

        // Store env_ptr
        let env_ptr_slot = self.builder
            .build_struct_gep(closure_struct_ty, closure_ptr, 1, "closure.env")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        self.builder.build_store(env_ptr_slot, env_alloc)
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;

        Ok(Some(closure_ptr.into()))
    }

    /// Call a closure value. The closure is a pointer to `{fn_ptr, env_ptr}`.
    /// We extract both, then call `fn_ptr(env_ptr, args...)`.
    fn compile_closure_call(
        &mut self,
        callee: BasicValueEnum<'ctx>,
        args: &[BasicValueEnum<'ctx>],
        _function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        let closure_struct_ty = self.context.struct_type(
            &[ptr_ty.into(), ptr_ty.into()],
            false,
        );

        let closure_ptr = if callee.is_pointer_value() {
            callee.into_pointer_value()
        } else {
            // It might be an i64 encoding of a pointer — convert
            self.builder
                .build_int_to_ptr(callee.into_int_value(), ptr_ty, "itop")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?
        };

        // Load fn_ptr from closure[0]
        let fn_ptr_slot = self.builder
            .build_struct_gep(closure_struct_ty, closure_ptr, 0, "closure.fn.ptr")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        let fn_ptr = self.builder
            .build_load(ptr_ty, fn_ptr_slot, "fn_ptr")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?
            .into_pointer_value();

        // Load env_ptr from closure[1]
        let env_ptr_slot = self.builder
            .build_struct_gep(closure_struct_ty, closure_ptr, 1, "closure.env.ptr")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        let env_ptr = self.builder
            .build_load(ptr_ty, env_ptr_slot, "env_ptr")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;

        // Build indirect call: fn_ptr(env_ptr, arg0, arg1, ...)
        let mut call_args: Vec<BasicMetadataValueEnum> = Vec::new();
        call_args.push(env_ptr.into());
        for a in args {
            call_args.push((*a).into());
        }

        // Build the function type: fn(ptr, i64, i64, ...) -> i64
        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        param_types.push(BasicMetadataTypeEnum::PointerType(ptr_ty));
        for _ in args {
            param_types.push(BasicMetadataTypeEnum::IntType(i64_ty));
        }
        let fn_type = i64_ty.fn_type(&param_types, false);

        let result = self.builder
            .build_indirect_call(fn_type, fn_ptr, &call_args, "closure_call")
            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        Ok(result.try_as_basic_value().left())
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

    // ============================================================
    // Effect System Runtime
    // ============================================================

    /// Declare the effect handler runtime: global handler stack + push/pop/perform functions.
    ///
    /// Handler entry layout: { effect_hash: i64, op_hash: i64, handler_fn: ptr, user_data: ptr }
    /// Handler stack: global array of 256 entries
    /// Handler top: global i64 counter
    fn declare_effect_runtime(&mut self) {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let void_ty = self.context.void_type();

        // Handler entry struct: { effect_hash: i64, op_hash: i64, handler_fn: ptr, user_data: ptr }
        let handler_entry_ty = self.context.struct_type(
            &[i64_ty.into(), i64_ty.into(), ptr_ty.into(), ptr_ty.into()],
            false,
        );

        // Global handler stack: [256 x handler_entry]
        let stack_ty = handler_entry_ty.array_type(256);
        let handler_stack = self.llvm_module.add_global(stack_ty, None, "vibe_handler_stack");
        handler_stack.set_initializer(&stack_ty.const_zero());
        handler_stack.set_linkage(inkwell::module::Linkage::Internal);
        self.handler_stack_global = Some(handler_stack.as_pointer_value());

        // Global handler top counter
        let handler_top = self.llvm_module.add_global(i64_ty, None, "vibe_handler_top");
        handler_top.set_initializer(&i64_ty.const_int(0, false));
        handler_top.set_linkage(inkwell::module::Linkage::Internal);
        self.handler_top_global = Some(handler_top.as_pointer_value());

        // vibe_handler_push(effect_hash: i64, op_hash: i64, handler_fn: ptr, user_data: ptr)
        let push_ty = void_ty.fn_type(
            &[i64_ty.into(), i64_ty.into(), ptr_ty.into(), ptr_ty.into()],
            false,
        );
        let push_fn = self.llvm_module.add_function("vibe_handler_push", push_ty, None);
        self.functions.insert("vibe_handler_push".into(), push_fn);

        {
            let entry = self.context.append_basic_block(push_fn, "entry");
            self.builder.position_at_end(entry);

            let effect_hash = push_fn.get_nth_param(0).unwrap().into_int_value();
            let op_hash = push_fn.get_nth_param(1).unwrap().into_int_value();
            let handler_fn_param = push_fn.get_nth_param(2).unwrap().into_pointer_value();
            let user_data = push_fn.get_nth_param(3).unwrap().into_pointer_value();

            let top_ptr = self.handler_top_global.unwrap();
            let top = self.builder.build_load(i64_ty, top_ptr, "top").unwrap().into_int_value();

            let stack_ptr = self.handler_stack_global.unwrap();
            let stack_array_ty = handler_entry_ty.array_type(256);

            // entry_ptr = &handler_stack[0][top]
            let entry_ptr = unsafe {
                self.builder.build_gep(stack_array_ty, stack_ptr, &[i64_ty.const_int(0, false), top], "entry_ptr").unwrap()
            };

            // Store fields
            let f0 = self.builder.build_struct_gep(handler_entry_ty, entry_ptr, 0, "f0").unwrap();
            self.builder.build_store(f0, effect_hash).unwrap();
            let f1 = self.builder.build_struct_gep(handler_entry_ty, entry_ptr, 1, "f1").unwrap();
            self.builder.build_store(f1, op_hash).unwrap();
            let f2 = self.builder.build_struct_gep(handler_entry_ty, entry_ptr, 2, "f2").unwrap();
            self.builder.build_store(f2, handler_fn_param).unwrap();
            let f3 = self.builder.build_struct_gep(handler_entry_ty, entry_ptr, 3, "f3").unwrap();
            self.builder.build_store(f3, user_data).unwrap();

            // top++
            let new_top = self.builder.build_int_add(top, i64_ty.const_int(1, false), "new_top").unwrap();
            self.builder.build_store(top_ptr, new_top).unwrap();

            self.builder.build_return(None).unwrap();
        }

        // vibe_handler_pop(count: i64)
        let pop_ty = void_ty.fn_type(&[i64_ty.into()], false);
        let pop_fn = self.llvm_module.add_function("vibe_handler_pop", pop_ty, None);
        self.functions.insert("vibe_handler_pop".into(), pop_fn);

        {
            let entry = self.context.append_basic_block(pop_fn, "entry");
            self.builder.position_at_end(entry);

            let count = pop_fn.get_nth_param(0).unwrap().into_int_value();
            let top_ptr = self.handler_top_global.unwrap();
            let top = self.builder.build_load(i64_ty, top_ptr, "top").unwrap().into_int_value();
            let new_top = self.builder.build_int_sub(top, count, "new_top").unwrap();
            self.builder.build_store(top_ptr, new_top).unwrap();
            self.builder.build_return(None).unwrap();
        }

        // vibe_handler_perform(effect_hash: i64, op_hash: i64, arg: i64) -> i64
        // Searches the handler stack from top down, calls matching handler
        let perform_ty = i64_ty.fn_type(
            &[i64_ty.into(), i64_ty.into(), i64_ty.into()],
            false,
        );
        let perform_fn = self.llvm_module.add_function("vibe_handler_perform", perform_ty, None);
        self.functions.insert("vibe_handler_perform".into(), perform_fn);

        {
            let entry = self.context.append_basic_block(perform_fn, "entry");
            let loop_bb = self.context.append_basic_block(perform_fn, "loop");
            let found_bb = self.context.append_basic_block(perform_fn, "found");
            let not_found_bb = self.context.append_basic_block(perform_fn, "not_found");

            self.builder.position_at_end(entry);
            let eff_hash = perform_fn.get_nth_param(0).unwrap().into_int_value();
            let o_hash = perform_fn.get_nth_param(1).unwrap().into_int_value();
            let arg = perform_fn.get_nth_param(2).unwrap().into_int_value();

            let top_ptr = self.handler_top_global.unwrap();
            let top = self.builder.build_load(i64_ty, top_ptr, "top").unwrap().into_int_value();

            // Start from top - 1
            let start_idx = self.builder.build_int_sub(top, i64_ty.const_int(1, false), "start").unwrap();
            self.builder.build_unconditional_branch(loop_bb).unwrap();

            // Loop: search handler stack
            self.builder.position_at_end(loop_bb);
            let idx_phi = self.builder.build_phi(i64_ty, "idx").unwrap();
            idx_phi.add_incoming(&[(&start_idx, entry)]);
            let idx = idx_phi.as_basic_value().into_int_value();

            // Check bounds: idx < 0 means not found
            let is_negative = self.builder.build_int_compare(
                IntPredicate::SLT, idx, i64_ty.const_int(0, false), "neg",
            ).unwrap();
            self.builder.build_conditional_branch(is_negative, not_found_bb, found_bb).unwrap();

            // Check if handler matches
            self.builder.position_at_end(found_bb);
            let stack_ptr = self.handler_stack_global.unwrap();
            let stack_array_ty = handler_entry_ty.array_type(256);
            let ep = unsafe {
                self.builder.build_gep(stack_array_ty, stack_ptr, &[i64_ty.const_int(0, false), idx], "ep").unwrap()
            };

            let stored_eff = self.builder.build_load(
                i64_ty,
                self.builder.build_struct_gep(handler_entry_ty, ep, 0, "eff_ptr").unwrap(),
                "stored_eff",
            ).unwrap().into_int_value();
            let stored_op = self.builder.build_load(
                i64_ty,
                self.builder.build_struct_gep(handler_entry_ty, ep, 1, "op_ptr").unwrap(),
                "stored_op",
            ).unwrap().into_int_value();

            let eff_match = self.builder.build_int_compare(IntPredicate::EQ, stored_eff, eff_hash, "eff_eq").unwrap();
            let op_match = self.builder.build_int_compare(IntPredicate::EQ, stored_op, o_hash, "op_eq").unwrap();
            let both_match = self.builder.build_and(eff_match, op_match, "match").unwrap();

            let call_bb = self.context.append_basic_block(perform_fn, "call");
            let next_bb = self.context.append_basic_block(perform_fn, "next");
            self.builder.build_conditional_branch(both_match, call_bb, next_bb).unwrap();

            // Call the handler
            self.builder.position_at_end(call_bb);
            let handler_fn_ptr = self.builder.build_load(
                ptr_ty,
                self.builder.build_struct_gep(handler_entry_ty, ep, 2, "fn_ptr").unwrap(),
                "handler_fn",
            ).unwrap().into_pointer_value();
            let user_data_val = self.builder.build_load(
                ptr_ty,
                self.builder.build_struct_gep(handler_entry_ty, ep, 3, "ud_ptr").unwrap(),
                "user_data",
            ).unwrap().into_pointer_value();

            // Call handler(arg, user_data) -> i64
            let handler_fn_ty = i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false);
            let result = self.builder.build_indirect_call(
                handler_fn_ty, handler_fn_ptr, &[arg.into(), user_data_val.into()], "result",
            ).unwrap();
            let result_val = result.try_as_basic_value().left().unwrap();
            self.builder.build_return(Some(&result_val)).unwrap();

            // Continue searching
            self.builder.position_at_end(next_bb);
            let next_idx = self.builder.build_int_sub(idx, i64_ty.const_int(1, false), "next_idx").unwrap();
            idx_phi.add_incoming(&[(&next_idx, next_bb)]);
            self.builder.build_unconditional_branch(loop_bb).unwrap();

            // Not found: return 0 (no handler)
            self.builder.position_at_end(not_found_bb);
            self.builder.build_return(Some(&i64_ty.const_int(0, false))).unwrap();
        }
    }

    /// Compile a handle expression: push handlers, run body, pop handlers
    fn compile_handle(
        &mut self,
        body_expr: &Expr,
        handlers: &[Handler],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let push_fn = self.functions["vibe_handler_push"];
        let pop_fn = self.functions["vibe_handler_pop"];

        let handler_count = handlers.len();

        // For each handler, compile the handler body as a separate function
        // Handler function signature: fn(arg: i64, user_data: ptr) -> i64
        for handler in handlers {
            let effect_hash = Self::hash_name(&handler.effect_name);
            let op_hash = Self::hash_name(&handler.operation);

            // Compile handler body as lambda
            let handler_fn_ty = i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false);
            let handler_fn_name = format!("handler_{}_{}", handler.effect_name, handler.operation);
            let handler_fn = self.llvm_module.add_function(&handler_fn_name, handler_fn_ty, None);

            let prev_bb = self.builder.get_insert_block();
            let handler_entry = self.context.append_basic_block(handler_fn, "entry");
            self.builder.position_at_end(handler_entry);

            self.push_scope();

            // Bind handler parameters to the arg value
            let arg_val = handler_fn.get_nth_param(0).unwrap();
            let _user_data = handler_fn.get_nth_param(1).unwrap();

            // If handler has params, bind the first one to arg
            if let Some(param_name) = handler.params.first() {
                self.set_var(param_name.clone(), arg_val);
            }

            // Compile handler body
            let result = self.compile_expr(&handler.body, handler_fn)?;
            let ret_val = result.unwrap_or_else(|| i64_ty.const_int(0, false).into());
            let ret_val = self.ensure_i64(ret_val);
            self.builder.build_return(Some(&ret_val))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;

            self.pop_scope();

            if let Some(bb) = prev_bb {
                self.builder.position_at_end(bb);
            }

            // Push this handler onto the stack
            self.builder.build_call(
                push_fn,
                &[
                    i64_ty.const_int(effect_hash, false).into(),
                    i64_ty.const_int(op_hash, false).into(),
                    handler_fn.as_global_value().as_pointer_value().into(),
                    ptr_ty.const_null().into(),
                ],
                "",
            ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
        }

        // Compile the body expression
        let result = self.compile_expr(body_expr, function)?;

        // Pop all handlers
        self.builder.build_call(
            pop_fn,
            &[i64_ty.const_int(handler_count as u64, false).into()],
            "",
        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;

        Ok(result)
    }

    /// Compile an effect operation: look up handler and call it
    fn compile_perform(
        &mut self,
        effect_name: &str,
        op_name: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let perform_fn = self.functions["vibe_handler_perform"];

        let effect_hash = Self::hash_name(effect_name);
        let op_hash = Self::hash_name(op_name);

        // Compile the first argument (or 0 if no args)
        let arg_val = if let Some(first_arg) = args.first() {
            let val = self.compile_expr(first_arg, function)?.unwrap();
            self.ensure_i64(val)
        } else {
            i64_ty.const_int(0, false).into()
        };

        let result = self.builder.build_call(
            perform_fn,
            &[
                i64_ty.const_int(effect_hash, false).into(),
                i64_ty.const_int(op_hash, false).into(),
                arg_val.into(),
            ],
            "perform_result",
        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;

        Ok(result.try_as_basic_value().left())
    }

    /// Hash a name to a u64 for handler lookup
    fn hash_name(name: &str) -> u64 {
        name.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
    }

    // ============================================================
    // Concurrency Runtime
    // ============================================================

    /// Declare threading runtime using pthreads
    fn declare_concurrency_runtime(&mut self) {
        let i64_ty = self.context.i64_type();
        let i32_ty = self.context.i32_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        // pthread_create(thread: ptr, attr: ptr, start_routine: ptr, arg: ptr) -> i32
        let pthread_create_ty = i32_ty.fn_type(
            &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into(), ptr_ty.into()],
            false,
        );
        let pthread_create = self.llvm_module.add_function("pthread_create", pthread_create_ty, None);
        self.functions.insert("pthread_create".into(), pthread_create);

        // pthread_join(thread: i64, retval: ptr) -> i32
        let pthread_join_ty = i32_ty.fn_type(
            &[i64_ty.into(), ptr_ty.into()],
            false,
        );
        let pthread_join = self.llvm_module.add_function("pthread_join", pthread_join_ty, None);
        self.functions.insert("pthread_join".into(), pthread_join);

        // Thread wrapper: struct { fn_ptr: ptr, result: i64 }
        // vibe_thread_entry(arg: ptr) -> ptr
        // Calls the function pointer stored in arg, stores result
        let thread_entry_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
        let thread_entry = self.llvm_module.add_function("vibe_thread_entry", thread_entry_ty, None);
        self.functions.insert("vibe_thread_entry".into(), thread_entry);

        {
            let entry = self.context.append_basic_block(thread_entry, "entry");
            self.builder.position_at_end(entry);

            let arg_ptr = thread_entry.get_nth_param(0).unwrap().into_pointer_value();

            // struct layout: { fn_ptr: ptr, result: i64 }
            let task_struct_ty = self.context.struct_type(
                &[ptr_ty.into(), i64_ty.into()],
                false,
            );

            // Load fn_ptr
            let fn_ptr_ptr = self.builder.build_struct_gep(task_struct_ty, arg_ptr, 0, "fn_ptr_ptr").unwrap();
            let fn_ptr = self.builder.build_load(ptr_ty, fn_ptr_ptr, "fn_ptr").unwrap().into_pointer_value();

            // Call the thunk: fn() -> i64
            let thunk_ty = i64_ty.fn_type(&[], false);
            let result = self.builder.build_indirect_call(thunk_ty, fn_ptr, &[], "thunk_result").unwrap();
            let result_val = result.try_as_basic_value().left()
                .unwrap_or_else(|| i64_ty.const_int(0, false).into());

            // Store result
            let result_ptr = self.builder.build_struct_gep(task_struct_ty, arg_ptr, 1, "result_ptr").unwrap();
            self.builder.build_store(result_ptr, result_val).unwrap();

            self.builder.build_return(Some(&ptr_ty.const_null())).unwrap();
        }
    }

    /// Compile par(expr1, expr2, ...) — parallel evaluation of expressions
    fn compile_par(
        &mut self,
        exprs: &[Expr],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        let task_struct_ty = self.context.struct_type(
            &[ptr_ty.into(), i64_ty.into()],
            false,
        );
        let task_struct_size = i64_ty.const_int(16, false); // ptr(8) + i64(8)

        let malloc_fn = self.functions["malloc"];
        let free_fn = self.functions["free"];
        let pthread_create_fn = self.functions["pthread_create"];
        let pthread_join_fn = self.functions["pthread_join"];
        let thread_entry_fn = self.functions["vibe_thread_entry"];

        // For each expression, compile it as a thunk (no-arg lambda)
        let mut thunk_fns = Vec::new();
        for (i, expr) in exprs.iter().enumerate() {
            let thunk_ty = i64_ty.fn_type(&[], false);
            let thunk_name = format!("par_thunk_{i}");
            let thunk_fn = self.llvm_module.add_function(&thunk_name, thunk_ty, None);

            let prev_bb = self.builder.get_insert_block();
            let thunk_entry = self.context.append_basic_block(thunk_fn, "entry");
            self.builder.position_at_end(thunk_entry);

            self.push_scope();
            let result = self.compile_expr(expr, thunk_fn)?;
            let ret_val = result.unwrap_or_else(|| i64_ty.const_int(0, false).into());
            let ret_val = self.ensure_i64(ret_val);
            self.builder.build_return(Some(&ret_val))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            self.pop_scope();

            if let Some(bb) = prev_bb {
                self.builder.position_at_end(bb);
            }
            thunk_fns.push(thunk_fn);
        }

        // Allocate task structs and thread handles
        let mut task_ptrs = Vec::new();
        let mut thread_allocs = Vec::new();

        for (i, thunk_fn) in thunk_fns.iter().enumerate() {
            // Allocate task struct
            let task_ptr = self.builder.build_call(malloc_fn, &[task_struct_size.into()], &format!("task_{i}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?
                .try_as_basic_value().left().unwrap().into_pointer_value();

            // Store fn_ptr
            let fn_ptr_field = self.builder.build_struct_gep(task_struct_ty, task_ptr, 0, "fn_field")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            self.builder.build_store(fn_ptr_field, thunk_fn.as_global_value().as_pointer_value())
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;

            // Initialize result to 0
            let res_field = self.builder.build_struct_gep(task_struct_ty, task_ptr, 1, "res_field")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            self.builder.build_store(res_field, i64_ty.const_int(0, false))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;

            // Allocate thread handle (pthread_t is i64 on most platforms)
            let thread_alloc = self.builder.build_alloca(i64_ty, &format!("thread_{i}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;

            // pthread_create(&thread, NULL, thread_entry, task_ptr)
            self.builder.build_call(
                pthread_create_fn,
                &[
                    thread_alloc.into(),
                    ptr_ty.const_null().into(),
                    thread_entry_fn.as_global_value().as_pointer_value().into(),
                    task_ptr.into(),
                ],
                &format!("create_{i}"),
            ).map_err(|e| CodegenError::Llvm(e.to_string()))?;

            task_ptrs.push(task_ptr);
            thread_allocs.push(thread_alloc);
        }

        // Join all threads and collect results
        let mut results = Vec::new();
        for (i, (task_ptr, thread_alloc)) in task_ptrs.iter().zip(thread_allocs.iter()).enumerate() {
            let thread_handle = self.builder.build_load(i64_ty, *thread_alloc, &format!("th_{i}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            self.builder.build_call(
                pthread_join_fn,
                &[thread_handle.into(), ptr_ty.const_null().into()],
                &format!("join_{i}"),
            ).map_err(|e| CodegenError::Llvm(e.to_string()))?;

            // Read result from task struct
            let res_field = self.builder.build_struct_gep(task_struct_ty, *task_ptr, 1, &format!("res_{i}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            let result_val = self.builder.build_load(i64_ty, res_field, &format!("result_{i}"))
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            results.push(result_val);

            // Free task struct
            self.builder.build_call(free_fn, &[(*task_ptr).into()], "")
                .map_err(|e| CodegenError::Llvm(e.to_string()))?;
        }

        // Pack results into a tuple
        if results.len() == 1 {
            Ok(Some(results[0]))
        } else {
            let field_types: Vec<BasicTypeEnum> = results.iter().map(|_| i64_ty.into()).collect();
            let tuple_ty = self.context.struct_type(&field_types, false);
            let size = i64_ty.const_int((results.len() * 8) as u64, false);
            let tuple_ptr = self.region_alloc(size, function)?;

            for (i, val) in results.iter().enumerate() {
                let field_ptr = self.builder.build_struct_gep(tuple_ty, tuple_ptr, i as u32, &format!("par_result_{i}"))
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                self.builder.build_store(field_ptr, *val)
                    .map_err(|e| CodegenError::Llvm(e.to_string()))?;
            }

            Ok(Some(tuple_ptr.into()))
        }
    }

    /// Compile pmap(collection, function) — parallel map over a list
    fn compile_pmap(
        &mut self,
        collection: &Expr,
        func: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        // For v0.1, implement pmap as sequential map with threading infrastructure
        // Compile collection and function
        let col_val = self.compile_expr(collection, function)?.unwrap();
        let func_val = self.compile_expr(func, function)?.unwrap();

        // Use the vibe_list_map runtime function
        if let Some(map_fn) = self.functions.get("vibe_list_map").copied() {
            let func_ptr = if func_val.is_pointer_value() {
                func_val.into_pointer_value()
            } else {
                return Err(CodegenError::Unsupported("pmap requires a function".into()));
            };

            let col_ptr = if col_val.is_pointer_value() {
                col_val.into_pointer_value()
            } else {
                return Ok(Some(col_val));
            };

            let region_ptr = self.region_stack.last().copied().flatten()
                .unwrap_or(ptr_ty.const_null());

            let result = self.builder.build_call(
                map_fn,
                &[col_ptr.into(), func_ptr.into(), region_ptr.into()],
                "pmap_result",
            ).map_err(|e| CodegenError::Llvm(e.to_string()))?;

            Ok(result.try_as_basic_value().left())
        } else {
            // Fallback: just return the collection
            Ok(Some(col_val))
        }
    }

    // ============================================================
    // Vibe Pipeline Runtime
    // ============================================================

    /// Declare pipeline runtime functions: source, map, filter, fold, collect, etc.
    fn declare_pipeline_runtime(&mut self) {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        // Cons cell type for lists: { value: i64, next: ptr }
        let cons_ty = self.context.struct_type(
            &[i64_ty.into(), ptr_ty.into()],
            false,
        );
        let cell_size = i64_ty.const_int(16, false);

        // vibe_list_map(list: ptr, fn: ptr, region: ptr) -> ptr
        // Maps a function over a linked list, returns new list in the given region
        let map_ty = ptr_ty.fn_type(
            &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into()],
            false,
        );
        let map_fn = self.llvm_module.add_function("vibe_list_map", map_ty, None);
        self.functions.insert("vibe_list_map".into(), map_fn);

        {
            let entry = self.context.append_basic_block(map_fn, "entry");
            let loop_bb = self.context.append_basic_block(map_fn, "loop");
            let body_bb = self.context.append_basic_block(map_fn, "body");
            let done_bb = self.context.append_basic_block(map_fn, "done");

            self.builder.position_at_end(entry);
            let list = map_fn.get_nth_param(0).unwrap().into_pointer_value();
            let func_ptr = map_fn.get_nth_param(1).unwrap().into_pointer_value();
            let region = map_fn.get_nth_param(2).unwrap().into_pointer_value();

            // result_head_alloca stores the head of the new list
            let result_head = self.builder.build_alloca(ptr_ty, "result_head").unwrap();
            self.builder.build_store(result_head, ptr_ty.const_null()).unwrap();
            // tail_ptr_alloca points to the "next" field of the last node
            let tail_ptr = self.builder.build_alloca(ptr_ty, "tail_ptr").unwrap();
            self.builder.build_store(tail_ptr, result_head).unwrap();

            let is_null = self.builder.build_is_null(list, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(loop_bb);
            let curr_phi = self.builder.build_phi(ptr_ty, "curr").unwrap();
            curr_phi.add_incoming(&[(&list, entry)]);
            let curr = curr_phi.as_basic_value().into_pointer_value();

            // Load value from current cons cell
            let val_ptr = self.builder.build_struct_gep(cons_ty, curr, 0, "val_ptr").unwrap();
            let val = self.builder.build_load(i64_ty, val_ptr, "val").unwrap();

            // Call fn(val)
            let fn_ty = i64_ty.fn_type(&[i64_ty.into()], false);
            let mapped = self.builder.build_indirect_call(fn_ty, func_ptr, &[val.into()], "mapped").unwrap();
            let mapped_val = mapped.try_as_basic_value().left()
                .unwrap_or_else(|| i64_ty.const_int(0, false).into());

            // Allocate new cons cell in region
            let region_alloc_fn = self.functions["vibe_region_alloc"];
            let new_cell = self.builder.build_call(region_alloc_fn, &[region.into(), cell_size.into()], "new_cell").unwrap()
                .try_as_basic_value().left().unwrap().into_pointer_value();

            // Store mapped value
            let new_val_ptr = self.builder.build_struct_gep(cons_ty, new_cell, 0, "new_val").unwrap();
            self.builder.build_store(new_val_ptr, mapped_val).unwrap();

            // Set next = null
            let new_next_ptr = self.builder.build_struct_gep(cons_ty, new_cell, 1, "new_next").unwrap();
            self.builder.build_store(new_next_ptr, ptr_ty.const_null()).unwrap();

            // Link: *tail_ptr = new_cell
            let current_tail = self.builder.build_load(ptr_ty, tail_ptr, "cur_tail").unwrap().into_pointer_value();
            self.builder.build_store(current_tail, new_cell).unwrap();

            // Update tail_ptr to point to new_cell's next field
            self.builder.build_store(tail_ptr, new_next_ptr).unwrap();

            // Advance to next element
            let next_ptr = self.builder.build_struct_gep(cons_ty, curr, 1, "next_ptr").unwrap();
            let next = self.builder.build_load(ptr_ty, next_ptr, "next").unwrap().into_pointer_value();
            let next_null = self.builder.build_is_null(next, "next_null").unwrap();
            curr_phi.add_incoming(&[(&next, body_bb)]);

            self.builder.build_unconditional_branch(body_bb).unwrap();

            self.builder.position_at_end(body_bb);
            self.builder.build_conditional_branch(next_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(done_bb);
            let result = self.builder.build_load(ptr_ty, result_head, "result").unwrap();
            self.builder.build_return(Some(&result)).unwrap();
        }

        // vibe_list_filter(list: ptr, predicate: ptr, region: ptr) -> ptr
        let filter_ty = ptr_ty.fn_type(
            &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into()],
            false,
        );
        let filter_fn = self.llvm_module.add_function("vibe_list_filter", filter_ty, None);
        self.functions.insert("vibe_list_filter".into(), filter_fn);

        {
            let entry = self.context.append_basic_block(filter_fn, "entry");
            let loop_bb = self.context.append_basic_block(filter_fn, "loop");
            let check_bb = self.context.append_basic_block(filter_fn, "check");
            let keep_bb = self.context.append_basic_block(filter_fn, "keep");
            let skip_bb = self.context.append_basic_block(filter_fn, "skip");
            let done_bb = self.context.append_basic_block(filter_fn, "done");

            self.builder.position_at_end(entry);
            let list = filter_fn.get_nth_param(0).unwrap().into_pointer_value();
            let pred_ptr = filter_fn.get_nth_param(1).unwrap().into_pointer_value();
            let region = filter_fn.get_nth_param(2).unwrap().into_pointer_value();

            let result_head = self.builder.build_alloca(ptr_ty, "result_head").unwrap();
            self.builder.build_store(result_head, ptr_ty.const_null()).unwrap();
            let tail_ptr = self.builder.build_alloca(ptr_ty, "tail_ptr").unwrap();
            self.builder.build_store(tail_ptr, result_head).unwrap();

            let is_null = self.builder.build_is_null(list, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(loop_bb);
            let curr_phi = self.builder.build_phi(ptr_ty, "curr").unwrap();
            curr_phi.add_incoming(&[(&list, entry)]);
            let curr = curr_phi.as_basic_value().into_pointer_value();

            let val_ptr = self.builder.build_struct_gep(cons_ty, curr, 0, "val_ptr").unwrap();
            let val = self.builder.build_load(i64_ty, val_ptr, "val").unwrap();

            // Call predicate(val)
            let pred_fn_ty = i64_ty.fn_type(&[i64_ty.into()], false);
            let pred_result = self.builder.build_indirect_call(pred_fn_ty, pred_ptr, &[val.into()], "pred").unwrap();
            let pred_val = pred_result.try_as_basic_value().left()
                .unwrap_or_else(|| i64_ty.const_int(0, false).into());

            self.builder.build_unconditional_branch(check_bb).unwrap();

            self.builder.position_at_end(check_bb);
            let keep = self.builder.build_int_compare(
                IntPredicate::NE, pred_val.into_int_value(), i64_ty.const_int(0, false), "keep",
            ).unwrap();
            self.builder.build_conditional_branch(keep, keep_bb, skip_bb).unwrap();

            // Keep: add to result list
            self.builder.position_at_end(keep_bb);
            let region_alloc_fn = self.functions["vibe_region_alloc"];
            let new_cell = self.builder.build_call(region_alloc_fn, &[region.into(), cell_size.into()], "new_cell").unwrap()
                .try_as_basic_value().left().unwrap().into_pointer_value();

            let new_val_ptr = self.builder.build_struct_gep(cons_ty, new_cell, 0, "nv").unwrap();
            self.builder.build_store(new_val_ptr, val).unwrap();
            let new_next_ptr = self.builder.build_struct_gep(cons_ty, new_cell, 1, "nn").unwrap();
            self.builder.build_store(new_next_ptr, ptr_ty.const_null()).unwrap();

            let current_tail = self.builder.build_load(ptr_ty, tail_ptr, "ct").unwrap().into_pointer_value();
            self.builder.build_store(current_tail, new_cell).unwrap();
            self.builder.build_store(tail_ptr, new_next_ptr).unwrap();
            self.builder.build_unconditional_branch(skip_bb).unwrap();

            // Skip / continue to next element
            self.builder.position_at_end(skip_bb);
            let next_ptr = self.builder.build_struct_gep(cons_ty, curr, 1, "next_ptr").unwrap();
            let next = self.builder.build_load(ptr_ty, next_ptr, "next").unwrap().into_pointer_value();
            let next_null = self.builder.build_is_null(next, "next_null").unwrap();
            curr_phi.add_incoming(&[(&next, skip_bb)]);
            self.builder.build_conditional_branch(next_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(done_bb);
            let result = self.builder.build_load(ptr_ty, result_head, "result").unwrap();
            self.builder.build_return(Some(&result)).unwrap();
        }

        // vibe_list_fold(list: ptr, init: i64, fn: ptr) -> i64
        // Folds a list: fn(acc, elem) -> acc
        let fold_ty = i64_ty.fn_type(
            &[ptr_ty.into(), i64_ty.into(), ptr_ty.into()],
            false,
        );
        let fold_fn = self.llvm_module.add_function("vibe_list_fold", fold_ty, None);
        self.functions.insert("vibe_list_fold".into(), fold_fn);

        {
            let entry = self.context.append_basic_block(fold_fn, "entry");
            let loop_bb = self.context.append_basic_block(fold_fn, "loop");
            let next_bb = self.context.append_basic_block(fold_fn, "next");
            let done_bb = self.context.append_basic_block(fold_fn, "done");

            self.builder.position_at_end(entry);
            let list = fold_fn.get_nth_param(0).unwrap().into_pointer_value();
            let init = fold_fn.get_nth_param(1).unwrap().into_int_value();
            let fn_ptr = fold_fn.get_nth_param(2).unwrap().into_pointer_value();

            let is_null = self.builder.build_is_null(list, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(loop_bb);
            let curr_phi = self.builder.build_phi(ptr_ty, "curr").unwrap();
            curr_phi.add_incoming(&[(&list, entry)]);
            let acc_phi = self.builder.build_phi(i64_ty, "acc").unwrap();
            acc_phi.add_incoming(&[(&init, entry)]);

            let curr = curr_phi.as_basic_value().into_pointer_value();
            let acc = acc_phi.as_basic_value().into_int_value();

            let val_ptr = self.builder.build_struct_gep(cons_ty, curr, 0, "val_ptr").unwrap();
            let val = self.builder.build_load(i64_ty, val_ptr, "val").unwrap();

            // Call fn(acc, val)
            let fold_fn_ty = i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false);
            let new_acc = self.builder.build_indirect_call(fold_fn_ty, fn_ptr, &[acc.into(), val.into()], "new_acc").unwrap();
            let new_acc_val = new_acc.try_as_basic_value().left()
                .unwrap_or_else(|| i64_ty.const_int(0, false).into());

            let next_ptr = self.builder.build_struct_gep(cons_ty, curr, 1, "next_ptr").unwrap();
            let next = self.builder.build_load(ptr_ty, next_ptr, "next").unwrap().into_pointer_value();
            let next_null = self.builder.build_is_null(next, "next_null").unwrap();

            curr_phi.add_incoming(&[(&next, next_bb)]);
            acc_phi.add_incoming(&[(&new_acc_val, next_bb)]);

            self.builder.build_unconditional_branch(next_bb).unwrap();

            self.builder.position_at_end(next_bb);
            self.builder.build_conditional_branch(next_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(done_bb);
            let result_phi = self.builder.build_phi(i64_ty, "result").unwrap();
            result_phi.add_incoming(&[(&init, entry), (&new_acc_val, next_bb)]);
            self.builder.build_return(Some(&result_phi.as_basic_value())).unwrap();
        }

        // vibe_list_length(list: ptr) -> i64
        let len_ty = i64_ty.fn_type(&[ptr_ty.into()], false);
        let len_fn = self.llvm_module.add_function("vibe_list_length", len_ty, None);
        self.functions.insert("vibe_list_length".into(), len_fn);

        {
            let entry = self.context.append_basic_block(len_fn, "entry");
            let loop_bb = self.context.append_basic_block(len_fn, "loop");
            let next_bb = self.context.append_basic_block(len_fn, "next");
            let done_bb = self.context.append_basic_block(len_fn, "done");

            self.builder.position_at_end(entry);
            let list = len_fn.get_nth_param(0).unwrap().into_pointer_value();
            let is_null = self.builder.build_is_null(list, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(loop_bb);
            let curr_phi = self.builder.build_phi(ptr_ty, "curr").unwrap();
            curr_phi.add_incoming(&[(&list, entry)]);
            let count_phi = self.builder.build_phi(i64_ty, "count").unwrap();
            count_phi.add_incoming(&[(&i64_ty.const_int(0, false), entry)]);

            let curr = curr_phi.as_basic_value().into_pointer_value();
            let count = count_phi.as_basic_value().into_int_value();
            let new_count = self.builder.build_int_add(count, i64_ty.const_int(1, false), "inc").unwrap();

            let next_ptr = self.builder.build_struct_gep(cons_ty, curr, 1, "next_ptr").unwrap();
            let next = self.builder.build_load(ptr_ty, next_ptr, "next").unwrap().into_pointer_value();
            let next_null = self.builder.build_is_null(next, "next_null").unwrap();

            curr_phi.add_incoming(&[(&next, next_bb)]);
            count_phi.add_incoming(&[(&new_count, next_bb)]);

            self.builder.build_unconditional_branch(next_bb).unwrap();

            self.builder.position_at_end(next_bb);
            self.builder.build_conditional_branch(next_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(done_bb);
            let result_phi = self.builder.build_phi(i64_ty, "result").unwrap();
            result_phi.add_incoming(&[(&i64_ty.const_int(0, false), entry), (&new_count, next_bb)]);
            self.builder.build_return(Some(&result_phi.as_basic_value())).unwrap();
        }

        // vibe_list_for_each(list: ptr, fn: ptr) -> void
        let foreach_ty = self.context.void_type().fn_type(
            &[ptr_ty.into(), ptr_ty.into()],
            false,
        );
        let foreach_fn = self.llvm_module.add_function("vibe_list_for_each", foreach_ty, None);
        self.functions.insert("vibe_list_for_each".into(), foreach_fn);

        {
            let entry = self.context.append_basic_block(foreach_fn, "entry");
            let loop_bb = self.context.append_basic_block(foreach_fn, "loop");
            let next_bb = self.context.append_basic_block(foreach_fn, "next");
            let done_bb = self.context.append_basic_block(foreach_fn, "done");

            self.builder.position_at_end(entry);
            let list = foreach_fn.get_nth_param(0).unwrap().into_pointer_value();
            let fn_ptr = foreach_fn.get_nth_param(1).unwrap().into_pointer_value();
            let is_null = self.builder.build_is_null(list, "is_null").unwrap();
            self.builder.build_conditional_branch(is_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(loop_bb);
            let curr_phi = self.builder.build_phi(ptr_ty, "curr").unwrap();
            curr_phi.add_incoming(&[(&list, entry)]);
            let curr = curr_phi.as_basic_value().into_pointer_value();

            let val_ptr = self.builder.build_struct_gep(cons_ty, curr, 0, "val_ptr").unwrap();
            let val = self.builder.build_load(i64_ty, val_ptr, "val").unwrap();

            let fn_ty = self.context.void_type().fn_type(&[i64_ty.into()], false);
            self.builder.build_indirect_call(fn_ty, fn_ptr, &[val.into()], "").unwrap();

            let next_ptr = self.builder.build_struct_gep(cons_ty, curr, 1, "next_ptr").unwrap();
            let next = self.builder.build_load(ptr_ty, next_ptr, "next").unwrap().into_pointer_value();
            let next_null = self.builder.build_is_null(next, "next_null").unwrap();

            curr_phi.add_incoming(&[(&next, next_bb)]);
            self.builder.build_unconditional_branch(next_bb).unwrap();

            self.builder.position_at_end(next_bb);
            self.builder.build_conditional_branch(next_null, done_bb, loop_bb).unwrap();

            self.builder.position_at_end(done_bb);
            self.builder.build_return(None).unwrap();
        }
    }

    /// Try to compile a built-in pipeline function call (source, map, filter, fold, collect, etc.)
    fn try_compile_pipeline_call(
        &mut self,
        name: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        match name {
            "source" => {
                // source(data) just returns the data as-is
                if let Some(arg) = args.first() {
                    let val = self.compile_expr(arg, function)?.unwrap();
                    Ok(Some(val))
                } else {
                    Ok(None)
                }
            }
            "collect" | "collect_vec" => {
                // collect is identity on lists
                if let Some(arg) = args.first() {
                    let val = self.compile_expr(arg, function)?.unwrap();
                    Ok(Some(val))
                } else {
                    Ok(None)
                }
            }
            "count" => {
                if let Some(arg) = args.first() {
                    let list = self.compile_expr(arg, function)?.unwrap();
                    if list.is_pointer_value() {
                        let len_fn = self.functions["vibe_list_length"];
                        let result = self.builder.build_call(
                            len_fn, &[list.into()], "count",
                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        Ok(result.try_as_basic_value().left())
                    } else {
                        Ok(Some(list))
                    }
                } else {
                    Ok(None)
                }
            }
            "length" => {
                if let Some(arg) = args.first() {
                    let list = self.compile_expr(arg, function)?.unwrap();
                    if list.is_pointer_value() {
                        let len_fn = self.functions["vibe_list_length"];
                        let result = self.builder.build_call(
                            len_fn, &[list.into()], "length",
                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        Ok(result.try_as_basic_value().left())
                    } else {
                        Ok(Some(list))
                    }
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None), // Not a pipeline function
        }
    }

    /// Compile a vibe pipeline expression
    fn compile_vibe_pipeline(
        &mut self,
        source: &Expr,
        stages: &[PipelineStage],
        function: FunctionValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        // Compile the source expression
        let mut current = self.compile_expr(source, function)?.unwrap();

        let region_ptr = self.region_stack.last().copied().flatten()
            .unwrap_or(ptr_ty.const_null());

        // Apply each stage sequentially
        for stage in stages {
            match stage {
                PipelineStage::Map(func_expr) => {
                    let func_val = self.compile_expr(func_expr, function)?.unwrap();
                    if current.is_pointer_value() && func_val.is_pointer_value() {
                        let map_fn = self.functions["vibe_list_map"];
                        let result = self.builder.build_call(
                            map_fn,
                            &[current.into(), func_val.into(), region_ptr.into()],
                            "mapped",
                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        current = result.try_as_basic_value().left()
                            .unwrap_or_else(|| ptr_ty.const_null().into());
                    }
                }
                PipelineStage::Filter(pred_expr) => {
                    let pred_val = self.compile_expr(pred_expr, function)?.unwrap();
                    if current.is_pointer_value() && pred_val.is_pointer_value() {
                        let filter_fn = self.functions["vibe_list_filter"];
                        let result = self.builder.build_call(
                            filter_fn,
                            &[current.into(), pred_val.into(), region_ptr.into()],
                            "filtered",
                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        current = result.try_as_basic_value().left()
                            .unwrap_or_else(|| ptr_ty.const_null().into());
                    }
                }
                PipelineStage::Fold(init_expr, func_expr) => {
                    let init_val = self.compile_expr(init_expr, function)?.unwrap();
                    let func_val = self.compile_expr(func_expr, function)?.unwrap();
                    if current.is_pointer_value() {
                        let fold_fn = self.functions["vibe_list_fold"];
                        let init_i64 = self.ensure_i64(init_val);
                        let result = self.builder.build_call(
                            fold_fn,
                            &[current.into(), init_i64.into(), func_val.into()],
                            "folded",
                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        current = result.try_as_basic_value().left()
                            .unwrap_or_else(|| i64_ty.const_int(0, false).into());
                    }
                }
                PipelineStage::ForEach(func_expr) => {
                    let func_val = self.compile_expr(func_expr, function)?.unwrap();
                    if current.is_pointer_value() && func_val.is_pointer_value() {
                        let foreach_fn = self.functions["vibe_list_for_each"];
                        self.builder.build_call(
                            foreach_fn,
                            &[current.into(), func_val.into()],
                            "",
                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        current = i64_ty.const_int(0, false).into();
                    }
                }
                PipelineStage::Collect | PipelineStage::Distinct => {
                    // Identity — list is already collected
                }
                PipelineStage::Count => {
                    if current.is_pointer_value() {
                        let len_fn = self.functions["vibe_list_length"];
                        let result = self.builder.build_call(
                            len_fn, &[current.into()], "count",
                        ).map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        current = result.try_as_basic_value().left()
                            .unwrap_or_else(|| i64_ty.const_int(0, false).into());
                    }
                }
                PipelineStage::First => {
                    if current.is_pointer_value() {
                        let cons_ty = self.context.struct_type(
                            &[i64_ty.into(), ptr_ty.into()],
                            false,
                        );
                        let is_null = self.builder.build_is_null(current.into_pointer_value(), "is_null").unwrap();
                        let then_bb = self.context.append_basic_block(function, "first.some");
                        let else_bb = self.context.append_basic_block(function, "first.none");
                        let merge_bb = self.context.append_basic_block(function, "first.merge");

                        self.builder.build_conditional_branch(is_null, else_bb, then_bb).unwrap();

                        self.builder.position_at_end(then_bb);
                        let val_ptr = self.builder.build_struct_gep(cons_ty, current.into_pointer_value(), 0, "first_val").unwrap();
                        let val = self.builder.build_load(i64_ty, val_ptr, "first").unwrap();
                        self.builder.build_unconditional_branch(merge_bb).unwrap();

                        self.builder.position_at_end(else_bb);
                        self.builder.build_unconditional_branch(merge_bb).unwrap();

                        self.builder.position_at_end(merge_bb);
                        let phi = self.builder.build_phi(i64_ty, "first_result").unwrap();
                        phi.add_incoming(&[(&val, then_bb), (&i64_ty.const_int(0, false), else_bb)]);
                        current = phi.as_basic_value();
                    }
                }
                _ => {
                    // Other stages: pass through for now
                }
            }
        }

        Ok(Some(current))
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
