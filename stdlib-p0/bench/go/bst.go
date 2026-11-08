package main

import "fmt"

type Node struct {
	key         int64
	left, right *Node
}

func insert(slot **Node, key int64) {
	cur := slot
	for {
		if *cur == nil {
			*cur = &Node{key: key}
			return
		}
		n := *cur
		if key < n.key {
			cur = &n.left
		} else {
			cur = &n.right
		}
	}
}

func contains(n *Node, key int64) bool {
	for n != nil {
		if n.key == key {
			return true
		}
		if key < n.key {
			n = n.left
		} else {
			n = n.right
		}
	}
	return false
}

func main() {
	var root *Node
	x := int64(42)
	for i := 0; i < 1000000; i++ {
		x = (x*1103515245 + 12345) % 2147483648
		insert(&root, x)
	}
	found := 0
	x = 42
	for i := 0; i < 2000000; i++ {
		x = (x*1103515245 + 12345) % 2147483648
		if contains(root, x) {
			found++
		}
	}
	fmt.Println(found)
}
