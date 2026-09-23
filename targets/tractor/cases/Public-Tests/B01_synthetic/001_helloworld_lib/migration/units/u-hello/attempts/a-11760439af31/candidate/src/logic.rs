pub fn helloworld() -> i32 {
    let msg = b"Hello World!\n";
    for &byte in msg {
        crate::ffi::put_byte(byte);
    }
    0
}
