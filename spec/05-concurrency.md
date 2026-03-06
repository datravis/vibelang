# 5. Concurrency and Parallelism

VibeLang is designed for **safe, high-performance parallel computation**. Because all data
is immutable, parallel execution is free from data races by construction. The language
provides structured concurrency primitives and a parallel-aware runtime.

## 5.1 Core Philosophy

- **No threads, no locks, no mutexes.** These low-level primitives do not exist in VibeLang.
- **Parallelism is declarative.** You describe *what* can run in parallel; the runtime
  decides *how* to schedule it.
- **Concurrency is structured.** All concurrent computations have a well-defined lifetime
  scoped to their parent.

## 5.2 Parallel Combinators

### par — Parallel Evaluation

Evaluates multiple independent expressions in parallel and returns all results:

```
let (users, posts, comments) = par(
    fn() = fetch_users(db),
    fn() = fetch_posts(db),
    fn() = fetch_comments(db),
)
```

Type signature:
```
fn par[A, B](a: fn() -> A with E, b: fn() -> B with E) -> (A, B) with E
fn par[A, B, C](a: fn() -> A with E, b: fn() -> B with E, c: fn() -> C with E) -> (A, B, C) with E
-- ... up to 8-ary par, then use par_list
```

### pmap — Parallel Map

Applies a function to every element of a collection in parallel:

```
let results = pmap(urls, fn(url) = fetch(url))

-- With chunk size hint for fine-grained control
let processed = pmap(data, transform, chunk_size: 1000)
```

Type signature:
```
fn pmap[A, B](items: List[A], f: fn(A) -> B with E) -> List[B] with E
fn pmap[A, B](items: Array[A], f: fn(A) -> B with E, chunk_size: UInt = auto) -> Array[B] with E
```

### pfilter — Parallel Filter

```
let valid = pfilter(records, fn(r) = is_valid(r))
```

### preduce — Parallel Reduce

Reduces a collection using an associative operation, parallelized via tree reduction:

```
let total = preduce(numbers, 0, fn(a, b) = a + b)
```

The combining function **must be associative**. The compiler does not verify this; incorrect
results may occur if this contract is violated. Use `reduce` for non-associative operations.

```
fn preduce[A](items: List[A], identity: A, combine: fn(A, A) -> A) -> A
```

### race — First to Complete

Runs multiple computations in parallel and returns the first to complete, cancelling the rest:

```
let fastest = race(
    fn() = query_primary_db(id),
    fn() = query_replica_db(id),
    fn() = query_cache(id),
)
```

## 5.3 Streams

Streams represent lazy, potentially infinite sequences of values that can be processed
in a pipeline with automatic parallelism:

```
type Stream[A]

fn from_list[A](items: List[A]) -> Stream[A]
fn range(start: Int, end: Int) -> Stream[Int]
fn repeat[A](value: A) -> Stream[A]            -- infinite
fn iterate[A](seed: A, f: fn(A) -> A) -> Stream[A]  -- infinite
```

### Stream Operators

```
fn map[A, B](s: Stream[A], f: fn(A) -> B) -> Stream[B]
fn filter[A](s: Stream[A], pred: fn(A) -> Bool) -> Stream[A]
fn flat_map[A, B](s: Stream[A], f: fn(A) -> Stream[B]) -> Stream[B]
fn take[A](s: Stream[A], n: UInt) -> Stream[A]
fn drop[A](s: Stream[A], n: UInt) -> Stream[A]
fn fold[A, B](s: Stream[A], init: B, f: fn(B, A) -> B) -> B
fn zip[A, B](a: Stream[A], b: Stream[B]) -> Stream[(A, B)]
fn chunk[A](s: Stream[A], size: UInt) -> Stream[List[A]]
fn window[A](s: Stream[A], size: UInt, stride: UInt) -> Stream[List[A]]
fn scan[A, B](s: Stream[A], init: B, f: fn(B, A) -> B) -> Stream[B]
fn merge[A](streams: List[Stream[A]]) -> Stream[A]
```

### Parallel Stream Execution

Streams are sequential by default. Use `.parallel()` to enable parallel execution of
pipeline stages:

```
let result = range(1, 1_000_000)
    |> parallel(chunk_size: 10_000)
    |> map(expensive_computation)
    |> filter(is_valid)
    |> fold(0, fn(acc, x) = acc + x)
```

### Stream Sinks (Collecting Results)

```
fn collect[A](s: Stream[A]) -> List[A]
fn collect_array[A](s: Stream[A]) -> Array[A]
fn for_each[A](s: Stream[A], f: fn(A) -> Unit with E) -> Unit with E
fn count[A](s: Stream[A]) -> UInt
fn any[A](s: Stream[A], pred: fn(A) -> Bool) -> Bool
fn all[A](s: Stream[A], pred: fn(A) -> Bool) -> Bool
```

## 5.4 Channels

Channels provide typed, bounded communication between concurrent computations:

```
effect Channel[A] {
    fn send(ch: Sender[A], value: A) -> Unit
    fn recv(ch: Receiver[A]) -> Option[A]
}

fn create_channel[A](capacity: UInt) -> (Sender[A], Receiver[A]) with Channel[A]
```

Example:

```
fn producer_consumer() -> List[Int] with IO = do
    let (tx, rx) = create_channel[Int](100)

    par(
        fn() = do
            range(1, 1000)
                |> for_each(fn(i) = send(tx, i * 2))
            close(tx)
        ,
        fn() = do
            recv_all(rx) |> collect
        ,
    ).1  -- return consumer's result
```

## 5.5 Structured Concurrency

All parallel computations in VibeLang are **structured** — they have a clear parent scope
and cannot outlive it:

```
fn process_batch(items: List[Item]) -> List[Result] with IO = do
    -- All parallel work completes before process_batch returns
    let results = pmap(items, fn(item) = do
        let data = fetch(item.url)
        transform(data)
    )
    results
```

If any parallel branch panics, all sibling branches are cancelled and the panic propagates
to the parent.

### Timeout

```
fn with_timeout[A](duration: Duration, f: fn() -> A with E) -> Option[A] with E
```

```
let result = with_timeout(seconds(5), fn() = slow_computation())
match result
| Some(value) -> print("Got: ${show(value)}")
| None -> print("Timed out")
```

## 5.6 Actors (Lightweight Processes)

For long-running concurrent services, VibeLang provides an actor model built on effects:

```
effect Actor[Msg] {
    fn receive() -> Msg
    fn self() -> ActorRef[Msg]
}

type ActorRef[Msg]

fn spawn_actor[Msg](handler: fn() -> Never with Actor[Msg]) -> ActorRef[Msg] with IO

fn send_to[Msg](actor: ActorRef[Msg], message: Msg) -> Unit with IO
```

Example:

```
type CounterMsg =
    | Increment
    | Decrement
    | GetCount(ActorRef[Int])

fn counter_actor() -> Never with Actor[CounterMsg], State[Int] = do
    let msg = receive()
    match msg
    | Increment -> do
        let n = get()
        put(n + 1)
    | Decrement -> do
        let n = get()
        put(n - 1)
    | GetCount(reply_to) -> do
        let n = get()
        send_to(reply_to, n)
    counter_actor()  -- loop forever
```

## 5.7 Runtime Execution Model

The VibeLang runtime uses:

- **Work-stealing thread pool** sized to the number of CPU cores (configurable).
- **Lightweight tasks** (green threads) multiplexed onto OS threads.
- **Cache-aware scheduling** that keeps related data on the same core when possible.
- **Automatic batching** for stream operations to minimize scheduling overhead.

The runtime is initialized by `main` and torn down when `main` returns. There is no way
to access the runtime outside of the effect system.
