package main

import "fmt"

type Node struct {
	v    int64
	next *Node
}

func sum(n *Node) int64 {
	s := int64(0)
	for ; n != nil; n = n.next {
		s += n.v
	}
	return s
}

func main() {
	var head *Node
	for i := int64(0); i < 1000; i++ {
		head = &Node{i, head}
	}
	total := int64(0)
	for k := 0; k < 100000; k++ {
		total += sum(head) % 1000
	}
	fmt.Println(total)
}
