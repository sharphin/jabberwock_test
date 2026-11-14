#include <cmath>
#include <cstdio>
#include <vector>
static double a(long i, long j) { return 1.0 / (double)((i + j) * (i + j + 1) / 2 + i + 1); }
static void times(const std::vector<double>& v, std::vector<double>& out) {
    for (size_t i = 0; i < v.size(); i++) {
        double s = 0;
        for (size_t j = 0; j < v.size(); j++) s += a(i, j) * v[j];
        out[i] = s;
    }
}
static void times_t(const std::vector<double>& v, std::vector<double>& out) {
    for (size_t i = 0; i < v.size(); i++) {
        double s = 0;
        for (size_t j = 0; j < v.size(); j++) s += a(j, i) * v[j];
        out[i] = s;
    }
}
static void times_ata(const std::vector<double>& v, std::vector<double>& tmp, std::vector<double>& out) {
    times(v, tmp);
    times_t(tmp, out);
}
int main() {
    int n = 2000;
    std::vector<double> u(n, 1.0), v(n, 0.0), tmp(n, 0.0);
    for (int k = 0; k < 10; k++) { times_ata(u, tmp, v); times_ata(v, tmp, u); }
    double vbv = 0, vv = 0;
    for (int i = 0; i < n; i++) { vbv += u[i] * v[i]; vv += v[i] * v[i]; }
    printf("%lld\n", (long long)(std::sqrt(vbv / vv) * 1e9));
}
