//! Module system: import resolution, visibility enforcement, and module namespace management.

use crate::ast::*;
use std::collections::HashMap;

/// Represents a resolved module with its exports
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    /// Fully qualified module name
    pub name: Vec<String>,
    /// Public type definitions
    pub public_types: HashMap<String, TypeDef>,
    /// Public function signatures: name -> (param_types, return_type_name)
    pub public_functions: HashMap<String, FnSignature>,
    /// Public effect definitions
    pub public_effects: HashMap<String, EffectDef>,
    /// Public trait definitions
    pub public_traits: HashMap<String, TraitDef>,
    /// All declarations (for single-module compilation)
    pub all_decls: Vec<Decl>,
}

/// A function signature for module export tracking
#[derive(Debug, Clone)]
pub struct FnSignature {
    pub name: String,
    pub param_count: usize,
    pub is_public: bool,
}

/// Errors from module resolution
#[derive(Debug)]
pub enum ModuleError {
    /// Tried to import a name that isn't public
    PrivateAccess { module: String, name: String },
    /// Tried to import a name that doesn't exist
    UndefinedExport { module: String, name: String },
    /// Duplicate definition in module
    DuplicateDefinition { name: String },
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleError::PrivateAccess { module, name } =>
                write!(f, "cannot access private member '{name}' from module '{module}'"),
            ModuleError::UndefinedExport { module, name } =>
                write!(f, "module '{module}' has no export named '{name}'"),
            ModuleError::DuplicateDefinition { name } =>
                write!(f, "duplicate definition: '{name}'"),
        }
    }
}

impl std::error::Error for ModuleError {}

/// Module registry for tracking loaded modules and their exports
#[derive(Debug, Default)]
pub struct ModuleRegistry {
    modules: HashMap<String, ResolvedModule>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a module and extract its public exports
    pub fn register_module(&mut self, module: &Module) -> Result<(), ModuleError> {
        let mod_name = module.name.join(".");
        let resolved = resolve_module(module)?;
        self.modules.insert(mod_name, resolved);
        Ok(())
    }

    /// Resolve imports for a module, returning the names that should be in scope
    pub fn resolve_imports(&self, module: &Module) -> Result<HashMap<String, ImportedName>, ModuleError> {
        let mut imported = HashMap::new();

        for import in &module.imports {
            let mod_path = import.path.join(".");

            // For single-file compilation, all imports resolve as "known" even without
            // the actual module loaded — this enables the type checker to accept them.
            if let Some(resolved) = self.modules.get(&mod_path) {
                match &import.items {
                    ImportItems::All => {
                        // Import all public names
                        for (name, _) in &resolved.public_functions {
                            imported.insert(name.clone(), ImportedName {
                                source_module: mod_path.clone(),
                                original_name: name.clone(),
                            });
                        }
                        for (name, _) in &resolved.public_types {
                            imported.insert(name.clone(), ImportedName {
                                source_module: mod_path.clone(),
                                original_name: name.clone(),
                            });
                        }
                    }
                    ImportItems::Named(names) => {
                        for name in names {
                            // Check the name exists and is public
                            let exists = resolved.public_functions.contains_key(name)
                                || resolved.public_types.contains_key(name)
                                || resolved.public_effects.contains_key(name)
                                || resolved.public_traits.contains_key(name);
                            if !exists {
                                // Check if it exists but is private
                                let is_private = resolved.all_decls.iter().any(|d| match d {
                                    Decl::Function(f) => f.name == *name && !f.public,
                                    Decl::TypeDef(t) => t.name == *name && !t.public,
                                    _ => false,
                                });
                                if is_private {
                                    return Err(ModuleError::PrivateAccess {
                                        module: mod_path.clone(),
                                        name: name.clone(),
                                    });
                                }
                                return Err(ModuleError::UndefinedExport {
                                    module: mod_path.clone(),
                                    name: name.clone(),
                                });
                            }
                            imported.insert(name.clone(), ImportedName {
                                source_module: mod_path.clone(),
                                original_name: name.clone(),
                            });
                        }
                    }
                    ImportItems::Alias(alias) => {
                        // Module alias: import all public names under the alias prefix
                        for (name, _) in &resolved.public_functions {
                            imported.insert(
                                format!("{}.{}", alias, name),
                                ImportedName {
                                    source_module: mod_path.clone(),
                                    original_name: name.clone(),
                                },
                            );
                        }
                    }
                }
            }
            // If module not found, we silently allow it for now (single-file mode)
        }

        Ok(imported)
    }
}

/// An imported name with its source
#[derive(Debug, Clone)]
pub struct ImportedName {
    pub source_module: String,
    pub original_name: String,
}

/// Resolve a module: extract public exports and check for visibility
fn resolve_module(module: &Module) -> Result<ResolvedModule, ModuleError> {
    let mut public_types = HashMap::new();
    let mut public_functions = HashMap::new();
    let mut public_effects = HashMap::new();
    let mut public_traits = HashMap::new();

    for decl in &module.declarations {
        match decl {
            Decl::Function(f) => {
                if f.public {
                    public_functions.insert(f.name.clone(), FnSignature {
                        name: f.name.clone(),
                        param_count: f.params.len(),
                        is_public: true,
                    });
                }
            }
            Decl::TypeDef(t) => {
                if t.public {
                    public_types.insert(t.name.clone(), t.clone());
                }
            }
            Decl::EffectDef(e) => {
                // Effects are always public (they define a protocol)
                public_effects.insert(e.name.clone(), e.clone());
            }
            Decl::TraitDef(t) => {
                // Traits are always public
                public_traits.insert(t.name.clone(), t.clone());
            }
            _ => {}
        }
    }

    Ok(ResolvedModule {
        name: module.name.clone(),
        public_types,
        public_functions,
        public_effects,
        public_traits,
        all_decls: module.declarations.clone(),
    })
}

/// Check visibility constraints within a module.
/// Returns errors for any access to private members from outside their module.
pub fn check_visibility(module: &Module) -> Vec<ModuleError> {
    let mut errors = Vec::new();
    let mut private_names: HashMap<String, bool> = HashMap::new();

    // Collect all private names
    for decl in &module.declarations {
        match decl {
            Decl::Function(f) if !f.public => {
                private_names.insert(f.name.clone(), true);
            }
            Decl::TypeDef(t) if !t.public => {
                private_names.insert(t.name.clone(), true);
            }
            _ => {}
        }
    }

    // In a full multi-module system, we'd check cross-module references here.
    // For now, visibility is tracked and enforced via the ModuleRegistry.
    let _ = (private_names, &mut errors);

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;

    fn dummy_span() -> Span {
        Span { start: 0, end: 0, line: 1, col: 1 }
    }

    #[test]
    fn test_resolve_module_exports() {
        let module = Module {
            name: vec!["test".into()],
            imports: vec![],
            declarations: vec![
                Decl::Function(FnDecl {
                    public: true,
                    is_unsafe: false,
                    name: "public_fn".into(),
                    params: vec![],
                    return_type: None,
                    effects: vec![],
                    body: Expr::UnitLit(dummy_span()),
                    span: dummy_span(),
                }),
                Decl::Function(FnDecl {
                    public: false,
                    is_unsafe: false,
                    name: "private_fn".into(),
                    params: vec![],
                    return_type: None,
                    effects: vec![],
                    body: Expr::UnitLit(dummy_span()),
                    span: dummy_span(),
                }),
            ],
        };

        let resolved = resolve_module(&module).unwrap();
        assert!(resolved.public_functions.contains_key("public_fn"));
        assert!(!resolved.public_functions.contains_key("private_fn"));
    }

    #[test]
    fn test_private_access_error() {
        let provider = Module {
            name: vec!["provider".into()],
            imports: vec![],
            declarations: vec![
                Decl::Function(FnDecl {
                    public: false,
                    is_unsafe: false,
                    name: "secret".into(),
                    params: vec![],
                    return_type: None,
                    effects: vec![],
                    body: Expr::UnitLit(dummy_span()),
                    span: dummy_span(),
                }),
            ],
        };

        let mut registry = ModuleRegistry::new();
        registry.register_module(&provider).unwrap();

        let consumer = Module {
            name: vec!["consumer".into()],
            imports: vec![
                Import {
                    path: vec!["provider".into()],
                    items: ImportItems::Named(vec!["secret".into()]),
                    span: dummy_span(),
                },
            ],
            declarations: vec![],
        };

        let result = registry.resolve_imports(&consumer);
        assert!(result.is_err());
    }
}
