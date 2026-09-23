use procspawn::{self, spawn};
use std::process::Command;

fn main() {
    procspawn::init();

    let mut wrapper = Command::new("perf");
    wrapper.arg("stat").arg("--");

    let handle = spawn((1u32, 2u32), |(a, b)| a + b, Some(wrapper));
    println!("result: {}", handle.join().unwrap());
}
