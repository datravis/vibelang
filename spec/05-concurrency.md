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

## 5.2 The `vibe` Keyword — Concurrent Pipelines

The `vibe` keyword declares a **concurrent data pipeline** — the central abstraction in
VibeLang. A vibe is a structured, parallel, composable dataflow computation expressed as
a chain of stages.

```
vibe word_frequencies =
    source(read_lines("corpus.txt"))
    |> map(to_lower)
    |> flat_map(fn(line) = split(line, " "))
    |> group_by(identity)
    |> map(fn((word, occurrences)) = (word, length(occurrences)))
    |> sort_by(fn((_, count)) = count, descending: true)
    |> take(100)
    |> collect
```

This is the construct that makes VibeLang *VibeLang*. Where other languages treat
parallelism as an afterthought (thread pools, async/await bolted onto sequential code),
VibeLang makes **parallel dataflow a first-class language construct**.

### What makes `vibe` special

1. **Automatic parallelism.** The runtime partitions data across cores, parallelizing
   stages that are safe to parallelize. The programmer describes the *what*; the runtime
   handles the *how*.

2. **Built-in backpressure.** If a downstream stage is slower than upstream, the pipeline
   automatically throttles producers. No unbounded queues, no out-of-memory crashes.

3. **Structured lifetime.** A `vibe` is a single structured concurrency scope. When the
   pipeline completes (or fails), all stages are torn down. No leaked goroutines, no
   orphaned threads.

4. **Fusion.** The compiler fuses adjacent stateless stages (map, filter) into a single
   pass, eliminating intermediate allocations. A chain of five `map` calls compiles to
   one loop.

5. **It's an expression.** A `vibe` evaluates to its terminal value. It can be bound,
   passed to functions, or composed with other vibes.

### Anatomy of a Vibe

```
vibe <name> =
    source(<data>)        -- where data comes from
    |> <stage>            -- zero or more transformation stages
    |> <stage>
    |> <terminal>         -- how to collect the result
```

A vibe has three parts:

- **Source**: Where data enters the pipeline.
- **Stages**: Transformations applied to each element (map, filter, window, etc.).
- **Terminal**: How results are collected (collect, fold, for_each, count, etc.).

### Sources

```
source(list)                         -- from a List or Vec
source(range(1, 1_000_000))         -- from a range
source(read_lines("file.txt"))      -- from a file (lazy, line by line)
source(channel_receiver)             -- from a channel
source(iterate(0, fn(n) = n + 1))   -- from an infinite generator
```

Sources are lazy — they produce values on demand, not all at once.

### Stages

Stages transform data as it flows through the pipeline:

```
-- Per-element transforms
|> map(f)                    -- apply f to each element
|> filter(pred)              -- keep elements where pred is true
|> flat_map(f)               -- map then flatten
|> filter_map(f)             -- map, keeping only Some results
|> inspect(f)                -- side-effect per element (logging), passes through

-- Ordering and uniqueness
|> sort_by(key_fn)           -- sort (requires finite stream)
|> distinct                  -- remove duplicates
|> distinct_by(key_fn)       -- remove duplicates by key

-- Grouping and windowing
|> group_by(key_fn)          -- group into Map[K, List[V]]
|> chunk(size)               -- group into fixed-size chunks
|> window(size, stride)      -- sliding window
|> batch(timeout, max_size)  -- time-or-size bounded batches

-- Cardinality
|> take(n)                   -- first n elements
|> drop(n)                   -- skip first n elements
|> take_while(pred)          -- take while predicate holds
|> drop_while(pred)          -- drop while predicate holds

-- Accumulation
|> scan(init, f)             -- running fold, emitting each intermediate value
|> zip(other_source)         -- pair elements from two sources

-- Fan-out / fan-in
|> broadcast(n)              -- duplicate stream to n consumers
|> merge(other_vibe)         -- interleave elements from another pipeline
```

### Terminals

Terminals consume the pipeline and produce a final result:

```
|> collect                   -- List[A]
|> collect_vec               -- Vec[A]
|> collect_map(key_fn)       -- Map[K, V]
|> fold(init, f)             -- single accumulated value
|> reduce(f)                 -- fold without initial value (requires non-empty)
|> for_each(f)               -- side-effect per element, returns Unit
|> count                     -- UInt
|> any(pred)                 -- Bool
|> all(pred)                 -- Bool
|> first                     -- Option[A]
|> last                      -- Option[A]
|> min_by(key_fn)            -- Option[A]
|> max_by(key_fn)            -- Option[A]
```

### Parallelism Control

By default, the runtime chooses how to parallelize. You can provide hints:

```
vibe result =
    source(data)
    |> parallel(workers: 8, chunk_size: 1000)  -- explicit parallelism hint
    |> map(expensive_computation)
    |> filter(is_valid)
    |> fold(0, fn(a, b) = a + b)
```

Or force sequential execution:

```
vibe result =
    source(data)
    |> sequential              -- no parallelism, preserve order strictly
    |> map(computation)
    |> collect
```

### Composing Vibes

Vibes are values. They can be composed:

```
vibe parsed_events =
    source(read_lines("events.jsonl"))
    |> filter_map(parse_event)

vibe high_value_events =
    parsed_events
    |> filter(fn(e) = e.value > 1000.0)
    |> collect

vibe event_summary =
    parsed_events
    |> group_by(fn(e) = e.action)
    |> map(fn((action, events)) = (action, length(events)))
    |> collect_map(fn((k, v)) = (k, v))
```

### Named Vibes as Reusable Pipelines

Top-level `vibe` declarations can take parameters, creating reusable pipeline templates:

```
vibe normalize_text(input: List[String]) -> List[String] =
    source(input)
    |> map(trim)
    |> map(to_lower)
    |> filter(fn(s) = !is_empty(s))
    |> distinct
    |> sort_by(identity)
    |> collect

-- Use it like a function
let clean_words = normalize_text(raw_words)
```

### Infinite Vibes

Vibes over infinite sources run until stopped by a cardinality stage or external signal:

```
vibe sensor_monitor =
    source(sensor_stream())
    |> window(100, 1)
    |> map(fn(readings) = average(readings))
    |> filter(fn(avg) = avg > threshold)
    |> for_each(fn(avg) = send_alert(avg))
```

### Error Handling in Vibes

Vibes interact with the effect system. Stages can perform effects:

```
vibe processed =
    source(file_paths)
    |> map(fn(path) = read_file(path))  -- with IO
    |> filter_map(fn(result) =
        match result
        | Ok(content) -> Some(content)
        | Err(_) -> None                 -- skip failed reads
    )
    |> collect
```

If a stage panics, the entire vibe is cancelled and the panic propagates to the caller.

## 5.3 Parallel Combinators

For parallelism outside of pipelines, VibeLang provides combinators:

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

### race — First to Complete

Runs multiple computations in parallel and returns the first to complete, cancelling the rest:

```
let fastest = race(
    fn() = query_primary_db(id),
    fn() = query_replica_db(id),
    fn() = query_cache(id),
)
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

Channels can also serve as sources for vibes:

```
vibe consumer_pipeline =
    source(rx)
    |> map(fn(x) = x * 2)
    |> filter(fn(x) = x > 100)
    |> collect
```

## 5.5 Structured Concurrency

All parallel computations in VibeLang are **structured** — they have a clear parent scope
and cannot outlive it. This applies to `par`, `pmap`, and `vibe` equally:

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
- **Automatic batching** for `vibe` pipeline stages to minimize scheduling overhead.
- **Backpressure propagation** through bounded internal buffers between pipeline stages.

The runtime is initialized by `main` and torn down when `main` returns. There is no way
to access the runtime outside of the effect system.

## 5.8 When to Use What

| Need | Use |
|------|-----|
| Transform a data collection through multiple steps | `vibe` pipeline |
| Run 2-8 independent tasks in parallel | `par` |
| Apply one function to many items in parallel | `pmap` |
| Process an infinite/streaming data source | `vibe` with infinite source |
| Communicate between concurrent tasks | Channels (optionally fed into a `vibe`) |
| Long-running stateful service | Actor |
| First result from redundant computations | `race` |
