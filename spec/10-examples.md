# 10. Complete Examples

## 10.1 Hello World

```vibe
module main

fn main() -> Unit with IO = do
    print("Hello, VibeLang!")
```

## 10.2 FizzBuzz

```vibe
module main

fn fizzbuzz(n: Int) -> String =
    match (n % 3, n % 5)
    | (0, 0) -> "FizzBuzz"
    | (0, _) -> "Fizz"
    | (_, 0) -> "Buzz"
    | _      -> show(n)

fn main() -> Unit with IO = do
    range(1, 101)
        |> map(fizzbuzz)
        |> for_each(fn(s) = print(s))
```

## 10.3 Parallel Word Count (MapReduce)

```vibe
module word_count

use core.collections.{Map}
use core.string.{split, to_lower}
use core.concurrency.{pmap, preduce}

fn count_words(text: String) -> Map[String, UInt] =
    text
        |> to_lower
        |> split(" ")
        |> fold(Map.empty(), fn(acc, word) =
            let count = Map.get(acc, word) |> unwrap_or(0)
            Map.insert(acc, word, count + 1)
        )

fn merge_counts(a: Map[String, UInt], b: Map[String, UInt]) -> Map[String, UInt] =
    Map.merge(a, b, fn(x, y) = x + y)

fn parallel_word_count(documents: List[String]) -> Map[String, UInt] =
    documents
        |> pmap(count_words)
        |> preduce(Map.empty(), merge_counts)

fn main() -> Unit with IO = do
    let docs = ["hello world hello", "world goodbye hello", "hello hello world"]
    let counts = parallel_word_count(docs)
    Map.entries(counts)
        |> for_each(fn((word, count)) = print("${word}: ${show(count)}"))
```

## 10.4 Concurrent HTTP Server (Sketch)

```vibe
module server

use core.net.{listen, accept, TcpStream}
use core.concurrency.{spawn_actor, ActorRef}

type Request = {
    method: String,
    path: String,
    headers: Map[String, String],
    body: String,
}

type Response = {
    status: UInt,
    headers: Map[String, String],
    body: String,
}

fn handle_request(req: Request) -> Response =
    match req.path
    | "/" -> { status: 200, headers: Map.empty(), body: "Welcome to VibeLang!" }
    | "/health" -> { status: 200, headers: Map.empty(), body: "ok" }
    | _ -> { status: 404, headers: Map.empty(), body: "Not Found" }

fn main() -> Unit with IO = do
    let listener = listen("0.0.0.0", 8080)
    print("Server listening on :8080")

    loop(fn() = do
        let conn = accept(listener)
        -- Each connection handled concurrently
        spawn(fn() = do
            let req = parse_request(conn)
            let resp = handle_request(req)
            send_response(conn, resp)
        )
    )
```

## 10.5 Stream Processing Pipeline

```vibe
module analytics

use core.stream.*
use core.json.{parse_json, Json}
use core.time.{Instant, Duration, seconds}

type Event = {
    timestamp: Instant,
    user_id: String,
    action: String,
    value: Float,
}

fn parse_event(line: String) -> Option[Event] =
    match parse_json(line)
    | Ok(json) -> Some({
        timestamp: json.get("ts") |> flat_map(parse_instant),
        user_id: json.get_string("uid") |> unwrap_or("unknown"),
        action: json.get_string("action") |> unwrap_or("unknown"),
        value: json.get_float("value") |> unwrap_or(0.0),
    })
    | Err(_) -> None

fn compute_metrics(events: Stream[Event]) -> Map[String, Float] =
    events
        |> parallel(chunk_size: 5000)
        |> window(1000, 500)
        |> map(fn(window) = do
            let total = fold(from_list(window), 0.0, fn(acc, e) = acc + e.value)
            let count = length(window) |> to_float
            (total, count)
        )
        |> fold(Map.empty(), fn(acc, (total, count)) =
            Map.insert(acc, "avg", total / count)
        )

fn main() -> Unit with IO = do
    let events = read_lines_stream("events.jsonl")
        |> filter_map(parse_event)

    let metrics = compute_metrics(events)

    Map.entries(metrics)
        |> for_each(fn((key, value)) =
            print("${key}: ${show(value)}")
        )
```

## 10.6 Binary Tree with Pattern Matching

```vibe
module binary_tree

type Tree[A] =
    | Leaf
    | Node(Tree[A], A, Tree[A])

fn insert[A: Ord](tree: Tree[A], value: A) -> Tree[A] =
    match tree
    | Leaf -> Node(Leaf, value, Leaf)
    | Node(left, v, right) ->
        match compare(value, v)
        | Less -> Node(insert(left, value), v, right)
        | Greater -> Node(left, v, insert(right, value))
        | Equal -> Node(left, value, right)

fn contains[A: Ord](tree: Tree[A], value: A) -> Bool =
    match tree
    | Leaf -> false
    | Node(left, v, right) ->
        match compare(value, v)
        | Equal -> true
        | Less -> contains(left, value)
        | Greater -> contains(right, value)

fn to_sorted_list[A](tree: Tree[A]) -> List[A] =
    match tree
    | Leaf -> Nil
    | Node(left, v, right) ->
        to_sorted_list(left) ++ [v] ++ to_sorted_list(right)

fn from_list[A: Ord](items: List[A]) -> Tree[A] =
    fold(items, Leaf, fn(tree, item) = insert(tree, item))

fn height[A](tree: Tree[A]) -> UInt =
    match tree
    | Leaf -> 0
    | Node(left, _, right) -> 1 + max(height(left), height(right))

fn size[A](tree: Tree[A]) -> UInt =
    match tree
    | Leaf -> 0
    | Node(left, _, right) -> 1 + size(left) + size(right)

fn map[A, B](tree: Tree[A], f: fn(A) -> B) -> Tree[B] =
    match tree
    | Leaf -> Leaf
    | Node(left, v, right) -> Node(map(left, f), f(v), map(right, f))

fn fold_in_order[A, B](tree: Tree[A], init: B, f: fn(B, A) -> B) -> B =
    match tree
    | Leaf -> init
    | Node(left, v, right) ->
        let left_result = fold_in_order(left, init, f)
        let with_current = f(left_result, v)
        fold_in_order(right, with_current, f)
```

## 10.7 Effect Handling: Stateful Computation

```vibe
module state_example

use core.collections.Map

type Env = Map[String, Int]

fn eval(expr: Expr) -> Int with State[Env], Fail[EvalError] =
    match expr
    | Lit(n) -> n
    | Var(name) -> do
        let env = get()
        match Map.get(env, name)
        | Some(value) -> value
        | None -> fail(UndefinedVariable(name))
    | Add(a, b) -> eval(a) + eval(b)
    | Let(name, value, body) -> do
        let v = eval(value)
        let env = get()
        put(Map.insert(env, name, v))
        eval(body)

fn run_eval(expr: Expr) -> Result[Int, EvalError] =
    let initial_env = Map.empty()
    catch(fn() =
        handle eval(expr)
            with State[Env] {
                get() -> resume(initial_env)
                put(new_env) -> do
                    let initial_env = new_env
                    resume(())
            }
    )

fn main() -> Unit with IO = do
    -- let x = 10 in let y = 20 in x + y
    let program = Let("x", Lit(10),
                    Let("y", Lit(20),
                      Add(Var("x"), Var("y"))))
    match run_eval(program)
    | Ok(result) -> print("Result: ${show(result)}")
    | Err(e) -> print("Error: ${show(e)}")
```
