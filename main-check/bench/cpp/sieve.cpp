#include <cstdio>
#include <vector>
int main() {
    long n = 20000000;
    std::vector<bool> comp;
    for (long i = 0; i <= n; i++) comp.push_back(false);
    long count = 0;
    for (long i = 2; i <= n; i++)
        if (!comp[i]) {
            count++;
            for (long j = i * i; j <= n; j += i) comp[j] = true;
        }
    printf("%ld\n", count);
}
