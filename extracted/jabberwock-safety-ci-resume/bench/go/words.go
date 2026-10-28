package main

import "fmt"

func main() {
	counts := map[string]int64{}
	x := int64(1)
	for i := 0; i < 5000000; i++ {
		x = (x*1103515245 + 12345) % 2147483648
		w := fmt.Sprintf("w%d", x%5000)
		counts[w]++
	}
	best := int64(0)
	for _, v := range counts {
		if v > best {
			best = v
		}
	}
	fmt.Println(len(counts), best)
}
