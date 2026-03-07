use crate::ast::*;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module as LLVMModule;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, PointerValue};
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

    fn declare_external_functions(&mut self) {
        // printf for IO
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let printf_type = self.context.i32_type().fn_type(
            &[BasicMetadataTypeEnum::PointerType(i8_ptr)],
            true,
        );
        let printf = self.llvm_module.add_function("printf", printf_type, None);
        self.functions.insert("printf".into(), printf);

        // puts
        let puts_type = self.context.i32_type().fn_type(
            &[BasicMetadataTypeEnum::PointerType(i8_ptr)],
            false,
        );
        let puts = self.llvm_module.add_function("puts", puts_type, None);
        self.functions.insert("puts".into(), puts);
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

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        self.push_scope();

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

            Expr::TypeConstructor(name, _) => {
                // For now, represent constructors as integer tags
                let tag = name.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
                let val = self.context.i64_type().const_int(tag, false);
                Ok(Some(val.into()))
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

                // Simple integer matching for now
                let merge_bb = self.context.append_basic_block(function, "match.end");
                let default_bb = self.context.append_basic_block(function, "match.default");

                // Build switch for integer patterns, cascade of ifs otherwise
                if scrut_val.is_int_value() {
                    let switch_cases: Vec<_> = arms
                        .iter()
                        .filter_map(|arm| {
                            if let Pattern::IntLit(n, _) = &arm.pattern {
                                let bb = self.context.append_basic_block(function, "match.arm");
                                Some((*n, bb, arm))
                            } else {
                                None
                            }
                        })
                        .collect();

                    // Find default arm (wildcard or ident pattern)
                    let default_arm = arms.iter().find(|a| {
                        matches!(&a.pattern, Pattern::Wildcard(_) | Pattern::Ident(_, _))
                    });

                    let actual_default = if default_arm.is_some() {
                        default_bb
                    } else {
                        default_bb
                    };

                    let switch = self.builder.build_switch(
                        scrut_val.into_int_value(),
                        actual_default,
                        &switch_cases
                            .iter()
                            .map(|(n, bb, _)| {
                                (self.context.i64_type().const_int(*n as u64, true), *bb)
                            })
                            .collect::<Vec<_>>(),
                    ).map_err(|e| CodegenError::Llvm(e.to_string()))?;

                    let mut phi_incoming: Vec<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();

                    for (_, bb, arm) in &switch_cases {
                        self.builder.position_at_end(*bb);
                        self.push_scope();
                        let val = self.compile_expr(&arm.body, function)?
                            .unwrap_or_else(|| self.context.i64_type().const_int(0, false).into());
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
                            .unwrap_or_else(|| self.context.i64_type().const_int(0, false).into());
                        self.pop_scope();
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        phi_incoming.push((val, self.builder.get_insert_block().unwrap()));
                    } else {
                        let zero = self.context.i64_type().const_int(0, false);
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                        phi_incoming.push((zero.into(), self.builder.get_insert_block().unwrap()));
                    }

                    self.builder.position_at_end(merge_bb);
                    if phi_incoming.is_empty() {
                        return Ok(Some(self.context.i64_type().const_int(0, false).into()));
                    }
                    let phi = self.builder.build_phi(self.context.i64_type(), "match.result")
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    for (val, bb) in &phi_incoming {
                        let i64_val = self.ensure_i64(*val);
                        phi.add_incoming(&[(&i64_val, *bb)]);
                    }
                    Ok(Some(phi.as_basic_value()))
                } else {
                    // Fall back: evaluate first arm
                    self.builder.position_at_end(default_bb);
                    self.builder.build_unconditional_branch(merge_bb)
                        .map_err(|e| CodegenError::Llvm(e.to_string()))?;
                    self.builder.position_at_end(merge_bb);

                    if let Some(arm) = arms.first() {
                        self.push_scope();
                        let val = self.compile_expr(&arm.body, function)?;
                        self.pop_scope();
                        Ok(val)
                    } else {
                        Ok(Some(self.context.i64_type().const_int(0, false).into()))
                    }
                }
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
                // For now, represent lists as first element or 0
                if let Some(first) = elems.first() {
                    self.compile_expr(first, function)
                } else {
                    Ok(Some(self.context.i64_type().const_int(0, false).into()))
                }
            }

            Expr::Tuple(elems, _) => {
                // Compile all elements, return first for now
                for elem in elems {
                    self.compile_expr(elem, function)?;
                }
                if let Some(first) = elems.first() {
                    self.compile_expr(first, function)
                } else {
                    Ok(Some(self.context.i64_type().const_int(0, false).into()))
                }
            }

            Expr::Record(fields, _) => {
                for (_, expr) in fields {
                    self.compile_expr(expr, function)?;
                }
                Ok(Some(self.context.i64_type().const_int(0, false).into()))
            }

            Expr::RecordUpdate(base, _, _) => self.compile_expr(base, function),

            Expr::FieldAccess(base, _, _) => self.compile_expr(base, function),

            Expr::Handle(expr, _, _) => self.compile_expr(expr, function),

            Expr::Resume(expr, _) => self.compile_expr(expr, function),
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
            _ => {} // More complex patterns would go here
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
                _ => LLVMType::I64,
            },
            TypeExpr::Unit => LLVMType::I64,
            TypeExpr::Tuple(_) => LLVMType::I64,
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
