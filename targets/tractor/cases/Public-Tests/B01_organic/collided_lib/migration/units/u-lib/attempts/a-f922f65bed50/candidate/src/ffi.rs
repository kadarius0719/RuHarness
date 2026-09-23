use crate::logic::{C2v, C2Circle, C2AABB};
use crate::logic;

#[no_mangle]
pub extern "C" fn c2V(x: f32, y: f32) -> C2v {
    logic::c2_v(x, y)
}

#[no_mangle]
pub extern "C" fn c2Maxv(a: C2v, b: C2v) -> C2v {
    logic::c2_maxv(a, b)
}

#[no_mangle]
pub extern "C" fn c2Minv(a: C2v, b: C2v) -> C2v {
    logic::c2_minv(a, b)
}

#[no_mangle]
pub extern "C" fn c2Clampv(a: C2v, lo: C2v, hi: C2v) -> C2v {
    logic::c2_clampv(a, lo, hi)
}

#[no_mangle]
pub extern "C" fn c2Sub(a: C2v, b: C2v) -> C2v {
    logic::c2_sub(a, b)
}

#[no_mangle]
pub extern "C" fn c2Dot(a: C2v, b: C2v) -> f32 {
    logic::c2_dot(a, b)
}

#[no_mangle]
pub extern "C" fn c2CircletoCircle(a: C2Circle, b: C2Circle) -> i32 {
    logic::c2_circle_to_circle(a, b)
}

#[no_mangle]
pub extern "C" fn c2CircletoAABB(a: C2Circle, b: C2AABB) -> i32 {
    logic::c2_circle_to_aabb(a, b)
}

#[no_mangle]
pub extern "C" fn c2AABBtoAABB(a: C2AABB, b: C2AABB) -> i32 {
    logic::c2_aabb_to_aabb(a, b)
}

#[no_mangle]
pub unsafe extern "C" fn collided(
    a: *const std::ffi::c_void,
    type_a: u32,
    b: *const std::ffi::c_void,
    type_b: u32,
) -> i32 {
    let a_circle = if type_a == 0 && !a.is_null() {
        Some(*(a as *const C2Circle))
    } else {
        None
    };

    let a_aabb = if type_a == 1 && !a.is_null() {
        Some(*(a as *const C2AABB))
    } else {
        None
    };

    let b_circle = if type_b == 0 && !b.is_null() {
        Some(*(b as *const C2Circle))
    } else {
        None
    };

    let b_aabb = if type_b == 1 && !b.is_null() {
        Some(*(b as *const C2AABB))
    } else {
        None
    };

    logic::collided_inner(a_circle, a_aabb, type_a, b_circle, b_aabb, type_b)
}
