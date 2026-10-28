package main

import "fmt"

func main() {
	c := make(chan int64, 1024)
	n := 5000000
	go func() {
		for i := 0; i < n; i++ {
			c <- int64(i)
		}
		close(c)
	}()
	sum := int64(0)
	for v := range c {
		sum += v
	}
	fmt.Println(sum)
}
