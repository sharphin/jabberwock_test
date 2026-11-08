package main

import (
	"fmt"
	"slices"
)

func sortedSum(v []int64) int64 {
	slices.Sort(v)
	s := int64(0)
	for i := 0; i < len(v); i += 997 {
		s += v[i] * int64(i%7)
	}
	return s
}

func main() {
	n := 3000000
	a, b, c, d := []int64{}, []int64{}, []int64{}, []int64{}
	for i := 0; i < n; i++ {
		a = append(a, int64(i))
		b = append(b, int64(n-i))
		c = append(c, 42)
		d = append(d, int64(i%1000))
	}
	fmt.Println(sortedSum(a), sortedSum(b), sortedSum(c), sortedSum(d))
}
