# VibeLang

A programming language designed to minimize the distance between intent and implementation.

VibeLang is expression-oriented, pattern-match-driven, and pipe-composed — optimized for how a reasoning agent naturally expresses computation.

## Key Features

- **Everything is an expression** — no statement/expression divide
- **Pattern matching** as the primary control flow (`match`)
- **Pipe operator** (`|>`) for left-to-right data transformation
- **Algebraic data types** — enums with associated data, structs
- **Type inference** — types inferred wherever possible
- **No null** — absence modeled with `Option[T]`
- **Minimal syntax** — no boilerplate, just the essential logic

## Quick Look

```
fn fib(n: i64) -> i64 {
    match n {
        0 -> 0,
        1 -> 1,
        n -> fib(n - 1) + fib(n - 2),
    }
}

fn main() {
    loop i in 0..10 {
        fib(i) |> println;
    };
}
```

## Status

- [x] Language specification ([SPEC.md](SPEC.md))
- [ ] Compiler (`vibec`) — x86_64 Linux ELF target

## File Extension

`.vibe`
