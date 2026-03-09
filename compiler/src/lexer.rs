use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    /// Interpolated string: alternating literal parts and expression token sequences.
    /// Parts are: (literal_before, Option<tokens_for_expr>). The last part has None.
    StringInterpStart(String), // literal part before first ${
    StringInterpPart(String),  // literal part between }...${
    StringInterpEnd(String),   // literal part after last }
    CharLit(char),
    BoolLit(bool),

    // Identifiers
    Ident(String),
    TypeIdent(String),

    // Keywords
    And,
    As,
    Do,
    Effect,
    Else,
    Fn,
    For,
    Handle,
    If,
    Impl,
    In,
    Let,
    Match,
    Module,
    Newtype,
    Not,
    Of,
    Or,
    Otherwise,
    Par,
    Pfilter,
    Pmap,
    Preduce,
    Pub,
    Race,
    Recv,
    Region,
    Resume,
    Return,
    SendChan,
    SendTo,
    Source,
    Spawn,
    Stream,
    Test,
    Then,
    Trait,
    Type,
    Unsafe,
    Use,
    Vibe,
    When,
    With,
    WithTimeout,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    BangEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    AmpAmp,
    PipePipe,
    Bang,
    Amp,
    Pipe,
    Caret,
    Tilde,
    LtLt,
    GtGt,
    PlusPlus,
    PipeGt,
    GtGt2, // >> as compose (contextual, same token as GtGt)
    ColonColon,

    // Delimiters
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // Punctuation
    Comma,
    Dot,
    Colon,
    Eq,
    Arrow,    // ->
    FatArrow, // =>
    Backslash,
    Underscore,

    // Special
    Eof,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::IntLit(n) => write!(f, "{n}"),
            TokenKind::FloatLit(n) => write!(f, "{n}"),
            TokenKind::StringLit(s) => write!(f, "\"{s}\""),
            TokenKind::CharLit(c) => write!(f, "'{c}'"),
            TokenKind::BoolLit(b) => write!(f, "{b}"),
            TokenKind::Ident(s) | TokenKind::TypeIdent(s) => write!(f, "{s}"),
            TokenKind::Eof => write!(f, "EOF"),
            other => write!(f, "{other:?}"),
        }
    }
}

#[derive(Error, Debug)]
pub enum LexError {
    #[error("unexpected character '{0}' at line {1}:{2}")]
    UnexpectedChar(char, usize, usize),
    #[error("unterminated string literal at line {0}:{1}")]
    UnterminatedString(usize, usize),
    #[error("unterminated char literal at line {0}:{1}")]
    UnterminatedChar(usize, usize),
    #[error("invalid number literal at line {0}:{1}")]
    InvalidNumber(usize, usize),
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer::new(source);
    lexer.lex_all()
}

struct Lexer<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn span_from(&self, start: usize, start_line: usize, start_col: usize) -> Span {
        Span {
            start,
            end: self.pos,
            line: start_line,
            col: start_col,
        }
    }

    fn lex_all(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace_and_comments();

            if self.pos >= self.chars.len() {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: Span {
                        start: self.pos,
                        end: self.pos,
                        line: self.line,
                        col: self.col,
                    },
                });
                break;
            }

            // String literals may produce multiple tokens (for interpolation)
            if self.peek() == Some('"') {
                let start = self.pos;
                let start_line = self.line;
                let start_col = self.col;
                self.lex_string_tokens(start, start_line, start_col, &mut tokens)?;
            } else {
                let tok = self.lex_token()?;
                tokens.push(tok);
            }
        }

        Ok(tokens)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace
            while let Some(ch) = self.peek() {
                if ch.is_whitespace() {
                    self.advance();
                } else {
                    break;
                }
            }

            // Skip line comments: --
            if self.peek() == Some('-') && self.peek_next() == Some('-') {
                // Check it's not a doc comment that we might want to preserve
                // For now, skip all -- comments
                while let Some(ch) = self.advance() {
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }

            // Skip block comments: {- ... -}
            if self.peek() == Some('{') && self.peek_next() == Some('-') {
                self.advance(); // {
                self.advance(); // -
                let mut depth = 1;
                while depth > 0 {
                    match self.advance() {
                        Some('{') if self.peek() == Some('-') => {
                            self.advance();
                            depth += 1;
                        }
                        Some('-') if self.peek() == Some('}') => {
                            self.advance();
                            depth -= 1;
                        }
                        None => break,
                        _ => {}
                    }
                }
                continue;
            }

            break;
        }
    }

    fn lex_token(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;

        let ch = self.peek().unwrap();

        // String literals - normally handled by lex_all via lex_string_tokens,
        // but lex_token also handles them for recursive lexing (e.g., inside interpolation)
        if ch == '"' {
            return self.lex_string(start, start_line, start_col);
        }

        // Char literals
        if ch == '\'' {
            return self.lex_char(start, start_line, start_col);
        }

        // Number literals
        if ch.is_ascii_digit() {
            return self.lex_number(start, start_line, start_col);
        }

        // Identifiers and keywords
        if ch.is_alphabetic() || ch == '_' {
            return Ok(self.lex_ident(start, start_line, start_col));
        }

        // Operators and punctuation
        self.lex_operator(start, start_line, start_col)
    }

    /// Lex a string literal, handling interpolation `${expr}` and triple-quoted strings `"""..."""`.
    /// Pushes one or more tokens to `out`.
    fn lex_string_tokens(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
        out: &mut Vec<Token>,
    ) -> Result<(), LexError> {
        self.advance(); // opening "

        // Check for triple-quoted string """
        if self.peek() == Some('"') && self.chars.get(self.pos + 1).copied() == Some('"') {
            self.advance(); // second "
            self.advance(); // third "
            return self.lex_triple_string(start, start_line, start_col, out);
        }

        let mut s = String::new();
        let mut has_interp = false;

        loop {
            match self.peek() {
                Some('"') => {
                    self.advance();
                    if has_interp {
                        out.push(Token {
                            kind: TokenKind::StringInterpEnd(s),
                            span: self.span_from(start, start_line, start_col),
                        });
                    } else {
                        out.push(Token {
                            kind: TokenKind::StringLit(s),
                            span: self.span_from(start, start_line, start_col),
                        });
                    }
                    return Ok(());
                }
                Some('$') if self.chars.get(self.pos + 1).copied() == Some('{') => {
                    self.advance(); // $
                    self.advance(); // {
                    let kind = if !has_interp {
                        has_interp = true;
                        TokenKind::StringInterpStart(std::mem::take(&mut s))
                    } else {
                        TokenKind::StringInterpPart(std::mem::take(&mut s))
                    };
                    out.push(Token {
                        kind,
                        span: self.span_from(start, start_line, start_col),
                    });
                    // Lex the interpolated expression tokens until matching '}'
                    let mut depth = 1;
                    loop {
                        self.skip_whitespace_and_comments();
                        if self.pos >= self.chars.len() {
                            return Err(LexError::UnterminatedString(start_line, start_col));
                        }
                        if self.peek() == Some('}') {
                            depth -= 1;
                            if depth == 0 {
                                self.advance(); // consume closing }
                                break;
                            }
                        }
                        let tok = self.lex_token()?;
                        match &tok.kind {
                            TokenKind::LBrace => depth += 1,
                            TokenKind::RBrace => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        out.push(tok);
                    }
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some('0') => s.push('\0'),
                        Some('$') => s.push('$'),
                        Some(c) => {
                            s.push('\\');
                            s.push(c);
                        }
                        None => return Err(LexError::UnterminatedString(start_line, start_col)),
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
                None => return Err(LexError::UnterminatedString(start_line, start_col)),
            }
        }
    }

    /// Lex a triple-quoted (multi-line) string literal `"""..."""`.
    fn lex_triple_string(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
        out: &mut Vec<Token>,
    ) -> Result<(), LexError> {
        let mut s = String::new();
        loop {
            match self.peek() {
                Some('"')
                    if self.chars.get(self.pos + 1).copied() == Some('"')
                        && self.chars.get(self.pos + 2).copied() == Some('"') =>
                {
                    self.advance(); // "
                    self.advance(); // "
                    self.advance(); // "
                    out.push(Token {
                        kind: TokenKind::StringLit(s),
                        span: self.span_from(start, start_line, start_col),
                    });
                    return Ok(());
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some('0') => s.push('\0'),
                        Some(c) => {
                            s.push('\\');
                            s.push(c);
                        }
                        None => return Err(LexError::UnterminatedString(start_line, start_col)),
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
                None => return Err(LexError::UnterminatedString(start_line, start_col)),
            }
        }
    }

    /// Legacy lex_string for use by lex_token (non-interpolated path).
    fn lex_string(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
    ) -> Result<Token, LexError> {
        // This should not be called from lex_all anymore (string handling moved to lex_string_tokens),
        // but kept for backwards compatibility with direct lex_token calls.
        self.advance(); // opening "
        let mut s = String::new();
        loop {
            match self.advance() {
                Some('"') => {
                    return Ok(Token {
                        kind: TokenKind::StringLit(s),
                        span: self.span_from(start, start_line, start_col),
                    });
                }
                Some('\\') => match self.advance() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some('0') => s.push('\0'),
                    Some(c) => {
                        s.push('\\');
                        s.push(c);
                    }
                    None => return Err(LexError::UnterminatedString(start_line, start_col)),
                },
                Some(c) => s.push(c),
                None => return Err(LexError::UnterminatedString(start_line, start_col)),
            }
        }
    }

    fn lex_char(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
    ) -> Result<Token, LexError> {
        self.advance(); // opening '
        let c = match self.advance() {
            Some('\\') => match self.advance() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('r') => '\r',
                Some('\\') => '\\',
                Some('\'') => '\'',
                Some('0') => '\0',
                _ => return Err(LexError::UnterminatedChar(start_line, start_col)),
            },
            Some(c) => c,
            None => return Err(LexError::UnterminatedChar(start_line, start_col)),
        };
        match self.advance() {
            Some('\'') => Ok(Token {
                kind: TokenKind::CharLit(c),
                span: self.span_from(start, start_line, start_col),
            }),
            _ => Err(LexError::UnterminatedChar(start_line, start_col)),
        }
    }

    fn lex_number(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
    ) -> Result<Token, LexError> {
        let mut num_str = String::new();
        let mut is_float = false;

        // Check for hex, binary, octal prefix
        if self.peek() == Some('0') {
            match self.peek_next() {
                Some('x') | Some('X') => {
                    self.advance();
                    self.advance();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_hexdigit() || c == '_' {
                            if c != '_' {
                                num_str.push(c);
                            }
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let val = i64::from_str_radix(&num_str, 16)
                        .map_err(|_| LexError::InvalidNumber(start_line, start_col))?;
                    return Ok(Token {
                        kind: TokenKind::IntLit(val),
                        span: self.span_from(start, start_line, start_col),
                    });
                }
                Some('b') | Some('B') => {
                    self.advance();
                    self.advance();
                    while let Some(c) = self.peek() {
                        if c == '0' || c == '1' || c == '_' {
                            if c != '_' {
                                num_str.push(c);
                            }
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let val = i64::from_str_radix(&num_str, 2)
                        .map_err(|_| LexError::InvalidNumber(start_line, start_col))?;
                    return Ok(Token {
                        kind: TokenKind::IntLit(val),
                        span: self.span_from(start, start_line, start_col),
                    });
                }
                Some('o') | Some('O') => {
                    self.advance();
                    self.advance();
                    while let Some(c) = self.peek() {
                        if ('0'..='7').contains(&c) || c == '_' {
                            if c != '_' {
                                num_str.push(c);
                            }
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let val = i64::from_str_radix(&num_str, 8)
                        .map_err(|_| LexError::InvalidNumber(start_line, start_col))?;
                    return Ok(Token {
                        kind: TokenKind::IntLit(val),
                        span: self.span_from(start, start_line, start_col),
                    });
                }
                _ => {}
            }
        }

        // Decimal number
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' {
                    num_str.push(c);
                }
                self.advance();
            } else if c == '.' && !is_float {
                // Check next char is a digit (not a method call)
                if let Some(next) = self.peek_next() {
                    if next.is_ascii_digit() {
                        is_float = true;
                        num_str.push('.');
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        if is_float {
            let val: f64 = num_str
                .parse()
                .map_err(|_| LexError::InvalidNumber(start_line, start_col))?;
            Ok(Token {
                kind: TokenKind::FloatLit(val),
                span: self.span_from(start, start_line, start_col),
            })
        } else {
            let val: i64 = num_str
                .parse()
                .map_err(|_| LexError::InvalidNumber(start_line, start_col))?;
            Ok(Token {
                kind: TokenKind::IntLit(val),
                span: self.span_from(start, start_line, start_col),
            })
        }
    }

    fn lex_ident(&mut self, start: usize, start_line: usize, start_col: usize) -> Token {
        let mut ident = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                ident.push(c);
                self.advance();
            } else {
                break;
            }
        }

        let kind = match ident.as_str() {
            "and" => TokenKind::And,
            "as" => TokenKind::As,
            "do" => TokenKind::Do,
            "effect" => TokenKind::Effect,
            "else" => TokenKind::Else,
            "false" => TokenKind::BoolLit(false),
            "fn" => TokenKind::Fn,
            "for" => TokenKind::For,
            "handle" => TokenKind::Handle,
            "if" => TokenKind::If,
            "impl" => TokenKind::Impl,
            "in" => TokenKind::In,
            "let" => TokenKind::Let,
            "match" => TokenKind::Match,
            "module" => TokenKind::Module,
            "newtype" => TokenKind::Newtype,
            "not" => TokenKind::Not,
            "of" => TokenKind::Of,
            "or" => TokenKind::Or,
            "otherwise" => TokenKind::Otherwise,
            "par" => TokenKind::Par,
            "pfilter" => TokenKind::Pfilter,
            "pmap" => TokenKind::Pmap,
            "preduce" => TokenKind::Preduce,
            "pub" => TokenKind::Pub,
            "race" => TokenKind::Race,
            "recv" => TokenKind::Recv,
            "region" => TokenKind::Region,
            "resume" => TokenKind::Resume,
            "return" => TokenKind::Return,
            "send" => TokenKind::SendChan,
            "send_to" => TokenKind::SendTo,
            "source" => TokenKind::Source,
            "spawn" => TokenKind::Spawn,
            "stream" => TokenKind::Stream,
            "test" => TokenKind::Test,
            "then" => TokenKind::Then,
            "trait" => TokenKind::Trait,
            "true" => TokenKind::BoolLit(true),
            "type" => TokenKind::Type,
            "unsafe" => TokenKind::Unsafe,
            "use" => TokenKind::Use,
            "vibe" => TokenKind::Vibe,
            "when" => TokenKind::When,
            "with" => TokenKind::With,
            "with_timeout" => TokenKind::WithTimeout,
            _ => {
                // PascalCase = TypeIdent, lower_snake = Ident
                if ident.chars().next().unwrap().is_uppercase() {
                    TokenKind::TypeIdent(ident)
                } else if ident == "_" {
                    TokenKind::Underscore
                } else {
                    TokenKind::Ident(ident)
                }
            }
        };

        Token {
            kind,
            span: self.span_from(start, start_line, start_col),
        }
    }

    fn lex_operator(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
    ) -> Result<Token, LexError> {
        let ch = self.advance().unwrap();
        let kind = match ch {
            '+' => {
                if self.peek() == Some('+') {
                    self.advance();
                    TokenKind::PlusPlus
                } else {
                    TokenKind::Plus
                }
            }
            '-' => {
                if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '=' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::EqEq
                } else if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::FatArrow
                } else {
                    TokenKind::Eq
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::BangEq
                } else {
                    TokenKind::Bang
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::LtEq
                } else if self.peek() == Some('<') {
                    self.advance();
                    TokenKind::LtLt
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::GtEq
                } else if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::GtGt
                } else {
                    TokenKind::Gt
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    TokenKind::AmpAmp
                } else {
                    TokenKind::Amp
                }
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    TokenKind::PipePipe
                } else if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::PipeGt
                } else {
                    TokenKind::Pipe
                }
            }
            '^' => TokenKind::Caret,
            '~' => TokenKind::Tilde,
            ':' => {
                if self.peek() == Some(':') {
                    self.advance();
                    TokenKind::ColonColon
                } else {
                    TokenKind::Colon
                }
            }
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            '\\' => TokenKind::Backslash,
            _ => return Err(LexError::UnexpectedChar(ch, start_line, start_col)),
        };

        Ok(Token {
            kind,
            span: self.span_from(start, start_line, start_col),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keywords() {
        let tokens = lex("fn let if then else match do vibe").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Fn));
        assert!(matches!(tokens[1].kind, TokenKind::Let));
        assert!(matches!(tokens[2].kind, TokenKind::If));
        assert!(matches!(tokens[3].kind, TokenKind::Then));
        assert!(matches!(tokens[4].kind, TokenKind::Else));
        assert!(matches!(tokens[5].kind, TokenKind::Match));
        assert!(matches!(tokens[6].kind, TokenKind::Do));
        assert!(matches!(tokens[7].kind, TokenKind::Vibe));
    }

    #[test]
    fn test_numbers() {
        let tokens = lex("42 3.14 0xFF 0b1010 0o77 1_000").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::IntLit(42));
        assert_eq!(tokens[1].kind, TokenKind::FloatLit(3.14));
        assert_eq!(tokens[2].kind, TokenKind::IntLit(255));
        assert_eq!(tokens[3].kind, TokenKind::IntLit(10));
        assert_eq!(tokens[4].kind, TokenKind::IntLit(63));
        assert_eq!(tokens[5].kind, TokenKind::IntLit(1000));
    }

    #[test]
    fn test_string() {
        let tokens = lex(r#""hello world""#).unwrap();
        assert_eq!(tokens[0].kind, TokenKind::StringLit("hello world".into()));
    }

    #[test]
    fn test_operators() {
        let tokens = lex("|> ++ -> == != <= >= && ||").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::PipeGt));
        assert!(matches!(tokens[1].kind, TokenKind::PlusPlus));
        assert!(matches!(tokens[2].kind, TokenKind::Arrow));
        assert!(matches!(tokens[3].kind, TokenKind::EqEq));
        assert!(matches!(tokens[4].kind, TokenKind::BangEq));
        assert!(matches!(tokens[5].kind, TokenKind::LtEq));
        assert!(matches!(tokens[6].kind, TokenKind::GtEq));
        assert!(matches!(tokens[7].kind, TokenKind::AmpAmp));
        assert!(matches!(tokens[8].kind, TokenKind::PipePipe));
    }

    #[test]
    fn test_comments() {
        let tokens = lex("foo -- this is a comment\nbar").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident("foo".into()));
        assert_eq!(tokens[1].kind, TokenKind::Ident("bar".into()));
    }

    #[test]
    fn test_block_comment() {
        let tokens = lex("foo {- block comment -} bar").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident("foo".into()));
        assert_eq!(tokens[1].kind, TokenKind::Ident("bar".into()));
    }

    #[test]
    fn test_string_interpolation() {
        let tokens = lex(r#""hello ${name}""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringInterpStart(ref s) if s == "hello "));
        assert!(matches!(tokens[1].kind, TokenKind::Ident(ref s) if s == "name"));
        assert!(matches!(tokens[2].kind, TokenKind::StringInterpEnd(ref s) if s == ""));
    }

    #[test]
    fn test_string_interpolation_multiple() {
        let tokens = lex(r#""${a} and ${b}""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringInterpStart(ref s) if s == ""));
        assert!(matches!(tokens[1].kind, TokenKind::Ident(ref s) if s == "a"));
        assert!(matches!(tokens[2].kind, TokenKind::StringInterpPart(ref s) if s == " and "));
        assert!(matches!(tokens[3].kind, TokenKind::Ident(ref s) if s == "b"));
        assert!(matches!(tokens[4].kind, TokenKind::StringInterpEnd(ref s) if s == ""));
    }

    #[test]
    fn test_triple_quoted_string() {
        let tokens = lex(r#""""hello
world""""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s) if s == "hello\nworld"));
    }

    #[test]
    fn test_simple_function() {
        let tokens = lex("fn add(x: Int, y: Int) -> Int = x + y").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Fn));
        assert_eq!(tokens[1].kind, TokenKind::Ident("add".into()));
        assert!(matches!(tokens[2].kind, TokenKind::LParen));
        assert_eq!(tokens[3].kind, TokenKind::Ident("x".into()));
        assert!(matches!(tokens[4].kind, TokenKind::Colon));
        assert_eq!(tokens[5].kind, TokenKind::TypeIdent("Int".into()));
    }
}
