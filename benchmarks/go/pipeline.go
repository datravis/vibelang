package main

import "fmt"

func double(x int64) int64 { return x * 2 }
func addOne(x int64) int64 { return x + 1 }

func main() {
	result := addOne(double(5))
	fmt.Println(result)
}
