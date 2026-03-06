# VibeLang

A purely functional, natively compiled programming language with memory safety guarantees.

Designed for high-performance concurrent and parallel data processing, optimized for
both human readability and LLM-assisted development.

## Quick Look

```vibe
module main

use core.concurrency.{pmap, preduce}
use core.collections.Map

fn count_words(text: String) -> Map[String, UInt] =
    text
        |> to_lower
        |> split(" ")
        |> fold(Map.empty(), fn(acc, word) =
            Map.insert(acc, word, Map.get(ref acc, ref word) |> unwrap_or(0) + 1)
        )

fn main() -> Unit with IO = do
    let documents = read_lines("corpus.txt") |> collect
    let word_counts = documents
        |> pmap(count_words)
        |> preduce(Map.empty(), fn(a, b) = Map.merge(a, b, fn(x, y) = x + y))
    print("Total unique words: ${show(Map.size(ref word_counts))}")
```

## Key Features

- **Purely functional** — immutable data, algebraic effects for side effects
- **Memory safe** — ownership-based, no GC, no null, no data races
- **Natively compiled** — LLVM backend, zero-cost abstractions
- **Parallel by design** — structured concurrency with `par`, `pmap`, streams
- **LLM-optimized** — regular syntax, explicit semantics, structured annotations

## Specification

See [spec/](spec/README.md) for the full language specification.

## Status

VibeLang is in the design phase. The language specification is v0.1.
