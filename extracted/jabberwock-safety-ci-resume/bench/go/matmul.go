package main

import "fmt"

func main() {
	n := 600
	a, b, c := []float64{}, []float64{}, []float64{}
	for i := 0; i < n*n; i++ {
		a = append(a, float64(i%7)*0.5)
		b = append(b, float64(i%5)*0.25)
		c = append(c, 0)
	}
	for i := 0; i < n; i++ {
		for k := 0; k < n; k++ {
			aik := a[i*n+k]
			for j := 0; j < n; j++ {
				c[i*n+j] += aik * b[k*n+j]
			}
		}
	}
	s := 0.0
	for i := 0; i < n*n; i++ {
		s += c[i]
	}
	fmt.Println(int64(s * 1000))
}
