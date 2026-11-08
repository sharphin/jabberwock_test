#include <cmath>
#include <cstdio>
#include <vector>

static const double PI = 3.141592653589793;
static const double SOLAR_MASS = 4.0 * PI * PI;
static const double DAYS = 365.24;

struct Body { double x, y, z, vx, vy, vz, mass; };

static Body planet(double x, double y, double z, double vx, double vy, double vz, double mass) {
    return Body{x, y, z, vx * DAYS, vy * DAYS, vz * DAYS, mass * SOLAR_MASS};
}
static void advance(std::vector<Body>& bs, double dt) {
    size_t n = bs.size();
    for (size_t i = 0; i < n; i++) {
        for (size_t j = i + 1; j < n; j++) {
            double dx = bs[i].x - bs[j].x, dy = bs[i].y - bs[j].y, dz = bs[i].z - bs[j].z;
            double d2 = dx * dx + dy * dy + dz * dz;
            double mag = dt / (d2 * std::sqrt(d2));
            double mi = bs[i].mass * mag, mj = bs[j].mass * mag;
            bs[i].vx -= dx * mj; bs[i].vy -= dy * mj; bs[i].vz -= dz * mj;
            bs[j].vx += dx * mi; bs[j].vy += dy * mi; bs[j].vz += dz * mi;
        }
    }
    for (size_t i = 0; i < n; i++) {
        bs[i].x += dt * bs[i].vx; bs[i].y += dt * bs[i].vy; bs[i].z += dt * bs[i].vz;
    }
}
static double energy(const std::vector<Body>& bs) {
    double e = 0.0;
    size_t n = bs.size();
    for (size_t i = 0; i < n; i++) {
        const Body& b = bs[i];
        e += 0.5 * b.mass * (b.vx * b.vx + b.vy * b.vy + b.vz * b.vz);
        for (size_t j = i + 1; j < n; j++) {
            double dx = b.x - bs[j].x, dy = b.y - bs[j].y, dz = b.z - bs[j].z;
            e -= b.mass * bs[j].mass / std::sqrt(dx * dx + dy * dy + dz * dz);
        }
    }
    return e;
}
static void offset(std::vector<Body>& bs) {
    double px = 0, py = 0, pz = 0;
    for (const Body& b : bs) { px += b.vx * b.mass; py += b.vy * b.mass; pz += b.vz * b.mass; }
    bs[0].vx = -px / SOLAR_MASS; bs[0].vy = -py / SOLAR_MASS; bs[0].vz = -pz / SOLAR_MASS;
}
int main() {
    std::vector<Body> bs = {
        Body{0, 0, 0, 0, 0, 0, SOLAR_MASS},
        planet(4.84143144246472090e+00, -1.16032004402742839e+00, -1.03622044471123109e-01,
               1.66007664274403694e-03, 7.69901118419740425e-03, -6.90460016972063023e-05, 9.54791938424326609e-04),
        planet(8.34336671824457987e+00, 4.12479856412430479e+00, -4.03523417114321381e-01,
               -2.76742510726862411e-03, 4.99852801234917238e-03, 2.30417297573763929e-05, 2.85885980666130812e-04),
        planet(1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01,
               2.96460137564761618e-03, 2.37847173959480950e-03, -2.96589568540237556e-05, 4.36624404335156298e-05),
        planet(1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01,
               2.68067772490389322e-03, 1.62824170038242295e-03, -9.51592254519715870e-05, 5.15138902046611451e-05),
    };
    offset(bs);
    printf("%lld\n", (long long)(energy(bs) * 1e9));
    for (int k = 0; k < 5000000; k++) advance(bs, 0.01);
    printf("%lld\n", (long long)(energy(bs) * 1e9));
}
