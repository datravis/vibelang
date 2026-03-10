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

#ifdef __cplusplus
}
#endif

#endif /* VIBE_RUNTIME_H */
