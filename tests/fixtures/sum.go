package main

import "fmt"

func main() {
	var a, b int64
	if _, err := fmt.Scan(&a, &b); err != nil {
		panic(err)
	}
	fmt.Println(a + b)
}

