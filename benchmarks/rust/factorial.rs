fn factorial(n: i64) -> i64 {
    if n <= 1 { 1 } else { n.wrapping_mul(factorial(n - 1)) }
}

fn main() {
    let mut acc: i64 = 0;
    for _ in 0..10_000_000 {
        acc = acc.wrapping_add(factorial(20));
    }
    println!("{}", acc);
}
