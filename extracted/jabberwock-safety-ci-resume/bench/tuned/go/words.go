// words の速くした書き方: バイト列を使い回し、探すときは文字列を作らない（m[string(b)] は割り当てない）
package main

import (
	"fmt"
	"strconv"
)

func main() {
	counts := make(map[string]int64, 8192)
	x := int64(1)
	buf := make([]byte, 0, 32)
	for i := 0; i < 5000000; i++ {
		x = (x*1103515245 + 12345) % 2147483648
		buf = append(buf[:0], 'w')
		buf = strconv.AppendInt(buf, x%5000, 10)
		if _, ok := counts[string(buf)]; ok {
			counts[string(buf)]++
		} else {
			counts[string(buf)] = 1
		}
	}
	best := int64(0)
	for _, v := range counts {
		if v > best {
			best = v
		}
	}
	fmt.Println(len(counts), best)
}
