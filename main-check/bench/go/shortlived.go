package main

import "fmt"

type P struct{ x, y int64 }

func main() {
	ring := make([]*P, 16)
	for i := range ring {
		ring[i] = &P{}
	}
	s := int64(0)
	for i := int64(0); i < 100000000; i++ {
		ring[i%16] = &P{i, i * 2}
		s += ring[(i+5)%16].y - ring[(i+3)%16].x
	}
	fmt.Println(s)
}
