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

#ifdef __cplusplus
}
#endif

#endif /* VIBE_RUNTIME_H */
