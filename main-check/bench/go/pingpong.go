package main

import "fmt"

func main() {
	ping := make(chan int64)
	pong := make(chan int64)
	n := 50000
	done := make(chan bool)
	go func() {
		for v := range ping {
			pong <- v + 1
		}
		done <- true
	}()
	v := int64(0)
	for i := 0; i < n; i++ {
		ping <- v
		v = <-pong
	}
	close(ping)
	<-done
	fmt.Println(v)
}
