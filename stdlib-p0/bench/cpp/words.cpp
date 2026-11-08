#include <cstdio>
#include <cstdint>
#include <string>
#include <unordered_map>
int main() {
    std::unordered_map<std::string, int64_t> counts;
    int64_t x = 1;
    for (int i = 0; i < 5000000; i++) {
        x = (x * 1103515245 + 12345) % 2147483648;
        std::string w = "w" + std::to_string(x % 5000);
        counts[w]++;
    }
    int64_t best = 0;
    for (auto& kv : counts) if (kv.second > best) best = kv.second;
    printf("%zu %lld\n", counts.size(), (long long)best);
}
