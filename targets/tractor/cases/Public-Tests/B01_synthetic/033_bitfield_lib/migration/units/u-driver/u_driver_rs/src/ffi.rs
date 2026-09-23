extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[repr(C)]
pub struct foo_t {
    bits: u32,
    z: i32,
}

#[no_mangle]
pub unsafe extern "C" fn print_foo(foo: *const foo_t) {
    let foo_ref = &*foo;

    let x = (foo_ref.bits & 0b11) as u32;
    let y = ((foo_ref.bits >> 2) & 0b111) as u32;
    let b = ((foo_ref.bits >> 5) & 0b1) != 0;
    let z = foo_ref.z;

    crate::logic::print_foo_impl(x, y, b, z);
}

#[no_mangle]
pub unsafe extern "C" fn driver(x: u32, y: u32, b: bool, z: i32) {
    crate::logic::driver_impl(x, y, b, z);
}
