#include <algorithm>
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <thread>
#include <vector>
static int64_t count(int64_t w, int64_t t, int64_t n) {
    int64_t c = 0;
    for (int64_t blk = w; blk * 1000 < n; blk += t)
        for (int64_t x = std::max<int64_t>(blk * 1000, 2); x < std::min(blk * 1000 + 1000, n); x++) {
            bool p = true;
            for (int64_t d = 2; d * d <= x; d++) if (x % d == 0) { p = false; break; }
            if (p) c++;
        }
    return c;
}
int main(int argc, char** argv) {
    int64_t t = argc > 1 ? atoll(argv[1]) : 4, n = 3000000;
    std::vector<int64_t> res(t);
    std::vector<std::thread> ts;
    for (int64_t w = 0; w < t; w++) ts.emplace_back([&, w] { res[w] = count(w, t, n); });
    int64_t total = 0;
    for (int64_t w = 0; w < t; w++) { ts[w].join(); total += res[w]; }
    printf("%lld\n", (long long)total);
}
