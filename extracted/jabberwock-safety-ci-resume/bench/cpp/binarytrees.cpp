#include <cstdio>
#include <memory>
struct Tree { std::unique_ptr<Tree> l, r; };
static std::unique_ptr<Tree> make(int d) {
    auto t = std::make_unique<Tree>();
    if (d > 0) { t->l = make(d - 1); t->r = make(d - 1); }
    return t;
}
static long long check(const Tree& t) { return t.l ? 1 + check(*t.l) + check(*t.r) : 1; }
int main() {
    int n = 18;
    { auto s = make(n + 1); printf("%d %lld\n", n + 1, check(*s)); }
    auto long_lived = make(n);
    for (int d = 4; d <= n; d += 2) {
        long long iters = 1LL << (n - d + 4), chk = 0;
        for (long long i = 0; i < iters; i++) chk += check(*make(d));
        printf("%lld %d %lld\n", iters, d, chk);
    }
    printf("%d %lld\n", n, check(*long_lived));
}
