#include <cstdio>
#include <cstdint>
#include <memory>
struct Node { int64_t v; std::unique_ptr<Node> next; };
static int64_t sum(const Node* n) { int64_t s = 0; for (; n; n = n->next.get()) s += n->v; return s; }
int main() {
    std::unique_ptr<Node> head;
    for (int64_t i = 0; i < 1000; i++) head = std::make_unique<Node>(Node{i, std::move(head)});
    int64_t total = 0;
    for (int k = 0; k < 100000; k++) total += sum(head.get()) % 1000;
    printf("%lld\n", (long long)total);
    // 長いリストを再帰で解放しないよう、前から外す
    while (head) head = std::move(head->next);
}
