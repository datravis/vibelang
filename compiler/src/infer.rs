//! Hindley-Milner type inference with unification, occurs check, and let-polymorphism.
//! Also includes row polymorphism for extensible records.

use std::collections::HashMap;

/// Type representation for inference — separate from the checked Type enum
/// to support richer type variables and row types.
#[derive(Debug, Clone, PartialEq)]
pub enum InferType {
    /// Primitive types
    Int,
    Float,
    Bool,
    Str,
    Char,
    Unit,
    Never,
    /// Fixed-width integer types
    Int8,
    Int16,
    Int32,
    Int64,
    Int128,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UInt128,
    /// Fixed-width float
    Float32,
    Float64,
    /// Function type
    Fn(Vec<InferType>, Box<InferType>),
    /// Tuple type
    Tuple(Vec<InferType>),
    /// List type
    List(Box<InferType>),
    /// Named/user-defined type with type arguments
    Named(String, Vec<InferType>),
    /// Type variable (for unification)
    Var(usize),
    /// Record type with row polymorphism
    /// Fields are known fields, Option<usize> is the row variable for extensibility
    Record(Vec<(String, InferType)>, Option<usize>),
}

impl std::fmt::Display for InferType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InferType::Int => write!(f, "Int"),
            InferType::Float => write!(f, "Float"),
            InferType::Bool => write!(f, "Bool"),
            InferType::Str => write!(f, "String"),
            InferType::Char => write!(f, "Char"),
            InferType::Unit => write!(f, "Unit"),
            InferType::Never => write!(f, "Never"),
            InferType::Int8 => write!(f, "Int8"),
            InferType::Int16 => write!(f, "Int16"),
            InferType::Int32 => write!(f, "Int32"),
            InferType::Int64 => write!(f, "Int64"),
            InferType::Int128 => write!(f, "Int128"),
            InferType::UInt8 => write!(f, "UInt8"),
            InferType::UInt16 => write!(f, "UInt16"),
            InferType::UInt32 => write!(f, "UInt32"),
            InferType::UInt64 => write!(f, "UInt64"),
            InferType::UInt128 => write!(f, "UInt128"),
            InferType::Float32 => write!(f, "Float32"),
            InferType::Float64 => write!(f, "Float64"),
            InferType::Fn(params, ret) => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            InferType::Tuple(ts) => {
                write!(f, "(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
            InferType::List(t) => write!(f, "List[{t}]"),
            InferType::Named(name, args) => {
                write!(f, "{name}")?;
                if !args.is_empty() {
                    write!(f, "[")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 { write!(f, ", ")?; }
                        write!(f, "{a}")?;
                    }
                    write!(f, "]")?;
                }
                Ok(())
            }
            InferType::Var(id) => write!(f, "?{id}"),
            InferType::Record(fields, row) => {
                write!(f, "{{ ")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{name}: {ty}")?;
                }
                if let Some(r) = row {
                    write!(f, " | ?{r}")?;
                }
                write!(f, " }}")
            }
        }
    }
}

/// Unification error
#[derive(Debug)]
pub enum UnifyError {
    /// Two types cannot be unified
    Mismatch(InferType, InferType),
    /// Occurs check failed (infinite type)
    OccursCheck(usize, InferType),
    /// Row types have incompatible fields
    RowMismatch(String, InferType, InferType),
    /// Missing field in record
    MissingField(String),
}

impl std::fmt::Display for UnifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnifyError::Mismatch(a, b) => write!(f, "type mismatch: {a} vs {b}"),
            UnifyError::OccursCheck(v, t) => write!(f, "infinite type: ?{v} occurs in {t}"),
            UnifyError::RowMismatch(field, a, b) =>
                write!(f, "field '{field}' type mismatch: {a} vs {b}"),
            UnifyError::MissingField(name) => write!(f, "missing field: {name}"),
        }
    }
}

impl std::error::Error for UnifyError {}

/// The substitution: a mapping from type variable IDs to their resolved types.
#[derive(Debug, Default)]
pub struct Substitution {
    bindings: HashMap<usize, InferType>,
}

impl Substitution {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a type variable, following chains
    pub fn resolve(&self, var: usize) -> Option<&InferType> {
        self.bindings.get(&var)
    }

    /// Bind a type variable to a type
    pub fn bind(&mut self, var: usize, ty: InferType) {
        self.bindings.insert(var, ty);
    }

    /// Apply substitution to a type, resolving all known variables
    pub fn apply(&self, ty: &InferType) -> InferType {
        match ty {
            InferType::Var(id) => {
                if let Some(resolved) = self.bindings.get(id) {
                    // Follow chains
                    self.apply(resolved)
                } else {
                    ty.clone()
                }
            }
            InferType::Fn(params, ret) => {
                InferType::Fn(
                    params.iter().map(|p| self.apply(p)).collect(),
                    Box::new(self.apply(ret)),
                )
            }
            InferType::Tuple(ts) => {
                InferType::Tuple(ts.iter().map(|t| self.apply(t)).collect())
            }
            InferType::List(t) => InferType::List(Box::new(self.apply(t))),
            InferType::Named(name, args) => {
                InferType::Named(name.clone(), args.iter().map(|a| self.apply(a)).collect())
            }
            InferType::Record(fields, row) => {
                let resolved_fields: Vec<_> = fields.iter()
                    .map(|(name, ty)| (name.clone(), self.apply(ty)))
                    .collect();
                let resolved_row = row.and_then(|r| {
                    if let Some(InferType::Record(_, inner_row)) = self.bindings.get(&r) {
                        *inner_row
                    } else if self.bindings.contains_key(&r) {
                        None // Row resolved to a concrete type
                    } else {
                        Some(r) // Still unresolved
                    }
                });
                InferType::Record(resolved_fields, resolved_row)
            }
            // Primitive types pass through
            _ => ty.clone(),
        }
    }
}

/// Type inference engine using Hindley-Milner algorithm
pub struct InferEngine {
    /// Next fresh type variable ID
    next_var: usize,
    /// The substitution (unification results)
    pub subst: Substitution,
}

impl Default for InferEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl InferEngine {
    pub fn new() -> Self {
        Self {
            next_var: 0,
            subst: Substitution::new(),
        }
    }

    /// Generate a fresh type variable
    pub fn fresh_var(&mut self) -> InferType {
        let v = InferType::Var(self.next_var);
        self.next_var += 1;
        v
    }

    /// Generate a fresh row variable (for record types)
    pub fn fresh_row_var(&mut self) -> usize {
        let v = self.next_var;
        self.next_var += 1;
        v
    }

    /// Occurs check: does variable `var` occur in `ty`?
    fn occurs_in(&self, var: usize, ty: &InferType) -> bool {
        match ty {
            InferType::Var(id) => {
                if *id == var {
                    return true;
                }
                // Follow substitution chain
                if let Some(resolved) = self.subst.resolve(*id) {
                    self.occurs_in(var, resolved)
                } else {
                    false
                }
            }
            InferType::Fn(params, ret) => {
                params.iter().any(|p| self.occurs_in(var, p)) || self.occurs_in(var, ret)
            }
            InferType::Tuple(ts) => ts.iter().any(|t| self.occurs_in(var, t)),
            InferType::List(t) => self.occurs_in(var, t),
            InferType::Named(_, args) => args.iter().any(|a| self.occurs_in(var, a)),
            InferType::Record(fields, row) => {
                fields.iter().any(|(_, t)| self.occurs_in(var, t))
                    || row.is_some_and(|r| r == var)
            }
            _ => false,
        }
    }

    /// Unify two types, updating the substitution
    pub fn unify(&mut self, a: &InferType, b: &InferType) -> Result<(), UnifyError> {
        let a = self.subst.apply(a);
        let b = self.subst.apply(b);

        if a == b {
            return Ok(());
        }

        match (&a, &b) {
            // Type variable unification
            (InferType::Var(id), _) => {
                if self.occurs_in(*id, &b) {
                    return Err(UnifyError::OccursCheck(*id, b));
                }
                self.subst.bind(*id, b);
                Ok(())
            }
            (_, InferType::Var(id)) => {
                if self.occurs_in(*id, &a) {
                    return Err(UnifyError::OccursCheck(*id, a));
                }
                self.subst.bind(*id, a);
                Ok(())
            }

            // Function types
            (InferType::Fn(params_a, ret_a), InferType::Fn(params_b, ret_b)) => {
                if params_a.len() != params_b.len() {
                    return Err(UnifyError::Mismatch(a.clone(), b.clone()));
                }
                for (pa, pb) in params_a.iter().zip(params_b.iter()) {
                    self.unify(pa, pb)?;
                }
                self.unify(ret_a, ret_b)
            }

            // Tuple types
            (InferType::Tuple(ts_a), InferType::Tuple(ts_b)) => {
                if ts_a.len() != ts_b.len() {
                    return Err(UnifyError::Mismatch(a.clone(), b.clone()));
                }
                for (ta, tb) in ts_a.iter().zip(ts_b.iter()) {
                    self.unify(ta, tb)?;
                }
                Ok(())
            }

            // List types
            (InferType::List(inner_a), InferType::List(inner_b)) => {
                self.unify(inner_a, inner_b)
            }

            // Named types
            (InferType::Named(name_a, args_a), InferType::Named(name_b, args_b)) => {
                if name_a != name_b || args_a.len() != args_b.len() {
                    return Err(UnifyError::Mismatch(a.clone(), b.clone()));
                }
                for (aa, ab) in args_a.iter().zip(args_b.iter()) {
                    self.unify(aa, ab)?;
                }
                Ok(())
            }

            // Record types with row polymorphism
            (InferType::Record(fields_a, row_a), InferType::Record(fields_b, row_b)) => {
                self.unify_records(fields_a, *row_a, fields_b, *row_b)
            }

            // Int subtypes unify with Int
            (InferType::Int, InferType::Int8 | InferType::Int16 | InferType::Int32
                | InferType::Int64 | InferType::Int128) => Ok(()),
            (InferType::Int8 | InferType::Int16 | InferType::Int32
                | InferType::Int64 | InferType::Int128, InferType::Int) => Ok(()),

            // Float subtypes unify with Float
            (InferType::Float, InferType::Float32 | InferType::Float64) => Ok(()),
            (InferType::Float32 | InferType::Float64, InferType::Float) => Ok(()),

            // Never unifies with anything (bottom type)
            (InferType::Never, _) | (_, InferType::Never) => Ok(()),

            _ => Err(UnifyError::Mismatch(a, b)),
        }
    }

    /// Unify two record types with row polymorphism
    fn unify_records(
        &mut self,
        fields_a: &[(String, InferType)],
        row_a: Option<usize>,
        fields_b: &[(String, InferType)],
        row_b: Option<usize>,
    ) -> Result<(), UnifyError> {
        // Find common fields and unify their types
        let names_a: HashMap<&str, &InferType> = fields_a.iter()
            .map(|(n, t)| (n.as_str(), t))
            .collect();
        let names_b: HashMap<&str, &InferType> = fields_b.iter()
            .map(|(n, t)| (n.as_str(), t))
            .collect();

        // Unify common fields
        for (name, ty_a) in &names_a {
            if let Some(ty_b) = names_b.get(name) {
                self.unify(ty_a, ty_b).map_err(|_| {
                    UnifyError::RowMismatch(name.to_string(), (*ty_a).clone(), (*ty_b).clone())
                })?;
            }
        }

        // Fields only in A
        let only_in_a: Vec<(String, InferType)> = fields_a.iter()
            .filter(|(n, _)| !names_b.contains_key(n.as_str()))
            .cloned()
            .collect();

        // Fields only in B
        let only_in_b: Vec<(String, InferType)> = fields_b.iter()
            .filter(|(n, _)| !names_a.contains_key(n.as_str()))
            .cloned()
            .collect();

        match (row_a, row_b) {
            // Both open: unify row variables to capture extra fields
            (Some(ra), Some(rb)) => {
                let a_empty = only_in_a.is_empty();
                let b_empty = only_in_b.is_empty();
                if !b_empty {
                    let new_row = self.fresh_row_var();
                    self.subst.bind(ra, InferType::Record(only_in_b, Some(new_row)));
                }
                if !a_empty {
                    let new_row = self.fresh_row_var();
                    self.subst.bind(rb, InferType::Record(only_in_a, Some(new_row)));
                }
                if a_empty && b_empty {
                    // Unify the row variables
                    self.unify(&InferType::Var(ra), &InferType::Var(rb))?;
                }
                Ok(())
            }
            // A is open, B is closed: extra fields in B go into A's row
            (Some(ra), None) => {
                if !only_in_a.is_empty() {
                    return Err(UnifyError::MissingField(only_in_a[0].0.clone()));
                }
                self.subst.bind(ra, InferType::Record(only_in_b, None));
                Ok(())
            }
            // A is closed, B is open
            (None, Some(rb)) => {
                if !only_in_b.is_empty() {
                    return Err(UnifyError::MissingField(only_in_b[0].0.clone()));
                }
                self.subst.bind(rb, InferType::Record(only_in_a, None));
                Ok(())
            }
            // Both closed: must have exactly the same fields
            (None, None) => {
                if !only_in_a.is_empty() {
                    return Err(UnifyError::MissingField(only_in_a[0].0.clone()));
                }
                if !only_in_b.is_empty() {
                    return Err(UnifyError::MissingField(only_in_b[0].0.clone()));
                }
                Ok(())
            }
        }
    }

    /// Instantiate a polymorphic type scheme by replacing its bound variables with fresh ones
    pub fn instantiate(&mut self, ty: &InferType, bound_vars: &[usize]) -> InferType {
        let mut var_map: HashMap<usize, InferType> = HashMap::new();
        for &var in bound_vars {
            var_map.insert(var, self.fresh_var());
        }
        self.substitute_vars(ty, &var_map)
    }

    fn substitute_vars(&self, ty: &InferType, var_map: &HashMap<usize, InferType>) -> InferType {
        match ty {
            InferType::Var(id) => {
                if let Some(replacement) = var_map.get(id) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            InferType::Fn(params, ret) => InferType::Fn(
                params.iter().map(|p| self.substitute_vars(p, var_map)).collect(),
                Box::new(self.substitute_vars(ret, var_map)),
            ),
            InferType::Tuple(ts) => InferType::Tuple(
                ts.iter().map(|t| self.substitute_vars(t, var_map)).collect(),
            ),
            InferType::List(t) => InferType::List(Box::new(self.substitute_vars(t, var_map))),
            InferType::Named(name, args) => InferType::Named(
                name.clone(),
                args.iter().map(|a| self.substitute_vars(a, var_map)).collect(),
            ),
            InferType::Record(fields, row) => InferType::Record(
                fields.iter().map(|(n, t)| (n.clone(), self.substitute_vars(t, var_map))).collect(),
                row.map(|r| {
                    if let Some(InferType::Var(new_id)) = var_map.get(&r) {
                        *new_id
                    } else {
                        r
                    }
                }),
            ),
            _ => ty.clone(),
        }
    }

    /// Collect free type variables in a type
    pub fn free_vars(&self, ty: &InferType) -> Vec<usize> {
        let resolved = self.subst.apply(ty);
        let mut vars = Vec::new();
        self.collect_free_vars(&resolved, &mut vars);
        vars.sort_unstable();
        vars.dedup();
        vars
    }

    fn collect_free_vars(&self, ty: &InferType, vars: &mut Vec<usize>) {
        match ty {
            InferType::Var(id) => vars.push(*id),
            InferType::Fn(params, ret) => {
                for p in params { self.collect_free_vars(p, vars); }
                self.collect_free_vars(ret, vars);
            }
            InferType::Tuple(ts) => {
                for t in ts { self.collect_free_vars(t, vars); }
            }
            InferType::List(t) => self.collect_free_vars(t, vars),
            InferType::Named(_, args) => {
                for a in args { self.collect_free_vars(a, vars); }
            }
            InferType::Record(fields, row) => {
                for (_, t) in fields { self.collect_free_vars(t, vars); }
                if let Some(r) = row { vars.push(*r); }
            }
            _ => {}
        }
    }
}

/// A type scheme: a type with universally quantified variables (for let-polymorphism)
#[derive(Debug, Clone)]
pub struct TypeScheme {
    /// Bound (universally quantified) type variables
    pub bound_vars: Vec<usize>,
    /// The underlying type
    pub ty: InferType,
}

impl TypeScheme {
    /// A monomorphic scheme (no bound variables)
    pub fn mono(ty: InferType) -> Self {
        Self { bound_vars: Vec::new(), ty }
    }

    /// Generalize a type over variables not free in the environment
    pub fn generalize(ty: InferType, env_free_vars: &[usize], engine: &InferEngine) -> Self {
        let ty_vars = engine.free_vars(&ty);
        let bound: Vec<usize> = ty_vars.into_iter()
            .filter(|v| !env_free_vars.contains(v))
            .collect();
        Self { bound_vars: bound, ty }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unify_basic() {
        let mut engine = InferEngine::new();
        assert!(engine.unify(&InferType::Int, &InferType::Int).is_ok());
        assert!(engine.unify(&InferType::Int, &InferType::Bool).is_err());
    }

    #[test]
    fn test_unify_var() {
        let mut engine = InferEngine::new();
        let v = engine.fresh_var();
        engine.unify(&v, &InferType::Int).unwrap();
        let resolved = engine.subst.apply(&v);
        assert_eq!(resolved, InferType::Int);
    }

    #[test]
    fn test_occurs_check() {
        let mut engine = InferEngine::new();
        let v = engine.fresh_var();
        // Try to unify ?0 with List[?0] — should fail with occurs check
        let list_v = InferType::List(Box::new(v.clone()));
        assert!(engine.unify(&v, &list_v).is_err());
    }

    #[test]
    fn test_unify_functions() {
        let mut engine = InferEngine::new();
        let v1 = engine.fresh_var();
        let v2 = engine.fresh_var();
        let fn1 = InferType::Fn(vec![InferType::Int], Box::new(v1.clone()));
        let fn2 = InferType::Fn(vec![v2.clone()], Box::new(InferType::Bool));
        engine.unify(&fn1, &fn2).unwrap();
        assert_eq!(engine.subst.apply(&v1), InferType::Bool);
        assert_eq!(engine.subst.apply(&v2), InferType::Int);
    }

    #[test]
    fn test_row_polymorphism() {
        let mut engine = InferEngine::new();
        let row_var = engine.fresh_row_var();

        // { name: String | r } should unify with { name: String, age: Int }
        let open_record = InferType::Record(
            vec![("name".into(), InferType::Str)],
            Some(row_var),
        );
        let closed_record = InferType::Record(
            vec![
                ("name".into(), InferType::Str),
                ("age".into(), InferType::Int),
            ],
            None,
        );
        engine.unify(&open_record, &closed_record).unwrap();
    }

    #[test]
    fn test_row_polymorphism_mismatch() {
        let mut engine = InferEngine::new();
        // { name: String } should NOT unify with { name: Int }
        let rec_a = InferType::Record(
            vec![("name".into(), InferType::Str)],
            None,
        );
        let rec_b = InferType::Record(
            vec![("name".into(), InferType::Int)],
            None,
        );
        assert!(engine.unify(&rec_a, &rec_b).is_err());
    }

    #[test]
    fn test_fixed_width_unify() {
        let mut engine = InferEngine::new();
        // Int8 should unify with Int (widening)
        assert!(engine.unify(&InferType::Int8, &InferType::Int).is_ok());
        assert!(engine.unify(&InferType::Float32, &InferType::Float).is_ok());
    }

    #[test]
    fn test_let_polymorphism() {
        let mut engine = InferEngine::new();
        // id: forall a. a -> a
        let a = engine.fresh_var();
        let id_ty = InferType::Fn(vec![a.clone()], Box::new(a.clone()));
        let scheme = TypeScheme {
            bound_vars: vec![0],
            ty: id_ty,
        };

        // Instantiate for Int usage
        let inst1 = engine.instantiate(&scheme.ty, &scheme.bound_vars);
        engine.unify(
            &if let InferType::Fn(ref params, _) = inst1 { params[0].clone() } else { panic!() },
            &InferType::Int,
        ).unwrap();

        // Instantiate again for String usage — should get fresh variables
        let inst2 = engine.instantiate(&scheme.ty, &scheme.bound_vars);
        engine.unify(
            &if let InferType::Fn(ref params, _) = inst2 { params[0].clone() } else { panic!() },
            &InferType::Str,
        ).unwrap();
    }
}
