package main

import "fmt"

func main() {
	n := 1600
	bits := make([]uint8, n*n/8)
	pos := 0
	for y := 0; y < n; y++ {
		ci := 2.0*float64(y)/float64(n) - 1.0
		for x := 0; x < n; x += 8 {
			b8 := 0
			for b := 0; b < 8; b++ {
				cr := 2.0*float64(x+b)/float64(n) - 1.5
				zr, zi := 0.0, 0.0
				inside := true
				for i := 0; i < 50; i++ {
					tr := zr*zr - zi*zi + cr
					zi = 2.0*zr*zi + ci
					zr = tr
					if zr*zr+zi*zi > 4.0 {
						inside = false
						break
					}
				}
				b8 <<= 1
				if inside {
					b8 |= 1
				}
			}
			bits[pos] = uint8(b8)
			pos++
		}
	}
	sum := 0
	for _, v := range bits {
		sum += int(v) * 7 % 255
	}
	fmt.Println(pos, sum)
}
