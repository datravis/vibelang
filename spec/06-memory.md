# 6. Memory Management and Safety

VibeLang provides **Rust-level memory safety** — no null pointer dereferences, no
use-after-free, no double-free, no buffer overflows, no data races — without requiring a
garbage collector.

## 6.1 Memory Safety Guarantees

The following properties are enforced **statically at compile time**:

1. **No null references.** There is no null value. Optional values use `Option[A]`.
2. **No use-after-move.** Once a value is moved, the previous binding is invalidated.
3. **No double-free.** Each value has exactly one owner; it is freed exactly once.
4. **No dangling references.** `ref` borrows are lifetime-checked; they cannot outlive
   the referent.
5. **No data races.** All data is immutable; concurrent access is safe by construction.
6. **No buffer overflows.** Array access is bounds-checked (with an unsafe escape hatch
   in the `unsafe` module for performance-critical code).

## 6.2 Ownership Rules

1. Every value has exactly **one owner** at any point in time.
2. When the owner goes out of scope, the value is **dropped** (memory freed).
3. Ownership can be **transferred** (moved) to another binding or function.
4. **Borrowing** (`ref`) creates a temporary, non-owning reference.

```
fn example() = do
    let data = create_large_data()    -- `data` is the owner
    let processed = transform(data)   -- `data` is moved into `transform`
    -- `data` is no longer accessible here
    print(show(processed))            -- `processed` is moved into `show`
```

## 6.3 Borrowing

Borrowing allows read-only access to a value without taking ownership:

```
fn print_length(s: ref String) -> Unit with IO =
    print("Length: ${show(length(s))}")

fn example() -> Unit with IO = do
    let name = "VibeLang"
    print_length(ref name)    -- borrow `name`
    print(name)               -- `name` is still valid — it was only borrowed
```

### Borrow Rules

1. Multiple `ref` borrows can coexist (immutable aliasing is safe).
2. A `ref` borrow cannot outlive the owner's scope.
3. Moving a value while it is borrowed is a compile error.

```
fn bad_example() = do
    let data = create_data()
    let r = ref data
    let moved = data          -- COMPILE ERROR: `data` is borrowed by `r`
```

## 6.4 Shared Ownership

When multiple parts of the program need to co-own a value, use `share`:

```
let config = share(load_config())
-- config: share Config

let handler_a = create_handler(config)  -- refcount incremented
let handler_b = create_handler(config)  -- refcount incremented
-- config is freed when all three handles are dropped
```

`share` uses atomic reference counting, which is safe for concurrent access since the
underlying data is immutable.

## 6.5 Region-Based Allocation

For high-performance scenarios, VibeLang supports region-based allocation where many
values are allocated in a contiguous memory region and freed all at once:

```
fn process_batch(items: List[Item]) -> Summary = do
    region local = do
        -- All allocations in this block use the `local` region
        let intermediates = map(items, fn(item) = compute(item))
        let aggregated = reduce(intermediates, combine)
        -- `intermediates` are freed here when the region ends
        summarize(aggregated)
    -- The returned `Summary` is copied out of the region
```

Benefits:
- **Allocation is O(1)** — just bump a pointer.
- **Deallocation is O(1)** — free the entire region at once.
- **Cache-friendly** — sequential allocation means sequential memory layout.

## 6.6 Arrays (Contiguous Memory)

For performance-critical code, VibeLang provides stack-allocated fixed arrays and
heap-allocated dynamic arrays:

```
type Array[A]           -- heap-allocated, contiguous, fixed-size after creation
type Slice[A]           -- borrowed view into an Array

fn create[A](size: UInt, default: A) -> Array[A]
fn from_list[A](items: List[A]) -> Array[A]
fn get[A](arr: ref Array[A], index: UInt) -> Option[A]
fn get_unchecked[A](arr: ref Array[A], index: UInt) -> A  -- unsafe: no bounds check
fn length[A](arr: ref Array[A]) -> UInt
fn slice[A](arr: ref Array[A], start: UInt, end: UInt) -> Slice[A]

-- Functional update: returns a new array with one element replaced
fn set[A](arr: Array[A], index: UInt, value: A) -> Array[A]
```

### Persistent Data Structures

For efficient functional updates, the standard library provides persistent (immutable,
structurally shared) data structures:

```
type Vec[A]             -- persistent vector (HAMTrie-based), O(log32 n) operations
type Map[K, V]          -- persistent hash map, O(log32 n) operations
type Set[A]             -- persistent hash set
type Queue[A]           -- persistent double-ended queue
```

## 6.7 Stack vs Heap Allocation

The compiler automatically decides allocation strategy:

- **Stack-allocated**: Small value types, fixed-size records, function arguments.
- **Heap-allocated**: Large values, recursive data types, values that escape their scope.
- **Region-allocated**: Values within a `region` block.

The programmer does not manually choose stack vs heap. The compiler's escape analysis
determines the optimal placement.

## 6.8 Drop and Resource Management

Values are dropped (freed) deterministically when their owner goes out of scope. For
resources that need cleanup (file handles, network connections), use the `Resource` effect:

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
similar to Rust's `Drop` trait or Python's context managers.

## 6.9 Unsafe Escape Hatch

For performance-critical code that needs to bypass safety checks, VibeLang provides an
`unsafe` module with unchecked operations:

```
use core.unsafe

fn fast_sum(arr: ref Array[Float]) -> Float = do
    let len = length(arr)
    let rec loop(i: UInt, acc: Float) -> Float =
        if i == len then acc
        else loop(i + 1, acc + unsafe.get_unchecked(arr, i))
    loop(0, 0.0)
```

Using `unsafe` operations requires the calling function to be marked `unsafe` or wrapped
in an `unsafe` block:

```
unsafe fn fast_sum(arr: ref Array[Float]) -> Float = ...

-- or --

fn fast_sum(arr: ref Array[Float]) -> Float = do
    unsafe do
        -- unchecked operations allowed here
        ...
```

The compiler emits a warning for every `unsafe` usage to encourage safe alternatives.
