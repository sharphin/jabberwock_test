// sieve の速くした書き方: 最初に大きさを決める
package main

import "fmt"

func main() {
	n := 20000000
	comp := make([]bool, n+1)
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
