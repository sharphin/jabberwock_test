package main

import (
	"fmt"
	"slices"
)

func main() {
	v := []int64{}
	x := int64(7)
	for i := 0; i < 3000000; i++ {
		x = (x*1103515245 + 12345) % 2147483648
		v = append(v, x)
	}
	slices.Sort(v)
	sum := int64(0)
	for i := 0; i < len(v); i += 1000 {
		sum += v[i]
	}
	fmt.Println(v[0], v[len(v)-1], sum)
}
