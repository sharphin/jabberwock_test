#include <cstdio>
#include <cstdint>
#include <vector>
int main() {
    int n = 1600;
    std::vector<uint8_t> bits(n * n / 8);
    int pos = 0;
    for (int y = 0; y < n; y++) {
        double ci = 2.0 * y / n - 1.0;
        for (int x = 0; x < n; x += 8) {
            int b8 = 0;
            for (int b = 0; b < 8; b++) {
                double cr = 2.0 * (x + b) / n - 1.5, zr = 0, zi = 0;
                bool inside = true;
                for (int i = 0; i < 50; i++) {
                    double tr = zr * zr - zi * zi + cr;
                    zi = 2.0 * zr * zi + ci;
                    zr = tr;
                    if (zr * zr + zi * zi > 4.0) { inside = false; break; }
                }
                b8 = (b8 << 1) | (inside ? 1 : 0);
            }
            bits[pos++] = (uint8_t)b8;
        }
    }
    long sum = 0;
    for (uint8_t v : bits) sum += v * 7 % 255;
    printf("%d %ld\n", pos, sum);
}
