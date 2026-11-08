#include <cstdio>
#include <cstdint>
int64_t fib(int64_t n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int main() { printf("%lld\n", (long long)fib(40)); }
