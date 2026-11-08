#include <cstdio>
#include <cstdint>
#include <memory>
struct Node {
    int64_t key;
    std::unique_ptr<Node> left, right;
};
static void insert(std::unique_ptr<Node>* cur, int64_t key) {
    while (*cur) cur = key < (*cur)->key ? &(*cur)->left : &(*cur)->right;
    *cur = std::make_unique<Node>(Node{key, nullptr, nullptr});
}
static bool contains(const Node* n, int64_t key) {
    while (n) {
        if (n->key == key) return true;
        n = key < n->key ? n->left.get() : n->right.get();
    }
    return false;
}
int main() {
    std::unique_ptr<Node> root;
    int64_t x = 42;
    for (int i = 0; i < 1000000; i++) { x = (x * 1103515245 + 12345) % 2147483648; insert(&root, x); }
    long found = 0;
    x = 42;
    for (int i = 0; i < 2000000; i++) { x = (x * 1103515245 + 12345) % 2147483648; if (contains(root.get(), x)) found++; }
    printf("%ld\n", found);
}
