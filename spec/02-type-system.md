# 2. Type System

VibeLang uses a **Hindley-Milner type system** extended with:
- Algebraic data types (sum and product types)
- Parametric polymorphism (generics)
- Algebraic effects and effect handlers
- Compiler-managed memory (region inference + refcounting)
- Row-polymorphic records

All types are inferred at the module level unless explicitly annotated. Type annotations are
required on all public (`pub`) function signatures.

## 2.1 Primitive Types

```
Bool            -- true, false
Int             -- 64-bit signed integer
UInt            -- 64-bit unsigned integer
Float           -- 64-bit IEEE 754
Byte            -- 8-bit unsigned
Char            -- Unicode scalar value (32-bit)
String          -- UTF-8 encoded, immutable, owned
Unit            -- the empty product type, written ()
Never           -- the empty type (uninhabited), for diverging functions
```

### Fixed-Width Variants

```
Int8, Int16, Int32, Int64, Int128
UInt8, UInt16, UInt32, UInt64, UInt128
Float32, Float64
```

## 2.2 Algebraic Data Types

### Product Types (Records)

```
type Point = { x: Float, y: Float }

type User = {
    name: String,
    age: UInt,
    email: String,
}
```

Records are structurally typed by default but can be made nominal with `nominal type`.

### Sum Types (Variants)

```
type Option[A] =
    | Some(A)
    | None

type Result[A, E] =
    | Ok(A)
    | Err(E)

type List[A] =
    | Cons(A, List[A])
    | Nil
```

### Newtype Wrappers

Newtypes provide zero-cost nominal wrappers:

```
newtype UserId = UInt
newtype Celsius = Float
```

## 2.3 Generics (Parametric Polymorphism)

Type parameters are written in square brackets:

```
fn identity[A](x: A) -> A = x

fn map[A, B](list: List[A], f: fn(A) -> B) -> List[B] =
    match list
    | Cons(head, tail) -> Cons(f(head), map(tail, f))
    | Nil -> Nil
```

## 2.4 Traits (Type Classes)

Traits define shared behavior across types:

```
trait Eq[A] {
    fn eq(a: A, b: A) -> Bool
}

trait Ord[A] requires Eq[A] {
    fn compare(a: A, b: A) -> Ordering
}

trait Show[A] {
    fn show(a: A) -> String
}

trait Hash[A] {
    fn hash(a: A) -> UInt64
}
```

Implementations are provided with `impl`:

```
impl Eq[Int] {
    fn eq(a: Int, b: Int) -> Bool = intrinsic_eq_int(a, b)
}

impl Show[Point] {
    fn show(p: Point) -> String =
        "Point(" ++ show(p.x) ++ ", " ++ show(p.y) ++ ")"
}
```

Trait bounds constrain generic parameters:

```
fn sort[A: Ord](list: List[A]) -> List[A] = ...

fn deduplicate[A: Eq + Hash](list: List[A]) -> List[A] = ...
```

## 2.5 Values and Memory

All values in VibeLang are **immutable**. There is no concept of programmer-visible
ownership, borrowing, or move semantics. Values can be freely shared, passed to
functions, and used multiple times:

```
let a = "hello"
let b = a           -- both `a` and `b` refer to the same string
print(a)            -- still valid
print(b)            -- also valid
```

The compiler manages memory automatically using region inference and reference counting
(see [Chapter 6: Memory](06-memory.md)). The programmer never annotates lifetimes or
ownership — this is the benefit of immutability.

## 2.6 Row Polymorphism (Extensible Records)

Functions can operate on any record containing certain fields:

```
fn greet(person: { name: String | r }) -> String =
    "Hello, " ++ person.name

-- Works with any record that has a `name: String` field
greet({ name: "Alice", age: 30 })
greet({ name: "Bob", email: "bob@example.com" })
```

## 2.7 Type Aliases

```
type alias Predicate[A] = fn(A) -> Bool
type alias Transform[A, B] = fn(A) -> B
type alias Pipeline[A, B] = List[Transform[A, B]]
```

## 2.8 Never and Divergence

The `Never` type is the bottom type — it has no values. Functions returning `Never`
diverge (loop forever or halt the program):

```
fn panic(message: String) -> Never = intrinsic_panic(message)

fn infinite_loop() -> Never = infinite_loop()
```

`Never` coerces to any type, making it usable in any branch of a `match`.
