#include <cstdio>
#include <cstdint>
#include <vector>
#include <algorithm>
static int64_t sorted_sum(std::vector<int64_t> v) {
    std::sort(v.begin(), v.end());
    int64_t s = 0;
    for (size_t i = 0; i < v.size(); i += 997) s += v[i] * (int64_t)(i % 7);
    return s;
}
int main() {
    int64_t n = 3000000;
    std::vector<int64_t> a, b, c, d;
    for (int64_t i = 0; i < n; i++) { a.push_back(i); b.push_back(n - i); c.push_back(42); d.push_back(i % 1000); }
    printf("%lld %lld %lld %lld\n", (long long)sorted_sum(std::move(a)), (long long)sorted_sum(std::move(b)),
           (long long)sorted_sum(std::move(c)), (long long)sorted_sum(std::move(d)));
}
