package main

import (
	"fmt"
	"strings"
)

func main() {
	var b strings.Builder
	for i := 0; i < 3000000; i++ {
		fmt.Fprintf(&b, "v%d,", i%1000)
	}
	lines := 0
	for i := 0; i < 1000; i++ {
		line := fmt.Sprintf("%%t%d = add i64 %%a, %%b", i)
		line += "\n"
		lines += len(line)
	}
	fmt.Println(b.Len(), lines)
}
