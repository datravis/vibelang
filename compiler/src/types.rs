use crate::ast::*;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TypeError {
    #[error("undefined variable: {0}")]
    UndefinedVar(String),
    #[error("type mismatch: expected {expected}, found {found}")]
    Mismatch { expected: String, found: String },
    #[error("undefined type: {0}")]
    UndefinedType(String),
    #[error("wrong number of arguments: expected {expected}, found {found}")]
    ArityMismatch { expected: usize, found: usize },
    #[error("cannot infer type for: {0}")]
    CannotInfer(String),
    #[error("duplicate definition: {0}")]
    Duplicate(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Char,
    Unit,
    Never,
    Fn(Vec<Type>, Box<Type>),
    Tuple(Vec<Type>),
    List(Box<Type>),
    Named(String, Vec<Type>),
    Var(usize), // type variable for inference
    Unknown,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Bool => write!(f, "Bool"),
            Type::String => write!(f, "String"),
            Type::Char => write!(f, "Char"),
            Type::Unit => write!(f, "Unit"),
            Type::Never => write!(f, "Never"),
            Type::Fn(params, ret) => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            Type::Tuple(ts) => {
                write!(f, "(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
            Type::List(t) => write!(f, "List[{t}]"),
            Type::Named(name, args) => {
                write!(f, "{name}")?;
                if !args.is_empty() {
                    write!(f, "[")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{a}")?;
                    }
                    write!(f, "]")?;
                }
                Ok(())
            }
            Type::Var(id) => write!(f, "?{id}"),
            Type::Unknown => write!(f, "?"),
        }
    }
}

struct TypeChecker {
    env: Vec<HashMap<String, Type>>,
    type_defs: HashMap<String, TypeDef>,
    /// Nominal types: names that are NOT interchangeable with their inner type
    nominal_types: HashMap<String, TypeExpr>,
    /// Effect definitions: effect_name -> list of (operation_name, param_types, return_type)
    effect_defs: HashMap<String, Vec<(String, Vec<Type>, Type)>>,
    /// All effect operation names -> (effect_name, param_types, return_type)
    effect_ops: HashMap<String, (String, Vec<Type>, Type)>,
    /// Trait definitions: trait_name -> list of (method_name, param_types, return_type)
    trait_defs: HashMap<String, Vec<(String, Vec<Type>, Type)>>,
    /// Trait implementations: (trait_name, target_type) -> list of method names
    trait_impls: HashMap<(String, String), Vec<String>>,
    next_var: usize,
}

impl TypeChecker {
    fn new() -> Self {
        let mut env = HashMap::new();
        // Prelude functions
        env.insert("print".into(), Type::Fn(vec![Type::String], Box::new(Type::Unit)));
        env.insert("show".into(), Type::Fn(vec![Type::Unknown], Box::new(Type::String)));
        env.insert("to_lower".into(), Type::Fn(vec![Type::String], Box::new(Type::String)));
        env.insert(
            "split".into(),
            Type::Fn(vec![Type::String], Box::new(Type::List(Box::new(Type::String)))),
        );
        env.insert("identity".into(), Type::Fn(vec![Type::Unknown], Box::new(Type::Unknown)));
        env.insert("panic".into(), Type::Fn(vec![Type::String], Box::new(Type::Never)));
        env.insert("todo".into(), Type::Fn(vec![], Box::new(Type::Never)));
        env.insert(
            "read_lines".into(),
            Type::Fn(vec![Type::String], Box::new(Type::List(Box::new(Type::String)))),
        );
        env.insert(
            "collect".into(),
            Type::Fn(vec![Type::Unknown], Box::new(Type::Unknown)),
        );

        // List operations (used with pipe operator: list |> map(f) |> filter(p) |> fold(init, f))
        let list_t = Type::List(Box::new(Type::Unknown));
        let fn_t = Type::Fn(vec![Type::Unknown], Box::new(Type::Unknown));
        env.insert(
            "map".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(list_t.clone())),
        );
        env.insert(
            "filter".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(list_t.clone())),
        );
        env.insert(
            "fold".into(),
            Type::Fn(vec![Type::Unknown, fn_t.clone()], Box::new(Type::Unknown)),
        );
        env.insert(
            "for_each".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(Type::Unit)),
        );
        env.insert(
            "count".into(),
            Type::Fn(vec![list_t.clone()], Box::new(Type::Int)),
        );
        env.insert(
            "length".into(),
            Type::Fn(vec![list_t.clone()], Box::new(Type::Int)),
        );
        env.insert(
            "take".into(),
            Type::Fn(vec![Type::Int], Box::new(list_t.clone())),
        );
        env.insert(
            "drop".into(),
            Type::Fn(vec![Type::Int], Box::new(list_t.clone())),
        );
        env.insert(
            "take_while".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(list_t.clone())),
        );
        env.insert(
            "drop_while".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(list_t.clone())),
        );
        env.insert(
            "any".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(Type::Bool)),
        );
        env.insert(
            "all".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(Type::Bool)),
        );
        env.insert(
            "reduce".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(Type::Unknown)),
        );
        env.insert(
            "flat_map".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(list_t.clone())),
        );
        env.insert(
            "first".into(),
            Type::Fn(vec![], Box::new(Type::Unknown)),
        );
        env.insert(
            "last".into(),
            Type::Fn(vec![], Box::new(Type::Unknown)),
        );
        env.insert(
            "inspect".into(),
            Type::Fn(vec![fn_t.clone()], Box::new(list_t)),
        );

        // Channel operations
        env.insert(
            "create_channel".into(),
            Type::Fn(vec![Type::Int], Box::new(Type::Int)),
        );
        env.insert(
            "close".into(),
            Type::Fn(vec![Type::Int], Box::new(Type::Unit)),
        );

        // Actor operations
        env.insert(
            "spawn".into(),
            Type::Fn(vec![Type::Fn(vec![Type::Unknown], Box::new(Type::Unit))], Box::new(Type::Unknown)),
        );
        env.insert(
            "send_to".into(),
            Type::Fn(vec![Type::Unknown, Type::Unknown], Box::new(Type::Unit)),
        );
        env.insert(
            "receive".into(),
            Type::Fn(vec![], Box::new(Type::Unknown)),
        );

        // Timeout
        env.insert(
            "with_timeout".into(),
            Type::Fn(vec![Type::Int, Type::Unknown], Box::new(Type::Unknown)),
        );

        // Resource management
        env.insert(
            "acquire".into(),
            Type::Fn(vec![Type::Fn(vec![], Box::new(Type::Unknown)), Type::Fn(vec![Type::Unknown], Box::new(Type::Unit))], Box::new(Type::Unknown)),
        );
        env.insert(
            "release".into(),
            Type::Fn(vec![Type::Unknown], Box::new(Type::Unit)),
        );

        // Built-in trait definitions
        let mut trait_defs = HashMap::new();
        // Eq trait: fn eq(self, other) -> Bool
        trait_defs.insert("Eq".into(), vec![
            ("eq".into(), vec![Type::Unknown, Type::Unknown], Type::Bool),
            ("neq".into(), vec![Type::Unknown, Type::Unknown], Type::Bool),
        ]);
        // Ord trait: fn compare(self, other) -> Int (-1, 0, 1)
        trait_defs.insert("Ord".into(), vec![
            ("compare".into(), vec![Type::Unknown, Type::Unknown], Type::Int),
            ("lt".into(), vec![Type::Unknown, Type::Unknown], Type::Bool),
            ("lte".into(), vec![Type::Unknown, Type::Unknown], Type::Bool),
            ("gt".into(), vec![Type::Unknown, Type::Unknown], Type::Bool),
            ("gte".into(), vec![Type::Unknown, Type::Unknown], Type::Bool),
        ]);
        // Show trait: fn show(self) -> String
        trait_defs.insert("Show".into(), vec![
            ("to_string".into(), vec![Type::Unknown], Type::String),
        ]);
        // Hash trait: fn hash(self) -> Int
        trait_defs.insert("Hash".into(), vec![
            ("hash".into(), vec![Type::Unknown], Type::Int),
        ]);
        // Default trait: fn default() -> Self
        trait_defs.insert("Default".into(), vec![
            ("default".into(), vec![], Type::Unknown),
        ]);

        Self {
            env: vec![env],
            type_defs: HashMap::new(),
            nominal_types: HashMap::new(),
            effect_defs: HashMap::new(),
            effect_ops: HashMap::new(),
            trait_defs,
            trait_impls: HashMap::new(),
            next_var: 0,
        }
    }

    fn fresh_var(&mut self) -> Type {
        let v = Type::Var(self.next_var);
        self.next_var += 1;
        v
    }

    fn push_scope(&mut self) {
        self.env.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.env.pop();
    }

    fn define(&mut self, name: String, ty: Type) {
        if let Some(scope) = self.env.last_mut() {
            scope.insert(name, ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<&Type> {
        for scope in self.env.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    fn resolve_type_expr(&self, ty: &TypeExpr) -> Type {
        match ty {
            TypeExpr::Named(name, args) => {
                let resolved_args: Vec<Type> = args.iter().map(|a| self.resolve_type_expr(a)).collect();
                match name.as_str() {
                    "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "Int128" => Type::Int,
                    "UInt" | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" | "Byte" => Type::Int,
                    "Float" | "Float32" | "Float64" => Type::Float,
                    "Bool" => Type::Bool,
                    "String" => Type::String,
                    "Char" => Type::Char,
                    "Unit" => Type::Unit,
                    "Never" => Type::Never,
                    "List" | "Vec" => {
                        if let Some(inner) = resolved_args.first() {
                            Type::List(Box::new(inner.clone()))
                        } else {
                            Type::List(Box::new(Type::Unknown))
                        }
                    }
                    _ => Type::Named(name.clone(), resolved_args),
                }
            }
            TypeExpr::Function(params, ret, _effects) => {
                let param_types: Vec<Type> = params.iter().map(|p| self.resolve_type_expr(p)).collect();
                let ret_type = self.resolve_type_expr(ret);
                Type::Fn(param_types, Box::new(ret_type))
            }
            TypeExpr::Tuple(types) => {
                Type::Tuple(types.iter().map(|t| self.resolve_type_expr(t)).collect())
            }
            TypeExpr::Record(fields, _row) => {
                let field_types: Vec<Type> = fields.iter().map(|(_, t)| self.resolve_type_expr(t)).collect();
                Type::Tuple(field_types)
            }
            TypeExpr::Unit => Type::Unit,
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<Type, TypeError> {
        match expr {
            Expr::IntLit(_, _) => Ok(Type::Int),
            Expr::FloatLit(_, _) => Ok(Type::Float),
            Expr::StringLit(_, _) => Ok(Type::String),
            Expr::CharLit(_, _) => Ok(Type::Char),
            Expr::BoolLit(_, _) => Ok(Type::Bool),
            Expr::UnitLit(_) => Ok(Type::Unit),

            Expr::StringInterp(parts, _) => {
                for part in parts {
                    if let StringPart::Expr(e) = part {
                        self.check_expr(e)?;
                    }
                }
                Ok(Type::String)
            }

            Expr::Ident(name, _) => self
                .lookup(name)
                .cloned()
                .ok_or_else(|| TypeError::UndefinedVar(name.clone())),

            Expr::TypeConstructor(name, _) => {
                // Type constructors are valid expressions
                Ok(Type::Named(name.clone(), Vec::new()))
            }

            Expr::List(elems, _) => {
                if elems.is_empty() {
                    Ok(Type::List(Box::new(self.fresh_var())))
                } else {
                    let first = self.check_expr(&elems[0])?;
                    for elem in &elems[1..] {
                        self.check_expr(elem)?;
                    }
                    Ok(Type::List(Box::new(first)))
                }
            }

            Expr::Tuple(elems, _) => {
                let types: Result<Vec<_>, _> = elems.iter().map(|e| self.check_expr(e)).collect();
                Ok(Type::Tuple(types?))
            }

            Expr::Record(fields, _) => {
                let types: Result<Vec<_>, _> =
                    fields.iter().map(|(_, e)| self.check_expr(e)).collect();
                Ok(Type::Tuple(types?))
            }

            Expr::RecordUpdate(base, _fields, _) => self.check_expr(base),

            Expr::FieldAccess(base, _field, _) => {
                self.check_expr(base)?;
                Ok(Type::Unknown)
            }

            Expr::BinOp(lhs, op, rhs, _) => {
                let lt = self.check_expr(lhs)?;
                let rt = self.check_expr(rhs)?;
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        match (&lt, &rt) {
                            (Type::Int, Type::Int) => Ok(Type::Int),
                            (Type::Float, Type::Float) => Ok(Type::Float),
                            (Type::Int, Type::Float) | (Type::Float, Type::Int) => Ok(Type::Float),
                            _ => Ok(lt),
                        }
                    }
                    BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
                        Ok(Type::Bool)
                    }
                    BinOp::And | BinOp::Or => Ok(Type::Bool),
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        Ok(Type::Int)
                    }
                    BinOp::Concat => Ok(lt),
                    BinOp::Compose => {
                        // f >> g composes two functions: result is a function
                        match (&lt, &rt) {
                            (Type::Fn(params, _), Type::Fn(_, ret)) => {
                                Ok(Type::Fn(params.clone(), ret.clone()))
                            }
                            _ => Ok(Type::Unknown),
                        }
                    }
                }
            }

            Expr::UnaryOp(op, expr, _) => {
                let t = self.check_expr(expr)?;
                match op {
                    UnaryOp::Neg => Ok(t),
                    UnaryOp::Not => Ok(Type::Bool),
                    UnaryOp::BitNot => Ok(Type::Int),
                }
            }

            Expr::Pipe(lhs, rhs, _) => {
                let _input = self.check_expr(lhs)?;
                let func = self.check_expr(rhs)?;
                match func {
                    Type::Fn(_, ret) => Ok(*ret),
                    _ => Ok(Type::Unknown),
                }
            }

            Expr::Call(func, args, _) => {
                let ft = self.check_expr(func)?;
                for arg in args {
                    self.check_expr(arg)?;
                }
                match ft {
                    Type::Fn(_, ret) => Ok(*ret),
                    _ => Ok(Type::Unknown),
                }
            }

            Expr::Lambda(params, body, _) => {
                self.push_scope();
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        let t = p
                            .type_ann
                            .as_ref()
                            .map(|ta| self.resolve_type_expr(ta))
                            .unwrap_or_else(|| self.fresh_var());
                        self.define(p.name.clone(), t.clone());
                        t
                    })
                    .collect();
                let ret = self.check_expr(body)?;
                self.pop_scope();
                Ok(Type::Fn(param_types, Box::new(ret)))
            }

            Expr::If(cond, then_branch, else_branch, _) => {
                self.check_expr(cond)?;
                let then_type = self.check_expr(then_branch)?;
                if let Some(else_br) = else_branch {
                    self.check_expr(else_br)?;
                }
                Ok(then_type)
            }

            Expr::Match(scrutinee, arms, _) => {
                self.check_expr(scrutinee)?;
                let mut result_type = Type::Unknown;
                for arm in arms {
                    self.push_scope();
                    self.bind_pattern(&arm.pattern)?;
                    if let Some(guard) = &arm.guard {
                        self.check_expr(guard)?;
                    }
                    result_type = self.check_expr(&arm.body)?;
                    self.pop_scope();
                }
                Ok(result_type)
            }

            Expr::When(clauses, _) => {
                let mut result_type = Type::Unit;
                for clause in clauses {
                    self.check_expr(&clause.condition)?;
                    result_type = self.check_expr(&clause.body)?;
                }
                Ok(result_type)
            }

            Expr::DoBlock(exprs, _) => {
                self.push_scope();
                let mut last = Type::Unit;
                for expr in exprs {
                    last = self.check_expr(expr)?;
                }
                self.pop_scope();
                Ok(last)
            }

            Expr::Let(pattern, type_ann, value, body, _) => {
                let val_type = self.check_expr(value)?;
                let ty = type_ann
                    .as_ref()
                    .map(|ta| self.resolve_type_expr(ta))
                    .unwrap_or(val_type);
                self.push_scope();
                self.bind_pattern_with_type(pattern, &ty)?;
                let result = self.check_expr(body)?;
                self.pop_scope();
                Ok(result)
            }

            Expr::LetElse(pattern, type_ann, value, fallback, _) => {
                let val_type = self.check_expr(value)?;
                let ty = type_ann
                    .as_ref()
                    .map(|ta| self.resolve_type_expr(ta))
                    .unwrap_or(val_type);
                self.bind_pattern_with_type(pattern, &ty)?;
                self.check_expr(fallback)?;
                Ok(Type::Unit)
            }

            Expr::LetBind(pattern, type_ann, value, _) => {
                let val_type = self.check_expr(value)?;
                let ty = type_ann
                    .as_ref()
                    .map(|ta| self.resolve_type_expr(ta))
                    .unwrap_or(val_type);
                self.bind_pattern_with_type(pattern, &ty)?;
                Ok(Type::Unit)
            }

            Expr::Handle(expr, handlers, _) => {
                let result = self.check_expr(expr)?;
                for handler in handlers {
                    self.push_scope();
                    for param in &handler.params {
                        self.define(param.clone(), Type::Unknown);
                    }
                    self.check_expr(&handler.body)?;
                    self.pop_scope();
                }
                Ok(result)
            }

            Expr::Resume(expr, _) => self.check_expr(expr),

            Expr::Perform(_effect, _op, args, _) => {
                for arg in args {
                    self.check_expr(arg)?;
                }
                Ok(Type::Unknown)
            }

            Expr::Par(exprs, _) => {
                let mut types = Vec::new();
                for expr in exprs {
                    types.push(self.check_expr(expr)?);
                }
                Ok(Type::Tuple(types))
            }

            Expr::Pmap(collection, func, _) => {
                let col_type = self.check_expr(collection)?;
                self.check_expr(func)?;
                Ok(col_type)
            }

            Expr::Pfilter(collection, func, _) => {
                let col_type = self.check_expr(collection)?;
                self.check_expr(func)?;
                Ok(col_type)
            }

            Expr::Preduce(collection, init, func, _) => {
                self.check_expr(collection)?;
                let init_type = self.check_expr(init)?;
                self.check_expr(func)?;
                Ok(init_type)
            }

            Expr::Race(exprs, _) => {
                let mut result_type = Type::Unknown;
                for expr in exprs {
                    result_type = self.check_expr(expr)?;
                }
                Ok(result_type)
            }

            Expr::ChanCreate(capacity, _) => {
                self.check_expr(capacity)?;
                Ok(Type::Int)
            }

            Expr::ChanSend(channel, value, _) => {
                self.check_expr(channel)?;
                self.check_expr(value)?;
                Ok(Type::Unit)
            }

            Expr::ChanRecv(channel, _) => {
                self.check_expr(channel)?;
                Ok(Type::Int)
            }

            Expr::SpawnActor(handler, _) => {
                self.check_expr(handler)?;
                Ok(Type::Unknown) // returns ActorRef
            }

            Expr::SendTo(actor, message, _) => {
                self.check_expr(actor)?;
                self.check_expr(message)?;
                Ok(Type::Unit)
            }

            Expr::WithTimeout(duration, body, _) => {
                self.check_expr(duration)?;
                let body_type = self.check_expr(body)?;
                Ok(body_type) // returns Option[A] conceptually
            }

            Expr::VibePipeline(source, _stages, _) => {
                self.check_expr(source)
            }
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern) -> Result<(), TypeError> {
        self.bind_pattern_with_type(pattern, &Type::Unknown)
    }

    fn bind_pattern_with_type(&mut self, pattern: &Pattern, ty: &Type) -> Result<(), TypeError> {
        match pattern {
            Pattern::Wildcard(_) => Ok(()),
            Pattern::Ident(name, _) => {
                self.define(name.clone(), ty.clone());
                Ok(())
            }
            Pattern::IntLit(_, _)
            | Pattern::FloatLit(_, _)
            | Pattern::StringLit(_, _)
            | Pattern::BoolLit(_, _)
            | Pattern::CharLit(_, _) => Ok(()),
            Pattern::Constructor(_name, pats, _) => {
                for p in pats {
                    self.bind_pattern(p)?;
                }
                Ok(())
            }
            Pattern::Tuple(pats, _) => {
                for p in pats {
                    self.bind_pattern(p)?;
                }
                Ok(())
            }
            Pattern::Record(fields, _) => {
                for (_, p) in fields {
                    self.bind_pattern(p)?;
                }
                Ok(())
            }
        }
    }

    fn check_fn_decl(&mut self, decl: &FnDecl) -> Result<(), TypeError> {
        self.push_scope();

        let mut param_types = Vec::new();
        for p in &decl.params {
            let ty = p
                .type_ann
                .as_ref()
                .map(|ta| self.resolve_type_expr(ta))
                .unwrap_or_else(|| self.fresh_var());
            self.define(p.name.clone(), ty.clone());
            param_types.push(ty);
        }

        let body_type = self.check_expr(&decl.body)?;

        if let Some(ret_ann) = &decl.return_type {
            let _expected = self.resolve_type_expr(ret_ann);
            // In a full implementation, we'd unify body_type with expected
        }

        self.pop_scope();

        let ret = decl
            .return_type
            .as_ref()
            .map(|r| self.resolve_type_expr(r))
            .unwrap_or(body_type);

        // Register function in outer scope
        self.define(
            decl.name.clone(),
            Type::Fn(param_types, Box::new(ret)),
        );

        Ok(())
    }
}

pub fn check(module: &Module) -> Result<(), TypeError> {
    let mut checker = TypeChecker::new();

    // First pass: register all type definitions
    for decl in &module.declarations {
        match decl {
            Decl::TypeDef(td) => {
                checker.type_defs.insert(td.name.clone(), td.clone());
            }
            Decl::NewtypeDef(nt) => {
                // Register newtype as a named type wrapping its inner type
                checker.type_defs.insert(
                    nt.name.clone(),
                    TypeDef {
                        public: nt.public,
                        name: nt.name.clone(),
                        type_params: nt.type_params.clone(),
                        body: TypeBody::Alias(nt.inner_type.clone()),
                        span: nt.span,
                    },
                );
            }
            Decl::NominalDef(nd) => {
                // Register nominal type as a distinct named type (NOT an alias)
                checker.nominal_types.insert(nd.name.clone(), nd.inner_type.clone());
                checker.type_defs.insert(
                    nd.name.clone(),
                    TypeDef {
                        public: nd.public,
                        name: nd.name.clone(),
                        type_params: nd.type_params.clone(),
                        body: TypeBody::Alias(nd.inner_type.clone()),
                        span: nd.span,
                    },
                );
            }
            _ => {}
        }
    }

    // Register trait definitions and bring trait methods into scope
    for decl in &module.declarations {
        if let Decl::TraitDef(td) = decl {
            let mut methods = Vec::new();
            for m in &td.methods {
                let param_types: Vec<Type> = m
                    .params
                    .iter()
                    .map(|p| {
                        p.type_ann
                            .as_ref()
                            .map(|ta| checker.resolve_type_expr(ta))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();
                let ret = m
                    .return_type
                    .as_ref()
                    .map(|r| checker.resolve_type_expr(r))
                    .unwrap_or(Type::Unknown);
                methods.push((m.name.clone(), param_types.clone(), ret.clone()));
                // Register trait methods as callable functions
                checker.define(
                    m.name.clone(),
                    Type::Fn(param_types, Box::new(ret)),
                );
            }
            checker.trait_defs.insert(td.name.clone(), methods);
        }
    }

    // Register trait implementations
    for decl in &module.declarations {
        if let Decl::ImplBlock(ib) = decl {
            let target_name = match &ib.target {
                TypeExpr::Named(name, _) => name.clone(),
                _ => "Unknown".to_string(),
            };
            let method_names: Vec<String> = ib.methods.iter().map(|m| m.name.clone()).collect();
            checker.trait_impls.insert(
                (ib.trait_name.clone(), target_name),
                method_names,
            );
            // Register impl methods as functions (with their concrete types)
            for m in &ib.methods {
                let param_types: Vec<Type> = m
                    .params
                    .iter()
                    .map(|p| {
                        p.type_ann
                            .as_ref()
                            .map(|ta| checker.resolve_type_expr(ta))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();
                let ret = m
                    .return_type
                    .as_ref()
                    .map(|r| checker.resolve_type_expr(r))
                    .unwrap_or(Type::Unknown);
                checker.define(
                    m.name.clone(),
                    Type::Fn(param_types, Box::new(ret)),
                );
            }
        }
    }

    // Register effect definitions and bring operations into scope
    for decl in &module.declarations {
        if let Decl::EffectDef(ed) = decl {
            let mut ops = Vec::new();
            for op in &ed.operations {
                let param_types: Vec<Type> = op
                    .params
                    .iter()
                    .map(|p| {
                        p.type_ann
                            .as_ref()
                            .map(|ta| checker.resolve_type_expr(ta))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();
                let ret = op
                    .return_type
                    .as_ref()
                    .map(|r| checker.resolve_type_expr(r))
                    .unwrap_or(Type::Unknown);
                ops.push((op.name.clone(), param_types.clone(), ret.clone()));
                checker.effect_ops.insert(
                    op.name.clone(),
                    (ed.name.clone(), param_types.clone(), ret.clone()),
                );
                // Register effect operations as callable functions
                checker.define(
                    op.name.clone(),
                    Type::Fn(param_types, Box::new(ret)),
                );
            }
            checker.effect_defs.insert(ed.name.clone(), ops);
        }
    }

    // Second pass: register all function signatures
    for decl in &module.declarations {
        if let Decl::Function(f) = decl {
            let param_types: Vec<Type> = f
                .params
                .iter()
                .map(|p| {
                    p.type_ann
                        .as_ref()
                        .map(|ta| checker.resolve_type_expr(ta))
                        .unwrap_or(Type::Unknown)
                })
                .collect();
            let ret = f
                .return_type
                .as_ref()
                .map(|r| checker.resolve_type_expr(r))
                .unwrap_or(Type::Unknown);
            checker.define(f.name.clone(), Type::Fn(param_types, Box::new(ret)));
        }
    }

    // Register vibe declarations as functions
    for decl in &module.declarations {
        if let Decl::VibeDecl(v) = decl {
            let param_types: Vec<Type> = v
                .params
                .iter()
                .map(|p| {
                    p.type_ann
                        .as_ref()
                        .map(|ta| checker.resolve_type_expr(ta))
                        .unwrap_or(Type::Unknown)
                })
                .collect();
            let ret = v
                .return_type
                .as_ref()
                .map(|r| checker.resolve_type_expr(r))
                .unwrap_or(Type::Unknown);
            checker.define(v.name.clone(), Type::Fn(param_types, Box::new(ret)));
        }
    }

    // Third pass: check function bodies (including impl block methods)
    for decl in &module.declarations {
        match decl {
            Decl::Function(f) => checker.check_fn_decl(f)?,
            Decl::VibeDecl(v) => {
                // Type check vibe body like a function
                let fn_decl = FnDecl {
                    public: false,
                    name: v.name.clone(),
                    params: v.params.clone(),
                    return_type: v.return_type.clone(),
                    effects: Vec::new(),
                    body: v.body.clone(),
                    span: v.span,
                };
                checker.check_fn_decl(&fn_decl)?;
            }
            Decl::ImplBlock(ib) => {
                for method in &ib.methods {
                    checker.check_fn_decl(method)?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}
