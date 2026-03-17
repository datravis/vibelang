//! Unit-level tests for the VibeLang lexer.

use vibelang::lexer::{self, TokenKind};

fn kinds(src: &str) -> Vec<TokenKind> {
    lexer::lex(src)
        .expect("lex failed")
        .into_iter()
        .map(|t| t.kind)
        .filter(|k| *k != TokenKind::Eof)
        .collect()
}

#[test]
fn lex_integer_literals() {
    let k = kinds("0 1 42 1000000");
    assert_eq!(
        k,
        vec![
            TokenKind::IntLit(0),
            TokenKind::IntLit(1),
            TokenKind::IntLit(42),
            TokenKind::IntLit(1000000),
        ]
    );
}

#[test]
fn lex_string_literal() {
    let k = kinds("\"hello world\"");
    assert_eq!(k, vec![TokenKind::StringLit("hello world".into())]);
}

#[test]
fn lex_keywords() {
    let k = kinds("fn let if then else match do");
    let names: Vec<&str> = k
        .iter()
        .map(|t| match t {
            TokenKind::Fn => "fn",
            TokenKind::Let => "let",
            TokenKind::If => "if",
            TokenKind::Then => "then",
            TokenKind::Else => "else",
            TokenKind::Match => "match",
            TokenKind::Do => "do",
            _ => panic!("unexpected token: {:?}", t),
        })
        .collect();
    assert_eq!(names, vec!["fn", "let", "if", "then", "else", "match", "do"]);
}

#[test]
fn lex_operators() {
    let k = kinds("+ - * / == != <= >= |>");
    assert_eq!(
        k,
        vec![
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::EqEq,
            TokenKind::BangEq,
            TokenKind::LtEq,
            TokenKind::GtEq,
            TokenKind::PipeGt,
        ]
    );
}

#[test]
fn lex_arrow_and_equals() {
    let k = kinds("-> =");
    assert_eq!(k, vec![TokenKind::Arrow, TokenKind::Eq]);
}

#[test]
fn lex_empty_input() {
    let k = kinds("");
    assert!(k.is_empty());
}

#[test]
fn lex_comments_ignored() {
    let k = kinds("-- this is a comment\n42");
    assert_eq!(k, vec![TokenKind::IntLit(42)]);
}

#[test]
fn lex_bool_literals() {
    let k = kinds("true false");
    assert_eq!(k, vec![TokenKind::BoolLit(true), TokenKind::BoolLit(false)]);
}

#[test]
fn lex_identifiers_vs_type_idents() {
    let k = kinds("foo Bar baz Option");
    assert_eq!(
        k,
        vec![
            TokenKind::Ident("foo".into()),
            TokenKind::TypeIdent("Bar".into()),
            TokenKind::Ident("baz".into()),
            TokenKind::TypeIdent("Option".into()),
        ]
    );
}

#[test]
fn lex_hex_literals() {
    let k = kinds("0xFF 0x1A 0xDEAD");
    assert_eq!(
        k,
        vec![
            TokenKind::IntLit(0xFF),
            TokenKind::IntLit(0x1A),
            TokenKind::IntLit(0xDEAD),
        ]
    );
}

#[test]
fn lex_binary_literals() {
    let k = kinds("0b1010 0b11110000 0b0");
    assert_eq!(
        k,
        vec![
            TokenKind::IntLit(0b1010),
            TokenKind::IntLit(0b11110000),
            TokenKind::IntLit(0b0),
        ]
    );
}

#[test]
fn lex_octal_literals() {
    let k = kinds("0o77 0o755 0o0");
    assert_eq!(
        k,
        vec![
            TokenKind::IntLit(0o77),
            TokenKind::IntLit(0o755),
            TokenKind::IntLit(0o0),
        ]
    );
}

#[test]
fn lex_underscored_numbers() {
    let k = kinds("1_000_000 0xFF_FF 0b1111_0000");
    assert_eq!(
        k,
        vec![
            TokenKind::IntLit(1_000_000),
            TokenKind::IntLit(0xFFFF),
            TokenKind::IntLit(0b11110000),
        ]
    );
}

#[test]
fn lex_nominal_keyword() {
    let k = kinds("nominal type");
    assert_eq!(k, vec![TokenKind::Nominal, TokenKind::Type]);
}
