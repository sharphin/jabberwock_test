/* Linked only into isolated safety probes, never the production runtime. */
#include <stdlib.h>
#include <stdio.h>
#include <stdatomic.h>

static _Atomic unsigned long long calls;
static unsigned long long fail_at;
static int initialized;

static void report_calls(void) {
    fprintf(stderr, "[allocation-count] %llu\n", (unsigned long long)atomic_load(&calls));
}

static int fail_this_call(void) {
    /* The allocation probes use a single thread. */
    if (!initialized) {
        const char *value = getenv("JW_TEST_FAIL_ALLOCATION");
        fail_at = value ? strtoull(value, NULL, 10) : 0;
        initialized = 1;
        atexit(report_calls);
    }
    unsigned long long n = atomic_fetch_add(&calls, 1) + 1;
    if (fail_at && n == fail_at) {
        fprintf(stderr, "[allocation-failure] %llu\n", n);
        return 1;
    }
    return 0;
}

void *jw_test_malloc(size_t n) { return fail_this_call() ? NULL : malloc(n); }
void *jw_test_calloc(size_t n, size_t size) { return fail_this_call() ? NULL : calloc(n, size); }
void *jw_test_realloc(void *p, size_t n) { return fail_this_call() ? NULL : realloc(p, n); }
