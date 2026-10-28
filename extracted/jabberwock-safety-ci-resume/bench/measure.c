// ベンチマークの測り方: コマンドを N 回走らせて、かかった秒数の最小・中央値・最大と、最大メモリ（最大 RSS、MB）を出す。
// 使い方: measure N 出力ファイル コマンド [引数...]   （標準出力は出力ファイルへ。最後の回のものが残る）
// 出力: "最小 中央値 最大 最大RSS" の 1 行（秒・秒・秒・MB）。Linux 用（wait4 の rusage で RSS を取る）
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int cmp(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b;
    return x < y ? -1 : x > y;
}

int main(int argc, char **argv) {
    if (argc < 4) {
        fprintf(stderr, "使い方: measure N 出力ファイル コマンド [引数...]\n");
        return 2;
    }
    char *end = NULL;
    long count = strtol(argv[1], &end, 10);
    if (!end || *end != 0 || count < 1 || count > 100000) {
        fprintf(stderr, "measure: invalid repetition count\n");
        return 2;
    }
    int n = (int)count;
    double *t = malloc(sizeof(double) * (size_t)n);
    if (!t) { perror("malloc"); return 2; }
    long rss = 0;
    for (int i = 0; i < n; i++) {
        struct timespec s, e;
        clock_gettime(CLOCK_MONOTONIC, &s);
        pid_t p = fork();
        if (p < 0) { perror("fork"); free(t); return 1; }
        if (p == 0) {
            // Retain every run output for deterministic cross-run checks.
            size_t cap = strlen(argv[2]) + 48;
            char *destination = malloc(cap);
            if (!destination) _exit(127);
            if (i == n - 1) snprintf(destination, cap, "%s", argv[2]);
            else snprintf(destination, cap, "%s.run%d", argv[2], i);
            int fd = open(destination, O_WRONLY | O_CREAT | O_TRUNC, 0644);
            free(destination);
            if (fd < 0 || dup2(fd, 1) < 0) _exit(127);
            close(fd);
            execvp(argv[3], argv + 3);
            _exit(127);
        }
        int st;
        struct rusage ru;
        if (wait4(p, &st, 0, &ru) < 0) {
            perror("wait4");
            return 1;
        }
        clock_gettime(CLOCK_MONOTONIC, &e);
        if (!WIFEXITED(st) || WEXITSTATUS(st) != 0) {
            fprintf(stderr, "measure: %s が失敗しました（%d）\n", argv[3], st);
            return 1;
        }
        t[i] = (double)(e.tv_sec - s.tv_sec) + (double)(e.tv_nsec - s.tv_nsec) / 1e9;
        if (ru.ru_maxrss > rss) rss = ru.ru_maxrss;
    }
    qsort(t, (size_t)n, sizeof(double), cmp);
    double med = n % 2 ? t[n / 2] : (t[n / 2 - 1] + t[n / 2]) / 2;
    printf("%.6f %.6f %.6f %.1f\n", t[0], med, t[n - 1], (double)rss / 1024.0);
    return 0;
}
