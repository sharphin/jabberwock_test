package main

import "fmt"

type Tree struct{ l, r *Tree }

func make_(d int) *Tree {
	if d == 0 {
		return &Tree{}
	}
	return &Tree{make_(d - 1), make_(d - 1)}
}

func (t *Tree) check() int {
	if t.l == nil {
		return 1
	}
	return 1 + t.l.check() + t.r.check()
}

func main() {
	n := 18
	fmt.Println(n+1, make_(n+1).check())
	long := make_(n)
	for d := 4; d <= n; d += 2 {
		iters := 1 << (n - d + 4)
		chk := 0
		for i := 0; i < iters; i++ {
			chk += make_(d).check()
		}
		fmt.Println(iters, d, chk)
	}
	fmt.Println(n, long.check())
}
