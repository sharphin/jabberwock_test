// runtimeの内部状態を境界へ置くテスト専用harness。言語から世代を変更するAPIは公開しない。
#include "jw_rt.c"
#include <assert.h>

const int32_t jw_nfiles = 0;
const WSrc jw_files[1] = {{"retirement.jw", ""}};
static const WSite site = {"retirement.jw", 1, 1, 1, 0, "世代の境界", NULL};

static void test_release(int64_t bytes) {
    uint64_t retired = retired_count();
    void *p = jw_alloc(bytes);
    gen_set(p, GEN_LIMIT & ~(uint64_t)1);
    uint64_t old = gen_get(p);
    jw_free(p, bytes, &site);
    assert(gen_get(p) == GEN_RETIRED && retired_count() == retired + 1);
    assert(freed_at(p, old) == &site);
    for (int i = 0; i < 32; i++) {
        void *q = jw_alloc(bytes);
        assert(q != p && gen_get(q) != GEN_RETIRED);
        jw_free(q, bytes, &site);
        assert(gen_get(p) == GEN_RETIRED);
    }
}

static void test_reuse(int64_t bytes) {
    void *p = block_alloc(bytes);
    gen_set(p, GEN_LIMIT | 1);
    FREED_SITE(p) = &site;
    size_t s = round8(bytes);
    if (s / 8 <= NCLASS) {
        *(void **)p = free_small[s / 8];
        free_small[s / 8] = p;
    } else {
        Big *b = malloc(sizeof(Big));
        assert(b);
        b->next = free_big; b->size = s; b->payload = p; free_big = b;
    }
    void *q = block_alloc(bytes);
    assert(q != p && gen_get(p) == GEN_RETIRED && gen_get(q) != GEN_RETIRED);
    block_free(q, bytes, &site);
}

static void test_advance(void) {
    void *p = block_alloc(16);
    gen_set(p, 0);
    assert(gen_advance(p, 1) && gen_get(p) == 1);
    assert(gen_advance(p, 1) && gen_get(p) == 2);
    if (GEN_LIMIT >= 7) {
        WList list = {p, 1, 1};
        gen_set(p, ((uint64_t)1 << 62) | 2);
        jw_list_moved(&list, &site);
        assert(gen_get(p) == (((uint64_t)1 << 62) | 4));
    }
    gen_set(p, ((uint64_t)1 << 62) | (GEN_LIMIT & ~(uint64_t)1));
    assert(!gen_advance(p, 2) && gen_get(p) == GEN_RETIRED);
    uint64_t retired = retired_count();
    assert(!gen_advance(p, 1) && retired_count() == retired);
    assert(freed_at(p, GEN_RETIRED) == NULL);
}

#ifndef _WIN32
static void *release_in_worker(void *p) {
    jw_free(p, 16, &site);
    for (int i = 0; i < 32; i++) {
        void *q = jw_alloc(16);
        jw_free(q, 16, &site);
    }
    return NULL;
}

static void test_retired_metadata_across_threads(void) {
    void *p = jw_alloc(16);
    gen_set(p, (GEN_LIMIT & ~(uint64_t)1) - 2);
    uint64_t old = gen_get(p);
    jw_send_block(p, &site);
    pthread_t worker;
    assert(pthread_create(&worker, NULL, release_in_worker, p) == 0);
    for (int i = 0; i < 10000; i++) {
        assert(gen_get(p) != old);
        (void)freed_at(p, old);
    }
    assert(pthread_join(worker, NULL) == 0);
    assert(gen_get(p) == GEN_RETIRED);
}
#endif

void jw_main(void) {
    const char *mode = g_argc > 1 ? g_argv[1] : "checks";
    if (!strcmp(mode, "checks")) {
        test_release(16);
        test_release((NCLASS + 1) * 8);
        test_reuse(16);
        test_reuse((NCLASS + 1) * 8);
        test_advance();
#ifndef _WIN32
        test_retired_metadata_across_threads();
#endif
        // 退役識別子を保持表で別計上し、payloadを再利用しない。
        uint64_t count = 0;
        for (void *p = __atomic_load_n(&retired_head, __ATOMIC_ACQUIRE); p; p = *(void **)p) {
            assert(gen_get(p) == GEN_RETIRED);
            assert(++count <= retired_count());
        }
        assert(count == retired_count());
        puts("retirement checks passed");
    } else if (!strcmp(mode, "send")) {
        void *p = jw_alloc(16);
        gen_set(p, GEN_LIMIT & ~(uint64_t)1);
        jw_send_block(p, &site);
        abort();
    } else if (!strcmp(mode, "list")) {
        WList l = {0};
        jw_list_reserve(&l, 8, 4);
        gen_set(l.data, ((uint64_t)1 << 62) | (GEN_LIMIT & ~(uint64_t)3) | 2);
        jw_list_moved(&l, &site);
        abort();
    } else if (!strcmp(mode, "view")) {
        void *p = jw_alloc(16);
        gen_retire(p);
        // 壊れたrefがmarkerと同じ世代を持っても、Retiredを有効とみなさない。
        void *v = jw_view(p, &site);
        jw_view_target(v, &site);
        abort();
    } else abort();
}
