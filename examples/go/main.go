// cdev watch examples/go/main.go      (untested: go not installed when written)
package main

import (
	"cdev"
	"fmt"
)

type Node struct {
	Val         int
	Left, Right *Node
}

func insert(n *Node, v int) *Node {
	if n == nil {
		return &Node{Val: v}
	}
	if v < n.Val {
		n.Left = insert(n.Left, v)
	} else {
		n.Right = insert(n.Right, v)
	}
	return n
}

func binarySearch(xs []int, target int) int {
	defer cdev.Trace(xs, target)()
	lo, hi := 0, len(xs)-1
	for lo <= hi {
		mid := (lo + hi) / 2
		cdev.W(map[string]any{"xs": xs, "lo": lo, "hi": hi, "mid": mid}, "search")
		switch {
		case xs[mid] == target:
			return mid
		case xs[mid] < target:
			lo = mid + 1
		default:
			hi = mid - 1
		}
	}
	return -1
}

func main() {
	defer cdev.Flush()
	var root *Node
	for _, v := range []int{5, 2, 8, 1, 3} {
		root = insert(root, v)
		cdev.W(root, "bst")
	}
	i := cdev.W(binarySearch([]int{1, 3, 5, 7, 9, 11}, 9))
	cdev.Log("found at %d", i)
	fmt.Println("done")
}
