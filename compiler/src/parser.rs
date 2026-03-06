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
                loop {
                    match self.peek().clone() {
                        TokenKind::Ident(n) | TokenKind::TypeIdent(n) => {
                            self.advance();
                            names.push(n);
                        }
                        _ => break,
                    }
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

        match self.peek() {
            TokenKind::Fn => Ok(Decl::Function(self.parse_fn_decl(public)?)),
            TokenKind::Type => Ok(Decl::TypeDef(self.parse_type_def(public)?)),
            TokenKind::Trait => Ok(Decl::TraitDef(self.parse_trait_def()?)),
            TokenKind::Impl => Ok(Decl::ImplBlock(self.parse_impl_block()?)),
            TokenKind::Effect => Ok(Decl::EffectDef(self.parse_effect_def()?)),
            _ => {
                let span = self.span();
                Err(ParseError::Unexpected(
                    format!("{}", self.peek()),
                    span.line,
                    span.col,
                    "declaration (fn, type, trait, impl, effect)".into(),
                ))
            }
        }
    }

    fn parse_fn_decl(&mut self, public: bool) -> Result<FnDecl, ParseError> {
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

        self.expect(&TokenKind::Eq)?;
        let body = self.parse_expr()?;

        Ok(FnDecl {
            public,
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
            TokenKind::Ident(name) => {
                // Lowercase type name (type variable or builtin alias)
                self.advance();
                Ok(TypeExpr::Named(name, Vec::new()))
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
            // Record type
            self.advance();
            let mut fields = Vec::new();
            loop {
                if *self.peek() == TokenKind::RBrace {
                    break;
                }
                let (fname, _) = self.expect_ident()?;
                self.expect(&TokenKind::Colon)?;
                let ftype = self.parse_type_expr()?;
                fields.push((fname, ftype));
                if *self.peek() == TokenKind::Comma {
                    self.advance();
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
        params.push(first);
        while *self.peek() == TokenKind::Comma {
            self.advance();
            let (name, _) = self.expect_type_ident()?;
            params.push(name);
        }
        self.expect(&TokenKind::RBracket)?;
        Ok(params)
    }

    // ---- Trait & Impl ----

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
            methods.push(self.parse_fn_decl(false)?);
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
            methods.push(self.parse_fn_decl(false)?);
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
            operations.push(self.parse_fn_decl(false)?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(EffectDef {
            name,
            type_params,
            operations,
            span,
        })
    }

    // ---- Expressions ----

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_pipe_expr()
    }

    fn parse_pipe_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_or_expr()?;

        while *self.peek() == TokenKind::PipeGt {
            let span = self.span();
            self.advance();
            let rhs = self.parse_or_expr()?;
            expr = Expr::Pipe(Box::new(expr), Box::new(rhs), span);
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
                    let mut args = Vec::new();
                    if *self.peek() != TokenKind::RParen {
                        args.push(self.parse_expr()?);
                        while *self.peek() == TokenKind::Comma {
                            self.advance();
                            if *self.peek() == TokenKind::RParen {
                                break;
                            }
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    expr = Expr::Call(Box::new(expr), args, span);
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

            // List literal
            TokenKind::LBracket => {
                let span = self.span();
                self.advance();
                let mut elems = Vec::new();
                if *self.peek() != TokenKind::RBracket {
                    elems.push(self.parse_expr()?);
                    while *self.peek() == TokenKind::Comma {
                        self.advance();
                        if *self.peek() == TokenKind::RBracket {
                            break;
                        }
                        elems.push(self.parse_expr()?);
                    }
                }
                self.expect(&TokenKind::RBracket)?;
                Ok(Expr::List(elems, span))
            }

            // Record literal
            TokenKind::LBrace => {
                let span = self.span();
                self.advance();
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

            // Do block
            TokenKind::Do => self.parse_do_block(),

            // Let expression
            TokenKind::Let => self.parse_let_expr(),

            // Lambda: fn(x, y) = body
            TokenKind::Fn => self.parse_lambda(),

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
                while let TokenKind::Ident(_) = self.peek() {
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
            let guard = if *self.peek() == TokenKind::When {
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
        match self.peek() {
            TokenKind::Fn | TokenKind::Type | TokenKind::Trait
            | TokenKind::Impl | TokenKind::Effect | TokenKind::Pub => true,
            _ => false,
        }
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
        self.expect(&TokenKind::Eq)?;
        let body = self.parse_expr()?;
        Ok(Expr::Lambda(params, Box::new(body), span))
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
}
