#include <cstdio>
#include <cstdint>
#include <memory>
#include <vector>
struct P { int64_t x, y; };
int main() {
    std::vector<std::unique_ptr<P>> ring;
    for (int i = 0; i < 16; i++) ring.push_back(std::make_unique<P>(P{0, 0}));
    int64_t s = 0;
    for (int64_t i = 0; i < 100000000; i++) {
        ring[i % 16] = std::make_unique<P>(P{i, i * 2});
        s += ring[(i + 5) % 16]->y - ring[(i + 3) % 16]->x;
    }
    printf("%lld\n", (long long)s);
}
