#include <cstdio>
#include <vector>
struct P { double x, y, vx, vy; };
static void step(std::vector<P>& ps, double dt) {
    for (size_t i = 0; i < ps.size(); i++) {
        ps[i].vy -= 9.8 * dt;
        ps[i].x += ps[i].vx * dt;
        ps[i].y += ps[i].vy * dt;
        if (ps[i].y < 0) { ps[i].y = -ps[i].y; ps[i].vy = -ps[i].vy * 0.9; }
    }
}
int main() {
    std::vector<P> ps;
    for (int i = 0; i < 10000; i++) ps.push_back(P{(double)i, 10.0 + i % 13, 1, 0});
    for (int t = 0; t < 10000; t++) step(ps, 0.01);
    double s = 0;
    for (auto& p : ps) s += p.y;
    printf("%lld\n", (long long)(s * 1000));
}
