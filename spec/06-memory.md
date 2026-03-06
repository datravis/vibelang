# 6. Memory Management and Safety

VibeLang provides memory safety — no null pointer dereferences, no use-after-free,
no double-free, no buffer overflows, no data races — without requiring a garbage collector
or programmer-visible ownership annotations.

## 6.1 Memory Safety Guarantees

The following properties are enforced **statically at compile time**:

1. **No null references.** There is no null value. Optional values use `Option[A]`.
2. **No data races.** All data is immutable; concurrent access is safe by construction.
3. **No buffer overflows.** Array access is bounds-checked (with an unsafe escape hatch
   in the `unsafe` module for performance-critical code).

The following properties are enforced by the **compiler's memory management strategy**:

4. **No use-after-free.** The compiler determines value lifetimes; memory is never accessed
   after it is freed.
5. **No double-free.** Each allocation is freed exactly once.
6. **No memory leaks.** All memory is reclaimed deterministically (no GC pauses).

## 6.2 How Memory Works (Compiler-Managed)

Since all data in VibeLang is immutable, the compiler has significant freedom in choosing
how to manage memory. **The programmer does not annotate ownership, borrowing, or lifetimes.**
Instead, the compiler uses a layered strategy:

### Strategy 1: Region Inference (Primary)

The compiler performs whole-program region inference (inspired by MLKit). Each allocation is
assigned to a **region** — a contiguous block of memory that is freed all at once when
execution leaves its scope.

```
fn process(items: List[Item]) -> Summary = do
    -- The compiler infers that `intermediates` does not escape this function.
    -- It allocates into a function-local region and frees everything on return.
    let intermediates = map(items, fn(item) = compute(item))
    let aggregated = reduce(intermediates, combine)
    summarize(aggregated)
    -- `intermediates` memory is freed here automatically
```

The programmer writes natural functional code. The compiler figures out lifetimes.

Benefits:
- **Allocation is O(1)** — bump a pointer in the current region.
- **Deallocation is O(1)** — free the entire region at once.
- **Cache-friendly** — sequential allocation produces sequential memory layout.
- **No programmer burden** — no ownership annotations, no borrow checker.

### Strategy 2: Reference Counting (Fallback)

When the compiler cannot statically determine a value's lifetime — for example, when a
value is stored in a data structure with a dynamic lifetime, or shared across concurrent
tasks — it falls back to **atomic reference counting**.

This is safe because:
- Immutable data cannot create reference cycles (no mutable back-pointers).
- Atomic refcounting is thread-safe for concurrent access.

The compiler minimizes refcount overhead through:
- **Elision**: Skipping refcount increments/decrements when the compiler can prove they
  are unnecessary (e.g., the last use of a value).
- **Move optimization**: When a value is used exactly once after binding, the compiler
  transfers it without touching the refcount.
- **Batching**: Combining multiple refcount operations into one.

### Strategy 3: Stack Allocation

Small value types and values that don't escape their scope are stack-allocated:

- All primitives (`Bool`, `Int`, `Float`, `Byte`, `Char`)
- Small fixed-size records where all fields are stack-allocatable
- Function arguments that are not captured by closures

The compiler's escape analysis determines this automatically.

### How the Strategies Interact

```
fn example() = do
    let x = 42                    -- stack allocated (primitive)
    let point = { x: 1.0, y: 2.0 }  -- stack allocated (small, doesn't escape)
    let items = [1, 2, 3, 4, 5]  -- region allocated (list in function-local region)
    let shared = spawn_task(items) -- refcounted (escapes into concurrent task)
    ...
```

The programmer never chooses. The compiler always picks the most efficient strategy.

## 6.3 Explicit Regions (Optional, for Performance)

While the compiler infers regions automatically, programmers can declare explicit regions
for fine-grained control in performance-critical code:

```
fn process_batch(items: List[Item]) -> Summary = do
    region scratch = do
        -- All allocations in this block use the `scratch` region
        let intermediates = map(items, fn(item) = compute(item))
        let aggregated = reduce(intermediates, combine)
        summarize(aggregated)
    -- Everything in `scratch` is freed here, all at once
```

This is an optimization hint, not a correctness requirement. The compiler verifies that
no references to region-allocated data escape the region.

## 6.4 Arrays (Contiguous Memory)

For performance-critical code, VibeLang provides heap-allocated contiguous arrays:

```
type Array[A]           -- heap-allocated, contiguous, fixed-size after creation
type Slice[A]           -- a view into an Array (start, end, pointer to Array)

fn create[A](size: UInt, default: A) -> Array[A]
fn from_list[A](items: List[A]) -> Array[A]
fn get[A](arr: Array[A], index: UInt) -> Option[A]
fn get_unchecked[A](arr: Array[A], index: UInt) -> A  -- unsafe: no bounds check
fn length[A](arr: Array[A]) -> UInt
fn slice[A](arr: Array[A], start: UInt, end: UInt) -> Slice[A]

-- Functional update: returns a new array with one element replaced
fn set[A](arr: Array[A], index: UInt, value: A) -> Array[A]
```

Since data is immutable and the compiler manages memory, arrays can be freely shared
across functions and concurrent tasks without any annotation.

### Persistent Data Structures

For efficient functional updates, the standard library provides persistent (immutable,
structurally shared) data structures:

```
type Vec[A]             -- persistent vector (RRB-tree-based), O(log32 n) operations
type Map[K, V]          -- persistent hash map (HAMT), O(log32 n) operations
type Set[A]             -- persistent hash set
type Queue[A]           -- persistent double-ended queue
```

Structural sharing means that "updating" a persistent data structure reuses most of the
existing memory — only the changed path is newly allocated.

## 6.5 Stack vs Heap — Compiler Decides

The compiler automatically decides allocation strategy:

- **Stack-allocated**: Small value types, fixed-size records, function arguments.
- **Region-allocated**: Values whose lifetimes can be statically determined.
- **Refcounted**: Values with dynamic lifetimes, shared across tasks.

The programmer does not manually choose. The compiler's escape analysis and region
inference determine the optimal placement.

## 6.6 Resource Management

For resources that need cleanup (file handles, network connections), use the `Resource`
effect. This ensures deterministic cleanup regardless of how memory is managed:

```
effect Resource {
    fn acquire[A](open: fn() -> A with IO, close: fn(A) -> Unit with IO) -> A
}

fn with_file(path: String) -> String with Resource, IO = do
    let handle = acquire(
        fn() = open_file(path),
        fn(h) = close_file(h),
    )
    read_all(handle)
```

The `Resource` handler guarantees that `close` is called even if the computation fails,
similar to Python's context managers or Java's try-with-resources.

## 6.7 Unsafe Escape Hatch

For performance-critical code that needs to bypass safety checks, VibeLang provides an
`unsafe` module with unchecked operations:

```
use core.unsafe

fn fast_sum(arr: Array[Float]) -> Float = do
    let len = length(arr)
    let rec loop(i: UInt, acc: Float) -> Float =
        if i == len then acc
        else loop(i + 1, acc + unsafe.get_unchecked(arr, i))
    loop(0, 0.0)
```

Using `unsafe` operations requires the calling function to be marked `unsafe` or wrapped
in an `unsafe` block:

```
unsafe fn fast_sum(arr: Array[Float]) -> Float = ...

-- or --

fn fast_sum(arr: Array[Float]) -> Float = do
    unsafe do
        -- unchecked operations allowed here
        ...
```

The compiler emits a warning for every `unsafe` usage to encourage safe alternatives.

## 6.8 Why Not a Garbage Collector?

A tracing GC would be simpler to implement but conflicts with VibeLang's goals:

- **Latency**: GC pauses are unacceptable for real-time stream processing.
- **Parallelism**: GC synchronization adds overhead to parallel workloads.
- **Predictability**: Region inference and refcounting provide deterministic deallocation.
- **Memory efficiency**: No need to keep dead objects around until the next GC cycle.

The combination of immutability + region inference + refcounting achieves GC-level
programmer ergonomics with manual-memory-management-level performance.
