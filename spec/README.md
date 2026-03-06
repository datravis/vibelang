# VibeLang Language Specification v0.1

A purely functional, natively compiled language with memory safety guarantees, designed
for high-performance concurrent data processing and LLM-assisted development.

## Table of Contents

1. [Overview](01-overview.md) — Design principles, goals, and non-goals
2. [Type System](02-type-system.md) — Primitives, ADTs, generics, traits, ownership, row polymorphism
3. [Syntax](03-syntax.md) — Lexical structure, bindings, functions, pattern matching, modules
4. [Effects](04-effects.md) — Algebraic effects, handlers, effect polymorphism, purity
5. [Concurrency](05-concurrency.md) — Parallel combinators, streams, channels, actors, runtime model
6. [Memory](06-memory.md) — Ownership, borrowing, regions, arrays, resource management, unsafe
7. [Compilation](07-compilation.md) — Pipeline, optimizations, build system, cross-compilation
8. [Standard Library](08-stdlib.md) — Core modules, collections, I/O, testing
9. [LLM Design](09-llm-design.md) — Regularity, annotations, error messages, code generation patterns
10. [Examples](10-examples.md) — Complete programs: hello world to stream processing
11. [Grammar](11-grammar.md) — Formal EBNF grammar, operator precedence, reserved keywords

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Purity | Pure by default | Enables safe parallelism, memoization, and equational reasoning |
| Effects | Algebraic effects | Composable, no monad transformer stacks, handler-swappable |
| Memory | Ownership + regions | Deterministic, no GC, Rust-level safety without borrow complexity |
| Parallelism | Structured combinators | No data races by construction, declarative intent |
| Syntax | Regular, unambiguous | LLM-friendly generation, single canonical form |
| Compilation | LLVM native | Maximum performance, cross-platform support |
| Generics | Monomorphized | Zero-cost abstractions, type-specific optimizations |
