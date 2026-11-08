package main

import "fmt"

type P struct{ x, y, vx, vy float64 }

func step(ps []P, dt float64) {
	for i := range ps {
		p := &ps[i]
		p.vy -= 9.8 * dt
		p.x += p.vx * dt
		p.y += p.vy * dt
		if p.y < 0 {
			p.y = -p.y
			p.vy = -p.vy * 0.9
		}
	}
}

func main() {
	ps := []P{}
	for i := 0; i < 10000; i++ {
		ps = append(ps, P{float64(i), 10.0 + float64(i%13), 1, 0})
	}
	for t := 0; t < 10000; t++ {
		step(ps, 0.01)
	}
	s := 0.0
	for _, p := range ps {
		s += p.y
	}
	fmt.Println(int64(s * 1000))
}
