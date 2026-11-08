// ベンチマークの測り方: コマンドを N 回走らせて、かかった秒数の最小・中央値・最大と、最大メモリ（最大 RSS、MB）を出す。
// 使い方: measure N 出力ファイル コマンド [引数...]   （標準出力は出力ファイルへ。最後の回のものが残る）
// 出力: "最小 中央値 最大 最大RSS" の 1 行（秒・秒・秒・MB）。Linux 用（wait4 の rusage で RSS を取る）
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
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
    int n = atoi(argv[1]);
    if (n < 1) n = 1;
    double *t = malloc(sizeof(double) * (size_t)n);
    long rss = 0;
    for (int i = 0; i < n; i++) {
        struct timespec s, e;
        clock_gettime(CLOCK_MONOTONIC, &s);
        pid_t p = fork();
        if (p == 0) {
            int fd = open(argv[2], O_WRONLY | O_CREAT | O_TRUNC, 0644);
            if (fd < 0 || dup2(fd, 1) < 0) _exit(127);
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
