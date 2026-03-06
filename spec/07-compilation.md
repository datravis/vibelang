# 7. Compilation Model

VibeLang is a natively compiled language. There is no interpreter, no virtual machine,
and no garbage collector in the final executable.

## 7.1 Compilation Pipeline

```
Source (.vibe)
    │
    ▼
┌──────────┐
│  Parser   │  ──▶  Concrete Syntax Tree
└──────────┘
    │
    ▼
┌──────────────┐
│ Type Checker  │  ──▶  Typed AST + Effect annotations
│ & Inference   │       + Region inference
└──────────────┘
    │
    ▼
┌──────────────┐
│  Optimizer    │  ──▶  Optimized IR
│  (VibeLang)  │       - Inlining
│              │       - Dead code elimination
│              │       - Effect handler fusion
│              │       - Tail call optimization
│              │       - Deforestation
│              │       - Region inference
└──────────────┘
    │
    ▼
┌──────────────┐
│  LLVM IR Gen  │  ──▶  LLVM IR
└──────────────┘
    │
    ▼
┌──────────────┐
│  LLVM Backend │  ──▶  Native Machine Code
│  (opt + llc)  │       (.o → linked executable)
└──────────────┘
```

## 7.2 Key Optimizations

### Tail Call Optimization

All tail-recursive functions are optimized to loops, enabling idiomatic recursive code
without stack overflow:

```
fn sum(list: List[Int], acc: Int) -> Int =
    match list
    | Nil -> acc
    | Cons(x, rest) -> sum(rest, acc + x)  -- tail position → compiled as loop
```

### Deforestation (Stream Fusion)

Chains of list/stream operations are fused into a single pass, eliminating intermediate
allocations:

```
-- This produces ZERO intermediate lists at runtime
let result = numbers
    |> map(fn(x) = x * 2)
    |> filter(fn(x) = x > 10)
    |> take(100)
    |> fold(0, fn(a, b) = a + b)
```

### Effect Handler Inlining

When effect handlers are statically known, the handler dispatch is inlined, reducing
the overhead to zero:

```
-- The State effect compiles down to a local variable, not a heap allocation
handle computation()
    with State[Int] { ... }
```

### Monomorphization

Generic functions are specialized to their concrete type arguments at compile time,
enabling type-specific optimizations:

```
fn max[A: Ord](a: A, b: A) -> A = if compare(a, b) == Greater then a else b

-- Compiled as separate optimized functions:
-- max_int(a: Int, b: Int) -> Int
-- max_float(a: Float, b: Float) -> Float
```

### Escape Analysis

The compiler determines whether values escape their defining scope:
- **Non-escaping values** → stack-allocated
- **Escaping values** → heap-allocated
- **Region-bound values** → region-allocated

## 7.3 Build System

### Project Structure

```
my_project/
├── vibe.toml          -- project manifest
├── src/
│   ├── main.vibe      -- entry point
│   ├── parser.vibe
│   └── utils/
│       ├── string.vibe
│       └── math.vibe
├── test/
│   ├── parser_test.vibe
│   └── utils_test.vibe
└── deps/              -- downloaded dependencies
```

### Project Manifest (vibe.toml)

```toml
[project]
name = "my_project"
version = "0.1.0"
edition = "2026"

[dependencies]
json = "1.2.0"
http = "0.5.0"

[build]
target = "native"         # or "x86_64-linux", "aarch64-macos", etc.
optimization = "release"  # "debug", "release", "size"
lto = true                # link-time optimization

[test]
parallel = true
```

### Build Commands

```bash
vibe build              # compile the project
vibe run                # compile and run
vibe test               # compile and run tests
vibe check              # type-check without generating code
vibe fmt                # format source code
vibe doc                # generate documentation
```

## 7.4 Separate Compilation

Each module is compiled independently. The compiler produces:
- An object file (`.o`) with the compiled code
- A module interface file (`.vibei`) with type signatures and effect annotations

This enables incremental compilation — only changed modules are recompiled.

## 7.5 Link-Time Optimization (LTO)

When LTO is enabled, the LLVM backend performs cross-module optimizations including:
- Cross-module inlining
- Whole-program dead code elimination
- Interprocedural constant propagation

## 7.6 Debug Mode

In debug mode, the compiler:
- Preserves all bounds checks
- Includes debug symbols
- Disables optimizations for faster compilation
- Enables runtime assertions
- Produces human-readable error messages with source locations

## 7.7 Cross-Compilation

VibeLang supports cross-compilation to any target supported by LLVM:

```bash
vibe build --target x86_64-unknown-linux-gnu
vibe build --target aarch64-apple-darwin
vibe build --target wasm32-wasi
```
