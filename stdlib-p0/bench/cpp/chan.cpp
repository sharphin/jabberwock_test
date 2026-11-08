#include <cstdio>
#include <cstdint>
#include <deque>
#include <mutex>
#include <condition_variable>
#include <thread>
#include <optional>
// jabberwock と同じ「上限なし」のチャンネル
struct Chan {
    std::mutex m; std::condition_variable cv; std::deque<int64_t> q; bool closed = false;
    void send(int64_t v) { { std::lock_guard<std::mutex> g(m); q.push_back(v); } cv.notify_one(); }
    void close() { { std::lock_guard<std::mutex> g(m); closed = true; } cv.notify_all(); }
    std::optional<int64_t> recv() {
        std::unique_lock<std::mutex> g(m);
        cv.wait(g, [&] { return !q.empty() || closed; });
        if (q.empty()) return std::nullopt;
        int64_t v = q.front(); q.pop_front(); return v;
    }
};
int main() {
    Chan c; int n = 5000000;
    std::thread t([&] { for (int i = 0; i < n; i++) c.send(i); c.close(); });
    int64_t sum = 0;
    while (auto v = c.recv()) sum += *v;
    t.join();
    printf("%lld\n", (long long)sum);
}
