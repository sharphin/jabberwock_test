package main

import "fmt"

func main() {
	n := 10
	perm := make([]int, n)
	perm1 := make([]int, n)
	count := make([]int, n)
	for i := range perm1 {
		perm1[i] = i
	}
	r, checksum, maxflips, permcount := n, 0, 0, 0
	for {
		for r != 1 {
			count[r-1] = r
			r--
		}
		copy(perm, perm1)
		flips := 0
		for k := perm[0]; k != 0; k = perm[0] {
			for i, j := 0, k; i < j; i, j = i+1, j-1 {
				perm[i], perm[j] = perm[j], perm[i]
			}
			flips++
		}
		if flips > maxflips {
			maxflips = flips
		}
		if permcount%2 == 0 {
			checksum += flips
		} else {
			checksum -= flips
		}
		for {
			if r == n {
				fmt.Println(checksum, maxflips)
				return
			}
			p0 := perm1[0]
			for i := 0; i < r; i++ {
				perm1[i] = perm1[i+1]
			}
			perm1[r] = p0
			count[r]--
			if count[r] > 0 {
				break
			}
			r++
		}
		permcount++
	}
}
