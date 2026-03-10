/*
 * VibeLang Concurrency Runtime
 *
 * Work-stealing thread pool with support for:
 *   - par()  : parallel evaluation of independent expressions
 *   - pmap() : parallel map over linked lists
 *   - race() : first-to-complete among N computations
 *
 * Design:
 *   - Fixed-size pool of worker threads (default: num CPU cores)
 *   - Lock-free task queue per worker (Chase-Lev deque simplified to mutex-based)
 *   - Work stealing: idle workers steal from other workers' queues
 *   - Batch submission for par/pmap: all tasks submitted, then waited on
 */

#include "vibe_runtime.h"

#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>
#include <stdatomic.h>

/* ============================================================
 * Configuration
 * ============================================================ */

#define VIBE_MAX_WORKERS     64
#define VIBE_QUEUE_CAPACITY  4096

/* ============================================================
 * Task Queue (per-worker, mutex-protected ring buffer)
 * ============================================================ */

typedef struct {
    vibe_thunk_fn fn;
    int64_t      *result_slot;   /* where to store the result */
    atomic_int   *completion;    /* atomically incremented on completion */
} vibe_task_t;

typedef struct {
    vibe_task_t  tasks[VIBE_QUEUE_CAPACITY];
    int          head;  /* pop from head (owner) */
    int          tail;  /* push to tail (owner), steal from head */
    pthread_mutex_t lock;
} vibe_queue_t;

static void queue_init(vibe_queue_t *q) {
    q->head = 0;
    q->tail = 0;
    pthread_mutex_init(&q->lock, NULL);
}

static void queue_destroy(vibe_queue_t *q) {
    pthread_mutex_destroy(&q->lock);
}

static int queue_push(vibe_queue_t *q, vibe_task_t task) {
    pthread_mutex_lock(&q->lock);
    int next_tail = (q->tail + 1) % VIBE_QUEUE_CAPACITY;
    if (next_tail == q->head) {
        pthread_mutex_unlock(&q->lock);
        return -1;  /* full */
    }
    q->tasks[q->tail] = task;
    q->tail = next_tail;
    pthread_mutex_unlock(&q->lock);
    return 0;
}

/* Pop from own queue (LIFO for locality). Returns 1 on success, 0 if empty. */
static int queue_pop(vibe_queue_t *q, vibe_task_t *out) {
    pthread_mutex_lock(&q->lock);
    if (q->head == q->tail) {
        pthread_mutex_unlock(&q->lock);
        return 0;
    }
    /* Pop from tail (LIFO) */
    q->tail = (q->tail - 1 + VIBE_QUEUE_CAPACITY) % VIBE_QUEUE_CAPACITY;
    *out = q->tasks[q->tail];
    pthread_mutex_unlock(&q->lock);
    return 1;
}

/* Steal from another worker's queue (FIFO for fairness). */
static int queue_steal(vibe_queue_t *q, vibe_task_t *out) {
    pthread_mutex_lock(&q->lock);
    if (q->head == q->tail) {
        pthread_mutex_unlock(&q->lock);
        return 0;
    }
    *out = q->tasks[q->head];
    q->head = (q->head + 1) % VIBE_QUEUE_CAPACITY;
    pthread_mutex_unlock(&q->lock);
    return 1;
}

/* ============================================================
 * Thread Pool
 * ============================================================ */

typedef struct {
    pthread_t     thread;
    int           id;
    vibe_queue_t  queue;
    int           active;  /* set to 0 to signal shutdown */
} vibe_worker_t;

static struct {
    vibe_worker_t workers[VIBE_MAX_WORKERS];
    int           num_workers;
    int           initialized;
    atomic_int    global_has_work;   /* signal that work is available */
    pthread_mutex_t submit_lock;     /* serialize task submission batches */
    pthread_cond_t  work_available;  /* wake workers */
    pthread_mutex_t work_mutex;
} g_pool;

/* Forward declaration */
static void execute_task(vibe_task_t *task);
static int try_steal(int worker_id);

static void *worker_loop(void *arg) {
    vibe_worker_t *self = (vibe_worker_t *)arg;
    vibe_task_t task;

    while (self->active) {
        /* Try own queue first */
        if (queue_pop(&self->queue, &task)) {
            execute_task(&task);
            continue;
        }

        /* Try stealing from other workers */
        if (try_steal(self->id)) {
            continue;
        }

        /* No work available — wait briefly then retry */
        pthread_mutex_lock(&g_pool.work_mutex);
        if (self->active && atomic_load(&g_pool.global_has_work) == 0) {
            struct timespec ts;
            clock_gettime(CLOCK_REALTIME, &ts);
            ts.tv_nsec += 1000000;  /* 1ms timeout */
            if (ts.tv_nsec >= 1000000000) {
                ts.tv_sec += 1;
                ts.tv_nsec -= 1000000000;
            }
            pthread_cond_timedwait(&g_pool.work_available, &g_pool.work_mutex, &ts);
        }
        pthread_mutex_unlock(&g_pool.work_mutex);
    }

    return NULL;
}

static void execute_task(vibe_task_t *task) {
    int64_t result = task->fn();
    if (task->result_slot) {
        *task->result_slot = result;
    }
    if (task->completion) {
        atomic_fetch_add(task->completion, 1);
    }
}

static int try_steal(int worker_id) {
    vibe_task_t task;
    int n = g_pool.num_workers;
    /* Start stealing from a random offset to avoid contention */
    int start = (worker_id + 1) % n;
    for (int i = 0; i < n - 1; i++) {
        int victim = (start + i) % n;
        if (queue_steal(&g_pool.workers[victim].queue, &task)) {
            execute_task(&task);
            return 1;
        }
    }
    return 0;
}

static void wake_workers(void) {
    atomic_store(&g_pool.global_has_work, 1);
    pthread_cond_broadcast(&g_pool.work_available);
}

/* ============================================================
 * Public API: Init / Shutdown
 * ============================================================ */

void vibe_runtime_init(int num_threads) {
    if (g_pool.initialized) return;

    if (num_threads <= 0) {
        num_threads = (int)sysconf(_SC_NPROCESSORS_ONLN);
        if (num_threads <= 0) num_threads = 4;
    }
    if (num_threads > VIBE_MAX_WORKERS)
        num_threads = VIBE_MAX_WORKERS;

    g_pool.num_workers = num_threads;
    atomic_store(&g_pool.global_has_work, 0);
    pthread_mutex_init(&g_pool.submit_lock, NULL);
    pthread_mutex_init(&g_pool.work_mutex, NULL);
    pthread_cond_init(&g_pool.work_available, NULL);

    for (int i = 0; i < num_threads; i++) {
        g_pool.workers[i].id = i;
        g_pool.workers[i].active = 1;
        queue_init(&g_pool.workers[i].queue);
        pthread_create(&g_pool.workers[i].thread, NULL, worker_loop, &g_pool.workers[i]);
    }

    g_pool.initialized = 1;
}

void vibe_runtime_shutdown(void) {
    if (!g_pool.initialized) return;

    /* Signal all workers to stop */
    for (int i = 0; i < g_pool.num_workers; i++) {
        g_pool.workers[i].active = 0;
    }
    /* Wake them so they see the shutdown flag */
    pthread_cond_broadcast(&g_pool.work_available);

    for (int i = 0; i < g_pool.num_workers; i++) {
        pthread_join(g_pool.workers[i].thread, NULL);
        queue_destroy(&g_pool.workers[i].queue);
    }

    pthread_mutex_destroy(&g_pool.submit_lock);
    pthread_mutex_destroy(&g_pool.work_mutex);
    pthread_cond_destroy(&g_pool.work_available);
    g_pool.initialized = 0;
}

/* Ensure pool is initialized (lazy init for JIT mode). */
static void ensure_init(void) {
    if (!g_pool.initialized) {
        vibe_runtime_init(0);
    }
}

/* ============================================================
 * Par Combinator
 * ============================================================ */

void vibe_par_execute(vibe_thunk_fn *thunks, int64_t *out_results, int n) {
    if (n <= 0) return;

    ensure_init();

    /* For single task, just run it directly */
    if (n == 1) {
        out_results[0] = thunks[0]();
        return;
    }

    atomic_int completion = 0;

    pthread_mutex_lock(&g_pool.submit_lock);

    /* Submit n-1 tasks to the pool (run last one on caller thread) */
    int submitted = 0;
    for (int i = 0; i < n - 1; i++) {
        vibe_task_t task = {
            .fn = thunks[i],
            .result_slot = &out_results[i],
            .completion = &completion,
        };
        /* Round-robin distribute to worker queues */
        int worker = i % g_pool.num_workers;
        if (queue_push(&g_pool.workers[worker].queue, task) == 0) {
            submitted++;
        } else {
            /* Queue full — execute inline */
            out_results[i] = thunks[i]();
        }
    }

    pthread_mutex_unlock(&g_pool.submit_lock);

    /* Wake workers */
    if (submitted > 0) {
        wake_workers();
    }

    /* Run last task on caller thread */
    out_results[n - 1] = thunks[n - 1]();

    /* Wait for all submitted tasks */
    while (atomic_load(&completion) < submitted) {
        /* Help by stealing work while waiting */
        vibe_task_t stolen;
        int found = 0;
        for (int w = 0; w < g_pool.num_workers && !found; w++) {
            if (queue_steal(&g_pool.workers[w].queue, &stolen)) {
                execute_task(&stolen);
                found = 1;
            }
        }
        if (!found) {
            sched_yield();
        }
    }
}

/* ============================================================
 * Pmap Combinator
 * ============================================================ */

/* Internal: worker context for parallel map chunks */
typedef struct {
    int64_t *src;       /* source array */
    int64_t *dst;       /* destination array */
    int      start;
    int      count;
    vibe_map_fn fn;
} pmap_chunk_t;

static void *pmap_chunk_worker(void *arg) {
    pmap_chunk_t *chunk = (pmap_chunk_t *)arg;
    for (int i = 0; i < chunk->count; i++) {
        chunk->dst[chunk->start + i] = chunk->fn(chunk->src[chunk->start + i]);
    }
    return NULL;
}

int64_t vibe_list_count(vibe_cons_t *list) {
    int64_t count = 0;
    while (list) {
        count++;
        list = list->next;
    }
    return count;
}

vibe_cons_t *vibe_pmap_list(vibe_cons_t *list, vibe_map_fn fn, void *region_ptr) {
    if (!list) return NULL;

    ensure_init();

    /* Convert list to array for parallel processing */
    int64_t n = vibe_list_count(list);
    if (n == 0) return NULL;

    int64_t *src = (int64_t *)malloc(n * sizeof(int64_t));
    int64_t *dst = (int64_t *)malloc(n * sizeof(int64_t));

    /* Flatten list to array */
    vibe_cons_t *cur = list;
    for (int64_t i = 0; i < n; i++) {
        src[i] = cur->value;
        cur = cur->next;
    }

    /* Determine chunking */
    int num_workers = g_pool.num_workers;
    if (num_workers <= 0) num_workers = 1;

    /* For small lists, just run sequentially */
    if (n <= 64 || num_workers == 1) {
        for (int64_t i = 0; i < n; i++) {
            dst[i] = fn(src[i]);
        }
    } else {
        /* Split work into chunks */
        int num_chunks = num_workers;
        if (num_chunks > (int)n) num_chunks = (int)n;
        int chunk_size = (int)(n / num_chunks);
        int remainder = (int)(n % num_chunks);

        pmap_chunk_t *chunks = (pmap_chunk_t *)malloc(num_chunks * sizeof(pmap_chunk_t));

        int offset = 0;
        for (int c = 0; c < num_chunks; c++) {
            int this_size = chunk_size + (c < remainder ? 1 : 0);
            chunks[c].src = src;
            chunks[c].dst = dst;
            chunks[c].start = offset;
            chunks[c].count = this_size;
            chunks[c].fn = fn;
            offset += this_size;
        }

        /* Use pthreads directly for chunk parallelism */
        if (num_chunks > 1) {
            pthread_t *threads = (pthread_t *)malloc((num_chunks - 1) * sizeof(pthread_t));

            for (int c = 0; c < num_chunks - 1; c++) {
                pthread_create(&threads[c], NULL,
                    (void *(*)(void *))pmap_chunk_worker, &chunks[c]);
            }

            /* Run last chunk on caller thread */
            pmap_chunk_worker(&chunks[num_chunks - 1]);

            /* Join worker threads */
            for (int c = 0; c < num_chunks - 1; c++) {
                pthread_join(threads[c], NULL);
            }

            free(threads);
        } else {
            pmap_chunk_worker(&chunks[0]);
        }

        free(chunks);
    }

    /* Build result list from array (reverse order for correct cons ordering) */
    vibe_cons_t *result = NULL;
    for (int64_t i = n - 1; i >= 0; i--) {
        vibe_cons_t *cell;
        if (region_ptr) {
            /* TODO: use region allocator */
            cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
        } else {
            cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
        }
        cell->value = dst[i];
        cell->next = result;
        result = cell;
    }

    free(src);
    free(dst);
    return result;
}

/* ============================================================
 * Pfilter Combinator
 * ============================================================ */

typedef struct {
    int64_t    *src;
    int64_t    *flags;
    int         start;
    int         count;
    vibe_pred_fn pred;
} pfilter_chunk_t;

static void *pfilter_chunk_worker(void *arg) {
    pfilter_chunk_t *chunk = (pfilter_chunk_t *)arg;
    for (int i = 0; i < chunk->count; i++) {
        int idx = chunk->start + i;
        chunk->flags[idx] = chunk->pred(chunk->src[idx]) ? 1 : 0;
    }
    return NULL;
}

vibe_cons_t *vibe_pfilter_list(vibe_cons_t *list, vibe_pred_fn pred, void *region_ptr) {
    if (!list) return NULL;

    ensure_init();

    int64_t n = vibe_list_count(list);
    if (n == 0) return NULL;

    int64_t *src = (int64_t *)malloc(n * sizeof(int64_t));
    int64_t *flags = (int64_t *)calloc(n, sizeof(int64_t));

    vibe_cons_t *cur = list;
    for (int64_t i = 0; i < n; i++) {
        src[i] = cur->value;
        cur = cur->next;
    }

    int num_workers = g_pool.num_workers;
    if (num_workers <= 0) num_workers = 1;

    if (n <= 64 || num_workers == 1) {
        for (int64_t i = 0; i < n; i++) {
            flags[i] = pred(src[i]) ? 1 : 0;
        }
    } else {
        int num_chunks = num_workers;
        if (num_chunks > (int)n) num_chunks = (int)n;
        int chunk_size = (int)(n / num_chunks);
        int remainder = (int)(n % num_chunks);

        pfilter_chunk_t *chunks = (pfilter_chunk_t *)malloc(num_chunks * sizeof(pfilter_chunk_t));
        pthread_t *threads = (pthread_t *)malloc((num_chunks - 1) * sizeof(pthread_t));

        int offset = 0;
        for (int c = 0; c < num_chunks; c++) {
            int this_size = chunk_size + (c < remainder ? 1 : 0);
            chunks[c].src = src;
            chunks[c].flags = flags;
            chunks[c].start = offset;
            chunks[c].count = this_size;
            chunks[c].pred = pred;
            offset += this_size;
        }

        for (int c = 0; c < num_chunks - 1; c++) {
            pthread_create(&threads[c], NULL, pfilter_chunk_worker, &chunks[c]);
        }
        pfilter_chunk_worker(&chunks[num_chunks - 1]);

        for (int c = 0; c < num_chunks - 1; c++) {
            pthread_join(threads[c], NULL);
        }

        free(threads);
        free(chunks);
    }

    vibe_cons_t *result = NULL;
    for (int64_t i = n - 1; i >= 0; i--) {
        if (flags[i]) {
            vibe_cons_t *cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
            cell->value = src[i];
            cell->next = result;
            result = cell;
        }
    }

    free(src);
    free(flags);
    return result;
}

/* ============================================================
 * Preduce Combinator — Parallel Tree Reduction
 * ============================================================ */

typedef struct {
    int64_t       *data;
    int            start;
    int            count;
    vibe_reduce_fn fn;
    int64_t        result;
} preduce_chunk_t;

static void *preduce_chunk_worker(void *arg) {
    preduce_chunk_t *chunk = (preduce_chunk_t *)arg;
    int64_t acc = chunk->data[chunk->start];
    for (int i = 1; i < chunk->count; i++) {
        acc = chunk->fn(acc, chunk->data[chunk->start + i]);
    }
    chunk->result = acc;
    return NULL;
}

int64_t vibe_preduce_list(vibe_cons_t *list, int64_t init, vibe_reduce_fn fn) {
    if (!list) return init;

    ensure_init();

    int64_t n = vibe_list_count(list);
    if (n == 0) return init;

    int64_t *data = (int64_t *)malloc(n * sizeof(int64_t));
    vibe_cons_t *cur = list;
    for (int64_t i = 0; i < n; i++) {
        data[i] = cur->value;
        cur = cur->next;
    }

    int num_workers = g_pool.num_workers;
    if (num_workers <= 0) num_workers = 1;

    if (n <= 64 || num_workers == 1) {
        int64_t acc = init;
        for (int64_t i = 0; i < n; i++) {
            acc = fn(acc, data[i]);
        }
        free(data);
        return acc;
    }

    int num_chunks = num_workers;
    if (num_chunks > (int)n) num_chunks = (int)n;
    int chunk_size = (int)(n / num_chunks);
    int remainder = (int)(n % num_chunks);

    preduce_chunk_t *chunks = (preduce_chunk_t *)malloc(num_chunks * sizeof(preduce_chunk_t));
    pthread_t *threads = (pthread_t *)malloc((num_chunks - 1) * sizeof(pthread_t));

    int offset = 0;
    for (int c = 0; c < num_chunks; c++) {
        int this_size = chunk_size + (c < remainder ? 1 : 0);
        chunks[c].data = data;
        chunks[c].start = offset;
        chunks[c].count = this_size;
        chunks[c].fn = fn;
        chunks[c].result = 0;
        offset += this_size;
    }

    for (int c = 0; c < num_chunks - 1; c++) {
        pthread_create(&threads[c], NULL, preduce_chunk_worker, &chunks[c]);
    }
    preduce_chunk_worker(&chunks[num_chunks - 1]);

    for (int c = 0; c < num_chunks - 1; c++) {
        pthread_join(threads[c], NULL);
    }

    int64_t final_result = init;
    for (int c = 0; c < num_chunks; c++) {
        final_result = fn(final_result, chunks[c].result);
    }

    free(threads);
    free(chunks);
    free(data);
    return final_result;
}

/* ============================================================
 * Channels — Bounded MPMC Queue
 * ============================================================ */

struct vibe_channel {
    int64_t         *buffer;
    int              capacity;
    int              head;
    int              tail;
    int              count;
    int              closed;
    pthread_mutex_t  lock;
    pthread_cond_t   not_full;
    pthread_cond_t   not_empty;
};

vibe_channel_t *vibe_channel_create(int64_t capacity) {
    if (capacity <= 0) capacity = 1;
    vibe_channel_t *ch = (vibe_channel_t *)malloc(sizeof(vibe_channel_t));
    ch->buffer = (int64_t *)malloc(capacity * sizeof(int64_t));
    ch->capacity = (int)capacity;
    ch->head = 0;
    ch->tail = 0;
    ch->count = 0;
    ch->closed = 0;
    pthread_mutex_init(&ch->lock, NULL);
    pthread_cond_init(&ch->not_full, NULL);
    pthread_cond_init(&ch->not_empty, NULL);
    return ch;
}

void vibe_channel_send(vibe_channel_t *ch, int64_t value) {
    pthread_mutex_lock(&ch->lock);
    while (ch->count == ch->capacity && !ch->closed) {
        pthread_cond_wait(&ch->not_full, &ch->lock);
    }
    if (ch->closed) {
        pthread_mutex_unlock(&ch->lock);
        return;
    }
    ch->buffer[ch->tail] = value;
    ch->tail = (ch->tail + 1) % ch->capacity;
    ch->count++;
    pthread_cond_signal(&ch->not_empty);
    pthread_mutex_unlock(&ch->lock);
}

int64_t vibe_channel_recv(vibe_channel_t *ch) {
    pthread_mutex_lock(&ch->lock);
    while (ch->count == 0 && !ch->closed) {
        pthread_cond_wait(&ch->not_empty, &ch->lock);
    }
    if (ch->count == 0 && ch->closed) {
        pthread_mutex_unlock(&ch->lock);
        return 0;
    }
    int64_t value = ch->buffer[ch->head];
    ch->head = (ch->head + 1) % ch->capacity;
    ch->count--;
    pthread_cond_signal(&ch->not_full);
    pthread_mutex_unlock(&ch->lock);
    return value;
}

void vibe_channel_close(vibe_channel_t *ch) {
    pthread_mutex_lock(&ch->lock);
    ch->closed = 1;
    pthread_cond_broadcast(&ch->not_full);
    pthread_cond_broadcast(&ch->not_empty);
    pthread_mutex_unlock(&ch->lock);
}

void vibe_channel_destroy(vibe_channel_t *ch) {
    if (!ch) return;
    pthread_mutex_destroy(&ch->lock);
    pthread_cond_destroy(&ch->not_full);
    pthread_cond_destroy(&ch->not_empty);
    free(ch->buffer);
    free(ch);
}

/* ============================================================
 * Race Combinator
 * ============================================================ */

typedef struct {
    vibe_thunk_fn fn;
    int64_t       result;
    atomic_int   *winner;      /* set to 1 by first finisher */
    int           id;
} race_task_t;

static void *race_worker(void *arg) {
    race_task_t *task = (race_task_t *)arg;

    /* Check if someone already won before even starting */
    if (atomic_load(task->winner) != -1) {
        return NULL;
    }

    task->result = task->fn();

    /* Try to claim victory */
    int expected = -1;
    atomic_compare_exchange_strong(task->winner, &expected, task->id);

    return NULL;
}

int64_t vibe_race_execute(vibe_thunk_fn *thunks, int n) {
    if (n <= 0) return 0;
    if (n == 1) return thunks[0]();

    ensure_init();

    atomic_int winner = -1;
    race_task_t *tasks = (race_task_t *)malloc(n * sizeof(race_task_t));
    pthread_t *threads = (pthread_t *)malloc(n * sizeof(pthread_t));

    for (int i = 0; i < n; i++) {
        tasks[i].fn = thunks[i];
        tasks[i].result = 0;
        tasks[i].winner = &winner;
        tasks[i].id = i;
        pthread_create(&threads[i], NULL, race_worker, &tasks[i]);
    }

    /* Wait for all threads (they self-terminate quickly after winner is set) */
    for (int i = 0; i < n; i++) {
        pthread_join(threads[i], NULL);
    }

    int winner_id = atomic_load(&winner);
    int64_t result = (winner_id >= 0 && winner_id < n) ? tasks[winner_id].result : 0;

    free(tasks);
    free(threads);
    return result;
}

/* ============================================================
 * Pipeline Utility Functions
 * ============================================================ */

typedef int64_t (*vibe_key_fn)(int64_t);

/* distinct_by(list, key_fn, region): keep first element per key_fn result.
 * Uses a simple O(n^2) scan (good enough for moderate lists). */
vibe_cons_t *vibe_list_distinct_by(vibe_cons_t *list, vibe_key_fn key_fn, void *region) {
    if (!list) return NULL;
    (void)region;

    /* Collect all elements and keys */
    int64_t n = vibe_list_count(list);
    int64_t *values = (int64_t *)malloc(n * sizeof(int64_t));
    int64_t *keys = (int64_t *)malloc(n * sizeof(int64_t));
    vibe_cons_t *cur = list;
    for (int64_t i = 0; i < n; i++) {
        values[i] = cur->value;
        keys[i] = key_fn(cur->value);
        cur = cur->next;
    }

    /* Mark duplicates */
    int64_t *keep = (int64_t *)calloc(n, sizeof(int64_t));
    for (int64_t i = 0; i < n; i++) {
        int dup = 0;
        for (int64_t j = 0; j < i; j++) {
            if (keys[j] == keys[i]) { dup = 1; break; }
        }
        keep[i] = !dup;
    }

    /* Build result list */
    vibe_cons_t *result = NULL;
    for (int64_t i = n - 1; i >= 0; i--) {
        if (keep[i]) {
            vibe_cons_t *cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
            cell->value = values[i];
            cell->next = result;
            result = cell;
        }
    }

    free(values);
    free(keys);
    free(keep);
    return result;
}

/* window(list, size, stride, region): sliding window over list */
vibe_cons_t *vibe_list_window(vibe_cons_t *list, int64_t size, int64_t stride, void *region) {
    if (!list || size <= 0 || stride <= 0) return NULL;
    (void)region;

    int64_t n = vibe_list_count(list);
    int64_t *arr = (int64_t *)malloc(n * sizeof(int64_t));
    vibe_cons_t *cur = list;
    for (int64_t i = 0; i < n; i++) {
        arr[i] = cur->value;
        cur = cur->next;
    }

    /* Build windows as sub-lists */
    vibe_cons_t *result = NULL;
    for (int64_t start = n - 1; start >= 0; start -= stride) {
        int64_t wstart = start - size + 1;
        if (wstart < 0) wstart = 0;
        if (start - wstart + 1 < size && start != n - 1) continue;

        /* Build sub-list for this window */
        vibe_cons_t *window = NULL;
        for (int64_t j = start; j >= wstart; j--) {
            vibe_cons_t *cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
            cell->value = arr[j];
            cell->next = window;
            window = cell;
        }

        /* Wrap the window list as a value in the outer list */
        vibe_cons_t *outer = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
        outer->value = (int64_t)(intptr_t)window;
        outer->next = result;
        result = outer;
    }

    free(arr);
    return result;
}

/* zip(list1, list2, region): pair elements from two lists */
vibe_cons_t *vibe_list_zip(vibe_cons_t *a, vibe_cons_t *b, void *region) {
    (void)region;
    if (!a || !b) return NULL;

    vibe_cons_t *result = NULL;
    vibe_cons_t **tail = &result;

    while (a && b) {
        /* Create a pair as a 2-element sub-list: (a.value, b.value) */
        vibe_cons_t *pair_b = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
        pair_b->value = b->value;
        pair_b->next = NULL;

        vibe_cons_t *pair_a = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
        pair_a->value = a->value;
        pair_a->next = pair_b;

        vibe_cons_t *cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
        cell->value = (int64_t)(intptr_t)pair_a;
        cell->next = NULL;
        *tail = cell;
        tail = &cell->next;

        a = a->next;
        b = b->next;
    }

    return result;
}

/* min_by(list, key_fn): find element with minimum key */
int64_t vibe_list_min_by(vibe_cons_t *list, vibe_key_fn key_fn) {
    if (!list) return 0;
    int64_t best_val = list->value;
    int64_t best_key = key_fn(list->value);
    list = list->next;
    while (list) {
        int64_t k = key_fn(list->value);
        if (k < best_key) {
            best_key = k;
            best_val = list->value;
        }
        list = list->next;
    }
    return best_val;
}

/* max_by(list, key_fn): find element with maximum key */
int64_t vibe_list_max_by(vibe_cons_t *list, vibe_key_fn key_fn) {
    if (!list) return 0;
    int64_t best_val = list->value;
    int64_t best_key = key_fn(list->value);
    list = list->next;
    while (list) {
        int64_t k = key_fn(list->value);
        if (k > best_key) {
            best_key = k;
            best_val = list->value;
        }
        list = list->next;
    }
    return best_val;
}

/* merge(list1, list2, region): interleave elements from two lists */
vibe_cons_t *vibe_list_merge(vibe_cons_t *a, vibe_cons_t *b, void *region) {
    (void)region;
    vibe_cons_t *result = NULL;
    vibe_cons_t **tail = &result;

    while (a || b) {
        if (a) {
            vibe_cons_t *cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
            cell->value = a->value;
            cell->next = NULL;
            *tail = cell;
            tail = &cell->next;
            a = a->next;
        }
        if (b) {
            vibe_cons_t *cell = (vibe_cons_t *)malloc(sizeof(vibe_cons_t));
            cell->value = b->value;
            cell->next = NULL;
            *tail = cell;
            tail = &cell->next;
            b = b->next;
        }
    }

    return result;
}

/* ============================================================
 * String Utility Functions
 * ============================================================ */

#include <ctype.h>

/* trim_start: remove leading whitespace */
char *vibe_trim_start(const char *s) {
    if (!s) return NULL;
    while (*s && isspace((unsigned char)*s)) s++;
    size_t len = strlen(s);
    char *result = (char *)malloc(len + 1);
    memcpy(result, s, len + 1);
    return result;
}

/* trim_end: remove trailing whitespace */
char *vibe_trim_end(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    while (len > 0 && isspace((unsigned char)s[len - 1])) len--;
    char *result = (char *)malloc(len + 1);
    memcpy(result, s, len);
    result[len] = '\0';
    return result;
}

/* from_chars: construct string from a linked list of characters (vibe_cons_t of char values) */
char *vibe_from_chars(vibe_cons_t *chars) {
    /* Count characters */
    int64_t n = vibe_list_count(chars);
    char *result = (char *)malloc(n + 1);
    vibe_cons_t *cur = chars;
    for (int64_t i = 0; i < n; i++) {
        result[i] = (char)cur->value;
        cur = cur->next;
    }
    result[n] = '\0';
    return result;
}
