package main

import "fmt"

func main() {
	w := 300
	n := w * w
	start := []int{}
	edges := []int{}
	for v := 0; v < n; v++ {
		start = append(start, len(edges))
		x, y := v%w, v/w
		if x > 0 {
			edges = append(edges, v-1)
		}
		if x < w-1 {
			edges = append(edges, v+1)
		}
		if y > 0 {
			edges = append(edges, v-w)
		}
		if y < w-1 {
			edges = append(edges, v+w)
		}
	}
	start = append(start, len(edges))
	dist := make([]int, n)
	queue := make([]int, n)
	total := 0
	for round := 0; round < 300; round++ {
		for v := range dist {
			dist[v] = -1
		}
		src := (round * 7919) % n
		dist[src] = 0
		queue[0] = src
		head, tail := 0, 1
		for head < tail {
			v := queue[head]
			head++
			for e := start[v]; e < start[v+1]; e++ {
				u := edges[e]
				if dist[u] < 0 {
					dist[u] = dist[v] + 1
					queue[tail] = u
					tail++
				}
			}
		}
		for _, d := range dist {
			total += d
		}
	}
	fmt.Println(total)
}
