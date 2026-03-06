# VibeLang Language Specification v0.1

## 1. Design Philosophy

VibeLang is a programming language designed to minimize the cognitive distance between intent and implementation. It is optimized for how a reasoning agent naturally expresses computation:

- **Expression-oriented**: Everything is an expression that produces a value. There are no statements.
- **Pattern matching as primary control flow**: Branching is done through structural pattern matching, not if/else chains.
- **Pipe-oriented composition**: Data flows left-to-right through transformation pipelines.
- **Algebraic data types**: Model domains precisely with sum and product types.
- **Type inference**: Types are inferred wherever possible; annotations are optional but available.
- **No null**: Absence is modeled explicitly with `Option` types.
- **Minimal ceremony**: No boilerplate. Programs express only the essential logic.

## 2. Lexical Structure

### 2.1 Encoding

Source files are UTF-8 encoded.

### 2.2 Comments

```
// Single-line comment

/* Multi-line
   comment */
```

### 2.3 Identifiers

Identifiers start with a letter or underscore, followed by letters, digits, or underscores.

```
identifier = [a-zA-Z_][a-zA-Z0-9_]*
```

### 2.4 Keywords

```
fn let mut match if else loop break return
type struct enum true false and or not
import as pub
```

### 2.5 Literals

#### Integer literals
```
42        // decimal
0xFF      // hexadecimal
0b1010    // binary
0o77      // octal
1_000_000 // underscores for readability
```

Integer literals are `i64` by default.

#### Float literals
```
3.14
1.0e10
2.5e-3
```

Float literals are `f64` by default.

#### String literals
```
"hello world"           // basic string
"line one\nline two"    // escape sequences
```

Escape sequences: `\n`, `\t`, `\\`, `\"`, `\0`, `\r`, `\x{HH}`.

#### Boolean literals
```
true
false
```

### 2.6 Operators and Punctuation

```
+  -  *  /  %          // arithmetic
== != < > <= >=        // comparison
and or not             // logical
|>                     // pipe
=                      // assignment/binding
->                     // function arrow / match arm
=>                     // fat arrow (type mapping)
:                      // type annotation
,                      // separator
.                      // field access
;                      // expression separator
( ) [ ] { }           // grouping
&  @                   // reference, pattern binding
```

### 2.7 Operator Precedence (lowest to highest)

| Precedence | Operators         | Associativity |
|------------|-------------------|---------------|
| 1          | `|>`              | Left          |
| 2          | `or`              | Left          |
| 3          | `and`             | Left          |
| 4          | `== != < > <= >=` | Left          |
| 5          | `+ -`             | Left          |
| 6          | `* / %`           | Left          |
| 7          | `not - (unary)`   | Prefix        |
| 8          | `. () []`         | Left          |

## 3. Type System

### 3.1 Primitive Types

| Type   | Description                  | Size    |
|--------|------------------------------|---------|
| `i8`   | Signed 8-bit integer         | 1 byte  |
| `i16`  | Signed 16-bit integer        | 2 bytes |
| `i32`  | Signed 32-bit integer        | 4 bytes |
| `i64`  | Signed 64-bit integer        | 8 bytes |
| `u8`   | Unsigned 8-bit integer       | 1 byte  |
| `u16`  | Unsigned 16-bit integer      | 2 bytes |
| `u32`  | Unsigned 32-bit integer      | 4 bytes |
| `u64`  | Unsigned 64-bit integer      | 8 bytes |
| `f32`  | 32-bit float (IEEE 754)      | 4 bytes |
| `f64`  | 64-bit float (IEEE 754)      | 8 bytes |
| `bool` | Boolean                      | 1 byte  |
| `u8`   | Byte (alias for u8)          | 1 byte  |
| `unit` | Unit type (zero-size)        | 0 bytes |

### 3.2 Compound Types

#### Arrays (fixed-size, stack-allocated)
```
[i32; 5]          // array of 5 i32 values
[1, 2, 3, 4, 5]  // array literal (type inferred)
```

#### Slices (dynamically-sized view into contiguous memory)
```
[i32]             // slice of i32
```

#### Tuples
```
(i32, bool)          // tuple type
(42, true)           // tuple literal
```

#### Pointers
```
&T                   // immutable reference
&mut T               // mutable reference
```

### 3.3 Struct Types

```
struct Point {
    x: f64,
    y: f64,
}
```

### 3.4 Enum Types (Algebraic Data Types)

```
enum Option[T] {
    Some(T),
    None,
}

enum Result[T, E] {
    Ok(T),
    Err(E),
}

enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Triangle(f64, f64, f64),
}
```

### 3.5 Function Types

```
fn(i32, i32) -> i32    // function type taking two i32, returning i32
fn() -> unit            // function taking nothing, returning unit
```

### 3.6 Type Parameters (Generics)

Type parameters are enclosed in square brackets:

```
fn identity[T](x: T) -> T { x }

struct Pair[A, B] {
    first: A,
    second: B,
}
```

### 3.7 Type Aliases

```
type Coordinate = (f64, f64);
type IntList = [i32];
```

### 3.8 Type Inference

Types are inferred from usage wherever possible. Explicit annotations are only required on:
- Function parameter types
- Function return types (can be omitted if body is a single expression and type is unambiguous)

```
let x = 42;                // inferred as i64
let y = 3.14;              // inferred as f64
let p = Point { x: 1.0, y: 2.0 };  // inferred as Point
```

## 4. Expressions

Everything in VibeLang is an expression. There are no statements — every construct produces a value.

### 4.1 Let Bindings

```
let x = 5;
let mut counter = 0;
let (a, b) = (1, 2);           // destructuring
let Point { x, y } = origin;   // struct destructuring
```

A let binding evaluates to `unit`. When used in a block, the last expression (without a trailing semicolon) is the block's value.

### 4.2 Blocks

A block is a sequence of expressions separated by semicolons. The value of a block is the value of its last expression.

```
{
    let x = 5;
    let y = 10;
    x + y           // this is the block's value: 15
}
```

If the last expression ends with `;`, the block evaluates to `unit`.

### 4.3 Function Definitions

```
fn add(a: i32, b: i32) -> i32 {
    a + b
}

// Single expression body (return type inferred):
fn double(x: i32) -> i32 { x * 2 }

// No return value:
fn greet(name: [u8]) {
    print("Hello, ");
    println(name);
}
```

Functions are first-class values and can be assigned to variables or passed as arguments.

### 4.4 Anonymous Functions (Closures)

```
fn(x, y) { x + y }             // anonymous function
fn(x: i32) -> i32 { x * 2 }   // with annotations
```

Short form using pipe syntax for single-expression closures:

```
|x| x * 2
|x, y| x + y
```

### 4.5 Function Calls

```
add(1, 2)
print("hello")
identity[i32](42)    // explicit type arguments
```

### 4.6 Pipe Operator

The pipe operator `|>` passes the left-hand value as the first argument to the right-hand function.

```
5 |> double |> add(_, 3) |> print

// Equivalent to:
print(add(double(5), 3))
```

The placeholder `_` marks where the piped value is inserted when it's not the first argument.

### 4.7 Match Expressions

`match` is the primary control flow mechanism:

```
match value {
    0 -> "zero",
    1 -> "one",
    n if n > 0 -> "positive",
    _ -> "negative",
}
```

#### Pattern Types

```
// Literal patterns
match x {
    42 -> "the answer",
    _ -> "something else",
}

// Variable binding
match x {
    n -> n + 1,
}

// Tuple destructuring
match pair {
    (0, 0) -> "origin",
    (x, 0) -> "on x-axis",
    (0, y) -> "on y-axis",
    (x, y) -> "elsewhere",
}

// Struct destructuring
match point {
    Point { x: 0, y: 0 } -> "origin",
    Point { x, y } -> x + y,
}

// Enum destructuring
match opt {
    Some(val) -> val,
    None -> default_value,
}

// Nested patterns
match list {
    Some(Some(x)) -> x,
    _ -> 0,
}

// Or patterns
match x {
    1 or 2 or 3 -> "small",
    _ -> "big",
}

// Guards
match x {
    n if n % 2 == 0 -> "even",
    n -> "odd",
}

// Range patterns
match x {
    0..10 -> "single digit",
    10..100 -> "double digit",
    _ -> "big",
}

// Binding with @
match x {
    n @ 1..100 -> n,
    _ -> 0,
}
```

Match expressions must be exhaustive — all possible values must be covered.

### 4.8 If Expressions

Syntactic sugar for simple two-branch matches:

```
if condition { then_expr } else { else_expr }
```

Both branches must produce values of the same type. `if` without `else` returns `unit` and the body must also be `unit`.

### 4.9 Loops

```
// Infinite loop (breaks with a value)
let result = loop {
    counter = counter + 1;
    if counter == 10 {
        break counter
    }
};

// While-style loop (syntactic sugar)
loop condition {
    body
}

// Range iteration
loop i in 0..10 {
    print(i);
}

// Iteration over collections
loop item in collection {
    process(item);
}
```

`loop` without a `break` value evaluates to `unit`. `break` with a value makes the `loop` expression evaluate to that value.

### 4.10 Array/Slice Expressions

```
let arr = [1, 2, 3, 4, 5];
let first = arr[0];
let slice = arr[1..3];       // slice: [2, 3]
let len = arr.len;           // built-in length property
```

### 4.11 Struct Construction

```
let p = Point { x: 1.0, y: 2.0 };

// Shorthand when variable names match field names:
let x = 1.0;
let y = 2.0;
let p = Point { x, y };
```

### 4.12 Field Access

```
p.x
p.y
tuple.0
tuple.1
```

### 4.13 Index Access

```
arr[0]
matrix[i][j]
```

### 4.14 Return

`return` exits the enclosing function with a value:

```
fn find(arr: [i32], target: i32) -> Option[i32] {
    loop i in 0..arr.len {
        if arr[i] == target {
            return Some(i)
        }
    };
    None
}
```

## 5. Memory Model

VibeLang uses a simple, explicit memory model for the initial version:

### 5.1 Stack Allocation

All local variables are stack-allocated by default. This includes primitives, fixed-size arrays, structs, and enums whose size is known at compile time.

### 5.2 Heap Allocation

Heap allocation is explicit via the built-in `Box` type:

```
let boxed = Box.new(42);        // heap-allocated i32
let value = boxed.*;            // dereference with .*
```

### 5.3 References

```
let x = 42;
let r = &x;          // immutable reference
let value = r.*;     // dereference

let mut y = 42;
let mr = &mut y;     // mutable reference
mr.* = 100;          // assign through mutable reference
```

### 5.4 Ownership and Lifetime

For v0.1, VibeLang uses a simplified model:
- Values have a single owner (the binding).
- References must not outlive the value they reference (enforced by scoping rules: a reference is valid only within the block where the referent is defined).
- Only one mutable reference OR any number of immutable references may exist at a time.
- When a value goes out of scope, its memory is freed (stack pops; heap via Box calls free).

## 6. Module System

### 6.1 File-Based Modules

Each `.vibe` file is a module. The file name (without extension) is the module name.

### 6.2 Imports

```
import math;                    // import module
import math.sqrt;               // import specific item
import math.{sqrt, pow};        // import multiple items
import math as m;               // aliased import
```

### 6.3 Visibility

All definitions are private by default. Use `pub` to export:

```
pub fn add(a: i32, b: i32) -> i32 { a + b }

pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub enum Color {
    Red,
    Green,
    Blue,
}
```

## 7. Built-in Functions

```
print(value)        // print to stdout (no newline)
println(value)      // print to stdout (with newline)
assert(condition)   // panic if false
panic(message)      // abort with message
```

## 8. Program Entry Point

The entry point is a function named `main` in the root module:

```
fn main() {
    println("Hello, VibeLang!");
}
```

`main` may optionally return `i32` as an exit code:

```
fn main() -> i32 {
    0
}
```

## 9. Grammar (EBNF)

```ebnf
program        = { top_level_item } ;

top_level_item = import_decl
               | fn_def
               | struct_def
               | enum_def
               | type_alias
               ;

import_decl    = "import" path [ "as" IDENT ] ";"
               | "import" path "." "{" ident_list "}" ";"
               | "import" path "." IDENT ";"
               ;

path           = IDENT { "." IDENT } ;
ident_list     = IDENT { "," IDENT } ;

fn_def         = [ "pub" ] "fn" IDENT [ type_params ] "(" param_list ")"
                 [ "->" type ] block ;

param_list     = [ param { "," param } ] ;
param          = IDENT ":" type ;

type_params    = "[" IDENT { "," IDENT } "]" ;

struct_def     = [ "pub" ] "struct" IDENT [ type_params ] "{"
                 field_list "}" ;

field_list     = [ [ "pub" ] field { "," [ "pub" ] field } [ "," ] ] ;
field          = IDENT ":" type ;

enum_def       = [ "pub" ] "enum" IDENT [ type_params ] "{"
                 variant_list "}" ;

variant_list   = variant { "," variant } [ "," ] ;
variant        = IDENT [ "(" type_list ")" ] ;

type_list      = type { "," type } ;

type_alias     = "type" IDENT [ type_params ] "=" type ";" ;

type           = "i8" | "i16" | "i32" | "i64"
               | "u8" | "u16" | "u32" | "u64"
               | "f32" | "f64"
               | "bool" | "unit"
               | IDENT [ "[" type_list "]" ]       (* named type *)
               | "[" type "]"                       (* slice *)
               | "[" type ";" INTEGER "]"           (* array *)
               | "(" type_list ")"                  (* tuple *)
               | "&" [ "mut" ] type                 (* reference *)
               | "fn" "(" type_list ")" "->" type   (* function type *)
               ;

block          = "{" { expr ";" } [ expr ] "}" ;

expr           = let_expr
               | assign_expr
               | pipe_expr
               | return_expr
               ;

let_expr       = "let" [ "mut" ] pattern [ ":" type ] "=" expr ;

assign_expr    = place "=" expr ;

return_expr    = "return" [ expr ] ;

pipe_expr      = or_expr { "|>" or_expr } ;

or_expr        = and_expr { "or" and_expr } ;
and_expr       = cmp_expr { "and" cmp_expr } ;
cmp_expr       = add_expr { ( "==" | "!=" | "<" | ">" | "<=" | ">=" ) add_expr } ;
add_expr       = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr       = unary_expr { ( "*" | "/" | "%" ) unary_expr } ;

unary_expr     = ( "not" | "-" ) unary_expr
               | postfix_expr ;

postfix_expr   = primary { "." IDENT | "." INTEGER | "." "*"
                          | "[" expr "]"
                          | "(" arg_list ")" } ;

primary        = INTEGER | FLOAT | STRING | "true" | "false"
               | IDENT
               | "(" expr ")"
               | block
               | match_expr
               | if_expr
               | loop_expr
               | fn_expr
               | array_literal
               | struct_literal
               ;

match_expr     = "match" expr "{" match_arm { "," match_arm } [ "," ] "}" ;
match_arm      = pattern [ "if" expr ] "->" expr ;

if_expr        = "if" expr block [ "else" ( block | if_expr ) ] ;

loop_expr      = "loop" block                                  (* infinite *)
               | "loop" expr block                             (* while *)
               | "loop" IDENT "in" expr block                  (* for-in *)
               ;

fn_expr        = "fn" "(" param_list ")" [ "->" type ] block  (* anonymous fn *)
               | "|" param_short_list "|" expr                 (* closure *)
               ;

param_short_list = IDENT { "," IDENT } ;

array_literal  = "[" [ expr { "," expr } [ "," ] ] "]" ;

struct_literal = IDENT "{" struct_field_list "}" ;
struct_field_list = struct_field_init { "," struct_field_init } [ "," ] ;
struct_field_init = IDENT ":" expr | IDENT ;  (* shorthand *)

pattern        = "_"                                        (* wildcard *)
               | INTEGER | FLOAT | STRING | "true" | "false" (* literal *)
               | IDENT                                       (* binding *)
               | IDENT "@" pattern                           (* named binding *)
               | "(" pattern_list ")"                        (* tuple *)
               | IDENT "{" field_pattern_list "}"            (* struct *)
               | IDENT "(" pattern_list ")"                  (* enum variant *)
               | pattern "or" pattern                        (* alternation *)
               | INTEGER ".." INTEGER                        (* range *)
               ;

pattern_list       = [ pattern { "," pattern } ] ;
field_pattern_list = [ field_pattern { "," field_pattern } ] ;
field_pattern      = IDENT ":" pattern | IDENT ;

place          = IDENT { "." IDENT | "." "*" | "[" expr "]" } ;

arg_list       = [ arg { "," arg } ] ;
arg            = expr | "_" ;   (* _ is placeholder for pipe *)
```

## 10. Turing Completeness

VibeLang is Turing complete. Proof sketch: it supports recursive functions with conditional branching (`match`/`if`) and unbounded storage (heap allocation via `Box`, dynamic loops). Any partial recursive function can be expressed. Here is a minimal demonstration:

```
// Turing completeness via general recursion + conditional
fn ackermann(m: i64, n: i64) -> i64 {
    match (m, n) {
        (0, n) -> n + 1,
        (m, 0) -> ackermann(m - 1, 1),
        (m, n) -> ackermann(m - 1, ackermann(m, n - 1)),
    }
}

fn main() {
    ackermann(3, 4) |> println;
}
```

## 11. Example Programs

### Hello World
```
fn main() {
    println("Hello, World!");
}
```

### Fibonacci
```
fn fib(n: i64) -> i64 {
    match n {
        0 -> 0,
        1 -> 1,
        n -> fib(n - 1) + fib(n - 2),
    }
}

fn main() {
    loop i in 0..10 {
        fib(i) |> println;
    };
}
```

### FizzBuzz
```
fn fizzbuzz(n: i64) {
    loop i in 1..n + 1 {
        match (i % 3, i % 5) {
            (0, 0) -> println("FizzBuzz"),
            (0, _) -> println("Fizz"),
            (_, 0) -> println("Buzz"),
            _      -> println(i),
        };
    };
}

fn main() {
    fizzbuzz(100);
}
```

### Linked List
```
enum List[T] {
    Cons(T, Box[List[T]]),
    Nil,
}

fn sum(list: &List[i64]) -> i64 {
    match list.* {
        Cons(val, rest) -> val + sum(&rest.*),
        Nil -> 0,
    }
}

fn main() {
    let list = Cons(1, Box.new(Cons(2, Box.new(Cons(3, Box.new(Nil))))));
    sum(&list) |> println;
}
```

### Binary Search
```
fn binary_search(arr: &[i64], target: i64) -> Option[i64] {
    let mut low = 0;
    let mut high = arr.len - 1;

    loop low <= high {
        let mid = (low + high) / 2;
        match arr[mid] {
            v if v == target -> return Some(mid),
            v if v < target  -> { low = mid + 1 },
            _                -> { high = mid - 1 },
        };
    };

    None
}

fn main() {
    let arr = [1, 3, 5, 7, 9, 11, 13];
    match binary_search(&arr, 7) {
        Some(idx) -> {
            print("Found at index: ");
            println(idx);
        },
        None -> println("Not found"),
    };
}
```

## 12. Compiler Target

The VibeLang compiler (`vibec`) will target x86_64 Linux, producing ELF executables. The compilation pipeline:

1. **Lexer**: Source → Tokens
2. **Parser**: Tokens → AST
3. **Type Checker**: AST → Typed AST (with type inference via Hindley-Milner-style unification)
4. **IR Lowering**: Typed AST → VibeLang IR (SSA-based intermediate representation)
5. **Code Generation**: IR → x86_64 assembly (NASM syntax)
6. **Assembly & Linking**: NASM + ld → ELF binary

## 13. File Extension

VibeLang source files use the `.vibe` extension.
