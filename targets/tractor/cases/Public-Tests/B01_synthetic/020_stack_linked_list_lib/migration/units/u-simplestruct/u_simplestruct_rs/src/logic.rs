pub fn smallestValue(values: &[i32]) -> i32 {
    if values.is_empty() {
        -1
    } else {
        *values.iter().min().unwrap()
    }
}
