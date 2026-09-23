pub fn print_line(i: i32, j: i32) {
    let line = format!("{} {}\n", i, j);
    for byte in line.as_bytes() {
        crate::ffi::put_byte(*byte);
    }
}

pub fn driver_impl(x: i32) {
    let mut i: i32 = 0;
    let mut j: i32 = 0;
    while i < x {
        print_line(i, j);
        i = i.wrapping_add(1);
        j = j.wrapping_add(2);
    }
}
