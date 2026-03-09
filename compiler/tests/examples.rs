//! Integration tests: lex, parse, and type-check every example program.
//!
//! These tests ensure the compiler front-end doesn't regress on valid programs.

use std::path::Path;
use vibelang::{lexer, parser, types};

fn lex_file(path: &Path) -> Vec<vibelang::lexer::Token> {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    lexer::lex(&source).unwrap_or_else(|e| panic!("lex error in {}: {e}", path.display()))
}

fn parse_file(path: &Path) -> vibelang::ast::Module {
    let tokens = lex_file(path);
    parser::parse(tokens).unwrap_or_else(|e| panic!("parse error in {}: {e}", path.display()))
}

fn typecheck_file(path: &Path) {
    let module = parse_file(path);
    types::check(&module).unwrap_or_else(|e| panic!("type error in {}: {e}", path.display()));
}

fn example(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples")
        .join(name)
}

// ── Lexer tests ──────────────────────────────────────────────────────

macro_rules! lex_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            let tokens = lex_file(&example($file));
            assert!(!tokens.is_empty(), "lexer produced no tokens for {}", $file);
        }
    };
}

lex_test!(lex_hello, "hello.vibe");
lex_test!(lex_fibonacci, "fibonacci.vibe");
lex_test!(lex_factorial, "factorial.vibe");
lex_test!(lex_closures, "closures.vibe");
lex_test!(lex_currying, "currying.vibe");
lex_test!(lex_effects, "effects.vibe");
lex_test!(lex_lists, "lists.vibe");
lex_test!(lex_memory, "memory.vibe");
lex_test!(lex_pipeline, "pipeline.vibe");
lex_test!(lex_pipeline_stages, "pipeline_stages.vibe");
lex_test!(lex_records, "records.vibe");
lex_test!(lex_record_update, "record_update.vibe");
lex_test!(lex_traits, "traits.vibe");
lex_test!(lex_tuples, "tuples.vibe");
lex_test!(lex_types, "types.vibe");
lex_test!(lex_variants, "variants.vibe");
lex_test!(lex_vibe_pipeline, "vibe_pipeline.vibe");
lex_test!(lex_concurrency, "concurrency.vibe");
lex_test!(lex_race, "race.vibe");
lex_test!(lex_pfilter_preduce, "pfilter_preduce.vibe");
lex_test!(lex_channels, "channels.vibe");

// ── Parser tests ─────────────────────────────────────────────────────

macro_rules! parse_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            let module = parse_file(&example($file));
            assert!(
                !module.declarations.is_empty(),
                "parser produced no declarations for {}",
                $file
            );
        }
    };
}

parse_test!(parse_hello, "hello.vibe");
parse_test!(parse_fibonacci, "fibonacci.vibe");
parse_test!(parse_factorial, "factorial.vibe");
parse_test!(parse_closures, "closures.vibe");
parse_test!(parse_currying, "currying.vibe");
parse_test!(parse_effects, "effects.vibe");
parse_test!(parse_lists, "lists.vibe");
parse_test!(parse_memory, "memory.vibe");
parse_test!(parse_pipeline, "pipeline.vibe");
parse_test!(parse_pipeline_stages, "pipeline_stages.vibe");
parse_test!(parse_records, "records.vibe");
parse_test!(parse_record_update, "record_update.vibe");
parse_test!(parse_traits, "traits.vibe");
parse_test!(parse_tuples, "tuples.vibe");
parse_test!(parse_types, "types.vibe");
parse_test!(parse_variants, "variants.vibe");
parse_test!(parse_vibe_pipeline, "vibe_pipeline.vibe");
parse_test!(parse_concurrency, "concurrency.vibe");
parse_test!(parse_race, "race.vibe");
parse_test!(parse_pfilter_preduce, "pfilter_preduce.vibe");
parse_test!(parse_channels, "channels.vibe");

// ── Type-checker tests ───────────────────────────────────────────────

macro_rules! typecheck_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            typecheck_file(&example($file));
        }
    };
}

typecheck_test!(typecheck_hello, "hello.vibe");
typecheck_test!(typecheck_fibonacci, "fibonacci.vibe");
typecheck_test!(typecheck_factorial, "factorial.vibe");
typecheck_test!(typecheck_closures, "closures.vibe");
typecheck_test!(typecheck_currying, "currying.vibe");
typecheck_test!(typecheck_effects, "effects.vibe");
typecheck_test!(typecheck_lists, "lists.vibe");
typecheck_test!(typecheck_memory, "memory.vibe");
typecheck_test!(typecheck_pipeline, "pipeline.vibe");
typecheck_test!(typecheck_pipeline_stages, "pipeline_stages.vibe");
typecheck_test!(typecheck_records, "records.vibe");
typecheck_test!(typecheck_record_update, "record_update.vibe");
typecheck_test!(typecheck_traits, "traits.vibe");
typecheck_test!(typecheck_tuples, "tuples.vibe");
typecheck_test!(typecheck_types, "types.vibe");
typecheck_test!(typecheck_variants, "variants.vibe");
typecheck_test!(typecheck_vibe_pipeline, "vibe_pipeline.vibe");
typecheck_test!(typecheck_concurrency, "concurrency.vibe");
typecheck_test!(typecheck_race, "race.vibe");
typecheck_test!(typecheck_pfilter_preduce, "pfilter_preduce.vibe");
typecheck_test!(typecheck_channels, "channels.vibe");
