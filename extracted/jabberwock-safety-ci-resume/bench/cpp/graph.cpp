#include <cstdio>
#include <vector>
int main() {
    long w = 300, n = w * w;
    std::vector<long> start, edges;
    for (long v = 0; v < n; v++) {
        start.push_back(edges.size());
        long x = v % w, y = v / w;
        if (x > 0) edges.push_back(v - 1);
        if (x < w - 1) edges.push_back(v + 1);
        if (y > 0) edges.push_back(v - w);
        if (y < w - 1) edges.push_back(v + w);
    }
    start.push_back(edges.size());
    std::vector<long> dist(n), queue(n);
    long total = 0;
    for (long round = 0; round < 300; round++) {
        for (auto& d : dist) d = -1;
        long src = (round * 7919) % n;
        dist[src] = 0; queue[0] = src;
        long head = 0, tail = 1;
        while (head < tail) {
            long v = queue[head++];
            for (long e = start[v]; e < start[v + 1]; e++) {
                long u = edges[e];
                if (dist[u] < 0) { dist[u] = dist[v] + 1; queue[tail++] = u; }
            }
        }
        for (long d : dist) total += d;
    }
    printf("%ld\n", total);
}
