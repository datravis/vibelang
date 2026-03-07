fn double(x: i64) -> i64 { x * 2 }
fn add_one(x: i64) -> i64 { x + 1 }
fn square(x: i64) -> i64 { x.wrapping_mul(x) }

fn main() {
    let mut acc: i64 = 0;
    for n in (1..=10_000_000i64).rev() {
        acc = acc.wrapping_add(square(add_one(double(n))));
    }
    println!("{}", acc);
}
