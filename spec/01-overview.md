# VibeLang Specification v0.1

## 1. Overview

**VibeLang** is a purely functional, natively compiled programming language designed for
high-performance concurrent and parallel data processing. It combines compiler-managed
memory safety (inspired by MLKit and Rust's guarantees) with the composability and
reasoning clarity of purely functional languages (inspired by Haskell, Erlang, and ML).

VibeLang is explicitly designed to be **written and read by both humans and LLMs**. Its
syntax favors regularity, explicitness, and unambiguous structure over brevity or cleverness.

### 1.1 Design Principles

1. **Pure by default, effects by declaration.** All functions are pure unless they declare
   an effect signature. Side effects are tracked in the type system via algebraic effects.

2. **Immutable data, automatic memory.** All values are immutable. Since nothing is
   mutated, values can be freely shared without aliasing concerns. The compiler manages
   memory via region inference and reference counting — no GC, no ownership annotations.

3. **Explicit parallelism, fearless concurrency.** Parallel computation is expressed through
   the `vibe` keyword (concurrent data pipelines) and structured combinators (`par`, `pmap`,
   `race`), not threads and locks. The runtime schedules work onto OS threads via
   work-stealing.

4. **Regularity over clevity.** Every construct has exactly one way to express it. No
   operator overloading, no implicit conversions, no method resolution order ambiguity.
   LLMs and humans should be able to predict what code does from its syntax alone.

5. **Native compilation.** VibeLang compiles to native machine code via LLVM. There is no
   interpreter, no VM, no GC. Memory is managed through compiler-inferred regions and
   automatic reference counting.

6. **Turing complete.** VibeLang supports unbounded recursion and is fully Turing complete.
   Recursive data types and recursive functions are first-class.

### 1.2 Target Use Cases

- Large-scale data transformation pipelines
- Concurrent service backends
- Stream processing and real-time analytics
- Scientific and numerical computation
- LLM-assisted code generation and transformation

### 1.3 Non-Goals

- Object-oriented programming (no classes, no inheritance, no method dispatch)
- Untracked side effects (no hidden I/O, no global mutable state)
- Scripting / REPL-first workflows (compilation is the primary mode)
- C FFI in v0.1 (planned for v0.2)
