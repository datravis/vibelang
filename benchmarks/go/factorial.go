package main

import "fmt"

func factorial(n int64) int64 {
	if n <= 1 {
		return 1
	}
	return n * factorial(n-1)
}

func runLoop(n int64, acc int64) int64 {
	for n > 0 {
		acc += factorial(20)
		n--
	}
	return acc
}

func main() {
	fmt.Println(runLoop(10000000, 0))
}
