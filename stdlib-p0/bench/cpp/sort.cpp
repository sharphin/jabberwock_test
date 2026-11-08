#include <cstdio>
#include <cstdint>
#include <vector>
#include <algorithm>
int main() {
    std::vector<int64_t> v;
    int64_t x = 7;
    for (int i = 0; i < 3000000; i++) { x = (x * 1103515245 + 12345) % 2147483648; v.push_back(x); }
    std::sort(v.begin(), v.end());
    int64_t sum = 0;
    for (size_t i = 0; i < v.size(); i += 1000) sum += v[i];
    printf("%lld %lld %lld\n", (long long)v[0], (long long)v.back(), (long long)sum);
}
