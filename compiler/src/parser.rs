use crate::ast::*;
use crate::lexer::{Span, Token, TokenKind};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("unexpected token {0} at line {1}:{2}, expected {3}")]
    Unexpected(String, usize, usize, String),
    #[error("unexpected end of input, expected {0}")]
    UnexpectedEof(String),
}

pub fn parse(tokens: Vec<Token>) -> Result<Module, ParseError> {
    let mut parser = Parser::new(tokens);
    parser.parse_module()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        // Filter out doc comments — they don't affect parsing
        let tokens: Vec<Token> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::DocComment(_)))
            .collect();
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        self.tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    fn span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or(Span {
                start: 0,
                end: 0,
                line: 0,
                col: 0,
            })
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<Span, ParseError> {
        if self.peek() == expected {
            let span = self.span();
            self.advance();
            Ok(span)
        } else {
            let span = self.span();
            Err(ParseError::Unexpected(
                format!("{}", self.peek()),
                span.line,
                span.col,
                format!("{expected:?}"),
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span), ParseError> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                let span = self.span();
                self.advance();
                Ok((name, span))
            }
            _ => {
                let span = self.span();
                Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    span.line,
                    span.col,
                    "identifier".into(),
                ))
            }
        }
    }

    fn expect_type_ident(&mut self) -> Result<(String, Span), ParseError> {
        match self.peek().clone() {
            TokenKind::TypeIdent(name) => {
                let span = self.span();
                self.advance();
                Ok((name, span))
            }
            _ => {
                let span = self.span();
                Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    span.line,
                    span.col,
                    "type identifier".into(),
                ))
            }
        }
    }

    // ---- Module ----

    fn parse_module(&mut self) -> Result<Module, ParseError> {
        let name = if *self.peek() == TokenKind::Module {
            self.advance();
            self.parse_dotted_name()?
        } else {
            vec!["main".into()]
        };

        let mut imports = Vec::new();
        while *self.peek() == TokenKind::Use {
            imports.push(self.parse_import()?);
        }

        let mut declarations = Vec::new();
        while *self.peek() != TokenKind::Eof {
            declarations.push(self.parse_decl()?);
        }

        Ok(Module {
            name,
            imports,
            declarations,
        })
    }

    fn parse_dotted_name(&mut self) -> Result<Vec<String>, ParseError> {
        let mut parts = Vec::new();
        let (first, _) = self.expect_ident()?;
        parts.push(first);
        while *self.peek() == TokenKind::Dot {
            // Peek ahead: only consume the dot if followed by an identifier
            match self.tokens.get(self.pos + 1).map(|t| &t.kind) {
                Some(TokenKind::Ident(_)) | Some(TokenKind::TypeIdent(_)) => {
                    self.advance(); // consume dot
                    match self.peek().clone() {
                        TokenKind::Ident(name) | TokenKind::TypeIdent(name) => {
                            self.advance();
                            parts.push(name);
                        }
                        _ => break,
                    }
                }
                _ => break, // dot followed by non-ident (e.g. `{` or `*`), leave it
            }
        }
        Ok(parts)
    }

    fn parse_import(&mut self) -> Result<Import, ParseError> {
        let span = self.expect(&TokenKind::Use)?;
        let path = self.parse_dotted_name()?;

        let items = if *self.peek() == TokenKind::Dot {
            self.advance();
            if *self.peek() == TokenKind::Star {
                self.advance();
                ImportItems::All
            } else if *self.peek() == TokenKind::LBrace {
                self.advance();
                let mut names = Vec::new();
                while let TokenKind::Ident(n) | TokenKind::TypeIdent(n) = self.peek().clone() {
                    self.advance();
                    names.push(n);
                    if *self.peek() == TokenKind::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                ImportItems::Named(names)
            } else {
                ImportItems::Named(vec![])
            }
        } else if *self.peek() == TokenKind::As {
            self.advance();
            let (alias, _) = self.expect_ident()?;
            ImportItems::Alias(alias)
        } else {
            ImportItems::Named(vec![])
        };

        Ok(Import { path, items, span })
    }

    // ---- Declarations ----

    fn parse_decl(&mut self) -> Result<Decl, ParseError> {
        let public = if *self.peek() == TokenKind::Pub {
            self.advance();
            true
        } else {
            false
        };

        // Check for `unsafe fn` declarations
        let is_unsafe = if *self.peek() == TokenKind::Unsafe {
            self.advance();
            true
        } else {
            false
        };

        match self.peek() {
            TokenKind::Fn => Ok(Decl::Function(self.parse_fn_decl(public, is_unsafe)?)),
            TokenKind::Type => Ok(Decl::TypeDef(self.parse_type_def(public)?)),
            TokenKind::Newtype => Ok(Decl::NewtypeDef(self.parse_newtype_def(public)?)),
            TokenKind::Nominal => Ok(Decl::NominalDef(self.parse_nominal_def(public)?)),
            TokenKind::Trait => Ok(Decl::TraitDef(self.parse_trait_def()?)),
            TokenKind::Impl => Ok(Decl::ImplBlock(self.parse_impl_block()?)),
            TokenKind::Effect => Ok(Decl::EffectDef(self.parse_effect_def()?)),
            TokenKind::Vibe => Ok(Decl::VibeDecl(self.parse_vibe_decl()?)),
            TokenKind::Test => Ok(Decl::TestDecl(self.parse_test_decl()?)),
            _ => {
                let span = self.span();
                Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    span.line,
                    span.col,
                    "declaration (fn, type, newtype, nominal, trait, impl, effect, unsafe fn, test)".into(),
                ))
            }
        }
    }

    fn parse_fn_decl(&mut self, public: bool, is_unsafe: bool) -> Result<FnDecl, ParseError> {
        let span = self.expect(&TokenKind::Fn)?;
        let (name, _) = self.expect_ident()?;
        // Optional type parameters: fn name[A: Ord, B](...)
        let _type_params = self.parse_optional_type_params()?;
        let params = self.parse_params()?;

        let return_type = if *self.peek() == TokenKind::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        let effects = if *self.peek() == TokenKind::With {
            self.advance();
            let mut effs = vec![self.parse_type_expr()?];
            while *self.peek() == TokenKind::Comma {
                self.advance();
                effs.push(self.parse_type_expr()?);
            }
            effs
        } else {
            Vec::new()
        };

        self.expect(&TokenKind::Eq)?;
        let body = self.parse_expr()?;

        // Optional where clause: desugar `expr where a = x, b = y` into
        // `let a = x in let b = y in expr`
        let body = if *self.peek() == TokenKind::Where {
            self.advance();
            let mut bindings = Vec::new();
            loop {
                let (bname, bspan) = self.expect_ident()?;
                self.expect(&TokenKind::Eq)?;
                let bval = self.parse_expr()?;
                bindings.push((bname, bspan, bval));
                if *self.peek() == TokenKind::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            // Wrap: let b1 = v1 in (let b2 = v2 in (... body))
            bindings.into_iter().rev().fold(body, |inner, (bname, bspan, bval)| {
                Expr::Let(
                    Pattern::Ident(bname, bspan),
                    None,
                    Box::new(bval),
                    Box::new(inner),
                    span,
                )
            })
        } else {
            body
        };

        Ok(FnDecl {
            public,
            is_unsafe,
            name,
            params,
            return_type,
            effects,
            body,
            span,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();

        if *self.peek() != TokenKind::RParen {
            params.push(self.parse_param()?);
            while *self.peek() == TokenKind::Comma {
                self.advance();
                if *self.peek() == TokenKind::RParen {
                    break;
                }
                params.push(self.parse_param()?);
            }
        }

        self.expect(&TokenKind::RParen)?;
        Ok(params)
    }

    fn parse_param(&mut self) -> Result<Param, ParseError> {
        let (name, span) = self.expect_ident()?;
        let type_ann = if *self.peek() == TokenKind::Colon {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        Ok(Param {
            name,
            type_ann,
            span,
        })
    }

    // ---- Type expressions ----

    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        let base = self.parse_type_atom()?;

        // Function type: A -> B
        if *self.peek() == TokenKind::Arrow {
            self.advance();
            let ret = self.parse_type_expr()?;
            let effects = if *self.peek() == TokenKind::With {
                self.advance();
                let mut effs = vec![self.parse_type_expr()?];
                while *self.peek() == TokenKind::Comma {
                    self.advance();
                    effs.push(self.parse_type_expr()?);
                }
                effs
            } else {
                Vec::new()
            };
            return Ok(TypeExpr::Function(vec![base], Box::new(ret), effects));
        }

        Ok(base)
    }

    fn parse_type_atom(&mut self) -> Result<TypeExpr, ParseError> {
        match self.peek().clone() {
            TokenKind::TypeIdent(name) => {
                self.advance();
                let type_args = if *self.peek() == TokenKind::LBracket {
                    self.advance();
                    let mut args = vec![self.parse_type_expr()?];
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        args.push(self.parse_type_expr()?);
                    }
                    self.expect(&TokenKind::RBracket)?;
                    args
                } else {
                    Vec::new()
                };
                Ok(TypeExpr::Named(name, type_args))
            }
            TokenKind::LParen => {
                self.advance();
                if *self.peek() == TokenKind::RParen {
                    self.advance();
                    return Ok(TypeExpr::Unit);
                }
                let first = self.parse_type_expr()?;
                if *self.peek() == TokenKind::Comma {
                    let mut types = vec![first];
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        types.push(self.parse_type_expr()?);
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(TypeExpr::Tuple(types))
                } else {
                    self.expect(&TokenKind::RParen)?;
                    Ok(first)
                }
            }
            TokenKind::Fn => {
                // Function type: fn(A, B) -> C
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let mut param_types = Vec::new();
                if *self.peek() != TokenKind::RParen {
                    param_types.push(self.parse_type_expr()?);
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        param_types.push(self.parse_type_expr()?);
                    }
                }
                self.expect(&TokenKind::RParen)?;
                self.expect(&TokenKind::Arrow)?;
                let ret = self.parse_type_expr()?;
                let effects = if *self.peek() == TokenKind::With {
                    self.advance();
                    let mut effs = vec![self.parse_type_expr()?];
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        effs.push(self.parse_type_expr()?);
                    }
                    effs
                } else {
                    Vec::new()
                };
                Ok(TypeExpr::Function(param_types, Box::new(ret), effects))
            }
            TokenKind::Ident(name) => {
                // Lowercase type name (type variable or builtin alias)
                self.advance();
                Ok(TypeExpr::Named(name, Vec::new()))
            }
            // Record type expression: { field: Type, ... | r }
            TokenKind::LBrace => {
                self.advance();
                let mut fields = Vec::new();
                let mut row_var = None;
                loop {
                    if *self.peek() == TokenKind::RBrace {
                        break;
                    }
                    // Check for row variable: | r
                    if *self.peek() == TokenKind::Pipe {
                        self.advance();
                        let (rv, _) = self.expect_ident()?;
                        row_var = Some(rv);
                        break;
                    }
                    let (fname, _) = self.expect_ident()?;
                    self.expect(&TokenKind::Colon)?;
                    let ftype = self.parse_type_expr()?;
                    fields.push((fname, ftype));
                    if *self.peek() == TokenKind::Comma {
                        self.advance();
                    } else if *self.peek() == TokenKind::Pipe {
                        // Row variable after fields
                        self.advance();
                        let (rv, _) = self.expect_ident()?;
                        row_var = Some(rv);
                        break;
                    } else {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                Ok(TypeExpr::Record(fields, row_var))
            }
            _ => {
                let span = self.span();
                Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    span.line,
                    span.col,
                    "type expression".into(),
                ))
            }
        }
    }

    // ---- Type definitions ----

    /// Parse: newtype Name[A] = InnerType
    fn parse_newtype_def(&mut self, public: bool) -> Result<NewtypeDef, ParseError> {
        let span = self.expect(&TokenKind::Newtype)?;
        let (name, _) = self.expect_type_ident()?;
        let type_params = self.parse_optional_type_params()?;
        self.expect(&TokenKind::Eq)?;
        let inner_type = self.parse_type_expr()?;
        Ok(NewtypeDef {
            public,
            name,
            type_params,
            inner_type,
            span,
        })
    }

    /// Parse: `nominal type Name[A] = InnerType`
    fn parse_nominal_def(&mut self, public: bool) -> Result<NominalDef, ParseError> {
        let span = self.expect(&TokenKind::Nominal)?;
        self.expect(&TokenKind::Type)?;
        let (name, _) = self.expect_type_ident()?;
        let type_params = self.parse_optional_type_params()?;
        self.expect(&TokenKind::Eq)?;
        let inner_type = self.parse_type_expr()?;
        Ok(NominalDef {
            public,
            name,
            type_params,
            inner_type,
            span,
        })
    }

    fn parse_type_def(&mut self, public: bool) -> Result<TypeDef, ParseError> {
        let span = self.expect(&TokenKind::Type)?;

        // Check for 'type alias'
        if let TokenKind::Ident(ref s) = self.peek().clone() {
            if s == "alias" {
                self.advance();
                let (name, _) = self.expect_type_ident()?;
                let type_params = self.parse_optional_type_params()?;
                self.expect(&TokenKind::Eq)?;
                let ty = self.parse_type_expr()?;
                return Ok(TypeDef {
                    public,
                    name,
                    type_params,
                    body: TypeBody::Alias(ty),
                    span,
                });
            }
        }

        let (name, _) = self.expect_type_ident()?;
        let type_params = self.parse_optional_type_params()?;
        self.expect(&TokenKind::Eq)?;

        let body = if *self.peek() == TokenKind::Pipe {
            // Sum type
            let mut variants = Vec::new();
            while *self.peek() == TokenKind::Pipe {
                self.advance();
                let (vname, _) = self.expect_type_ident()?;
                let fields = if *self.peek() == TokenKind::LParen {
                    self.advance();
                    let mut fs = vec![self.parse_type_expr()?];
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        fs.push(self.parse_type_expr()?);
                    }
                    self.expect(&TokenKind::RParen)?;
                    fs
                } else {
                    Vec::new()
                };
                variants.push(Variant {
                    name: vname,
                    fields,
                });
            }
            TypeBody::Variants(variants)
        } else if *self.peek() == TokenKind::LBrace {
            // Record type (with optional row variable: { field: Type | r })
            self.advance();
            let mut fields = Vec::new();
            loop {
                if *self.peek() == TokenKind::RBrace {
                    break;
                }
                // Row variable: | r (skip in TypeBody::Record for now)
                if *self.peek() == TokenKind::Pipe {
                    self.advance();
                    let (_rv, _) = self.expect_ident()?;
                    break;
                }
                let (fname, _) = self.expect_ident()?;
                self.expect(&TokenKind::Colon)?;
                let ftype = self.parse_type_expr()?;
                fields.push((fname, ftype));
                if *self.peek() == TokenKind::Comma {
                    self.advance();
                } else if *self.peek() == TokenKind::Pipe {
                    self.advance();
                    let (_rv, _) = self.expect_ident()?;
                    break;
                } else {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace)?;
            TypeBody::Record(fields)
        } else {
            // Alias
            let ty = self.parse_type_expr()?;
            TypeBody::Alias(ty)
        };

        Ok(TypeDef {
            public,
            name,
            type_params,
            body,
            span,
        })
    }

    fn parse_optional_type_params(&mut self) -> Result<Vec<String>, ParseError> {
        if *self.peek() != TokenKind::LBracket {
            return Ok(Vec::new());
        }
        self.advance();
        let mut params = Vec::new();
        let (first, _) = self.expect_type_ident()?;
        // Skip bounds (A: Trait + Trait) — consume but return just the name
        if *self.peek() == TokenKind::Colon {
            self.advance();
            self.expect_type_ident()?; // first bound
            while *self.peek() == TokenKind::Plus {
                self.advance();
                self.expect_type_ident()?; // additional bound
            }
        }
        params.push(first);
        while *self.peek() == TokenKind::Comma {
            self.advance();
            let (name, _) = self.expect_type_ident()?;
            if *self.peek() == TokenKind::Colon {
                self.advance();
                self.expect_type_ident()?;
                while *self.peek() == TokenKind::Plus {
                    self.advance();
                    self.expect_type_ident()?;
                }
            }
            params.push(name);
        }
        self.expect(&TokenKind::RBracket)?;
        Ok(params)
    }

    /// Parse type params with bounds, returning full bound info: `[A: Eq + Ord, B]`
    fn parse_bounded_type_params(&mut self) -> Result<Vec<TypeParamBound>, ParseError> {
        if *self.peek() != TokenKind::LBracket {
            return Ok(Vec::new());
        }
        self.advance();
        let mut params = Vec::new();
        loop {
            if *self.peek() == TokenKind::RBracket {
                break;
            }
            let (name, _) = self.expect_type_ident()?;
            let mut bounds = Vec::new();
            if *self.peek() == TokenKind::Colon {
                self.advance();
                let (bound, _) = self.expect_type_ident()?;
                bounds.push(bound);
                while *self.peek() == TokenKind::Plus {
                    self.advance();
                    let (bound, _) = self.expect_type_ident()?;
                    bounds.push(bound);
                }
            }
            params.push(TypeParamBound { name, bounds });
            if *self.peek() == TokenKind::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(&TokenKind::RBracket)?;
        Ok(params)
    }

    // ---- Trait & Impl ----

    /// Parse a trait method signature (body is optional; defaults to a unit literal placeholder).
    fn parse_trait_method(&mut self) -> Result<FnDecl, ParseError> {
        let span = self.expect(&TokenKind::Fn)?;
        let (name, _) = self.expect_ident()?;
        let params = self.parse_params()?;

        let return_type = if *self.peek() == TokenKind::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        let effects = if *self.peek() == TokenKind::With {
            self.advance();
            let mut effs = vec![self.parse_type_expr()?];
            while *self.peek() == TokenKind::Comma {
                self.advance();
                effs.push(self.parse_type_expr()?);
            }
            effs
        } else {
            Vec::new()
        };

        // Body is optional in trait definitions: if `=` present, parse body; otherwise use placeholder
        let body = if *self.peek() == TokenKind::Eq {
            self.advance();
            self.parse_expr()?
        } else {
            Expr::UnitLit(span)
        };

        Ok(FnDecl {
            public: false,
            is_unsafe: false,
            name,
            params,
            return_type,
            effects,
            body,
            span,
        })
    }

    fn parse_trait_def(&mut self) -> Result<TraitDef, ParseError> {
        let span = self.expect(&TokenKind::Trait)?;
        let (name, _) = self.expect_type_ident()?;
        let type_params = self.parse_optional_type_params()?;

        let requires = if let TokenKind::Ident(ref s) = self.peek().clone() {
            if s == "requires" {
                self.advance();
                let mut reqs = vec![self.parse_type_expr()?];
                while *self.peek() == TokenKind::Comma {
                    self.advance();
                    reqs.push(self.parse_type_expr()?);
                }
                reqs
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        self.expect(&TokenKind::LBrace)?;
        let mut methods = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            methods.push(self.parse_trait_method()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(TraitDef {
            name,
            type_params,
            requires,
            methods,
            span,
        })
    }

    fn parse_impl_block(&mut self) -> Result<ImplBlock, ParseError> {
        let span = self.expect(&TokenKind::Impl)?;
        let (trait_name, _) = self.expect_type_ident()?;
        let type_params = self.parse_optional_type_params()?;

        self.expect(&TokenKind::For)?;
        let target = self.parse_type_expr()?;

        self.expect(&TokenKind::LBrace)?;
        let mut methods = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            methods.push(self.parse_fn_decl(false, false)?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(ImplBlock {
            trait_name,
            type_params,
            target,
            methods,
            span,
        })
    }

    fn parse_effect_def(&mut self) -> Result<EffectDef, ParseError> {
        let span = self.expect(&TokenKind::Effect)?;
        let (name, _) = self.expect_type_ident()?;
        let type_params = self.parse_optional_type_params()?;

        self.expect(&TokenKind::LBrace)?;
        let mut operations = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            operations.push(self.parse_effect_op()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(EffectDef {
            name,
            type_params,
            operations,
            span,
        })
    }

    /// Parse an effect operation signature: fn name(params) -> ReturnType
    /// Unlike regular fn decls, effect ops have no body (no `= expr`)
    fn parse_effect_op(&mut self) -> Result<FnDecl, ParseError> {
        let span = self.expect(&TokenKind::Fn)?;
        let (name, _) = self.expect_ident()?;
        let params = self.parse_params()?;

        let return_type = if *self.peek() == TokenKind::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        let effects = if *self.peek() == TokenKind::With {
            self.advance();
            let mut effs = vec![self.parse_type_expr()?];
            while *self.peek() == TokenKind::Comma {
                self.advance();
                effs.push(self.parse_type_expr()?);
            }
            effs
        } else {
            Vec::new()
        };

        // Effect operations have no body — use unit placeholder
        Ok(FnDecl {
            public: false,
            is_unsafe: false,
            name,
            params,
            return_type,
            effects,
            body: Expr::UnitLit(span),
            span,
        })
    }

    fn parse_vibe_decl(&mut self) -> Result<VibeDecl, ParseError> {
        let span = self.expect(&TokenKind::Vibe)?;
        let (name, _) = self.expect_ident()?;

        let params = if *self.peek() == TokenKind::LParen {
            self.parse_params()?
        } else {
            Vec::new()
        };

        let return_type = if *self.peek() == TokenKind::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        self.expect(&TokenKind::Eq)?;
        let body = self.parse_expr()?;

        Ok(VibeDecl {
            name,
            params,
            return_type,
            body,
            span,
        })
    }

    fn parse_test_decl(&mut self) -> Result<TestDecl, ParseError> {
        let span = self.expect(&TokenKind::Test)?;
        self.expect(&TokenKind::LParen)?;
        let name = match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                s
            }
            _ => {
                let sp = self.span();
                return Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    sp.line,
                    sp.col,
                    "string literal (test name)".into(),
                ));
            }
        };
        self.expect(&TokenKind::Comma)?;
        let body = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        Ok(TestDecl { name, body, span })
    }

    // ---- Expressions ----

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_pipe_expr()
    }

    fn parse_pipe_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_compose_expr()?;

        while *self.peek() == TokenKind::PipeGt {
            let span = self.span();
            self.advance();
            let rhs = self.parse_compose_expr()?;
            expr = Expr::Pipe(Box::new(expr), Box::new(rhs), span);
        }

        Ok(expr)
    }

    fn parse_compose_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_or_expr()?;

        while *self.peek() == TokenKind::GtGt {
            let span = self.span();
            self.advance();
            let rhs = self.parse_or_expr()?;
            expr = Expr::BinOp(Box::new(expr), BinOp::Compose, Box::new(rhs), span);
        }

        Ok(expr)
    }

    fn parse_or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_and_expr()?;
        while *self.peek() == TokenKind::PipePipe || *self.peek() == TokenKind::Or {
            let span = self.span();
            self.advance();
            let rhs = self.parse_and_expr()?;
            expr = Expr::BinOp(Box::new(expr), BinOp::Or, Box::new(rhs), span);
        }
        Ok(expr)
    }

    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_comparison_expr()?;
        while *self.peek() == TokenKind::AmpAmp || *self.peek() == TokenKind::And {
            let span = self.span();
            self.advance();
            let rhs = self.parse_comparison_expr()?;
            expr = Expr::BinOp(Box::new(expr), BinOp::And, Box::new(rhs), span);
        }
        Ok(expr)
    }

    fn parse_comparison_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_concat_expr()?;
        let op = match self.peek() {
            TokenKind::EqEq => Some(BinOp::Eq),
            TokenKind::BangEq => Some(BinOp::Neq),
            TokenKind::Lt => Some(BinOp::Lt),
            TokenKind::Gt => Some(BinOp::Gt),
            TokenKind::LtEq => Some(BinOp::Lte),
            TokenKind::GtEq => Some(BinOp::Gte),
            _ => None,
        };
        if let Some(op) = op {
            let span = self.span();
            self.advance();
            let rhs = self.parse_concat_expr()?;
            expr = Expr::BinOp(Box::new(expr), op, Box::new(rhs), span);
        }
        Ok(expr)
    }

    fn parse_concat_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_additive_expr()?;
        while *self.peek() == TokenKind::PlusPlus {
            let span = self.span();
            self.advance();
            let rhs = self.parse_additive_expr()?;
            expr = Expr::BinOp(Box::new(expr), BinOp::Concat, Box::new(rhs), span);
        }
        Ok(expr)
    }

    fn parse_additive_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_multiplicative_expr()?;
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let rhs = self.parse_multiplicative_expr()?;
            expr = Expr::BinOp(Box::new(expr), op, Box::new(rhs), span);
        }
        Ok(expr)
    }

    fn parse_multiplicative_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_unary_expr()?;
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let rhs = self.parse_unary_expr()?;
            expr = Expr::BinOp(Box::new(expr), op, Box::new(rhs), span);
        }
        Ok(expr)
    }

    fn parse_unary_expr(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            TokenKind::Minus => {
                let span = self.span();
                self.advance();
                let expr = self.parse_unary_expr()?;
                Ok(Expr::UnaryOp(UnaryOp::Neg, Box::new(expr), span))
            }
            TokenKind::Bang | TokenKind::Not => {
                let span = self.span();
                self.advance();
                let expr = self.parse_unary_expr()?;
                Ok(Expr::UnaryOp(UnaryOp::Not, Box::new(expr), span))
            }
            TokenKind::Tilde => {
                let span = self.span();
                self.advance();
                let expr = self.parse_unary_expr()?;
                Ok(Expr::UnaryOp(UnaryOp::BitNot, Box::new(expr), span))
            }
            _ => self.parse_call_expr(),
        }
    }

    fn parse_call_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary_expr()?;

        loop {
            match self.peek() {
                TokenKind::LParen => {
                    let span = self.span();
                    self.advance();
                    let mut args: Vec<Option<Expr>> = Vec::new();
                    let mut has_placeholder = false;
                    if *self.peek() != TokenKind::RParen {
                        if *self.peek() == TokenKind::Underscore {
                            self.advance();
                            args.push(None);
                            has_placeholder = true;
                        } else {
                            args.push(Some(self.parse_expr()?));
                        }
                        while *self.peek() == TokenKind::Comma {
                            self.advance();
                            if *self.peek() == TokenKind::RParen {
                                break;
                            }
                            if *self.peek() == TokenKind::Underscore {
                                self.advance();
                                args.push(None);
                                has_placeholder = true;
                            } else {
                                args.push(Some(self.parse_expr()?));
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    if has_placeholder {
                        expr = Expr::PartialApp(Box::new(expr), args, span);
                    } else {
                        let concrete_args: Vec<Expr> = args.into_iter().map(|a| a.unwrap()).collect();
                        expr = Expr::Call(Box::new(expr), concrete_args, span);
                    }
                }
                TokenKind::Dot => {
                    let span = self.span();
                    self.advance();
                    let (field, _) = self.expect_ident()?;
                    expr = Expr::FieldAccess(Box::new(expr), field, span);
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_primary_expr(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            TokenKind::IntLit(n) => {
                let span = self.span();
                self.advance();
                Ok(Expr::IntLit(n, span))
            }
            TokenKind::FloatLit(n) => {
                let span = self.span();
                self.advance();
                Ok(Expr::FloatLit(n, span))
            }
            TokenKind::StringLit(s) => {
                let span = self.span();
                self.advance();
                Ok(Expr::StringLit(s, span))
            }

            // String interpolation: StringInterpStart expr StringInterpPart expr ... StringInterpEnd
            TokenKind::StringInterpStart(s) => {
                let span = self.span();
                self.advance();
                let mut parts = vec![StringPart::Literal(s)];
                // Parse first interpolated expression
                parts.push(StringPart::Expr(self.parse_expr()?));
                // Parse remaining parts
                loop {
                    match self.peek().clone() {
                        TokenKind::StringInterpPart(lit) => {
                            self.advance();
                            parts.push(StringPart::Literal(lit));
                            parts.push(StringPart::Expr(self.parse_expr()?));
                        }
                        TokenKind::StringInterpEnd(lit) => {
                            self.advance();
                            parts.push(StringPart::Literal(lit));
                            break;
                        }
                        _ => {
                            let span = self.span();
                            return Err(ParseError::Unexpected(
                                format!("{}", self.peek()),
                                span.line,
                                span.col,
                                "string interpolation part or end".into(),
                            ));
                        }
                    }
                }
                Ok(Expr::StringInterp(parts, span))
            }
            TokenKind::CharLit(c) => {
                let span = self.span();
                self.advance();
                Ok(Expr::CharLit(c, span))
            }
            TokenKind::BoolLit(b) => {
                let span = self.span();
                self.advance();
                Ok(Expr::BoolLit(b, span))
            }
            TokenKind::Ident(name) => {
                let span = self.span();
                self.advance();
                Ok(Expr::Ident(name, span))
            }
            TokenKind::TypeIdent(name) => {
                let span = self.span();
                self.advance();
                Ok(Expr::TypeConstructor(name, span))
            }

            // Parenthesized expression or tuple or unit
            TokenKind::LParen => {
                let span = self.span();
                self.advance();
                if *self.peek() == TokenKind::RParen {
                    self.advance();
                    return Ok(Expr::UnitLit(span));
                }
                let first = self.parse_expr()?;
                if *self.peek() == TokenKind::Comma {
                    let mut elems = vec![first];
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        if *self.peek() == TokenKind::RParen {
                            break;
                        }
                        elems.push(self.parse_expr()?);
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::Tuple(elems, span))
                } else {
                    self.expect(&TokenKind::RParen)?;
                    Ok(first)
                }
            }

            // List literal or list comprehension
            TokenKind::LBracket => {
                let span = self.span();
                self.advance();
                if *self.peek() == TokenKind::RBracket {
                    self.advance();
                    return Ok(Expr::List(vec![], span));
                }
                let first = self.parse_expr()?;
                // Check for comprehension syntax: [expr | x <- list, ...]
                if *self.peek() == TokenKind::Pipe {
                    self.advance();
                    let mut generators = Vec::new();
                    let mut filters = Vec::new();
                    loop {
                        // Try to parse generator: var <- expr
                        if let TokenKind::Ident(_) = self.peek().clone() {
                            let save = self.pos;
                            let (var, _) = self.expect_ident()?;
                            if *self.peek() == TokenKind::LArrow {
                                self.advance();
                                let iter_expr = self.parse_expr()?;
                                generators.push(CompGenerator { var, iter: iter_expr });
                            } else {
                                // Not a generator, rewind and parse as filter
                                self.pos = save;
                                filters.push(self.parse_expr()?);
                            }
                        } else {
                            filters.push(self.parse_expr()?);
                        }
                        if *self.peek() == TokenKind::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RBracket)?;
                    Ok(Expr::ListComp(Box::new(first), generators, filters, span))
                } else {
                    // Regular list literal
                    let mut elems = vec![first];
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        if *self.peek() == TokenKind::RBracket {
                            break;
                        }
                        elems.push(self.parse_expr()?);
                    }
                    self.expect(&TokenKind::RBracket)?;
                    Ok(Expr::List(elems, span))
                }
            }

            // Record literal or record update: { field: val } or { base | field: val }
            TokenKind::LBrace => {
                let span = self.span();
                self.advance();

                // Check for record update syntax: { expr | field: val, ... }
                // We need to look ahead: if we see `ident |` it's an update
                if let TokenKind::Ident(_) = self.peek().clone() {
                    // Save position to backtrack if not an update
                    let saved_pos = self.pos;
                    let (name, _) = self.expect_ident()?;

                    if *self.peek() == TokenKind::Pipe {
                        // Record update: { base_var | field: val, ... }
                        self.advance(); // consume '|'
                        let base = Expr::Ident(name, span);
                        let mut updates = Vec::new();
                        loop {
                            if *self.peek() == TokenKind::RBrace {
                                break;
                            }
                            let (field_name, _) = self.expect_ident()?;
                            self.expect(&TokenKind::Colon)?;
                            let value = self.parse_expr()?;
                            updates.push((field_name, value));
                            if *self.peek() == TokenKind::Comma {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        self.expect(&TokenKind::RBrace)?;
                        return Ok(Expr::RecordUpdate(Box::new(base), updates, span));
                    } else {
                        // Not an update, backtrack and parse as normal record
                        self.pos = saved_pos;
                    }
                }

                let mut fields = Vec::new();
                loop {
                    if *self.peek() == TokenKind::RBrace {
                        break;
                    }
                    let (name, _) = self.expect_ident()?;
                    self.expect(&TokenKind::Colon)?;
                    let value = self.parse_expr()?;
                    fields.push((name, value));
                    if *self.peek() == TokenKind::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                Ok(Expr::Record(fields, span))
            }

            // If expression
            TokenKind::If => self.parse_if_expr(),

            // Match expression
            TokenKind::Match => self.parse_match_expr(),

            // When expression (guard-based conditional)
            TokenKind::When => self.parse_when_expr(),

            // Do block
            TokenKind::Do => self.parse_do_block(),

            // Unsafe block: unsafe { expr }
            TokenKind::Unsafe => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LBrace)?;
                let body = self.parse_expr()?;
                self.expect(&TokenKind::RBrace)?;
                Ok(Expr::UnsafeBlock(Box::new(body), span))
            }

            // For comprehension: for x in collection do body
            TokenKind::For => {
                let span = self.span();
                self.advance();
                let (var_name, _) = self.expect_ident()?;
                self.expect(&TokenKind::In)?;
                let collection = self.parse_expr()?;
                self.expect(&TokenKind::Do)?;
                let body = self.parse_expr()?;
                Ok(Expr::For(var_name, Box::new(collection), Box::new(body), span))
            }

            // Let expression
            TokenKind::Let => self.parse_let_expr(),

            // Lambda: fn(x, y) = body
            TokenKind::Fn => self.parse_lambda(),

            // Handle expression: handle <expr> with <EffectName> { handlers }
            TokenKind::Handle => self.parse_handle_expr(),

            // Resume expression: resume(<expr>)
            TokenKind::Resume => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let expr = if *self.peek() == TokenKind::RParen {
                    Expr::UnitLit(span)
                } else {
                    self.parse_expr()?
                };
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Resume(Box::new(expr), span))
            }

            // Par expression: par(expr1, expr2, ...)
            TokenKind::Par => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let mut exprs = vec![self.parse_expr()?];
                while *self.peek() == TokenKind::Comma {
                    self.advance();
                    if *self.peek() == TokenKind::RParen {
                        break;
                    }
                    exprs.push(self.parse_expr()?);
                }
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Par(exprs, span))
            }

            // Pmap expression: pmap(collection, function)
            TokenKind::Pmap => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let collection = self.parse_expr()?;
                self.expect(&TokenKind::Comma)?;
                let func = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Pmap(Box::new(collection), Box::new(func), span))
            }

            // Pfilter expression: pfilter(collection, predicate)
            TokenKind::Pfilter => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let collection = self.parse_expr()?;
                self.expect(&TokenKind::Comma)?;
                let func = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Pfilter(Box::new(collection), Box::new(func), span))
            }

            // Preduce expression: preduce(collection, init, function)
            TokenKind::Preduce => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let collection = self.parse_expr()?;
                self.expect(&TokenKind::Comma)?;
                let init = self.parse_expr()?;
                self.expect(&TokenKind::Comma)?;
                let func = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Preduce(Box::new(collection), Box::new(init), Box::new(func), span))
            }

            // Race expression: race(expr1, expr2, ...)
            TokenKind::Race => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let mut exprs = vec![self.parse_expr()?];
                while *self.peek() == TokenKind::Comma {
                    self.advance();
                    if *self.peek() == TokenKind::RParen {
                        break;
                    }
                    exprs.push(self.parse_expr()?);
                }
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Race(exprs, span))
            }

            // Channel operations
            TokenKind::SendChan => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let channel = self.parse_expr()?;
                self.expect(&TokenKind::Comma)?;
                let value = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::ChanSend(Box::new(channel), Box::new(value), span))
            }

            TokenKind::Recv => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let channel = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::ChanRecv(Box::new(channel), span))
            }

            // spawn expr  or  spawn(handler) for actors
            TokenKind::Spawn => {
                let span = self.span();
                self.advance();
                if *self.peek() == TokenKind::LParen {
                    // Could be spawn(handler) actor syntax or spawn(expr) task syntax
                    self.advance();
                    let expr = self.parse_expr()?;
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::Spawn(Box::new(expr), span))
                } else {
                    let expr = self.parse_expr()?;
                    Ok(Expr::Spawn(Box::new(expr), span))
                }
            }

            // async do { body }
            TokenKind::Async => {
                let span = self.span();
                self.advance();
                let body = self.parse_expr()?;
                Ok(Expr::Async(Box::new(body), span))
            }

            // await expr
            TokenKind::Await => {
                let span = self.span();
                self.advance();
                let expr = self.parse_expr()?;
                Ok(Expr::Await(Box::new(expr), span))
            }

            // select | msg <- ch -> body | ...
            TokenKind::Select => {
                let span = self.span();
                self.advance();
                let mut arms = Vec::new();
                while *self.peek() == TokenKind::Pipe {
                    self.advance();
                    let (var, _) = self.expect_ident()?;
                    self.expect(&TokenKind::LArrow)?;
                    let channel = self.parse_expr()?;
                    self.expect(&TokenKind::Arrow)?;
                    let body = self.parse_expr()?;
                    arms.push(SelectArm { var, channel, body });
                }
                Ok(Expr::Select(arms, span))
            }

            // Actor: send_to(actor, message)
            TokenKind::SendTo => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let actor = self.parse_expr()?;
                self.expect(&TokenKind::Comma)?;
                let message = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::SendTo(Box::new(actor), Box::new(message), span))
            }

            // Timeout: with_timeout(duration, expr)
            TokenKind::WithTimeout => {
                let span = self.span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let duration = self.parse_expr()?;
                self.expect(&TokenKind::Comma)?;
                let body = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::WithTimeout(Box::new(duration), Box::new(body), span))
            }

            // Backslash lambda: \x -> body
            TokenKind::Backslash => {
                let span = self.span();
                self.advance();
                let mut params = Vec::new();
                let (name, pspan) = self.expect_ident()?;
                params.push(Param {
                    name,
                    type_ann: None,
                    span: pspan,
                });
                while *self.peek() == TokenKind::Comma {
                    self.advance();
                    let (name, pspan) = self.expect_ident()?;
                    params.push(Param {
                        name,
                        type_ann: None,
                        span: pspan,
                    });
                }
                self.expect(&TokenKind::Arrow)?;
                let body = self.parse_expr()?;
                Ok(Expr::Lambda(params, Box::new(body), span))
            }

            _ => {
                let span = self.span();
                Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    span.line,
                    span.col,
                    "expression".into(),
                ))
            }
        }
    }

    fn parse_if_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::If)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Then)?;
        let then_branch = self.parse_expr()?;
        let else_branch = if *self.peek() == TokenKind::Else {
            self.advance();
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        Ok(Expr::If(
            Box::new(cond),
            Box::new(then_branch),
            else_branch,
            span,
        ))
    }

    fn parse_match_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::Match)?;
        let scrutinee = self.parse_expr()?;

        let mut arms = Vec::new();
        while *self.peek() == TokenKind::Pipe {
            self.advance();
            let pattern = self.parse_pattern()?;
            let guard = if *self.peek() == TokenKind::When || *self.peek() == TokenKind::If {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            self.expect(&TokenKind::Arrow)?;
            let body = self.parse_expr()?;
            arms.push(MatchArm {
                pattern,
                guard,
                body,
            });
        }

        Ok(Expr::Match(Box::new(scrutinee), arms, span))
    }

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        match self.peek().clone() {
            TokenKind::Underscore => {
                let span = self.span();
                self.advance();
                Ok(Pattern::Wildcard(span))
            }
            TokenKind::IntLit(n) => {
                let span = self.span();
                self.advance();
                Ok(Pattern::IntLit(n, span))
            }
            TokenKind::FloatLit(n) => {
                let span = self.span();
                self.advance();
                Ok(Pattern::FloatLit(n, span))
            }
            TokenKind::StringLit(s) => {
                let span = self.span();
                self.advance();
                Ok(Pattern::StringLit(s, span))
            }
            TokenKind::BoolLit(b) => {
                let span = self.span();
                self.advance();
                Ok(Pattern::BoolLit(b, span))
            }
            TokenKind::CharLit(c) => {
                let span = self.span();
                self.advance();
                Ok(Pattern::CharLit(c, span))
            }
            TokenKind::Ident(name) => {
                let span = self.span();
                self.advance();
                Ok(Pattern::Ident(name, span))
            }
            TokenKind::TypeIdent(name) => {
                let span = self.span();
                self.advance();
                let args = if *self.peek() == TokenKind::LParen {
                    self.advance();
                    let mut ps = vec![self.parse_pattern()?];
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        ps.push(self.parse_pattern()?);
                    }
                    self.expect(&TokenKind::RParen)?;
                    ps
                } else {
                    Vec::new()
                };
                Ok(Pattern::Constructor(name, args, span))
            }
            TokenKind::LParen => {
                let span = self.span();
                self.advance();
                let mut pats = vec![self.parse_pattern()?];
                while *self.peek() == TokenKind::Comma {
                    self.advance();
                    pats.push(self.parse_pattern()?);
                }
                self.expect(&TokenKind::RParen)?;
                Ok(Pattern::Tuple(pats, span))
            }
            TokenKind::LBrace => {
                // Record pattern: { x, y } or { x: pat, y: pat }
                let span = self.span();
                self.advance();
                let mut fields = Vec::new();
                while *self.peek() != TokenKind::RBrace {
                    let (fname, _) = self.expect_ident()?;
                    let pat = if *self.peek() == TokenKind::Colon {
                        self.advance();
                        self.parse_pattern()?
                    } else {
                        // Shorthand: { x } means { x: x }
                        Pattern::Ident(fname.clone(), span)
                    };
                    fields.push((fname, pat));
                    if *self.peek() == TokenKind::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                Ok(Pattern::Record(fields, span))
            }
            _ => {
                let span = self.span();
                Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    span.line,
                    span.col,
                    "pattern".into(),
                ))
            }
        }
    }

    /// Parse: when { cond1 -> expr1, cond2 -> expr2, otherwise -> default }
    fn parse_when_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::When)?;
        self.expect(&TokenKind::LBrace)?;

        let mut clauses = Vec::new();
        loop {
            if *self.peek() == TokenKind::RBrace {
                break;
            }
            // `otherwise` is the default/else clause
            let condition = if *self.peek() == TokenKind::Otherwise {
                self.advance();
                Expr::BoolLit(true, span)
            } else {
                self.parse_expr()?
            };
            self.expect(&TokenKind::Arrow)?;
            let body = self.parse_expr()?;
            clauses.push(WhenClause { condition, body });
            if *self.peek() == TokenKind::Comma {
                self.advance();
            }
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Expr::When(clauses, span))
    }

    fn parse_do_block(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::Do)?;

        let mut exprs = Vec::new();
        // Parse expressions until we hit something that isn't a valid expression start
        // In a do block, each expression is separated by newlines (but we don't track those)
        // so we parse as many expressions as we can
        loop {
            match self.peek() {
                TokenKind::Eof
                | TokenKind::RBrace
                | TokenKind::RParen
                | TokenKind::Pipe => break,
                _ => {
                    // Try to detect if we're at a new top-level declaration
                    if self.is_at_decl_start() {
                        break;
                    }
                    exprs.push(self.parse_expr()?);
                }
            }
        }

        if exprs.is_empty() {
            Ok(Expr::UnitLit(span))
        } else {
            Ok(Expr::DoBlock(exprs, span))
        }
    }

    fn is_at_decl_start(&self) -> bool {
        matches!(self.peek(),
            TokenKind::Fn | TokenKind::Type | TokenKind::Newtype | TokenKind::Nominal
            | TokenKind::Trait | TokenKind::Impl | TokenKind::Effect | TokenKind::Pub
            | TokenKind::Vibe | TokenKind::Test)
    }

    fn parse_let_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::Let)?;
        let pattern = self.parse_pattern()?;
        let type_ann = if *self.peek() == TokenKind::Colon {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expr()?;

        // Check for 'else' for let-else expression: let pat = expr else fallback
        if *self.peek() == TokenKind::Else {
            self.advance();
            let fallback = self.parse_expr()?;
            return Ok(Expr::LetElse(
                pattern,
                type_ann,
                Box::new(value),
                Box::new(fallback),
                span,
            ));
        }

        // Check if there's an 'in' for let-in expression
        if *self.peek() == TokenKind::In {
            self.advance();
            let body = self.parse_expr()?;
            Ok(Expr::Let(
                pattern,
                type_ann,
                Box::new(value),
                Box::new(body),
                span,
            ))
        } else {
            // Standalone let binding (in do block)
            Ok(Expr::LetBind(pattern, type_ann, Box::new(value), span))
        }
    }

    fn parse_lambda(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::Fn)?;
        let params = self.parse_params()?;
        // Optional return type annotation: -> Type
        if *self.peek() == TokenKind::Arrow {
            self.advance();
            let _ret_type = self.parse_type_expr()?;
        }
        self.expect(&TokenKind::Eq)?;
        let body = self.parse_expr()?;
        Ok(Expr::Lambda(params, Box::new(body), span))
    }

    /// Parse: handle <expr> with <EffectName>[<TypeArgs>] { op(params) -> body, ... }
    fn parse_handle_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        self.expect(&TokenKind::Handle)?;
        let body = self.parse_expr()?;

        let mut handlers = Vec::new();

        // Parse one or more `with Effect { handlers }` clauses
        while *self.peek() == TokenKind::With {
            self.advance();
            let (effect_name, _) = self.expect_type_ident()?;

            // Optional type arguments [A, B, ...]
            if *self.peek() == TokenKind::LBracket {
                self.advance();
                while *self.peek() != TokenKind::RBracket {
                    self.parse_type_expr()?; // consume but we don't use type args in handlers yet
                    if *self.peek() == TokenKind::Comma {
                        self.advance();
                    }
                }
                self.expect(&TokenKind::RBracket)?;
            }

            self.expect(&TokenKind::LBrace)?;

            while *self.peek() != TokenKind::RBrace {
                let (op_name, _) = self.expect_ident()?;
                self.expect(&TokenKind::LParen)?;
                let mut params = Vec::new();
                if *self.peek() != TokenKind::RParen {
                    let (pname, _) = self.expect_ident()?;
                    params.push(pname);
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        let (pname, _) = self.expect_ident()?;
                        params.push(pname);
                    }
                }
                self.expect(&TokenKind::RParen)?;
                self.expect(&TokenKind::Arrow)?;
                let handler_body = self.parse_expr()?;

                handlers.push(Handler {
                    effect_name: effect_name.clone(),
                    operation: op_name,
                    params,
                    body: handler_body,
                });
            }
            self.expect(&TokenKind::RBrace)?;
        }

        Ok(Expr::Handle(Box::new(body), handlers, span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer;

    fn parse_str(s: &str) -> Module {
        let tokens = lexer::lex(s).unwrap();
        parse(tokens).unwrap()
    }

    #[test]
    fn test_simple_function() {
        let m = parse_str("module main\nfn add(x: Int, y: Int) -> Int = x + y");
        assert_eq!(m.declarations.len(), 1);
        match &m.declarations[0] {
            Decl::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
            }
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_type_def() {
        let m = parse_str("module main\ntype Option[A] = | Some(A) | None");
        assert_eq!(m.declarations.len(), 1);
        match &m.declarations[0] {
            Decl::TypeDef(t) => {
                assert_eq!(t.name, "Option");
                assert_eq!(t.type_params, vec!["A"]);
            }
            _ => panic!("expected type def"),
        }
    }

    #[test]
    fn test_if_expr() {
        let m = parse_str("module main\nfn abs(x: Int) -> Int = if x >= 0 then x else -x");
        match &m.declarations[0] {
            Decl::Function(f) => match &f.body {
                Expr::If(..) => {}
                other => panic!("expected if, got {other:?}"),
            },
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_pipe_expr() {
        let m = parse_str("module main\nfn process(x: Int) -> Int = x |> double |> add_one");
        match &m.declarations[0] {
            Decl::Function(f) => match &f.body {
                Expr::Pipe(..) => {}
                other => panic!("expected pipe, got {other:?}"),
            },
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_lambda() {
        let m = parse_str("module main\nfn apply(f: Int -> Int, x: Int) -> Int = f(x)");
        assert_eq!(m.declarations.len(), 1);
    }

    #[test]
    fn test_imports() {
        let m = parse_str("module main\nuse core.io.{print, read_line}\nfn main() -> () = ()");
        assert_eq!(m.imports.len(), 1);
        match &m.imports[0].items {
            ImportItems::Named(names) => {
                assert_eq!(names, &["print", "read_line"]);
            }
            _ => panic!("expected named imports"),
        }
    }

    #[test]
    fn test_match() {
        let m = parse_str(
            "module main\nfn describe(x: Int) -> String = match x | 0 -> \"zero\" | _ -> \"other\"",
        );
        match &m.declarations[0] {
            Decl::Function(f) => match &f.body {
                Expr::Match(_, arms, _) => assert_eq!(arms.len(), 2),
                other => panic!("expected match, got {other:?}"),
            },
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_trait_def() {
        let m = parse_str("module main\ntrait Show {\n    fn show(x: Int) -> Int\n}");
        assert_eq!(m.declarations.len(), 1);
        match &m.declarations[0] {
            Decl::TraitDef(t) => {
                assert_eq!(t.name, "Show");
                assert_eq!(t.methods.len(), 1);
                assert_eq!(t.methods[0].name, "show");
                assert_eq!(t.methods[0].params.len(), 1);
            }
            _ => panic!("expected trait def"),
        }
    }

    #[test]
    fn test_impl_block() {
        let m = parse_str(
            "module main\ntrait Eq {\n    fn eq(a: Int, b: Int) -> Int\n}\nimpl Eq for Int {\n    fn eq(a: Int, b: Int) -> Int = a\n}"
        );
        assert_eq!(m.declarations.len(), 2);
        match &m.declarations[1] {
            Decl::ImplBlock(ib) => {
                assert_eq!(ib.trait_name, "Eq");
                assert_eq!(ib.methods.len(), 1);
                assert_eq!(ib.methods[0].name, "eq");
            }
            _ => panic!("expected impl block"),
        }
    }

    #[test]
    fn test_string_interpolation() {
        let m = parse_str(r#"module main
fn greet(name: String) -> String = "Hello, ${name}!""#);
        match &m.declarations[0] {
            Decl::Function(f) => match &f.body {
                Expr::StringInterp(parts, _) => {
                    assert_eq!(parts.len(), 3); // "Hello, " + name + "!"
                }
                other => panic!("expected string interp, got {other:?}"),
            },
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_compose_expr() {
        let m = parse_str("module main\nfn composed() -> Int = double >> add_one");
        match &m.declarations[0] {
            Decl::Function(f) => match &f.body {
                Expr::BinOp(_, BinOp::Compose, _, _) => {}
                other => panic!("expected compose, got {other:?}"),
            },
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_newtype_def() {
        let m = parse_str("module main\nnewtype UserId = Int");
        match &m.declarations[0] {
            Decl::NewtypeDef(nt) => {
                assert_eq!(nt.name, "UserId");
            }
            _ => panic!("expected newtype def"),
        }
    }

    #[test]
    fn test_when_expr() {
        let m = parse_str("module main\nfn f(x: Int) -> String = when { x > 0 -> \"pos\", otherwise -> \"neg\" }");
        match &m.declarations[0] {
            Decl::Function(f) => match &f.body {
                Expr::When(clauses, _) => {
                    assert_eq!(clauses.len(), 2);
                }
                other => panic!("expected when, got {other:?}"),
            },
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn test_trait_with_requires() {
        let m = parse_str(
            "module main\ntrait Ord requires Eq {\n    fn compare(a: Int, b: Int) -> Int\n}"
        );
        match &m.declarations[0] {
            Decl::TraitDef(t) => {
                assert_eq!(t.name, "Ord");
                assert_eq!(t.requires.len(), 1);
            }
            _ => panic!("expected trait def"),
        }
    }
}
