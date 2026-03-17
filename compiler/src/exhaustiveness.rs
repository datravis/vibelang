//! Exhaustiveness checking for pattern matching.
//!
//! Verifies that match expressions cover all possible values of the scrutinee type.
//! Uses a simplified algorithm based on "usefulness checking": a match is exhaustive
//! if adding a wildcard pattern would be useless (already covered).

use crate::ast::*;
use std::collections::HashMap;

/// Errors produced by exhaustiveness checking.
#[derive(Debug)]
pub struct ExhaustivenessError {
    pub message: String,
    pub span: crate::lexer::Span,
}

impl std::fmt::Display for ExhaustivenessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "non-exhaustive pattern match at line {}:{}: {}",
            self.span.line, self.span.col, self.message
        )
    }
}

/// A simplified type descriptor used during exhaustiveness analysis.
#[derive(Debug, Clone)]
enum TypeShape {
    /// A variant (sum) type with named constructors and their field counts
    Variant(Vec<(String, usize)>),
    /// Bool type (two constructors: true, false)
    Bool,
    /// Integer type (infinite domain — only exhaustive with wildcard/ident)
    Int,
    /// Float type (infinite domain)
    Float,
    /// String type (infinite domain)
    Str,
    /// Char type (large domain — treat as infinite)
    Char,
    /// Tuple with element count
    Tuple(usize),
    /// Record type
    Record(Vec<String>),
    /// Unit type (single value)
    Unit,
    /// Unknown type — be permissive
    Unknown,
}

/// Collect all type definitions from a module for exhaustiveness checking.
fn collect_type_shapes(module: &Module) -> HashMap<String, TypeShape> {
    let mut shapes = HashMap::new();

    // Built-in types
    shapes.insert(
        "Option".into(),
        TypeShape::Variant(vec![("Some".into(), 1), ("None".into(), 0)]),
    );
    shapes.insert(
        "Result".into(),
        TypeShape::Variant(vec![("Ok".into(), 1), ("Err".into(), 1)]),
    );
    shapes.insert(
        "List".into(),
        TypeShape::Variant(vec![("Cons".into(), 2), ("Nil".into(), 0)]),
    );
    shapes.insert(
        "Ordering".into(),
        TypeShape::Variant(vec![
            ("Less".into(), 0),
            ("Equal".into(), 0),
            ("Greater".into(), 0),
        ]),
    );
    shapes.insert("Bool".into(), TypeShape::Bool);

    for decl in &module.declarations {
        if let Decl::TypeDef(td) = decl {
            match &td.body {
                TypeBody::Variants(variants) => {
                    let ctors: Vec<(String, usize)> = variants
                        .iter()
                        .map(|v| (v.name.clone(), v.fields.len()))
                        .collect();
                    shapes.insert(td.name.clone(), TypeShape::Variant(ctors));
                }
                TypeBody::Record(fields) => {
                    shapes.insert(
                        td.name.clone(),
                        TypeShape::Record(fields.iter().map(|(n, _)| n.clone()).collect()),
                    );
                }
                TypeBody::Alias(_) => {}
            }
        }
    }

    shapes
}

/// Determine the type shape of a scrutinee by examining the patterns used.
fn infer_scrutinee_shape(
    arms: &[MatchArm],
    shapes: &HashMap<String, TypeShape>,
) -> TypeShape {
    for arm in arms {
        match &arm.pattern {
            Pattern::BoolLit(_, _) => return TypeShape::Bool,
            Pattern::IntLit(_, _) => return TypeShape::Int,
            Pattern::FloatLit(_, _) => return TypeShape::Float,
            Pattern::StringLit(_, _) => return TypeShape::Str,
            Pattern::CharLit(_, _) => return TypeShape::Char,
            Pattern::Constructor(name, _, _) => {
                // Find which type this constructor belongs to
                for (type_name, shape) in shapes {
                    if let TypeShape::Variant(ctors) = shape {
                        if ctors.iter().any(|(cn, _)| cn == name) {
                            return shapes.get(type_name).unwrap().clone();
                        }
                    }
                }
                return TypeShape::Unknown;
            }
            Pattern::Tuple(pats, _) => return TypeShape::Tuple(pats.len()),
            Pattern::Record(fields, _) => {
                return TypeShape::Record(fields.iter().map(|(n, _)| n.clone()).collect());
            }
            _ => {}
        }
    }
    TypeShape::Unknown
}

/// Check if a list of patterns is exhaustive for a given type shape.
#[allow(clippy::only_used_in_recursion)]
fn check_patterns_exhaustive(
    patterns: &[&Pattern],
    shape: &TypeShape,
    shapes: &HashMap<String, TypeShape>,
) -> Option<String> {
    // A wildcard or identifier pattern covers everything
    if patterns.iter().any(|p| matches!(p, Pattern::Wildcard(_) | Pattern::Ident(_, _))) {
        return None; // exhaustive
    }

    match shape {
        TypeShape::Bool => {
            let has_true = patterns.iter().any(|p| matches!(p, Pattern::BoolLit(true, _)));
            let has_false = patterns.iter().any(|p| matches!(p, Pattern::BoolLit(false, _)));
            if has_true && has_false {
                None
            } else if !has_true {
                Some("missing pattern: true".into())
            } else {
                Some("missing pattern: false".into())
            }
        }

        TypeShape::Variant(ctors) => {
            let mut missing = Vec::new();
            for (ctor_name, ctor_arity) in ctors {
                let ctor_patterns: Vec<&Pattern> = patterns
                    .iter()
                    .filter_map(|p| match p {
                        Pattern::Constructor(n, _, _) if n == ctor_name => Some(*p),
                        _ => None,
                    })
                    .collect();

                if ctor_patterns.is_empty() {
                    missing.push(ctor_name.clone());
                } else if *ctor_arity > 0 {
                    // Check sub-patterns for each field position
                    for i in 0..*ctor_arity {
                        let sub_pats: Vec<&Pattern> = ctor_patterns
                            .iter()
                            .filter_map(|p| {
                                if let Pattern::Constructor(_, fields, _) = p {
                                    fields.get(i)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        // Sub-patterns use Unknown shape (we don't track nested types)
                        // Not exhaustive at sub-pattern level, but if the constructor
                        // is at least present, we consider it covered at top level
                        // (deeper checking would require full type info)
                        let _ =
                            check_patterns_exhaustive(&sub_pats, &TypeShape::Unknown, shapes);
                    }
                }
            }
            if missing.is_empty() {
                None
            } else {
                Some(format!("missing pattern(s): {}", missing.join(", ")))
            }
        }

        TypeShape::Unit => {
            // Unit only has one value (), checked by wildcard/ident above
            Some("missing pattern for Unit value".into())
        }

        TypeShape::Tuple(n) => {
            // Tuple patterns: need at least one tuple pattern or wildcard
            let has_tuple = patterns
                .iter()
                .any(|p| matches!(p, Pattern::Tuple(_, _)));
            if has_tuple {
                // Check each position
                let sub_complete = (0..*n).all(|i| {
                    let sub_pats: Vec<&Pattern> = patterns
                        .iter()
                        .filter_map(|p| {
                            if let Pattern::Tuple(fields, _) = p {
                                fields.get(i)
                            } else {
                                None
                            }
                        })
                        .collect();
                    check_patterns_exhaustive(&sub_pats, &TypeShape::Unknown, shapes).is_none()
                });
                if sub_complete {
                    None
                } else {
                    Some("non-exhaustive tuple patterns".into())
                }
            } else {
                Some("missing tuple pattern".into())
            }
        }

        TypeShape::Record(_) => {
            // Record patterns: need wildcard/ident to be exhaustive
            let has_record = patterns
                .iter()
                .any(|p| matches!(p, Pattern::Record(_, _)));
            if has_record {
                None // At least one record pattern covers the structural shape
            } else {
                Some("missing record pattern".into())
            }
        }

        // Infinite domains: must have wildcard/ident (checked above)
        TypeShape::Int => Some("non-exhaustive integer patterns (add a wildcard `_` case)".into()),
        TypeShape::Float => {
            Some("non-exhaustive float patterns (add a wildcard `_` case)".into())
        }
        TypeShape::Str => {
            Some("non-exhaustive string patterns (add a wildcard `_` case)".into())
        }
        TypeShape::Char => {
            Some("non-exhaustive char patterns (add a wildcard `_` case)".into())
        }
        TypeShape::Unknown => {
            // Cannot determine — be permissive
            None
        }
    }
}

/// Check a single match expression for exhaustiveness.
fn check_match_exhaustive(
    arms: &[MatchArm],
    span: &crate::lexer::Span,
    shapes: &HashMap<String, TypeShape>,
) -> Option<ExhaustivenessError> {
    // No arms at all is always non-exhaustive (unless Unknown)
    if arms.is_empty() {
        return Some(ExhaustivenessError {
            message: "empty match expression".into(),
            span: *span,
        });
    }

    // Arms with guards don't guarantee coverage, but if there's a
    // guardless wildcard/ident, the match is exhaustive regardless
    let guardless_patterns: Vec<&Pattern> = arms
        .iter()
        .filter(|a| a.guard.is_none())
        .map(|a| &a.pattern)
        .collect();

    let shape = infer_scrutinee_shape(arms, shapes);

    check_patterns_exhaustive(&guardless_patterns, &shape, shapes).map(|msg| {
        ExhaustivenessError {
            message: msg,
            span: *span,
        }
    })
}

/// Recursively walk an expression tree to find all match expressions and check them.
fn check_expr(
    expr: &Expr,
    shapes: &HashMap<String, TypeShape>,
    errors: &mut Vec<ExhaustivenessError>,
) {
    match expr {
        Expr::Match(scrutinee, arms, span) => {
            check_expr(scrutinee, shapes, errors);
            if let Some(err) = check_match_exhaustive(arms, span, shapes) {
                errors.push(err);
            }
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    check_expr(guard, shapes, errors);
                }
                check_expr(&arm.body, shapes, errors);
            }
        }

        // Recurse into all sub-expressions
        Expr::IntLit(_, _)
        | Expr::FloatLit(_, _)
        | Expr::StringLit(_, _)
        | Expr::CharLit(_, _)
        | Expr::BoolLit(_, _)
        | Expr::UnitLit(_)
        | Expr::Ident(_, _)
        | Expr::TypeConstructor(_, _) => {}

        Expr::StringInterp(parts, _) => {
            for part in parts {
                if let StringPart::Expr(e) = part {
                    check_expr(e, shapes, errors);
                }
            }
        }

        Expr::List(elems, _) | Expr::Tuple(elems, _) => {
            for e in elems {
                check_expr(e, shapes, errors);
            }
        }

        Expr::Record(fields, _) => {
            for (_, e) in fields {
                check_expr(e, shapes, errors);
            }
        }

        Expr::RecordUpdate(base, fields, _) => {
            check_expr(base, shapes, errors);
            for (_, e) in fields {
                check_expr(e, shapes, errors);
            }
        }

        Expr::FieldAccess(base, _, _) => check_expr(base, shapes, errors),

        Expr::BinOp(lhs, _, rhs, _) => {
            check_expr(lhs, shapes, errors);
            check_expr(rhs, shapes, errors);
        }

        Expr::UnaryOp(_, expr, _) => check_expr(expr, shapes, errors),

        Expr::Pipe(lhs, rhs, _) => {
            check_expr(lhs, shapes, errors);
            check_expr(rhs, shapes, errors);
        }

        Expr::Call(func, args, _) => {
            check_expr(func, shapes, errors);
            for a in args {
                check_expr(a, shapes, errors);
            }
        }

        Expr::Lambda(_, body, _) => check_expr(body, shapes, errors),

        Expr::PartialApp(func, args, _) => {
            check_expr(func, shapes, errors);
            for a in args.iter().flatten() {
                check_expr(a, shapes, errors);
            }
        }

        Expr::If(cond, then_br, else_br, _) => {
            check_expr(cond, shapes, errors);
            check_expr(then_br, shapes, errors);
            if let Some(e) = else_br {
                check_expr(e, shapes, errors);
            }
        }

        Expr::When(clauses, _) => {
            for c in clauses {
                check_expr(&c.condition, shapes, errors);
                check_expr(&c.body, shapes, errors);
            }
        }

        Expr::For(_, collection, body, _) => {
            check_expr(collection, shapes, errors);
            check_expr(body, shapes, errors);
        }

        Expr::DoBlock(exprs, _) => {
            for e in exprs {
                check_expr(e, shapes, errors);
            }
        }

        Expr::Let(_, _, value, body, _) => {
            check_expr(value, shapes, errors);
            check_expr(body, shapes, errors);
        }

        Expr::LetBind(_, _, value, _) => {
            check_expr(value, shapes, errors);
        }

        Expr::LetElse(_, _, value, fallback, _) => {
            check_expr(value, shapes, errors);
            check_expr(fallback, shapes, errors);
        }

        Expr::Handle(expr, handlers, _) => {
            check_expr(expr, shapes, errors);
            for h in handlers {
                check_expr(&h.body, shapes, errors);
            }
        }

        Expr::Resume(expr, _) => check_expr(expr, shapes, errors),

        Expr::Perform(_, _, args, _) => {
            for a in args {
                check_expr(a, shapes, errors);
            }
        }

        Expr::ListComp(body, generators, filters, _) => {
            check_expr(body, shapes, errors);
            for g in generators {
                check_expr(&g.iter, shapes, errors);
            }
            for f in filters {
                check_expr(f, shapes, errors);
            }
        }

        Expr::Async(body, _) | Expr::Await(body, _) | Expr::Spawn(body, _) => {
            check_expr(body, shapes, errors);
        }

        Expr::Select(arms, _) => {
            for arm in arms {
                check_expr(&arm.channel, shapes, errors);
                check_expr(&arm.body, shapes, errors);
            }
        }

        Expr::Par(exprs, _) | Expr::Race(exprs, _) => {
            for e in exprs {
                check_expr(e, shapes, errors);
            }
        }

        Expr::Pmap(a, b, _) | Expr::Pfilter(a, b, _) => {
            check_expr(a, shapes, errors);
            check_expr(b, shapes, errors);
        }

        Expr::Preduce(a, b, c, _) => {
            check_expr(a, shapes, errors);
            check_expr(b, shapes, errors);
            check_expr(c, shapes, errors);
        }

        Expr::ChanCreate(expr, _) | Expr::ChanRecv(expr, _) => {
            check_expr(expr, shapes, errors);
        }

        Expr::ChanSend(a, b, _) | Expr::SendTo(a, b, _) | Expr::WithTimeout(a, b, _) => {
            check_expr(a, shapes, errors);
            check_expr(b, shapes, errors);
        }

        Expr::SpawnActor(expr, _) => check_expr(expr, shapes, errors),

        Expr::VibePipeline(source, stages, _) => {
            check_expr(source, shapes, errors);
            for stage in stages {
                match stage {
                    PipelineStage::Map(e)
                    | PipelineStage::Filter(e)
                    | PipelineStage::FlatMap(e)
                    | PipelineStage::FilterMap(e)
                    | PipelineStage::Take(e)
                    | PipelineStage::Drop(e)
                    | PipelineStage::TakeWhile(e)
                    | PipelineStage::DropWhile(e)
                    | PipelineStage::ForEach(e)
                    | PipelineStage::SortBy(e)
                    | PipelineStage::GroupBy(e)
                    | PipelineStage::Chunk(e)
                    | PipelineStage::Any(e)
                    | PipelineStage::All(e)
                    | PipelineStage::Reduce(e)
                    | PipelineStage::Inspect(e)
                    | PipelineStage::DistinctBy(e)
                    | PipelineStage::Zip(e)
                    | PipelineStage::MinBy(e)
                    | PipelineStage::MaxBy(e)
                    | PipelineStage::CollectMap(e)
                    | PipelineStage::Merge(e)
                    | PipelineStage::Broadcast(e) => check_expr(e, shapes, errors),
                    PipelineStage::Fold(a, b)
                    | PipelineStage::Scan(a, b)
                    | PipelineStage::Window(a, b)
                    | PipelineStage::Batch(a, b)
                    | PipelineStage::Parallel(a, b) => {
                        check_expr(a, shapes, errors);
                        check_expr(b, shapes, errors);
                    }
                    PipelineStage::Collect
                    | PipelineStage::Count
                    | PipelineStage::First
                    | PipelineStage::Last
                    | PipelineStage::Distinct
                    | PipelineStage::CollectVec
                    | PipelineStage::Sequential => {}
                }
            }
        }

        Expr::UnsafeBlock(body, _) => check_expr(body, shapes, errors),
    }
}

/// Check all match expressions in a module for exhaustiveness.
/// Returns a list of errors for non-exhaustive matches.
pub fn check_exhaustiveness(module: &Module) -> Vec<ExhaustivenessError> {
    let shapes = collect_type_shapes(module);
    let mut errors = Vec::new();

    for decl in &module.declarations {
        match decl {
            Decl::Function(f) => check_expr(&f.body, &shapes, &mut errors),
            Decl::VibeDecl(v) => check_expr(&v.body, &shapes, &mut errors),
            Decl::ImplBlock(ib) => {
                for m in &ib.methods {
                    check_expr(&m.body, &shapes, &mut errors);
                }
            }
            Decl::TestDecl(t) => check_expr(&t.body, &shapes, &mut errors),
            _ => {}
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{lex, Span};
    use crate::parser::parse;

    fn dummy_span() -> Span {
        Span {
            start: 0,
            end: 0,
            line: 1,
            col: 1,
        }
    }

    fn check_source(source: &str) -> Vec<ExhaustivenessError> {
        let tokens = lex(source).unwrap();
        let module = parse(tokens).unwrap();
        check_exhaustiveness(&module)
    }

    #[test]
    fn test_exhaustive_bool_match() {
        let errors = check_source(
            r#"
fn check_val(x: Bool) -> Int =
    match x
    | true -> 1
    | false -> 0
"#,
        );
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_non_exhaustive_bool_match() {
        let errors = check_source(
            r#"
fn check_val(x: Bool) -> Int =
    match x
    | true -> 1
"#,
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("false"));
    }

    #[test]
    fn test_exhaustive_with_wildcard() {
        let errors = check_source(
            r#"
fn check_val(x: Int) -> Int =
    match x
    | 0 -> 1
    | _ -> 2
"#,
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn test_non_exhaustive_int_match() {
        let errors = check_source(
            r#"
fn check_val(x: Int) -> Int =
    match x
    | 0 -> 1
    | 1 -> 2
"#,
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("wildcard"));
    }

    #[test]
    fn test_exhaustive_variant_match() {
        let errors = check_source(
            r#"
type Color =
    | Red
    | Green
    | Blue

fn check_val(c: Color) -> Int =
    match c
    | Red -> 1
    | Green -> 2
    | Blue -> 3
"#,
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn test_non_exhaustive_variant_match() {
        let errors = check_source(
            r#"
type Color =
    | Red
    | Green
    | Blue

fn check_val(c: Color) -> Int =
    match c
    | Red -> 1
    | Green -> 2
"#,
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Blue"));
    }

    #[test]
    fn test_exhaustive_with_ident() {
        let errors = check_source(
            r#"
fn check_val(x: Int) -> Int =
    match x
    | 0 -> 1
    | n -> n
"#,
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn test_builtin_option_exhaustive() {
        let errors = check_source(
            r#"
fn check_val(x: Int) -> Int =
    match x
    | Some(v) -> v
    | None -> 0
"#,
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn test_builtin_option_non_exhaustive() {
        let errors = check_source(
            r#"
fn check_val(x: Int) -> Int =
    match x
    | Some(v) -> v
"#,
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("None"));
    }
}
