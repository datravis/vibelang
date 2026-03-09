//! Memory management for VibeLang: regions, reference counting, and escape analysis.
//!
//! # Memory Strategy (per spec section 6)
//!
//! 1. **Stack allocation** — values that don't escape their defining scope
//! 2. **Region inference** — heap values assigned to a region (arena) that is freed
//!    when execution leaves the scope. No individual frees needed.
//! 3. **Reference counting** — values that escape their region get a refcount header.
//!    Compiler inserts retain/release calls automatically.
//!
//! The escape analysis pass determines which strategy applies to each allocation.

use crate::ast::*;
use std::collections::{HashMap, HashSet};

/// Allocation strategy for a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocStrategy {
    /// Value lives on the stack (doesn't escape its scope).
    Stack,
    /// Value lives in a region (escapes its let-binding but not the function).
    Region,
    /// Value is reference-counted (escapes the function / returned / stored in closures).
    RefCounted,
}

/// Result of escape analysis for a function.
#[derive(Debug, Clone)]
pub struct EscapeInfo {
    /// For each let-binding name, the allocation strategy.
    pub strategies: HashMap<String, AllocStrategy>,
    /// Variables that escape the function (returned or captured).
    pub escaping_vars: HashSet<String>,
}

/// Analyze a function body to determine allocation strategies.
pub fn analyze_function(decl: &FnDecl) -> EscapeInfo {
    let mut analyzer = EscapeAnalyzer::new();
    analyzer.analyze_expr(&decl.body, /* is_tail= */ true);

    let mut strategies = HashMap::new();
    for (name, info) in &analyzer.bindings {
        let strategy = if info.escapes_function {
            AllocStrategy::RefCounted
        } else if info.escapes_scope {
            AllocStrategy::Region
        } else {
            AllocStrategy::Stack
        };
        strategies.insert(name.clone(), strategy);
    }

    let escaping_vars: HashSet<String> = analyzer
        .bindings
        .iter()
        .filter(|(_, info)| info.escapes_function)
        .map(|(name, _)| name.clone())
        .collect();

    EscapeInfo {
        strategies,
        escaping_vars,
    }
}

#[derive(Debug, Clone)]
struct BindingInfo {
    /// Does this binding escape its defining let-scope?
    escapes_scope: bool,
    /// Does this binding escape the function (returned, captured in closure)?
    escapes_function: bool,
    /// Scope depth where this binding was defined.
    def_depth: usize,
}

struct EscapeAnalyzer {
    bindings: HashMap<String, BindingInfo>,
    scope_depth: usize,
}

impl EscapeAnalyzer {
    fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            scope_depth: 0,
        }
    }

    fn define(&mut self, name: &str) {
        self.bindings.insert(
            name.to_string(),
            BindingInfo {
                escapes_scope: false,
                escapes_function: false,
                def_depth: self.scope_depth,
            },
        );
    }

    fn mark_used(&mut self, name: &str) {
        if let Some(info) = self.bindings.get_mut(name) {
            if self.scope_depth > info.def_depth {
                info.escapes_scope = true;
            }
        }
    }

    fn mark_escapes_function(&mut self, name: &str) {
        if let Some(info) = self.bindings.get_mut(name) {
            info.escapes_function = true;
            info.escapes_scope = true;
        }
    }

    fn analyze_expr(&mut self, expr: &Expr, is_tail: bool) {
        match expr {
            Expr::IntLit(_, _)
            | Expr::FloatLit(_, _)
            | Expr::StringLit(_, _)
            | Expr::CharLit(_, _)
            | Expr::BoolLit(_, _)
            | Expr::UnitLit(_) => {}

            Expr::StringInterp(parts, _) => {
                for part in parts {
                    if let crate::ast::StringPart::Expr(e) = part {
                        self.analyze_expr(e, false);
                    }
                }
            }

            Expr::Ident(name, _) => {
                self.mark_used(name);
                if is_tail {
                    self.mark_escapes_function(name);
                }
            }

            Expr::TypeConstructor(_, _) => {}

            Expr::List(elems, _) => {
                for e in elems {
                    // Elements stored in a list escape to wherever the list goes
                    self.analyze_expr(e, is_tail);
                }
            }

            Expr::Tuple(elems, _) => {
                for e in elems {
                    self.analyze_expr(e, is_tail);
                }
            }

            Expr::Record(fields, _) => {
                for (_, e) in fields {
                    self.analyze_expr(e, is_tail);
                }
            }

            Expr::RecordUpdate(base, fields, _) => {
                self.analyze_expr(base, is_tail);
                for (_, e) in fields {
                    self.analyze_expr(e, is_tail);
                }
            }

            Expr::FieldAccess(base, _, _) => {
                self.analyze_expr(base, false);
            }

            Expr::BinOp(lhs, _, rhs, _) => {
                self.analyze_expr(lhs, false);
                self.analyze_expr(rhs, false);
            }

            Expr::UnaryOp(_, inner, _) => {
                self.analyze_expr(inner, false);
            }

            Expr::Pipe(lhs, rhs, _) => {
                self.analyze_expr(lhs, false);
                self.analyze_expr(rhs, is_tail);
            }

            Expr::Call(func, args, _) => {
                self.analyze_expr(func, false);
                for a in args {
                    // Arguments to function calls may escape
                    self.analyze_expr(a, false);
                }
            }

            Expr::Lambda(params, body, _) => {
                // Variables captured by lambdas escape the function
                self.scope_depth += 1;
                for p in params {
                    self.define(&p.name);
                }
                self.analyze_expr(body, true);
                self.scope_depth -= 1;
            }

            Expr::If(cond, then_br, else_br, _) => {
                self.analyze_expr(cond, false);
                self.analyze_expr(then_br, is_tail);
                if let Some(e) = else_br {
                    self.analyze_expr(e, is_tail);
                }
            }

            Expr::Match(scrutinee, arms, _) => {
                self.analyze_expr(scrutinee, false);
                for arm in arms {
                    self.scope_depth += 1;
                    self.bind_pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.analyze_expr(guard, false);
                    }
                    self.analyze_expr(&arm.body, is_tail);
                    self.scope_depth -= 1;
                }
            }

            Expr::When(clauses, _) => {
                for clause in clauses {
                    self.analyze_expr(&clause.condition, false);
                    self.analyze_expr(&clause.body, is_tail);
                }
            }

            Expr::DoBlock(exprs, _) => {
                self.scope_depth += 1;
                for (i, e) in exprs.iter().enumerate() {
                    let is_last = i == exprs.len() - 1;
                    self.analyze_expr(e, is_last && is_tail);
                }
                self.scope_depth -= 1;
            }

            Expr::Let(pattern, _, value, body, _) => {
                self.analyze_expr(value, false);
                self.scope_depth += 1;
                self.bind_pattern(pattern);
                self.analyze_expr(body, is_tail);
                self.scope_depth -= 1;
            }

            Expr::LetBind(pattern, _, value, _) => {
                self.analyze_expr(value, false);
                self.bind_pattern(pattern);
            }

            Expr::Handle(expr, handlers, _) => {
                self.analyze_expr(expr, is_tail);
                for handler in handlers {
                    self.scope_depth += 1;
                    for param in &handler.params {
                        self.define(param);
                    }
                    self.analyze_expr(&handler.body, false);
                    self.scope_depth -= 1;
                }
            }

            Expr::Resume(expr, _) => {
                self.analyze_expr(expr, is_tail);
            }

            Expr::Perform(_, _, args, _) => {
                for a in args {
                    self.analyze_expr(a, false);
                }
            }

            Expr::Par(exprs, _) | Expr::Race(exprs, _) => {
                for e in exprs {
                    self.analyze_expr(e, false);
                }
            }

            Expr::Pmap(collection, func, _) | Expr::Pfilter(collection, func, _) => {
                self.analyze_expr(collection, false);
                self.analyze_expr(func, false);
            }

            Expr::Preduce(collection, init, func, _) => {
                self.analyze_expr(collection, false);
                self.analyze_expr(init, false);
                self.analyze_expr(func, false);
            }

            Expr::ChanCreate(cap, _) => {
                self.analyze_expr(cap, false);
            }

            Expr::ChanSend(ch, val, _) => {
                self.analyze_expr(ch, false);
                self.analyze_expr(val, false);
            }

            Expr::ChanRecv(ch, _) => {
                self.analyze_expr(ch, false);
            }

            Expr::VibePipeline(source, stages, _) => {
                self.analyze_expr(source, false);
                for stage in stages {
                    match stage {
                        crate::ast::PipelineStage::Map(f) |
                        crate::ast::PipelineStage::Filter(f) |
                        crate::ast::PipelineStage::FlatMap(f) |
                        crate::ast::PipelineStage::FilterMap(f) |
                        crate::ast::PipelineStage::TakeWhile(f) |
                        crate::ast::PipelineStage::DropWhile(f) |
                        crate::ast::PipelineStage::ForEach(f) |
                        crate::ast::PipelineStage::SortBy(f) |
                        crate::ast::PipelineStage::GroupBy(f) |
                        crate::ast::PipelineStage::Chunk(f) |
                        crate::ast::PipelineStage::Take(f) |
                        crate::ast::PipelineStage::Drop(f) |
                        crate::ast::PipelineStage::Any(f) |
                        crate::ast::PipelineStage::All(f) |
                        crate::ast::PipelineStage::Reduce(f) |
                        crate::ast::PipelineStage::Inspect(f) => {
                            self.analyze_expr(f, false);
                        }
                        crate::ast::PipelineStage::Fold(init, f) |
                        crate::ast::PipelineStage::Scan(init, f) => {
                            self.analyze_expr(init, false);
                            self.analyze_expr(f, false);
                        }
                        crate::ast::PipelineStage::Collect |
                        crate::ast::PipelineStage::Count |
                        crate::ast::PipelineStage::First |
                        crate::ast::PipelineStage::Last |
                        crate::ast::PipelineStage::Distinct => {}
                    }
                }
            }
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Ident(name, _) => self.define(name),
            Pattern::Constructor(_, pats, _) => {
                for p in pats {
                    self.bind_pattern(p);
                }
            }
            Pattern::Tuple(pats, _) => {
                for p in pats {
                    self.bind_pattern(p);
                }
            }
            Pattern::Record(fields, _) => {
                for (_, p) in fields {
                    self.bind_pattern(p);
                }
            }
            _ => {}
        }
    }
}

/// Memory layout constants.
///
/// Every heap-allocated VibeLang value has a header:
/// ```text
///   +--------+--------+--------+
///   | refcount (i64)  | tag (i64) | region_id (i64) |
///   +--------+--------+--------+
///   | ... payload ...                                |
///   +------------------------------------------------+
/// ```
///
/// - refcount: reference count (0 = region-managed, >0 = refcounted)
/// - tag: type tag for variant discrimination
/// - region_id: which region owns this allocation (0 = refcounted/global)
pub const HEADER_SIZE: usize = 3; // 3 i64 fields
pub const HEADER_REFCOUNT_IDX: u32 = 0;
pub const HEADER_TAG_IDX: u32 = 1;
pub const HEADER_REGION_IDX: u32 = 2;
