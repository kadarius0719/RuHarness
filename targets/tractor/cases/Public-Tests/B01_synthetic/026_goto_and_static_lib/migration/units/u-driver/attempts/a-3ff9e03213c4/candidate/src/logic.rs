pub fn multi_stage(x: i32, y: i32, z: i32) -> i32 {
    let mut result = 0;

    if x != 1 {
        crate::ffi::printf_line("Error: x != 1");
        result = 1;
    } else if y != 2 {
        crate::ffi::printf_line("Error: x == 1 but y != 2");
        result = 2;
    } else if z != 3 {
        crate::ffi::printf_line("Error: x == 1 and y == 2, but z != 3");
        result = 3;
    } else {
        crate::ffi::printf_line("Ok!");
        return result;
    }

    crate::ffi::printf_line("Operation failed");
    result
}

pub fn driver_impl(x: i32, local_y: i32, z: i32) {
    let result = multi_stage(x, local_y, z);
    crate::ffi::printf_result(result);
}
