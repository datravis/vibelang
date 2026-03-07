package main

import "fmt"

func double(x int64) int64 { return x * 2 }
func addOne(x int64) int64 { return x + 1 }
func square(x int64) int64 { return x * x }

func runLoop(n int64, acc int64) int64 {
	for n > 0 {
		acc += square(addOne(double(n)))
		n--
	}
	return acc
}

func main() {
	fmt.Println(runLoop(10000000, 0))
}
