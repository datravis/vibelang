# VibeLang Language Specification v0.2

## 1. Design Philosophy

VibeLang is a **purely functional** programming language designed to minimize the cognitive distance between intent and implementation. It is optimized for how a reasoning agent naturally expresses computation:

- **Purely functional**: All values are immutable. There are no side effects in pure code. Functions are referentially transparent — calling a function with the same arguments always produces the same result.
- **Expression-oriented**: Everything is an expression that produces a value. There are no statements.
- **Pattern matching as primary control flow**: Branching is done through structural pattern matching, not if/else chains.
- **Pipe-oriented composition**: Data flows left-to-right through transformation pipelines.
- **Algebraic data types**: Model domains precisely with sum and product types.
- **Type inference**: Types are inferred wherever possible; annotations are optional but available.
- **No null**: Absence is modeled explicitly with `Option` types.
- **Effect tracking**: Side effects (IO, etc.) are tracked in the type system. Pure and effectful code are cleanly separated.
- **Tail-call optimization**: Guaranteed TCO enables recursion as the sole looping mechanism without stack overflow risk.
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
fn let match if else type struct enum
true false and or not import as pub
effect with
```

Note the absence of `mut`, `loop`, `break`, `return` — these concepts do not exist in a purely functional language.

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

Strings are immutable sequences of bytes (`[u8]`).

#### Boolean literals
```
true
false
```

#### List literals
```
[1, 2, 3, 4, 5]          // list literal
[]                        // empty list
1 :: 2 :: 3 :: []         // cons construction
```

### 2.6 Operators and Punctuation

```
+  -  *  /  %          // arithmetic
== != < > <= >=        // comparison
and or not             // logical
|>                     // pipe
::                     // list cons
++                     // list/string concatenation
=                      // binding (not assignment)
->                     // function arrow / match arm
:                      // type annotation
,                      // separator
.                      // field access / function composition
;                      // expression separator
( ) [ ] { }           // grouping
@                      // pattern binding
```

### 2.7 Operator Precedence (lowest to highest)

| Precedence | Operators         | Associativity |
|------------|-------------------|---------------|
| 1          | `\|>`             | Left          |
| 2          | `or`              | Left          |
| 3          | `and`             | Left          |
| 4          | `== != < > <= >=` | Left          |
| 5          | `++`              | Right         |
| 6          | `::`              | Right         |
| 7          | `+ -`             | Left          |
| 8          | `* / %`           | Left          |
| 9          | `not - (unary)`   | Prefix        |
| 10         | `. () []`         | Left          |

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
| `unit` | Unit type (zero-size)        | 0 bytes |

All primitive values are immutable.

### 3.2 Compound Types

#### Lists (immutable, persistent, singly-linked)
```
List[i32]             // list of i32
[1, 2, 3]             // list literal
```

Lists are the primary collection type. They are persistent — "modifying" a list produces a new list that shares structure with the original.

#### Arrays (immutable, fixed-size, for performance-critical paths)
```
Array[i32; 5]         // array of 5 i32 values
```

#### Tuples
```
(i32, bool)          // tuple type
(42, true)           // tuple literal
```

### 3.3 Struct Types

Structs are immutable product types:

```
struct Point {
    x: f64,
    y: f64,
}
```

#### Functional Update

Since structs are immutable, a `with` expression creates a new struct with some fields changed:

```
let p1 = Point { x: 1.0, y: 2.0 };
let p2 = p1 with { x: 3.0 };       // Point { x: 3.0, y: 2.0 }
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
fn(i32, i32) -> i32    // pure function type
fn(i32) -> IO[unit]    // effectful function type
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
type StringList = List[String];
```

### 3.8 Type Inference

Types are inferred from usage via Hindley-Milner unification. Explicit annotations are required on:
- Function parameter types
- Function return types (can be omitted if body is a single expression and type is unambiguous)

```
let x = 42;                            // inferred as i64
let y = 3.14;                          // inferred as f64
let p = Point { x: 1.0, y: 2.0 };     // inferred as Point
let xs = [1, 2, 3];                    // inferred as List[i64]
let add = fn(a: i64, b: i64) { a + b };  // inferred as fn(i64, i64) -> i64
```

## 4. Effect System

VibeLang is purely functional: all functions are pure by default. Side effects are tracked at the type level using the `IO` type.

### 4.1 Pure Functions

Pure functions have no side effects. They are referentially transparent:

```
fn add(a: i32, b: i32) -> i32 { a + b }
fn fib(n: i64) -> i64 {
    match n {
        0 -> 0,
        1 -> 1,
        n -> fib(n - 1) + fib(n - 2),
    }
}
```

A pure function **cannot** call an effectful function. This is enforced by the type system.

### 4.2 The IO Type

`IO[T]` represents a computation that, when executed, may perform side effects and produces a value of type `T`. An `IO[T]` value is itself pure — it is a *description* of an effectful computation, not the execution of one.

```
// println returns an IO action, it does not perform IO directly
// println : fn(String) -> IO[unit]

fn greet(name: String) -> IO[unit] {
    println("Hello, " ++ name ++ "!")
}
```

### 4.3 Composing IO Actions

IO actions are composed using `effect` blocks. Within an `effect` block, IO actions are sequenced and their results can be bound with `let`:

```
fn main() -> IO[unit] {
    effect {
        let name = readline();
        println("Hello, " ++ name ++ "!");
    }
}
```

The `effect` block is syntactic sugar for monadic bind. Each `let` binding in an `effect` block extracts the value from an IO action. The block itself evaluates to `IO[T]` where `T` is the type of the last expression.

Desugaring:

```
// effect { let x = a(); b(x) }
// becomes:
// a() |> flatmap(|x| b(x))
```

### 4.4 Pure Values in Effect Blocks

To lift a pure value into IO within an `effect` block, use `pure`:

```
fn make_greeting(name: String) -> IO[String] {
    effect {
        pure("Hello, " ++ name ++ "!")
    }
}
```

### 4.5 The Main Function

`main` must return `IO[unit]` or `IO[i32]` (exit code). It is the only place where IO actions are actually executed by the runtime:

```
fn main() -> IO[unit] {
    effect {
        println("Hello, World!");
    }
}
```

### 4.6 Built-in IO Functions

```
println(value: String) -> IO[unit]       // print line to stdout
print(value: String) -> IO[unit]         // print to stdout (no newline)
readline() -> IO[String]                 // read line from stdin
readfile(path: String) -> IO[String]     // read file contents
writefile(path: String, data: String) -> IO[unit]  // write file
exit(code: i32) -> IO[unit]              // exit process
```

## 5. Expressions

Everything in VibeLang is an expression. There are no statements — every construct produces a value.

### 5.1 Let Bindings

Let bindings are immutable. Once bound, a name cannot be rebound in the same scope (but can be shadowed in a nested scope):

```
let x = 5;
let (a, b) = (1, 2);               // destructuring
let Point { x, y } = origin;       // struct destructuring
let head :: tail = some_list;       // list destructuring
```

A let binding evaluates to `unit`. When used in a block, the last expression (without a trailing semicolon) is the block's value.

**Shadowing** is permitted — a new `let` binding can reuse the same name, creating a new binding that shadows the old one:

```
let x = 5;
let x = x + 1;   // shadows previous x; x is now 6
```

This is not mutation — it creates a new binding. The old value is unchanged and may still be referenced by closures that captured it.

### 5.2 Blocks

A block is a sequence of expressions separated by semicolons. The value of a block is the value of its last expression:

```
{
    let x = 5;
    let y = 10;
    x + y           // block value: 15
}
```

If the last expression ends with `;`, the block evaluates to `unit`.

### 5.3 Function Definitions

```
fn add(a: i32, b: i32) -> i32 {
    a + b
}

// Single expression body:
fn double(x: i32) -> i32 { x * 2 }
```

Functions are first-class values and can be passed as arguments, returned from functions, and stored in data structures.

### 5.4 Anonymous Functions (Closures)

```
fn(x, y) { x + y }                 // anonymous function
fn(x: i32) -> i32 { x * 2 }       // with annotations
```

Short form for single-expression closures:

```
|x| x * 2
|x, y| x + y
```

Closures capture their environment immutably (since all values are immutable, this is always safe).

### 5.5 Function Calls

```
add(1, 2)
identity[i32](42)          // explicit type arguments
```

### 5.6 Pipe Operator

The pipe operator `|>` passes the left-hand value as the first argument to the right-hand function:

```
5 |> double |> add(_, 3) |> to_string

// Equivalent to:
to_string(add(double(5), 3))
```

The placeholder `_` marks where the piped value is inserted when it's not the first argument.

Pipes compose naturally with the effect system:

```
fn main() -> IO[unit] {
    effect {
        "Hello" |> greet |> println;
    }
}
```

### 5.7 Match Expressions

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

// List destructuring
match xs {
    [] -> "empty",
    [x] -> "singleton",
    head :: tail -> "has elements",
}

// Nested patterns
match nested {
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

Match expressions must be **exhaustive** — all possible values must be covered by the patterns.

### 5.8 If Expressions

Syntactic sugar for simple two-branch matches:

```
if condition { then_expr } else { else_expr }
```

Both branches must produce values of the same type. `if` without `else` is only allowed when the body is `IO[unit]` inside an `effect` block.

### 5.9 List Operations

```
let xs = [1, 2, 3, 4, 5];
let ys = 0 :: xs;                // cons: [0, 1, 2, 3, 4, 5]
let zs = xs ++ [6, 7];          // concatenation: [1, 2, 3, 4, 5, 6, 7]
let head = List.head(xs);       // Option[i64]: Some(1)
let tail = List.tail(xs);       // Option[List[i64]]: Some([2, 3, 4, 5])
let len = List.length(xs);      // i64: 5
let third = List.at(xs, 2);     // Option[i64]: Some(3)
```

### 5.10 Struct Construction and Update

```
let p = Point { x: 1.0, y: 2.0 };

// Shorthand when variable names match field names:
let x = 1.0;
let y = 2.0;
let p = Point { x, y };

// Functional update — creates new struct:
let p2 = p with { x: 3.0 };    // Point { x: 3.0, y: 2.0 }
```

### 5.11 Field Access

```
p.x
p.y
tuple.0
tuple.1
```

### 5.12 String Operations

Strings are immutable. Concatenation produces a new string:

```
let greeting = "Hello, " ++ "World!";
let len = String.length(greeting);
```

## 6. Recursion and Tail-Call Optimization

Since VibeLang has no loops, all iteration is expressed through recursion. The compiler **guarantees** tail-call optimization (TCO) for functions where the recursive call is in tail position. This means tail-recursive functions run in constant stack space.

### 6.1 Tail Position

A call is in tail position if it is the last operation before the function returns — i.e., its result is returned directly without any further computation.

```
// Tail-recursive — guaranteed O(1) stack space
fn sum_acc(xs: List[i64], acc: i64) -> i64 {
    match xs {
        [] -> acc,
        head :: tail -> sum_acc(tail, acc + head),   // tail call
    }
}

// Not tail-recursive — O(n) stack space
fn sum(xs: List[i64]) -> i64 {
    match xs {
        [] -> 0,
        head :: tail -> head + sum(tail),   // NOT tail: addition happens after
    }
}
```

### 6.2 Common Recursion Patterns

#### Iteration via Accumulator
```
fn factorial(n: i64) -> i64 {
    fn go(n: i64, acc: i64) -> i64 {
        match n {
            0 -> acc,
            n -> go(n - 1, n * acc),
        }
    };
    go(n, 1)
}
```

#### Map
```
fn map[A, B](xs: List[A], f: fn(A) -> B) -> List[B] {
    match xs {
        [] -> [],
        head :: tail -> f(head) :: map(tail, f),
    }
}
```

#### Fold (tail-recursive)
```
fn foldl[A, B](xs: List[A], init: B, f: fn(B, A) -> B) -> B {
    match xs {
        [] -> init,
        head :: tail -> foldl(tail, f(init, head), f),
    }
}
```

#### Filter
```
fn filter[T](xs: List[T], pred: fn(T) -> bool) -> List[T] {
    match xs {
        [] -> [],
        head :: tail -> if pred(head) { head :: filter(tail, pred) }
                        else { filter(tail, pred) },
    }
}
```

### 6.3 Local Function Definitions

Functions can be defined inside other functions. This is the idiomatic way to define accumulator-based helpers:

```
fn reverse[T](xs: List[T]) -> List[T] {
    fn go(xs: List[T], acc: List[T]) -> List[T] {
        match xs {
            [] -> acc,
            head :: tail -> go(tail, head :: acc),
        }
    };
    go(xs, [])
}
```

## 7. Memory Model

### 7.1 Immutability and Sharing

Since all values are immutable, the compiler is free to share data structures. When a "new" list is created by consing onto an existing list, the new list shares the tail with the original. This is safe precisely because nothing can mutate the shared structure.

### 7.2 Reference Counting

VibeLang uses automatic reference counting (ARC) for memory management. When the last reference to a value is dropped, its memory is freed. Since all values are immutable, there are no data races and no need for atomic reference counts in single-threaded code.

#### Cycle-Free Guarantee

Purely functional data structures built from algebraic data types and cons cells are acyclic by construction — you cannot create a cycle without mutation. Therefore, reference counting is sound and complete for memory reclamation without a cycle collector.

### 7.3 Compiler Optimizations

The compiler may apply the following optimizations:

- **In-place update**: When the compiler can prove a value has a reference count of 1 (unique ownership), it may update it in place rather than copying. This is semantically invisible — the program behaves as if a new copy was made.
- **Unboxing**: Small values (primitives, small structs) are stored inline rather than heap-allocated.
- **Stack allocation**: Values that do not escape their scope are allocated on the stack.
- **Deforestation**: Intermediate data structures in a pipeline of list transformations may be eliminated (e.g., `map` followed by `filter` fused into a single pass).

## 8. Module System

### 8.1 File-Based Modules

Each `.vibe` file is a module. The file name (without extension) is the module name.

### 8.2 Imports

```
import math;                    // import module
import math.sqrt;               // import specific item
import math.{sqrt, pow};        // import multiple items
import math as m;               // aliased import
```

### 8.3 Visibility

All definitions are private by default. Use `pub` to export:

```
pub fn add(a: i32, b: i32) -> i32 { a + b }

pub struct Point {
    x: f64,
    y: f64,
}

pub enum Color {
    Red,
    Green,
    Blue,
}
```

## 9. Standard Library (Built-in)

### 9.1 List Functions

```
List.head    : fn(List[T]) -> Option[T]
List.tail    : fn(List[T]) -> Option[List[T]]
List.length  : fn(List[T]) -> i64
List.at      : fn(List[T], i64) -> Option[T]
List.map     : fn(List[T], fn(T) -> U) -> List[U]
List.filter  : fn(List[T], fn(T) -> bool) -> List[T]
List.foldl   : fn(List[T], B, fn(B, T) -> B) -> B
List.foldr   : fn(List[T], B, fn(T, B) -> B) -> B
List.reverse : fn(List[T]) -> List[T]
List.zip     : fn(List[A], List[B]) -> List[(A, B)]
List.range   : fn(i64, i64) -> List[i64]
```

### 9.2 Option Functions

```
Option.map      : fn(Option[T], fn(T) -> U) -> Option[U]
Option.flatmap  : fn(Option[T], fn(T) -> Option[U]) -> Option[U]
Option.unwrap   : fn(Option[T], T) -> T           // with default
Option.is_some  : fn(Option[T]) -> bool
Option.is_none  : fn(Option[T]) -> bool
```

### 9.3 Result Functions

```
Result.map      : fn(Result[T, E], fn(T) -> U) -> Result[U, E]
Result.flatmap  : fn(Result[T, E], fn(T) -> Result[U, E]) -> Result[U, E]
Result.unwrap   : fn(Result[T, E], T) -> T        // with default
Result.is_ok    : fn(Result[T, E]) -> bool
Result.is_err   : fn(Result[T, E]) -> bool
```

### 9.4 String Functions

```
String.length   : fn(String) -> i64
String.at       : fn(String, i64) -> Option[u8]
String.slice    : fn(String, i64, i64) -> String
String.split    : fn(String, String) -> List[String]
String.contains : fn(String, String) -> bool
String.to_i64   : fn(String) -> Option[i64]
String.from_i64 : fn(i64) -> String
```

### 9.5 IO Functions

```
println    : fn(String) -> IO[unit]
print      : fn(String) -> IO[unit]
readline   : fn() -> IO[String]
readfile   : fn(String) -> IO[Result[String, String]]
writefile  : fn(String, String) -> IO[Result[unit, String]]
exit       : fn(i32) -> IO[unit]
```

### 9.6 Math Functions

```
Math.abs    : fn(i64) -> i64
Math.min    : fn(i64, i64) -> i64
Math.max    : fn(i64, i64) -> i64
Math.pow    : fn(i64, i64) -> i64
```

### 9.7 Conversion Functions

```
to_string : fn(T) -> String      // polymorphic; works for all types with a string representation
```

## 10. Program Entry Point

The entry point is a function named `main` in the root module. It must return `IO[unit]` or `IO[i32]`:

```
fn main() -> IO[unit] {
    effect {
        println("Hello, VibeLang!");
    }
}
```

With exit code:

```
fn main() -> IO[i32] {
    effect {
        println("Done.");
        pure(0)
    }
}
```

## 11. Grammar (EBNF)

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

field_list     = [ field { "," field } [ "," ] ] ;
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
               | "String"
               | "IO" "[" type "]"                     (* IO effect type *)
               | IDENT [ "[" type_list "]" ]           (* named/generic type *)
               | "List" "[" type "]"                   (* list type *)
               | "Array" "[" type ";" INTEGER "]"      (* array type *)
               | "(" type_list ")"                     (* tuple type *)
               | "fn" "(" type_list ")" "->" type      (* function type *)
               ;

block          = "{" { expr ";" } [ expr ] "}" ;

expr           = let_expr
               | pipe_expr
               ;

let_expr       = "let" pattern [ ":" type ] "=" expr ;

pipe_expr      = cons_expr { "|>" cons_expr } ;

cons_expr      = concat_expr { "::" concat_expr } ;

concat_expr    = or_expr { "++" or_expr } ;

or_expr        = and_expr { "or" and_expr } ;
and_expr       = cmp_expr { "and" cmp_expr } ;
cmp_expr       = add_expr { ( "==" | "!=" | "<" | ">" | "<=" | ">=" ) add_expr } ;
add_expr       = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr       = unary_expr { ( "*" | "/" | "%" ) unary_expr } ;

unary_expr     = ( "not" | "-" ) unary_expr
               | postfix_expr ;

postfix_expr   = primary { "." IDENT
                          | "." INTEGER
                          | "(" arg_list ")" } ;

primary        = INTEGER | FLOAT | STRING | "true" | "false"
               | IDENT
               | "(" expr ")"
               | block
               | match_expr
               | if_expr
               | effect_expr
               | fn_expr
               | list_literal
               | struct_literal
               | with_expr
               | "pure" "(" expr ")"
               ;

match_expr     = "match" expr "{" match_arm { "," match_arm } [ "," ] "}" ;
match_arm      = pattern [ "if" expr ] "->" expr ;

if_expr        = "if" expr block [ "else" ( block | if_expr ) ] ;

effect_expr    = "effect" block ;

fn_expr        = "fn" "(" param_list ")" [ "->" type ] block
               | "|" param_short_list "|" expr
               ;

param_short_list = IDENT { "," IDENT } ;

list_literal   = "[" [ expr { "," expr } [ "," ] ] "]" ;

struct_literal = IDENT "{" struct_field_list "}" ;
struct_field_list = struct_field_init { "," struct_field_init } [ "," ] ;
struct_field_init = IDENT ":" expr | IDENT ;

with_expr      = expr "with" "{" struct_field_list "}" ;

pattern        = "_"                                        (* wildcard *)
               | INTEGER | FLOAT | STRING | "true" | "false" (* literal *)
               | IDENT                                       (* binding *)
               | IDENT "@" pattern                           (* named binding *)
               | "(" pattern_list ")"                        (* tuple *)
               | IDENT "{" field_pattern_list "}"            (* struct *)
               | IDENT "(" pattern_list ")"                  (* enum variant *)
               | pattern "::" pattern                        (* list cons *)
               | "[" "]"                                     (* empty list *)
               | "[" pattern { "," pattern } "]"             (* list literal pattern *)
               | pattern "or" pattern                        (* alternation *)
               | INTEGER ".." INTEGER                        (* range *)
               ;

pattern_list       = [ pattern { "," pattern } ] ;
field_pattern_list = [ field_pattern { "," field_pattern } ] ;
field_pattern      = IDENT ":" pattern | IDENT ;

arg_list       = [ arg { "," arg } ] ;
arg            = expr | "_" ;
```

## 12. Turing Completeness

VibeLang is Turing complete. Proof sketch: it supports general recursion (unbounded recursive function calls), conditional branching (`match`/`if`), and unbounded storage (heap-allocated lists can grow without bound). Any partial recursive function can be expressed.

Demonstration via the Ackermann function (which is not primitive recursive, establishing computation beyond primitive recursion):

```
fn ackermann(m: i64, n: i64) -> i64 {
    match (m, n) {
        (0, n) -> n + 1,
        (m, 0) -> ackermann(m - 1, 1),
        (m, n) -> ackermann(m - 1, ackermann(m, n - 1)),
    }
}

fn main() -> IO[unit] {
    effect {
        ackermann(3, 4) |> to_string |> println;
    }
}
```

## 13. Example Programs

### Hello World
```
fn main() -> IO[unit] {
    effect {
        println("Hello, World!");
    }
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

// Efficient tail-recursive version
fn fib_fast(n: i64) -> i64 {
    fn go(n: i64, a: i64, b: i64) -> i64 {
        match n {
            0 -> a,
            n -> go(n - 1, b, a + b),
        }
    };
    go(n, 0, 1)
}

fn main() -> IO[unit] {
    effect {
        List.range(0, 10)
            |> List.map(_, fib_fast)
            |> List.map(_, to_string)
            |> List.map(_, println);
    }
}
```

### FizzBuzz
```
fn fizzbuzz(n: i64) -> String {
    match (n % 3, n % 5) {
        (0, 0) -> "FizzBuzz",
        (0, _) -> "Fizz",
        (_, 0) -> "Buzz",
        _      -> to_string(n),
    }
}

fn main() -> IO[unit] {
    effect {
        List.range(1, 101)
            |> List.map(_, fizzbuzz)
            |> List.map(_, println);
    }
}
```

### Linked List Operations
```
fn sum(xs: List[i64]) -> i64 {
    List.foldl(xs, 0, |acc, x| acc + x)
}

fn product(xs: List[i64]) -> i64 {
    List.foldl(xs, 1, |acc, x| acc * x)
}

fn main() -> IO[unit] {
    effect {
        let xs = [1, 2, 3, 4, 5];
        let s = sum(xs);
        let p = product(xs);
        println("Sum: " ++ to_string(s));
        println("Product: " ++ to_string(p));
    }
}
```

### Binary Search (purely functional)
```
fn binary_search(xs: List[i64], target: i64) -> Option[i64] {
    fn go(xs: List[i64], target: i64, low: i64, high: i64) -> Option[i64] {
        if low > high { None }
        else {
            let mid = (low + high) / 2;
            match List.at(xs, mid) {
                None -> None,
                Some(v) -> match true {
                    _ if v == target -> Some(mid),
                    _ if v < target  -> go(xs, target, mid + 1, high),
                    _                -> go(xs, target, low, mid - 1),
                },
            }
        }
    };
    go(xs, target, 0, List.length(xs) - 1)
}

fn main() -> IO[unit] {
    effect {
        let xs = [1, 3, 5, 7, 9, 11, 13];
        match binary_search(xs, 7) {
            Some(idx) -> println("Found at index: " ++ to_string(idx)),
            None -> println("Not found"),
        };
    }
}
```

### Quicksort
```
fn quicksort(xs: List[i64]) -> List[i64] {
    match xs {
        [] -> [],
        pivot :: rest -> {
            let left = List.filter(rest, |x| x <= pivot);
            let right = List.filter(rest, |x| x > pivot);
            quicksort(left) ++ [pivot] ++ quicksort(right)
        },
    }
}

fn main() -> IO[unit] {
    effect {
        let xs = [3, 6, 1, 8, 2, 9, 4, 7, 5];
        let sorted = quicksort(xs);
        sorted |> List.map(_, to_string) |> List.map(_, println);
    }
}
```

### Tree Data Structure
```
enum Tree[T] {
    Leaf,
    Node(Tree[T], T, Tree[T]),
}

fn insert(tree: Tree[i64], value: i64) -> Tree[i64] {
    match tree {
        Leaf -> Node(Leaf, value, Leaf),
        Node(left, v, right) -> match true {
            _ if value < v  -> Node(insert(left, value), v, right),
            _ if value > v  -> Node(left, v, insert(right, value)),
            _               -> tree,
        },
    }
}

fn inorder(tree: Tree[i64]) -> List[i64] {
    match tree {
        Leaf -> [],
        Node(left, v, right) -> inorder(left) ++ [v] ++ inorder(right),
    }
}

fn main() -> IO[unit] {
    effect {
        let tree = [5, 3, 7, 1, 4, 6, 8]
            |> List.foldl(_, Leaf, insert);
        let sorted = inorder(tree);
        sorted |> List.map(_, to_string) |> List.map(_, println);
    }
}
```

## 14. Compiler Target

The VibeLang compiler (`vibec`) targets x86_64 Linux, producing ELF executables. The compilation pipeline:

1. **Lexer**: Source → Tokens
2. **Parser**: Tokens → AST
3. **Type Checker**: AST → Typed AST (Hindley-Milner type inference with effect tracking)
4. **IR Lowering**: Typed AST → VibeLang IR (ANF/CPS-based intermediate representation)
5. **Optimization**: Tail-call optimization, in-place updates for unique references, deforestation
6. **Code Generation**: IR → x86_64 assembly (NASM syntax)
7. **Runtime**: Reference counting, list/string allocation, IO primitives
8. **Assembly & Linking**: NASM + ld → ELF binary

## 15. File Extension

VibeLang source files use the `.vibe` extension.
