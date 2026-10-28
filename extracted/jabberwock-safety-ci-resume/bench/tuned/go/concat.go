// concat の速くした書き方: バイト列に直接書く
package main

import (
	"fmt"
	"strconv"
)

func main() {
	b := make([]byte, 0, 1<<20)
	for i := 0; i < 3000000; i++ {
		b = append(b, 'v')
		b = strconv.AppendInt(b, int64(i%1000), 10)
		b = append(b, ',')
	}
	lines := 0
	for i := 0; i < 1000; i++ {
		line := "%t" + strconv.Itoa(i) + " = add i64 %a, %b"
		line += "\n"
		lines += len(line)
	}
	fmt.Println(len(b), lines)
}
