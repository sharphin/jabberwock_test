package main

import (
	"fmt"
	"os"
	"strconv"
)

func count(w, t, n int64) int64 {
	c := int64(0)
	for blk := w; blk*1000 < n; blk += t {
		for x := max(blk*1000, 2); x < min(blk*1000+1000, n); x++ {
			p := true
			for d := int64(2); d*d <= x; d++ {
				if x%d == 0 {
					p = false
					break
				}
			}
			if p {
				c++
			}
		}
	}
	return c
}

func main() {
	t := int64(4)
	if len(os.Args) > 1 {
		if v, err := strconv.ParseInt(os.Args[1], 10, 64); err == nil {
			t = v
		}
	}
	n := int64(3000000)
	res := make(chan int64)
	for w := int64(0); w < t; w++ {
		go func(w int64) { res <- count(w, t, n) }(w)
	}
	total := int64(0)
	for w := int64(0); w < t; w++ {
		total += <-res
	}
	fmt.Println(total)
}
