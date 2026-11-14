package main

import "fmt"

func main() {
	n := 20000000
	comp := []bool{}
	for i := 0; i <= n; i++ {
		comp = append(comp, false)
	}
	count := 0
	for i := 2; i <= n; i++ {
		if !comp[i] {
			count++
			for j := i * i; j <= n; j += i {
				comp[j] = true
			}
		}
	}
	fmt.Println(count)
}
