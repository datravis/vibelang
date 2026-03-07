# 4. Algebraic Effects and Handlers

VibeLang is purely functional — all side effects are tracked in the type system using
**algebraic effects**. This provides the composability of monads without the syntactic
overhead, and enables the compiler to reason about what a function can and cannot do.

## 4.1 Declaring Effects

Effects are declared as a set of operations:

```
effect IO {
    fn read_file(path: String) -> Result[String, IOError]
    fn write_file(path: String, content: String) -> Result[Unit, IOError]
    fn print(message: String) -> Unit
    fn read_line() -> String
}

effect State[S] {
    fn get() -> S
    fn put(new_state: S) -> Unit
}

effect Fail[E] {
    fn fail(error: E) -> Never
}

effect Async {
    fn await[A](future: Future[A]) -> A
    fn spawn[A](task: fn() -> A) -> Future[A]
}

effect Random {
    fn random_int(min: Int, max: Int) -> Int
    fn random_float() -> Float
}
```

## 4.2 Using Effects

Functions that perform effects declare them in their signature with `with`:

```
fn greet_user() -> Unit with IO = do
    print("What is your name?")
    let name = read_line()
    print("Hello, ${name}!")

fn increment_counter() -> Int with State[Int] = do
    let current = get()
    put(current + 1)
    current + 1

fn parse_config(raw: String) -> Config with Fail[ParseError] =
    match parse(raw)
    | Ok(config) -> config
    | Err(e) -> fail(e)
```

### Multiple Effects

```
fn interactive_computation(input: String) -> Int with IO, State[Int], Fail[Error] = do
    print("Processing: ${input}")
    let current = get()
    let result = parse_int(input) |> map_err(fail)
    put(current + result)
    current + result
```

## 4.3 Effect Handlers

Handlers provide the implementation for effects, and they **remove** the effect from the
type. This is how effects are ultimately resolved:

```
fn main() -> Unit with IO = do
    let result = handle increment_counter()
        with State[Int] {
            get() -> resume(0)            -- initial state is 0
            put(new_state) -> resume(())  -- accept the state update
        }
    print("Result: ${show(result)}")
```

### Handler Semantics

A handler wraps a computation and intercepts its effect operations:
- `resume(value)` continues the computation with `value` as the result of the operation.
- Not calling `resume` aborts the handled computation.

```
fn run_with_state[S, A](initial: S, computation: fn() -> A with State[S]) -> (A, S) =
    let state = initial
    handle computation()
        with State[S] {
            get() -> resume(state)
            put(new_state) -> do
                let state = new_state
                resume(())
        }
    return (result, state)
```

### Fail Handler (Converting to Result)

```
fn catch[A, E](computation: fn() -> A with Fail[E]) -> Result[A, E] =
    handle computation()
        with Fail[E] {
            fail(error) -> Err(error)   -- do not resume; return Err
        }
    |> Ok
```

## 4.4 Effect Polymorphism

Functions can be polymorphic over effects:

```
fn twice[A, E](f: fn() -> A with E) -> (A, A) with E = do
    let first = f()
    let second = f()
    (first, second)
```

This function works with *any* set of effects `E` — it simply propagates whatever effects
`f` performs.

## 4.5 Pure Functions

A function with no `with` clause is pure — it performs no effects:

```
fn add(a: Int, b: Int) -> Int = a + b
-- Guaranteed: no I/O, no state, no failure, no randomness
```

The compiler enforces this statically. Pure functions can be:
- Memoized safely
- Reordered or parallelized without observable difference
- Called from any context

## 4.6 The IO Effect and main

The program entry point must have the signature:

```
fn main() -> Unit with IO = ...
```

This is the only place where `IO` effects are implicitly handled by the runtime. All other
functions must explicitly handle or propagate their effects.

## 4.7 Effect Safety Guarantees

The effect system guarantees at compile time:
1. **No hidden side effects.** If a function's signature has no `with` clause, it is pure.
2. **All effects are handled.** Every effect declared in a function must be handled before
   reaching `main`, except `IO` which is handled by the runtime.
3. **Effect encapsulation.** A handler completely encapsulates the behavior of an effect —
   swapping handlers changes behavior without modifying the computation.
