#include <cstdio>
#include <vector>
int main() {
    int n = 600;
    std::vector<double> a, b, c;
    for (int i = 0; i < n * n; i++) { a.push_back((i % 7) * 0.5); b.push_back((i % 5) * 0.25); c.push_back(0); }
    for (int i = 0; i < n; i++)
        for (int k = 0; k < n; k++) {
            double aik = a[i * n + k];
            for (int j = 0; j < n; j++) c[i * n + j] += aik * b[k * n + j];
        }
    double s = 0;
    for (int i = 0; i < n * n; i++) s += c[i];
    printf("%lld\n", (long long)(s * 1000));
}
