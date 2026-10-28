// words の速くした書き方: to_chars で文字列の入れ物を使い回し、表の大きさを先に決める
#include <charconv>
#include <cstdio>
#include <cstdint>
#include <string>
#include <unordered_map>
int main() {
    std::unordered_map<std::string, int64_t> counts;
    counts.reserve(8192);
    int64_t x = 1;
    std::string w;
    char buf[32];
    buf[0] = 'w';
    for (int i = 0; i < 5000000; i++) {
        x = (x * 1103515245 + 12345) % 2147483648;
        char* e = std::to_chars(buf + 1, buf + sizeof buf, x % 5000).ptr;
        w.assign(buf, e - buf);
        counts[w]++;
    }
    int64_t best = 0;
    for (auto& kv : counts) if (kv.second > best) best = kv.second;
    printf("%zu %lld\n", counts.size(), (long long)best);
}
