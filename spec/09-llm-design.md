# 9. LLM-Oriented Design

VibeLang is explicitly designed to be **generated, read, and transformed by large language
models** while remaining natural for human programmers. This section describes the design
decisions that support LLM interaction.

## 9.1 Design Principles for LLM Compatibility

### Regularity

Every construct has **exactly one canonical form**. There are no alternative syntaxes,
no optional semicolons, no ambiguous whitespace rules, and no context-dependent parsing.

This means:
- An LLM can generate valid VibeLang by learning a small, consistent grammar.
- Code transformation (refactoring, optimization) is straightforward — there's only one
  way to express any given construct.

### Explicitness

VibeLang avoids implicit behavior that requires deep contextual understanding:
- No implicit conversions between types
- No implicit function calls (no `Deref`, no implicit `toString`)
- No operator overloading (operators have fixed meanings)
- No method resolution order or dynamic dispatch
- No macros or compile-time metaprogramming in v0.1

### Composability

The pipe (`|>`) and compose (`>>`) operators make data flow explicit and linear — ideal
for LLM generation since the transformation pipeline reads left-to-right, top-to-bottom:

```
let result = raw_data
    |> parse
    |> validate
    |> transform
    |> encode
```

An LLM can easily append, remove, or reorder stages in this pipeline.

### Predictable Scoping

- All bindings are immutable — an LLM never needs to trace mutation.
- Ownership is explicit — an LLM can verify resource lifetimes by reading signatures.
- Effects are declared in types — an LLM can determine what a function does (I/O, state,
  failure) from its type signature alone.

## 9.2 Structured Annotations

VibeLang supports structured annotations that LLMs can use for code generation guidance:

```
@purpose("Validate user input and return sanitized data")
@precondition("input is non-empty UTF-8 string")
@postcondition("returned string contains no HTML entities")
fn sanitize(input: String) -> Result[String, ValidationError] = ...
```

Annotations are:
- Preserved in compiled module interfaces
- Available to tooling and documentation generators
- Ignored by the compiler (they don't affect semantics)
- Structured as key-value pairs for machine readability

### Standard Annotations

```
@purpose(description: String)          -- what the function does
@precondition(condition: String)       -- what must be true before calling
@postcondition(condition: String)      -- what is true after calling
@example(code: String)                 -- usage example
@complexity(time: String, space: String)  -- algorithmic complexity
@deprecated(reason: String, since: String, replacement: String)
@todo(description: String)            -- unfinished work
@safety(justification: String)        -- for unsafe blocks
```

## 9.3 Error Messages

Compiler errors are designed to be actionable by both humans and LLMs:

```
error[E0301]: type mismatch
  --> src/main.vibe:15:12
   |
15 |     let x: Int = "hello"
   |            ^^^   ^^^^^^^ expected `Int`, found `String`
   |
   = help: convert with `parse_int("hello")` which returns `Option[Int]`
   = note: String cannot be implicitly converted to Int
```

Error messages include:
- A unique error code for programmatic handling
- The exact source location
- A clear description of what went wrong
- A concrete suggestion for how to fix it

## 9.4 Canonical Formatting

VibeLang has a single canonical code format enforced by `vibe fmt`. There are no style
options or configuration — all VibeLang code looks the same:

- 4-space indentation
- No trailing whitespace
- One blank line between top-level declarations
- Consistent brace and keyword placement

This eliminates formatting as a variable, making LLM-generated code consistent with
human-written code.

## 9.5 Module Interface as Context

Module interface files (`.vibei`) provide a compact summary of a module's public API:

```
-- math_utils.vibei (auto-generated)
module math_utils

pub fn lerp(a: Float, b: Float, t: Float) -> Float
pub fn clamp(value: Float, min: Float, max: Float) -> Float
pub fn distance(p1: Point, p2: Point) -> Float

pub type Point = { x: Float, y: Float }
```

These files are ideal for providing an LLM with context about available APIs without
requiring it to read full implementations.

## 9.6 Code Generation Patterns

VibeLang's design supports common LLM code generation patterns:

### Fill-in-the-Blank

```
fn process_orders(orders: List[Order]) -> Summary = do
    let valid_orders = filter(orders, fn(o) = o.status == Active)
    let totals = map(valid_orders, fn(o) = todo("calculate order total"))
    let grand_total = fold(totals, 0.0, fn(a, b) = a + b)
    { count: length(valid_orders), total: grand_total }
```

The `todo()` function serves as a typed placeholder that an LLM can fill in.

### Type-Driven Generation

Given a type signature, an LLM can generate the implementation:

```
-- Given this signature:
fn merge_sorted[A: Ord](a: List[A], b: List[A]) -> List[A]

-- An LLM can generate:
fn merge_sorted[A: Ord](a: List[A], b: List[A]) -> List[A] =
    match (a, b)
    | (Nil, _) -> b
    | (_, Nil) -> a
    | (Cons(x, xs), Cons(y, ys)) ->
        if compare(x, y) == Less
        then Cons(x, merge_sorted(xs, b))
        else Cons(y, merge_sorted(a, ys))
```

### Effect-Guided Generation

Effect annotations tell the LLM exactly what capabilities a function needs:

```
-- This function can do I/O and fail — the LLM knows to use print, read_file, fail
fn load_config(path: String) -> Config with IO, Fail[ConfigError]

-- This function is pure — the LLM knows not to use any effects
fn validate_config(config: Config) -> Result[Config, ValidationError]
```
