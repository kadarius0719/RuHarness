#[repr(C)]
#[derive(Copy, Clone)]
pub struct C2v {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct C2Circle {
    pub p: C2v,
    pub r: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct C2AABB {
    pub min: C2v,
    pub max: C2v,
}

pub fn c2_v(x: f32, y: f32) -> C2v {
    C2v { x, y }
}

pub fn c2_maxv(a: C2v, b: C2v) -> C2v {
    c2_v(if a.x > b.x { a.x } else { b.x },
         if a.y > b.y { a.y } else { b.y })
}

pub fn c2_minv(a: C2v, b: C2v) -> C2v {
    c2_v(if a.x < b.x { a.x } else { b.x },
         if a.y < b.y { a.y } else { b.y })
}

pub fn c2_clampv(a: C2v, lo: C2v, hi: C2v) -> C2v {
    c2_maxv(lo, c2_minv(a, hi))
}

pub fn c2_sub(mut a: C2v, b: C2v) -> C2v {
    a.x -= b.x;
    a.y -= b.y;
    a
}

pub fn c2_dot(a: C2v, b: C2v) -> f32 {
    a.x * b.x + a.y * b.y
}

pub fn c2_circle_to_circle(a: C2Circle, b: C2Circle) -> i32 {
    let c = c2_sub(b.p, a.p);
    let d2 = c2_dot(c, c);
    let mut r2 = a.r + b.r;
    r2 = r2 * r2;
    if d2 < r2 { 1 } else { 0 }
}

pub fn c2_circle_to_aabb(a: C2Circle, b: C2AABB) -> i32 {
    let l = c2_clampv(a.p, b.min, b.max);
    let ab = c2_sub(a.p, l);
    let d2 = c2_dot(ab, ab);
    let r2 = a.r * a.r;
    if d2 < r2 { 1 } else { 0 }
}

pub fn c2_aabb_to_aabb(a: C2AABB, b: C2AABB) -> i32 {
    let d0 = if b.max.x < a.min.x { 1 } else { 0 };
    let d1 = if a.max.x < b.min.x { 1 } else { 0 };
    let d2 = if b.max.y < a.min.y { 1 } else { 0 };
    let d3 = if a.max.y < b.min.y { 1 } else { 0 };
    if (d0 | d1 | d2 | d3) == 0 { 1 } else { 0 }
}

pub fn collided_inner(a_circle: Option<C2Circle>, a_aabb: Option<C2AABB>, type_a: u32, b_circle: Option<C2Circle>, b_aabb: Option<C2AABB>, type_b: u32) -> i32 {
    match type_a {
        0 => {
            match type_b {
                0 => {
                    if let (Some(a), Some(b)) = (a_circle, b_circle) {
                        c2_circle_to_circle(a, b)
                    } else {
                        0
                    }
                }
                1 => {
                    if let (Some(a), Some(b)) = (a_circle, b_aabb) {
                        c2_circle_to_aabb(a, b)
                    } else {
                        0
                    }
                }
                _ => 0,
            }
        }
        1 => {
            match type_b {
                0 => {
                    if let (Some(a), Some(b)) = (a_aabb, b_circle) {
                        c2_circle_to_aabb(b, a)
                    } else {
                        0
                    }
                }
                1 => {
                    if let (Some(a), Some(b)) = (a_aabb, b_aabb) {
                        c2_aabb_to_aabb(a, b)
                    } else {
                        0
                    }
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}
