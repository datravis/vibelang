# 3. Syntax and Core Constructs

VibeLang syntax is designed for **regularity** and **unambiguous parsing**. Every construct
has a single canonical form. Indentation is not significant — blocks are delimited by
keywords and expressions.

## 3.1 Lexical Structure

### Comments

```
-- This is a line comment

{- This is a
   block comment -}

--- This is a doc comment (attached to the next declaration)
--- It supports **markdown** formatting.
```

### Identifiers

```
-- Values and functions: lower_snake_case
let my_value = 42
fn process_data(input: Data) -> Result = ...

-- Types, Traits, Variants: PascalCase
type UserAccount = { ... }
trait Serializable[A] { ... }

-- Modules: lower_snake_case
module data_processing
```

### Literals

```
-- Integers
42
1_000_000
0xFF
0b1010
0o77

-- Floats
3.14
1.0e-10
6.022e23

-- Strings (UTF-8, double-quoted)
"hello world"
"line one\nline two"

-- Multi-line strings (triple-quoted, strips common indentation)
"""
    This is a multi-line string.
    Indentation relative to the closing quotes is preserved.
    """

-- String interpolation
"Hello, ${name}! You are ${show(age)} years old."

-- Characters (single-quoted)
'a'
'\n'
'λ'

-- Booleans
true
false

-- Unit
()

-- List literals
[1, 2, 3, 4, 5]

-- Tuple literals
(1, "hello", true)
```

## 3.2 Bindings

All bindings are immutable. `let` introduces a binding:

```
let x = 42
let name = "Alice"
let point = { x: 1.0, y: 2.0 }
```

### Destructuring

```
let { x, y } = point
let (first, second, _) = my_tuple
let Cons(head, tail) = my_list    -- panics at runtime if Nil
```

### Pattern Bindings with Guards

```
let Some(value) = maybe_value else panic("was None")
```

## 3.3 Functions

Functions are defined with `fn`. The body is a single expression (the return value):

```
fn add(a: Int, b: Int) -> Int = a + b

fn factorial(n: UInt) -> UInt =
    match n
    | 0 -> 1
    | n -> n * factorial(n - 1)
```

### Multi-Expression Bodies

Use `do` blocks to sequence multiple expressions. The last expression is the return value:

```
fn process(data: ref Data) -> Result[Output, Error] = do
    let validated = validate(data)
    let transformed = transform(validated)
    let result = encode(transformed)
    Ok(result)
```

### Anonymous Functions (Lambdas)

```
let double = fn(x: Int) -> Int = x * 2

-- Type inference for lambdas
let double = fn(x) = x * 2

-- Short form with `\`
let double = \x -> x * 2

-- Multi-argument
let add = \x, y -> x + y
```

### Partial Application

All functions support automatic partial application (currying):

```
fn add(a: Int, b: Int) -> Int = a + b

let add_five = add(5)      -- fn(Int) -> Int
let result = add_five(3)   -- 8
```

### Function Composition

```
-- Pipe operator: feeds the left value into the right function
let result = data
    |> validate
    |> transform
    |> encode

-- Compose operator: creates a new function from two functions
let process = validate >> transform >> encode
```

## 3.4 Pattern Matching

`match` is an expression that must be exhaustive:

```
fn describe(shape: Shape) -> String =
    match shape
    | Circle(r) -> "Circle with radius ${show(r)}"
    | Rectangle(w, h) -> "Rectangle ${show(w)}x${show(h)}"
    | Triangle(a, b, c) -> "Triangle with sides ${show(a)}, ${show(b)}, ${show(c)}"
```

### Guards

```
fn classify(n: Int) -> String =
    match n
    | 0 -> "zero"
    | n when n > 0 -> "positive"
    | _ -> "negative"
```

### Nested Patterns

```
fn first_two(list: List[Int]) -> Option[(Int, Int)] =
    match list
    | Cons(a, Cons(b, _)) -> Some((a, b))
    | _ -> None
```

## 3.5 Control Flow

### If-Else (Expression)

```
let abs_value = if x >= 0 then x else -x
```

Multi-branch:

```
let category =
    if age < 13 then "child"
    else if age < 20 then "teenager"
    else if age < 65 then "adult"
    else "senior"
```

### When (Guard Expression)

Syntactic sugar for common pattern:

```
let status = when
    | temperature > 100.0 -> "boiling"
    | temperature > 0.0   -> "liquid"
    | otherwise            -> "frozen"
```

## 3.6 Let-In Expressions

For scoped bindings within expressions:

```
let area =
    let pi = 3.14159
    let r_squared = radius * radius
    in pi * r_squared
```

## 3.7 Records

### Construction

```
let user = { name: "Alice", age: 30, email: "alice@example.com" }
```

### Field Access

```
let user_name = user.name
```

### Functional Update (Creates New Record)

```
let older_user = { user with age: user.age + 1 }
```

## 3.8 Modules

Modules provide namespacing and encapsulation:

```
module math.vector

type Vec3 = { x: Float, y: Float, z: Float }

pub fn add(a: Vec3, b: Vec3) -> Vec3 =
    { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }

pub fn dot(a: Vec3, b: Vec3) -> Float =
    a.x * b.x + a.y * b.y + a.z * b.z

fn internal_helper(v: Vec3) -> Float = ...  -- not exported
```

### Imports

```
use math.vector              -- import module, access as vector.add(...)
use math.vector.{add, dot}   -- import specific items
use math.vector.*            -- import all public items
use math.vector as v         -- aliased import: v.add(...)
```

## 3.9 Operators

VibeLang has a fixed, non-overloadable set of operators:

| Operator | Meaning | Types |
|----------|---------|-------|
| `+` `-` `*` `/` | Arithmetic | Numeric types |
| `%` | Modulo | Integer types |
| `==` `!=` | Equality | `Eq` types |
| `<` `>` `<=` `>=` | Comparison | `Ord` types |
| `&&` `\|\|` `!` | Logical | `Bool` |
| `&` `\|` `^` `~` | Bitwise | Integer types |
| `<<` `>>` | Bit shift | Integer types |
| `++` | Concatenation | `String`, `List` |
| `\|>` | Pipe | Any |
| `>>` | Compose | Functions |
| `::` | Type annotation | Any |

Operator precedence follows conventional mathematical rules and is fully specified in
Appendix A.
