package main

import (
	"fmt"
	"math"
)

func a(i, j int) float64 { return 1.0 / float64((i+j)*(i+j+1)/2+i+1) }

func times(v, out []float64) {
	for i := range v {
		s := 0.0
		for j := range v {
			s += a(i, j) * v[j]
		}
		out[i] = s
	}
}

func timesT(v, out []float64) {
	for i := range v {
		s := 0.0
		for j := range v {
			s += a(j, i) * v[j]
		}
		out[i] = s
	}
}

func timesAtA(v, tmp, out []float64) {
	times(v, tmp)
	timesT(tmp, out)
}

func main() {
	n := 2000
	u := make([]float64, n)
	v := make([]float64, n)
	tmp := make([]float64, n)
	for i := range u {
		u[i] = 1
	}
	for k := 0; k < 10; k++ {
		timesAtA(u, tmp, v)
		timesAtA(v, tmp, u)
	}
	vbv, vv := 0.0, 0.0
	for i := 0; i < n; i++ {
		vbv += u[i] * v[i]
		vv += v[i] * v[i]
	}
	fmt.Println(int64(math.Sqrt(vbv/vv) * 1e9))
}
