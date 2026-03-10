#ifndef VIBE_RUNTIME_H
#define VIBE_RUNTIME_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ============================================================
 * VibeLang Concurrency Runtime
 *
 * Provides a work-stealing thread pool for parallel execution
 * of par(), pmap(), and race() combinators.
 * ============================================================ */

/* --- Thread Pool --- */

/* Initialize the global thread pool. Called once at program start.
 * num_threads=0 means auto-detect (number of CPU cores). */
void vibe_runtime_init(int num_threads);

/* Shut down the global thread pool. Called at program exit. */
void vibe_runtime_shutdown(void);

/* --- Task Definitions --- */

/* A thunk is a no-argument function returning i64. */
typedef int64_t (*vibe_thunk_fn)(void);

/* A map function takes an i64 and returns an i64. */
typedef int64_t (*vibe_map_fn)(int64_t);

/* --- Par Combinator --- */

/* Execute N thunks in parallel, store results in out_results[].
 * Blocks until all thunks complete. */
void vibe_par_execute(vibe_thunk_fn *thunks, int64_t *out_results, int n);

/* --- Pmap Combinator --- */

/* Cons cell layout: { int64_t value; void *next; } */
typedef struct vibe_cons {
    int64_t value;
    struct vibe_cons *next;
} vibe_cons_t;

/* Count elements in a cons list. */
int64_t vibe_list_count(vibe_cons_t *list);

/* Parallel map: apply fn to each element of list, return new list.
 * region_ptr is passed through for allocation (may be NULL for malloc). */
vibe_cons_t *vibe_pmap_list(vibe_cons_t *list, vibe_map_fn fn, void *region_ptr);

/* --- Race Combinator --- */

/* Execute N thunks in parallel, return the result of the first to complete.
 * Remaining thunks are cancelled (best-effort). */
int64_t vibe_race_execute(vibe_thunk_fn *thunks, int n);

/* --- Pfilter Combinator --- */

/* A predicate function takes an i64 and returns non-zero for true. */
typedef int64_t (*vibe_pred_fn)(int64_t);

/* A reduce function takes two i64 values and returns one. */
typedef int64_t (*vibe_reduce_fn)(int64_t, int64_t);

/* Parallel filter: keep elements where pred returns non-zero. */
vibe_cons_t *vibe_pfilter_list(vibe_cons_t *list, vibe_pred_fn pred, void *region_ptr);

/* --- Preduce Combinator --- */

/* Parallel reduce via tree reduction. Function must be associative. */
int64_t vibe_preduce_list(vibe_cons_t *list, int64_t init, vibe_reduce_fn fn);

/* --- Channels --- */

typedef struct vibe_channel vibe_channel_t;

vibe_channel_t *vibe_channel_create(int64_t capacity);
void vibe_channel_send(vibe_channel_t *ch, int64_t value);
int64_t vibe_channel_recv(vibe_channel_t *ch);
void vibe_channel_close(vibe_channel_t *ch);
void vibe_channel_destroy(vibe_channel_t *ch);

/* --- Pipeline Utility Functions --- */

typedef int64_t (*vibe_key_fn)(int64_t);

vibe_cons_t *vibe_list_distinct_by(vibe_cons_t *list, vibe_key_fn key_fn, void *region);
vibe_cons_t *vibe_list_window(vibe_cons_t *list, int64_t size, int64_t stride, void *region);
vibe_cons_t *vibe_list_zip(vibe_cons_t *a, vibe_cons_t *b, void *region);
int64_t vibe_list_min_by(vibe_cons_t *list, vibe_key_fn key_fn);
int64_t vibe_list_max_by(vibe_cons_t *list, vibe_key_fn key_fn);
vibe_cons_t *vibe_list_merge(vibe_cons_t *a, vibe_cons_t *b, void *region);

/* --- String Utility Functions --- */

char *vibe_trim_start(const char *s);
char *vibe_trim_end(const char *s);
char *vibe_from_chars(vibe_cons_t *chars);

/* ============================================================
 * Effect Handler Runtime
 *
 * Implements algebraic effect handlers with perform/resume.
 * Uses a handler stack (thread-local) for effect dispatch.
 * ============================================================ */

/* Handler function type: takes (arg, resume_fn, user_data) -> result */
typedef int64_t (*vibe_handler_fn)(int64_t arg, void *resume_fn, void *user_data);

/* Push a handler onto the handler stack.
 * effect_hash: hash of the effect name
 * op_hash: hash of the operation name
 * handler: the handler function
 * user_data: opaque pointer passed to handler */
void vibe_handler_push(uint64_t effect_hash, uint64_t op_hash,
                       vibe_handler_fn handler, void *user_data);

/* Pop the topmost handler from the stack. */
void vibe_handler_pop(void);

/* Perform an effect: look up the handler and invoke it.
 * Returns the handler's result. */
int64_t vibe_handler_perform(uint64_t effect_hash, uint64_t op_hash, int64_t arg);

/* Resume a suspended effect handler with a value. */
int64_t vibe_handler_resume(int64_t value);

/* ============================================================
 * Async/Await Runtime
 *
 * Lightweight futures backed by the thread pool.
 * ============================================================ */

/* Opaque future handle */
typedef struct vibe_future vibe_future_t;

/* Spawn an async task: runs thunk on the thread pool, returns a future. */
vibe_future_t *vibe_async_spawn(vibe_thunk_fn thunk);

/* Await a future: blocks until the result is available, returns the value. */
int64_t vibe_async_await(vibe_future_t *future);

/* Spawn a lightweight task (fire-and-forget). */
void vibe_task_spawn(vibe_thunk_fn thunk);

/* ============================================================
 * Actor Runtime
 *
 * Lightweight actors with message passing.
 * Each actor has a mailbox (bounded channel) and runs on
 * the thread pool.
 * ============================================================ */

/* Opaque actor handle */
typedef struct vibe_actor vibe_actor_t;

/* Actor message handler: takes (actor_state, message) -> new_state */
typedef int64_t (*vibe_actor_handler_fn)(int64_t state, int64_t message);

/* Spawn an actor with an initial state and message handler.
 * Returns an actor reference for sending messages. */
vibe_actor_t *vibe_actor_spawn(int64_t initial_state, vibe_actor_handler_fn handler);

/* Send a message to an actor. Non-blocking (buffered in mailbox). */
void vibe_actor_send(vibe_actor_t *actor, int64_t message);

/* Receive a message from the current actor's mailbox (blocking). */
int64_t vibe_actor_recv(vibe_actor_t *actor);

/* Stop an actor. */
void vibe_actor_stop(vibe_actor_t *actor);

/* ============================================================
 * Channel Select
 *
 * Multiplexed receive from multiple channels.
 * ============================================================ */

/* Select from multiple channels: returns the index of the first
 * channel that has data available, and stores the value in *out_value. */
int64_t vibe_channel_select(vibe_channel_t **channels, int n, int64_t *out_value);

/* Non-blocking receive: returns 1 if a value was available, 0 otherwise. */
int64_t vibe_channel_try_recv(vibe_channel_t *ch, int64_t *out_value);

/* ============================================================
 * Standard Library Runtime Support
 * ============================================================ */

/* Vec operations (backed by dynamic arrays) */
typedef struct vibe_vec {
    int64_t *data;
    int64_t length;
    int64_t capacity;
} vibe_vec_t;

vibe_vec_t *vibe_vec_new(void);
vibe_vec_t *vibe_vec_push(vibe_vec_t *v, int64_t value);
int64_t vibe_vec_get(vibe_vec_t *v, int64_t index);
vibe_vec_t *vibe_vec_set(vibe_vec_t *v, int64_t index, int64_t value);
int64_t vibe_vec_length(vibe_vec_t *v);
void vibe_vec_free(vibe_vec_t *v);
vibe_cons_t *vibe_vec_to_list(vibe_vec_t *v, void *region);

/* Map operations (backed by hash tables) */
typedef struct vibe_map vibe_map_t;

vibe_map_t *vibe_map_new(void);
vibe_map_t *vibe_map_insert(vibe_map_t *m, int64_t key, int64_t value);
int64_t vibe_map_get(vibe_map_t *m, int64_t key);
int64_t vibe_map_contains(vibe_map_t *m, int64_t key);
int64_t vibe_map_size(vibe_map_t *m);
void vibe_map_free(vibe_map_t *m);

/* Set operations (backed by hash sets) */
typedef struct vibe_set vibe_set_t;

vibe_set_t *vibe_set_new(void);
vibe_set_t *vibe_set_insert(vibe_set_t *s, int64_t value);
int64_t vibe_set_contains(vibe_set_t *s, int64_t value);
int64_t vibe_set_size(vibe_set_t *s);
void vibe_set_free(vibe_set_t *s);

/* String operations */
char *vibe_string_concat(const char *a, const char *b);
char *vibe_string_substring(const char *s, int64_t start, int64_t end);
int64_t vibe_string_length(const char *s);
int64_t vibe_string_contains(const char *haystack, const char *needle);
char *vibe_string_replace(const char *s, const char *from, const char *to);
char *vibe_string_to_upper(const char *s);
char *vibe_string_to_lower(const char *s);

#ifdef __cplusplus
}
#endif

#endif /* VIBE_RUNTIME_H */
