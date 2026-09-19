use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let values: Vec<i64> = input
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    if values.len() != 2 {
        std::process::exit(2);
    }
    println!("{}", values[0] + values[1]);
}

