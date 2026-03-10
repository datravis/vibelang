//! Automatic trait derivation for Show, Eq, Ord, Hash.
//! Generates synthetic ImplBlock declarations from `deriving(...)` clauses.

use crate::ast::*;
use crate::lexer::Span;

/// Errors from derivation
#[derive(Debug)]
pub enum DeriveError {
    UnsupportedTrait(String),
    CannotDerive { trait_name: String, type_name: String, reason: String },
}

impl std::fmt::Display for DeriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeriveError::UnsupportedTrait(name) =>
                write!(f, "cannot derive unknown trait: {name}"),
            DeriveError::CannotDerive { trait_name, type_name, reason } =>
                write!(f, "cannot derive {trait_name} for {type_name}: {reason}"),
        }
    }
}

impl std::error::Error for DeriveError {}

/// Supported derivable traits
const DERIVABLE_TRAITS: &[&str] = &["Show", "Eq", "Ord", "Hash", "Default"];

/// Generate ImplBlocks for all deriving clauses in a module
pub fn generate_derived_impls(module: &Module) -> Result<Vec<Decl>, DeriveError> {
    let mut generated = Vec::new();

    for decl in &module.declarations {
        if let Decl::TypeDef(td) = decl {
            for trait_name in &td.deriving {
                if !DERIVABLE_TRAITS.contains(&trait_name.as_str()) {
                    return Err(DeriveError::UnsupportedTrait(trait_name.clone()));
                }
                let impl_block = derive_trait(td, trait_name)?;
                generated.push(Decl::ImplBlock(impl_block));
            }
        }
    }

    Ok(generated)
}

fn dummy_span() -> Span {
    Span { start: 0, end: 0, line: 0, col: 0 }
}

/// Derive a single trait for a type definition
fn derive_trait(td: &TypeDef, trait_name: &str) -> Result<ImplBlock, DeriveError> {
    match trait_name {
        "Show" => derive_show(td),
        "Eq" => derive_eq(td),
        "Ord" => derive_ord(td),
        "Hash" => derive_hash(td),
        "Default" => derive_default(td),
        _ => Err(DeriveError::UnsupportedTrait(trait_name.to_string())),
    }
}

/// Derive Show: generates to_string method that produces a string representation
fn derive_show(td: &TypeDef) -> Result<ImplBlock, DeriveError> {
    let self_param = Param {
        name: "self".into(),
        type_ann: Some(TypeExpr::Named(td.name.clone(), Vec::new())),
        span: dummy_span(),
    };

    // Build a show body based on the type structure
    let body = match &td.body {
        TypeBody::Record(fields) => {
            // Generate: "TypeName { field1: <show(self.field1)>, field2: <show(self.field2)> }"
            let mut parts = vec![StringPart::Literal(format!("{} {{ ", td.name))];
            for (i, (fname, _)) in fields.iter().enumerate() {
                if i > 0 {
                    parts.push(StringPart::Literal(", ".into()));
                }
                parts.push(StringPart::Literal(format!("{fname}: ")));
                parts.push(StringPart::Expr(
                    Expr::Call(
                        Box::new(Expr::Ident("show".into(), dummy_span())),
                        vec![Expr::FieldAccess(
                            Box::new(Expr::Ident("self".into(), dummy_span())),
                            fname.clone(),
                            dummy_span(),
                        )],
                        dummy_span(),
                    ),
                ));
            }
            parts.push(StringPart::Literal(" }".into()));
            Expr::StringInterp(parts, dummy_span())
        }
        TypeBody::Variants(variants) => {
            // Generate a match expression
            let mut arms = Vec::new();
            for variant in variants {
                let pattern_vars: Vec<Pattern> = (0..variant.fields.len())
                    .map(|i| Pattern::Ident(format!("v{i}"), dummy_span()))
                    .collect();

                let body = if variant.fields.is_empty() {
                    Expr::StringLit(variant.name.clone(), dummy_span())
                } else {
                    let mut parts = vec![StringPart::Literal(format!("{}(", variant.name))];
                    for i in 0..variant.fields.len() {
                        if i > 0 {
                            parts.push(StringPart::Literal(", ".into()));
                        }
                        parts.push(StringPart::Expr(
                            Expr::Call(
                                Box::new(Expr::Ident("show".into(), dummy_span())),
                                vec![Expr::Ident(format!("v{i}"), dummy_span())],
                                dummy_span(),
                            ),
                        ));
                    }
                    parts.push(StringPart::Literal(")".into()));
                    Expr::StringInterp(parts, dummy_span())
                };

                arms.push(MatchArm {
                    pattern: Pattern::Constructor(variant.name.clone(), pattern_vars, dummy_span()),
                    guard: None,
                    body,
                });
            }
            Expr::Match(
                Box::new(Expr::Ident("self".into(), dummy_span())),
                arms,
                dummy_span(),
            )
        }
        TypeBody::Alias(_) => {
            // For aliases, delegate to show on the inner value
            Expr::Call(
                Box::new(Expr::Ident("show".into(), dummy_span())),
                vec![Expr::Ident("self".into(), dummy_span())],
                dummy_span(),
            )
        }
    };

    Ok(ImplBlock {
        trait_name: "Show".into(),
        type_params: td.type_params.clone(),
        target: TypeExpr::Named(td.name.clone(), Vec::new()),
        methods: vec![FnDecl {
            public: true,
            is_unsafe: false,
            name: "to_string".into(),
            params: vec![self_param],
            return_type: Some(TypeExpr::Named("String".into(), Vec::new())),
            effects: Vec::new(),
            body,
            span: dummy_span(),
        }],
        span: dummy_span(),
    })
}

/// Derive Eq: generates eq and neq methods
fn derive_eq(td: &TypeDef) -> Result<ImplBlock, DeriveError> {
    let self_param = Param {
        name: "self".into(),
        type_ann: Some(TypeExpr::Named(td.name.clone(), Vec::new())),
        span: dummy_span(),
    };
    let other_param = Param {
        name: "other".into(),
        type_ann: Some(TypeExpr::Named(td.name.clone(), Vec::new())),
        span: dummy_span(),
    };

    let eq_body = match &td.body {
        TypeBody::Record(fields) => {
            // All fields must be equal
            let mut expr: Option<Expr> = None;
            for (fname, _) in fields {
                let field_eq = Expr::BinOp(
                    Box::new(Expr::FieldAccess(
                        Box::new(Expr::Ident("self".into(), dummy_span())),
                        fname.clone(),
                        dummy_span(),
                    )),
                    BinOp::Eq,
                    Box::new(Expr::FieldAccess(
                        Box::new(Expr::Ident("other".into(), dummy_span())),
                        fname.clone(),
                        dummy_span(),
                    )),
                    dummy_span(),
                );
                expr = Some(match expr {
                    Some(prev) => Expr::BinOp(
                        Box::new(prev),
                        BinOp::And,
                        Box::new(field_eq),
                        dummy_span(),
                    ),
                    None => field_eq,
                });
            }
            expr.unwrap_or(Expr::BoolLit(true, dummy_span()))
        }
        TypeBody::Variants(variants) => {
            // Match on self and other, check if same variant with equal fields
            let mut arms = Vec::new();
            for variant in variants {
                let self_vars: Vec<Pattern> = (0..variant.fields.len())
                    .map(|i| Pattern::Ident(format!("a{i}"), dummy_span()))
                    .collect();
                let other_vars: Vec<Pattern> = (0..variant.fields.len())
                    .map(|i| Pattern::Ident(format!("b{i}"), dummy_span()))
                    .collect();

                let mut eq_expr: Option<Expr> = None;
                for i in 0..variant.fields.len() {
                    let field_eq = Expr::BinOp(
                        Box::new(Expr::Ident(format!("a{i}"), dummy_span())),
                        BinOp::Eq,
                        Box::new(Expr::Ident(format!("b{i}"), dummy_span())),
                        dummy_span(),
                    );
                    eq_expr = Some(match eq_expr {
                        Some(prev) => Expr::BinOp(
                            Box::new(prev), BinOp::And, Box::new(field_eq), dummy_span(),
                        ),
                        None => field_eq,
                    });
                }

                arms.push(MatchArm {
                    pattern: Pattern::Tuple(vec![
                        Pattern::Constructor(variant.name.clone(), self_vars, dummy_span()),
                        Pattern::Constructor(variant.name.clone(), other_vars, dummy_span()),
                    ], dummy_span()),
                    guard: None,
                    body: eq_expr.unwrap_or(Expr::BoolLit(true, dummy_span())),
                });
            }
            // Catch-all: different constructors are not equal
            arms.push(MatchArm {
                pattern: Pattern::Wildcard(dummy_span()),
                guard: None,
                body: Expr::BoolLit(false, dummy_span()),
            });

            Expr::Match(
                Box::new(Expr::Tuple(vec![
                    Expr::Ident("self".into(), dummy_span()),
                    Expr::Ident("other".into(), dummy_span()),
                ], dummy_span())),
                arms,
                dummy_span(),
            )
        }
        TypeBody::Alias(_) => {
            Expr::BinOp(
                Box::new(Expr::Ident("self".into(), dummy_span())),
                BinOp::Eq,
                Box::new(Expr::Ident("other".into(), dummy_span())),
                dummy_span(),
            )
        }
    };

    // neq is just !eq
    let neq_body = Expr::UnaryOp(
        UnaryOp::Not,
        Box::new(Expr::Call(
            Box::new(Expr::Ident("eq".into(), dummy_span())),
            vec![
                Expr::Ident("self".into(), dummy_span()),
                Expr::Ident("other".into(), dummy_span()),
            ],
            dummy_span(),
        )),
        dummy_span(),
    );

    Ok(ImplBlock {
        trait_name: "Eq".into(),
        type_params: td.type_params.clone(),
        target: TypeExpr::Named(td.name.clone(), Vec::new()),
        methods: vec![
            FnDecl {
                public: true,
                is_unsafe: false,
                name: "eq".into(),
                params: vec![self_param.clone(), other_param.clone()],
                return_type: Some(TypeExpr::Named("Bool".into(), Vec::new())),
                effects: Vec::new(),
                body: eq_body,
                span: dummy_span(),
            },
            FnDecl {
                public: true,
                is_unsafe: false,
                name: "neq".into(),
                params: vec![self_param, other_param],
                return_type: Some(TypeExpr::Named("Bool".into(), Vec::new())),
                effects: Vec::new(),
                body: neq_body,
                span: dummy_span(),
            },
        ],
        span: dummy_span(),
    })
}

/// Derive Ord: generates compare, lt, lte, gt, gte methods
fn derive_ord(td: &TypeDef) -> Result<ImplBlock, DeriveError> {
    let self_param = Param {
        name: "self".into(),
        type_ann: Some(TypeExpr::Named(td.name.clone(), Vec::new())),
        span: dummy_span(),
    };
    let other_param = Param {
        name: "other".into(),
        type_ann: Some(TypeExpr::Named(td.name.clone(), Vec::new())),
        span: dummy_span(),
    };

    // compare returns -1, 0, or 1
    // For simplicity, generate a basic structural comparison
    let compare_body = Expr::IntLit(0, dummy_span()); // Placeholder — full implementation would compare fields

    // Derived comparison methods
    let make_cmp_method = |name: &str, op: BinOp| -> FnDecl {
        FnDecl {
            public: true,
            is_unsafe: false,
            name: name.into(),
            params: vec![self_param.clone(), other_param.clone()],
            return_type: Some(TypeExpr::Named("Bool".into(), Vec::new())),
            effects: Vec::new(),
            body: Expr::BinOp(
                Box::new(Expr::Call(
                    Box::new(Expr::Ident("compare".into(), dummy_span())),
                    vec![
                        Expr::Ident("self".into(), dummy_span()),
                        Expr::Ident("other".into(), dummy_span()),
                    ],
                    dummy_span(),
                )),
                op,
                Box::new(Expr::IntLit(0, dummy_span())),
                dummy_span(),
            ),
            span: dummy_span(),
        }
    };

    Ok(ImplBlock {
        trait_name: "Ord".into(),
        type_params: td.type_params.clone(),
        target: TypeExpr::Named(td.name.clone(), Vec::new()),
        methods: vec![
            FnDecl {
                public: true,
                is_unsafe: false,
                name: "compare".into(),
                params: vec![self_param.clone(), other_param.clone()],
                return_type: Some(TypeExpr::Named("Int".into(), Vec::new())),
                effects: Vec::new(),
                body: compare_body,
                span: dummy_span(),
            },
            make_cmp_method("lt", BinOp::Lt),
            make_cmp_method("lte", BinOp::Lte),
            make_cmp_method("gt", BinOp::Gt),
            make_cmp_method("gte", BinOp::Gte),
        ],
        span: dummy_span(),
    })
}

/// Derive Hash: generates hash method
fn derive_hash(td: &TypeDef) -> Result<ImplBlock, DeriveError> {
    let self_param = Param {
        name: "self".into(),
        type_ann: Some(TypeExpr::Named(td.name.clone(), Vec::new())),
        span: dummy_span(),
    };

    // Simple hash: just return 0 as placeholder
    // A real implementation would combine field hashes
    let hash_body = match &td.body {
        TypeBody::Record(fields) => {
            // Hash each field and combine with XOR
            let mut expr: Option<Expr> = None;
            for (fname, _) in fields {
                let field_hash = Expr::Call(
                    Box::new(Expr::Ident("hash".into(), dummy_span())),
                    vec![Expr::FieldAccess(
                        Box::new(Expr::Ident("self".into(), dummy_span())),
                        fname.clone(),
                        dummy_span(),
                    )],
                    dummy_span(),
                );
                expr = Some(match expr {
                    Some(prev) => Expr::BinOp(
                        Box::new(prev),
                        BinOp::BitXor,
                        Box::new(field_hash),
                        dummy_span(),
                    ),
                    None => field_hash,
                });
            }
            expr.unwrap_or(Expr::IntLit(0, dummy_span()))
        }
        TypeBody::Variants(_) | TypeBody::Alias(_) => {
            Expr::IntLit(0, dummy_span())
        }
    };

    Ok(ImplBlock {
        trait_name: "Hash".into(),
        type_params: td.type_params.clone(),
        target: TypeExpr::Named(td.name.clone(), Vec::new()),
        methods: vec![FnDecl {
            public: true,
            is_unsafe: false,
            name: "hash".into(),
            params: vec![self_param],
            return_type: Some(TypeExpr::Named("Int".into(), Vec::new())),
            effects: Vec::new(),
            body: hash_body,
            span: dummy_span(),
        }],
        span: dummy_span(),
    })
}

/// Derive Default: generates default method
fn derive_default(td: &TypeDef) -> Result<ImplBlock, DeriveError> {
    let body = match &td.body {
        TypeBody::Record(fields) => {
            // Default record with default values for each field
            let field_exprs: Vec<(String, Expr)> = fields.iter()
                .map(|(name, _)| (name.clone(), Expr::Call(
                    Box::new(Expr::Ident("default".into(), dummy_span())),
                    vec![],
                    dummy_span(),
                )))
                .collect();
            Expr::Record(field_exprs, dummy_span())
        }
        TypeBody::Variants(variants) => {
            // Default to first variant with no fields
            if let Some(v) = variants.iter().find(|v| v.fields.is_empty()) {
                Expr::TypeConstructor(v.name.clone(), dummy_span())
            } else {
                return Err(DeriveError::CannotDerive {
                    trait_name: "Default".into(),
                    type_name: td.name.clone(),
                    reason: "no nullary constructor".into(),
                });
            }
        }
        TypeBody::Alias(_) => {
            Expr::Call(
                Box::new(Expr::Ident("default".into(), dummy_span())),
                vec![],
                dummy_span(),
            )
        }
    };

    Ok(ImplBlock {
        trait_name: "Default".into(),
        type_params: td.type_params.clone(),
        target: TypeExpr::Named(td.name.clone(), Vec::new()),
        methods: vec![FnDecl {
            public: true,
            is_unsafe: false,
            name: "default".into(),
            params: vec![],
            return_type: Some(TypeExpr::Named(td.name.clone(), Vec::new())),
            effects: Vec::new(),
            body,
            span: dummy_span(),
        }],
        span: dummy_span(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_span() -> Span {
        Span { start: 0, end: 0, line: 1, col: 1 }
    }

    #[test]
    fn test_derive_show_record() {
        let td = TypeDef {
            public: true,
            name: "Point".into(),
            type_params: vec![],
            body: TypeBody::Record(vec![
                ("x".into(), TypeExpr::Named("Int".into(), vec![])),
                ("y".into(), TypeExpr::Named("Int".into(), vec![])),
            ]),
            deriving: vec!["Show".into()],
            span: dummy_span(),
        };

        let result = derive_show(&td);
        assert!(result.is_ok());
        let impl_block = result.unwrap();
        assert_eq!(impl_block.trait_name, "Show");
        assert_eq!(impl_block.methods.len(), 1);
        assert_eq!(impl_block.methods[0].name, "to_string");
    }

    #[test]
    fn test_derive_eq_variants() {
        let td = TypeDef {
            public: true,
            name: "Color".into(),
            type_params: vec![],
            body: TypeBody::Variants(vec![
                Variant { name: "Red".into(), fields: vec![] },
                Variant { name: "Green".into(), fields: vec![] },
                Variant { name: "Blue".into(), fields: vec![] },
            ]),
            deriving: vec!["Eq".into()],
            span: dummy_span(),
        };

        let result = derive_eq(&td);
        assert!(result.is_ok());
        let impl_block = result.unwrap();
        assert_eq!(impl_block.trait_name, "Eq");
        assert_eq!(impl_block.methods.len(), 2); // eq and neq
    }

    #[test]
    fn test_unsupported_trait() {
        let td = TypeDef {
            public: true,
            name: "Foo".into(),
            type_params: vec![],
            body: TypeBody::Record(vec![]),
            deriving: vec!["Serialize".into()],
            span: dummy_span(),
        };

        let module = Module {
            name: vec!["test".into()],
            imports: vec![],
            declarations: vec![Decl::TypeDef(td)],
        };

        let result = generate_derived_impls(&module);
        assert!(result.is_err());
    }
}
