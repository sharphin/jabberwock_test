#include <cstdio>
#include <string>

int main() {
    std::string s;
    for (long i = 0; i < 3000000; i++) {
        s += "v" + std::to_string(i % 1000) + ",";
    }
    long lines = 0;
    for (long i = 0; i < 1000; i++) {
        std::string line = "%t" + std::to_string(i) + " = add i64 %a, %b";
        line += "\n";
        lines += (long)line.size();
    }
    std::printf("%zu %ld\n", s.size(), lines);
}
