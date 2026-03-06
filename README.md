# VibeLang

A **purely functional** programming language designed to minimize the distance between intent and implementation.

VibeLang is expression-oriented, pattern-match-driven, and pipe-composed — optimized for how a reasoning agent naturally expresses computation. All values are immutable. Side effects are tracked in the type system via `IO`.

## Key Features

- **Purely functional** — all values immutable, functions referentially transparent
- **Everything is an expression** — no statement/expression divide
- **Pattern matching** as the primary control flow (`match`)
- **Pipe operator** (`|>`) for left-to-right data transformation
- **Algebraic data types** — enums with associated data, structs with functional update
- **Effect system** — IO tracked at the type level, pure and effectful code cleanly separated
- **Type inference** — Hindley-Milner; types inferred wherever possible
- **No null** — absence modeled with `Option[T]`
- **Guaranteed tail-call optimization** — recursion replaces loops without stack overflow risk
- **Persistent data structures** — lists share structure; reference counted

## Quick Look

```
fn quicksort(xs: List[i64]) -> List[i64] {
    match xs {
        [] -> [],
        pivot :: rest -> {
            let left = List.filter(rest, |x| x <= pivot);
            let right = List.filter(rest, |x| x > pivot);
            quicksort(left) ++ [pivot] ++ quicksort(right)
        },
    }
}

fn main() -> IO[unit] {
    effect {
        [3, 6, 1, 8, 2, 9, 4, 7, 5]
            |> quicksort
            |> List.map(_, to_string)
            |> List.map(_, println);
    }
}
```

## Status

- [x] Language specification ([SPEC.md](SPEC.md))
- [ ] Compiler (`vibec`) — x86_64 Linux ELF target

## File Extension

`.vibe`
