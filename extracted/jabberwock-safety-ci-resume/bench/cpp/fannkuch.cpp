#include <cstdio>
#include <vector>
int main() {
    int n = 10;
    std::vector<int> perm(n), perm1(n), count(n);
    for (int i = 0; i < n; i++) perm1[i] = i;
    int r = n, checksum = 0, maxflips = 0, permcount = 0;
    for (;;) {
        while (r != 1) { count[r - 1] = r; r--; }
        perm = perm1;
        int flips = 0;
        for (int k = perm[0]; k != 0; k = perm[0]) {
            for (int i = 0, j = k; i < j; i++, j--) { int t = perm[i]; perm[i] = perm[j]; perm[j] = t; }
            flips++;
        }
        if (flips > maxflips) maxflips = flips;
        checksum += permcount % 2 == 0 ? flips : -flips;
        for (;;) {
            if (r == n) { printf("%d %d\n", checksum, maxflips); return 0; }
            int p0 = perm1[0];
            for (int i = 0; i < r; i++) perm1[i] = perm1[i + 1];
            perm1[r] = p0;
            if (--count[r] > 0) break;
            r++;
        }
        permcount++;
    }
}
