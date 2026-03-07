# VibeLang

A purely functional, natively compiled programming language with memory safety guarantees.

Designed for high-performance concurrent and parallel data processing, optimized for
both human readability and LLM-assisted development.

## Quick Look

```vibe
module main

type Color = | Red | Green | Blue

fn fib(n: Int) -> Int =
    if n <= 1 then n
    else fib(n - 1) + fib(n - 2)

fn double(x: Int) -> Int = x * 2
fn add_one(x: Int) -> Int = x + 1

fn main() -> Int = do
    let result = 10 |> double |> add_one
    print(result)
    print(fib(30))
    0
```

## Key Features

- **Purely functional** — immutable data, algebraic effects for side effects
- **Memory safe** — compiler-managed (region inference + refcounting), no GC, no null, no data races
- **Natively compiled** — LLVM backend with full optimization passes (O0–O3)
- **Tail-call optimization** — recursive loops compile to efficient machine loops
- **Parallel by design** — structured concurrency with `par`, `pmap`, streams
- **Tiny binaries** — ~15 KB executables (245x smaller than Rust, 139x smaller than Go)
- **LLM-optimized** — regular syntax, explicit semantics, structured annotations

## Getting Started

### Prerequisites

- **Rust** (1.70+) — to build the compiler
- **LLVM 18** — the codegen backend
- **A C linker** (`cc` / `gcc` / `clang`) — to link object files

On Ubuntu/Debian:

```bash
# Install LLVM 18
sudo apt-get install llvm-18-dev libpolly-18-dev

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Building the Compiler

```bash
LLVM_SYS_180_PREFIX=/usr/lib/llvm-18 cargo build --manifest-path compiler/Cargo.toml --release
```

The compiler binary will be at `compiler/target/release/vibe`.

### Hello World

Create `hello.vibe`:

```vibe
module main

fn main() -> Int = do
    print("Hello from VibeLang!")
    0
```

Compile and run:

```bash
# Compile to object file (defaults to -O2 optimization)
vibe build --target x86_64-unknown-linux-gnu hello.vibe -o hello.o

# Link
cc hello.o -o hello

# Run
./hello
```

Or use the JIT runner:

```bash
vibe run hello.vibe
```

## Language Guide

### Modules

Every file starts with a module declaration:

```vibe
module main
```

### Functions

Functions are expressions. The body is a single expression after `=`:

```vibe
fn square(x: Int) -> Int = x * x

fn max(a: Int, b: Int) -> Int =
    if a >= b then a else b
```

### Types

VibeLang has algebraic data types and pattern matching:

```vibe
type Option[A] = | Some(A) | None
type Color = | Red | Green | Blue

fn describe(x: Int) -> String =
    match x
        | 0 -> "zero"
        | 1 -> "one"
        | _ -> "many"
```

Built-in types: `Int`, `Float`, `Bool`, `String`, `Char`, `Unit`

### Pipeline Operator

Chain function calls left-to-right with `|>`:

```vibe
fn double(x: Int) -> Int = x * 2
fn add_one(x: Int) -> Int = x + 1
fn square(x: Int) -> Int = x * x

fn main() -> Int = do
    let result = 5 |> double |> add_one |> square
    print(result)   // 121
    0
```

### Do Blocks

Side effects (like printing) go in `do` blocks:

```vibe
fn main() -> Int = do
    let x = 42
    let y = x + 1
    print(y)
    0
```

### Recursion with TCO

Tail-recursive functions are automatically optimized into loops:

```vibe
fn sum_to(n: Int, acc: Int) -> Int =
    if n <= 0 then acc
    else sum_to(n - 1, acc + n)

fn main() -> Int = do
    print(sum_to(10000000, 0))  // no stack overflow
    0
```

## Compiler Usage

```
Usage: vibe <COMMAND>

Commands:
  build    Compile a VibeLang source file
  check    Type-check without compiling
  run      JIT compile and run
  lex      Print tokens (debug)
  parse    Print AST (debug)
  targets  List supported targets
```

### Build Options

```
vibe build [OPTIONS] <FILE>

Options:
  -o, --output <OUTPUT>         Output file path
      --target <TARGET>         Target triple [default: aarch64-apple-darwin]
      --emit-ir                 Emit LLVM IR instead of object code
  -O, --opt-level <OPT_LEVEL>  Optimization level: 0-3 [default: 2]
```

### Optimization Levels

| Flag | Pipeline | What it does |
|------|----------|-------------|
| `-O0` | None | Raw unoptimized IR, fastest compile |
| `-O1` | `default<O1>` | Basic optimizations (inlining, simplification) |
| `-O2` | `default<O2>` | Full optimizations + vectorization (recommended) |
| `-O3` | `default<O3>` | Aggressive optimizations (may increase code size) |

### Cross-Compilation Targets

```bash
vibe build --target aarch64-apple-darwin hello.vibe       # macOS ARM
vibe build --target x86_64-unknown-linux-gnu hello.vibe   # Linux x86-64
vibe build --target x86_64-pc-windows-msvc hello.vibe     # Windows x86-64
```

## Benchmarks

Run the benchmark suite comparing VibeLang against Rust, Go, and Python:

```bash
# Install benchmark dependencies
./benchmarks/setup.sh

# Run benchmarks (default: 5 iterations)
./benchmarks/bench.sh

# Run with custom iteration count
./benchmarks/bench.sh 10
```

### Sample Results (x86-64 Linux, -O2)

| Program | Vibe | Rust | Go | Python |
|---------|------|------|----|--------|
| fibonacci(40) | 300ms | 300ms | 570ms | 17,360ms |
| factorial (10M iters) | <1ms | <1ms | 270ms | 14,530ms |
| pipeline (10M iters) | <1ms | 10ms | 10ms | 2,910ms |

| | Vibe | Rust | Go |
|---|------|------|----|
| Binary size | **15.5 KB** | 3,801 KB | 2,157 KB |
| Peak memory | **3.5 MB** | 4.0 MB | 10.7 MB |

## Examples

See the [`examples/`](examples/) directory:

- [`hello.vibe`](examples/hello.vibe) — Hello world
- [`fibonacci.vibe`](examples/fibonacci.vibe) — Recursive Fibonacci
- [`factorial.vibe`](examples/factorial.vibe) — Factorial with iterative loop
- [`pipeline.vibe`](examples/pipeline.vibe) — Pipeline operator with function chaining
- [`types.vibe`](examples/types.vibe) — Algebraic types and pattern matching

## Specification

See [spec/](spec/README.md) for the full language specification.

## Project Structure

```
vibelang/
├── compiler/           Rust compiler implementation
│   └── src/
│       ├── main.rs     CLI entry point
│       ├── lexer.rs    Tokenizer
│       ├── parser.rs   Parser -> AST
│       ├── ast.rs      AST definitions
│       ├── types.rs    Type checker
│       └── codegen.rs  LLVM IR generation + optimization
├── examples/           Example VibeLang programs
├── benchmarks/         Performance benchmarks vs Rust/Go/Python
│   ├── bench.sh        Benchmark runner
│   ├── setup.sh        Dependency installer
│   ├── rust/           Rust implementations
│   ├── go/             Go implementations
│   └── python/         Python implementations
└── spec/               Language specification
```

## Status

VibeLang is in active development. The compiler supports:

- [x] Lexer and parser
- [x] Type checking
- [x] LLVM codegen with optimization passes (O0-O3)
- [x] Tail-call optimization
- [x] JIT execution
- [x] Cross-compilation (macOS ARM, Linux x86-64, Windows x86-64)
- [ ] Standard library
- [ ] Memory management (regions + refcounting)
- [ ] Effect system
- [ ] Concurrency primitives
