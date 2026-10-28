// sieve の速くした書き方: 最初に大きさを決める（vector<bool> のビット詰めはそのまま。1 バイトずつより速かった）
#include <cstdio>
#include <vector>
int main() {
    long n = 20000000;
    std::vector<bool> comp(n + 1, false);
    long count = 0;
    for (long i = 2; i <= n; i++)
        if (!comp[i]) {
            count++;
            for (long j = i * i; j <= n; j += i) comp[j] = true;
        }
    printf("%ld\n", count);
}
