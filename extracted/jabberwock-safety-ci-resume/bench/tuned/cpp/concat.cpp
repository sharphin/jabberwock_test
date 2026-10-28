// concat の速くした書き方: 一時的な文字列を作らずに to_chars で直接足す
#include <charconv>
#include <cstdio>
#include <string>
int main() {
    std::string s;
    char buf[32];
    for (long i = 0; i < 3000000; i++) {
        buf[0] = 'v';
        char* e = std::to_chars(buf + 1, buf + sizeof buf, i % 1000).ptr;
        *e++ = ',';
        s.append(buf, e - buf);
    }
    long lines = 0;
    for (long i = 0; i < 1000; i++) {
        std::string line = "%t" + std::to_string(i) + " = add i64 %a, %b";
        line += "\n";
        lines += (long)line.size();
    }
    std::printf("%zu %ld\n", s.size(), lines);
}
