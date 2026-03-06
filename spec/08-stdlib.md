# 8. Standard Library

The VibeLang standard library (`core`) provides foundational types, traits, and functions.
It is organized into focused modules.

## 8.1 Module Overview

```
core
├── prelude         -- auto-imported: Option, Result, List, basic traits
├── string          -- String manipulation
├── math            -- numeric operations, constants
├── collections     -- Vec, Map, Set, Queue
├── array           -- contiguous mutable-length arrays
├── io              -- file I/O, stdio
├── net             -- TCP/UDP networking
├── stream          -- lazy stream processing
├── concurrency     -- par, pmap, channels, actors
├── time            -- Duration, Instant, date/time
├── json            -- JSON serialization/deserialization
├── text            -- regex, Unicode utilities
├── hash            -- hashing algorithms
├── fmt             -- string formatting
├── test            -- testing framework
└── unsafe          -- unchecked operations
```

## 8.2 Prelude (Auto-Imported)

The following are available in every module without an explicit `use`:

```
-- Types
type Option[A] = Some(A) | None
type Result[A, E] = Ok(A) | Err(E)
type List[A] = Cons(A, List[A]) | Nil
type Ordering = Less | Equal | Greater
type Bool = true | false
type Unit = ()

-- Traits
trait Eq[A]
trait Ord[A]
trait Show[A]
trait Hash[A]
trait Copy[A]
trait Default[A]

-- Functions
fn identity[A](x: A) -> A
fn const[A, B](x: A, _: B) -> A
fn flip[A, B, C](f: fn(A, B) -> C) -> fn(B, A) -> C
fn compose[A, B, C](f: fn(B) -> C, g: fn(A) -> B) -> fn(A) -> C
fn panic(message: String) -> Never
fn todo(message: String) -> Never  -- placeholder for unimplemented code
fn assert(condition: Bool, message: String) -> Unit
```

## 8.3 core.string

```
fn length(s: ref String) -> UInt
fn is_empty(s: ref String) -> Bool
fn contains(s: ref String, sub: ref String) -> Bool
fn starts_with(s: ref String, prefix: ref String) -> Bool
fn ends_with(s: ref String, suffix: ref String) -> Bool
fn trim(s: String) -> String
fn trim_start(s: String) -> String
fn trim_end(s: String) -> String
fn to_upper(s: String) -> String
fn to_lower(s: String) -> String
fn split(s: String, delimiter: ref String) -> List[String]
fn join(parts: ref List[String], separator: ref String) -> String
fn replace(s: String, from: ref String, to: ref String) -> String
fn substring(s: ref String, start: UInt, end: UInt) -> String
fn chars(s: ref String) -> List[Char]
fn from_chars(chars: List[Char]) -> String
fn parse_int(s: ref String) -> Option[Int]
fn parse_float(s: ref String) -> Option[Float]
```

## 8.4 core.collections

### Vec[A] — Persistent Vector

```
fn empty[A]() -> Vec[A]
fn singleton[A](value: A) -> Vec[A]
fn from_list[A](items: List[A]) -> Vec[A]
fn push[A](vec: Vec[A], value: A) -> Vec[A]
fn get[A](vec: ref Vec[A], index: UInt) -> Option[A]
fn set[A](vec: Vec[A], index: UInt, value: A) -> Vec[A]
fn length[A](vec: ref Vec[A]) -> UInt
fn map[A, B](vec: Vec[A], f: fn(A) -> B) -> Vec[B]
fn filter[A](vec: Vec[A], pred: fn(ref A) -> Bool) -> Vec[A]
fn fold[A, B](vec: Vec[A], init: B, f: fn(B, A) -> B) -> B
fn concat[A](a: Vec[A], b: Vec[A]) -> Vec[A]
fn slice[A](vec: ref Vec[A], start: UInt, end: UInt) -> Vec[A]
fn to_list[A](vec: Vec[A]) -> List[A]
fn to_stream[A](vec: Vec[A]) -> Stream[A]
```

### Map[K, V] — Persistent Hash Map

```
fn empty[K, V]() -> Map[K, V]
fn singleton[K: Hash + Eq, V](key: K, value: V) -> Map[K, V]
fn insert[K: Hash + Eq, V](m: Map[K, V], key: K, value: V) -> Map[K, V]
fn get[K: Hash + Eq, V](m: ref Map[K, V], key: ref K) -> Option[ref V]
fn remove[K: Hash + Eq, V](m: Map[K, V], key: ref K) -> Map[K, V]
fn contains_key[K: Hash + Eq, V](m: ref Map[K, V], key: ref K) -> Bool
fn size[K, V](m: ref Map[K, V]) -> UInt
fn keys[K, V](m: ref Map[K, V]) -> List[K]
fn values[K, V](m: ref Map[K, V]) -> List[V]
fn entries[K, V](m: ref Map[K, V]) -> List[(K, V)]
fn merge[K: Hash + Eq, V](a: Map[K, V], b: Map[K, V], resolve: fn(V, V) -> V) -> Map[K, V]
fn map_values[K, V, W](m: Map[K, V], f: fn(V) -> W) -> Map[K, W]
fn filter[K: Hash + Eq, V](m: Map[K, V], pred: fn(ref K, ref V) -> Bool) -> Map[K, V]
```

### Set[A] — Persistent Hash Set

```
fn empty[A]() -> Set[A]
fn singleton[A: Hash + Eq](value: A) -> Set[A]
fn insert[A: Hash + Eq](s: Set[A], value: A) -> Set[A]
fn remove[A: Hash + Eq](s: Set[A], value: ref A) -> Set[A]
fn contains[A: Hash + Eq](s: ref Set[A], value: ref A) -> Bool
fn size[A](s: ref Set[A]) -> UInt
fn union[A: Hash + Eq](a: Set[A], b: Set[A]) -> Set[A]
fn intersection[A: Hash + Eq](a: Set[A], b: ref Set[A]) -> Set[A]
fn difference[A: Hash + Eq](a: Set[A], b: ref Set[A]) -> Set[A]
fn to_list[A](s: Set[A]) -> List[A]
```

## 8.5 core.math

```
-- Constants
let pi: Float = 3.14159265358979323846
let e: Float = 2.71828182845904523536
let tau: Float = 6.28318530717958647692

-- Functions
fn abs[A: Ord + Default](x: A) -> A
fn min[A: Ord](a: A, b: A) -> A
fn max[A: Ord](a: A, b: A) -> A
fn clamp[A: Ord](value: A, low: A, high: A) -> A
fn pow(base: Float, exponent: Float) -> Float
fn sqrt(x: Float) -> Float
fn log(x: Float) -> Float
fn log2(x: Float) -> Float
fn log10(x: Float) -> Float
fn sin(x: Float) -> Float
fn cos(x: Float) -> Float
fn tan(x: Float) -> Float
fn floor(x: Float) -> Int
fn ceil(x: Float) -> Int
fn round(x: Float) -> Int
```

## 8.6 core.test

```
--- Declare a test case.
fn test(name: String, body: fn() -> Unit with Fail[TestError]) -> TestCase

--- Assert that a condition is true.
fn expect(condition: Bool, message: String) -> Unit with Fail[TestError]

--- Assert equality.
fn expect_eq[A: Eq + Show](actual: A, expected: A) -> Unit with Fail[TestError]

--- Assert that a computation produces an Err.
fn expect_err[A, E](result: Result[A, E]) -> Unit with Fail[TestError]

--- Assert that a computation produces a Some.
fn expect_some[A](option: Option[A]) -> A with Fail[TestError]
```

Example test file:

```
module parser_test

use core.test.*
use parser.{parse, Expression}

test("parse integer literal", fn() = do
    let result = parse("42")
    expect_eq(result, Ok(Expression.IntLit(42)))
)

test("parse addition", fn() = do
    let result = parse("1 + 2")
    expect_eq(result, Ok(Expression.Add(IntLit(1), IntLit(2))))
)

test("parse error on empty input", fn() = do
    let result = parse("")
    expect_err(result)
)
```
