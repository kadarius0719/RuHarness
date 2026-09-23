pub fn echo(args: &[&str]) -> i32 {
    for arg in &args[1..] {
        crate::ffi::print_string(arg);
        crate::ffi::print_newline();
    }
    0
}
