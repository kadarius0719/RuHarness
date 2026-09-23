#[repr(C)]
#[derive(Copy, Clone)]
pub struct LmVec2 {
    pub x: f32,
    pub y: f32,
}

pub fn lm_v2(x: f32, y: f32) -> LmVec2 {
    LmVec2 { x, y }
}

pub fn lm_sub2(a: LmVec2, b: LmVec2) -> LmVec2 {
    lm_v2(a.x - b.x, a.y - b.y)
}

pub fn lm_dot2(a: LmVec2, b: LmVec2) -> f32 {
    a.x * b.x + a.y * b.y
}

pub fn to_barycentric(p1: LmVec2, p2: LmVec2, p3: LmVec2, p: LmVec2) -> LmVec2 {
    let v0 = lm_sub2(p3, p1);
    let v1 = lm_sub2(p2, p1);
    let v2 = lm_sub2(p, p1);
    let dot00 = lm_dot2(v0, v0);
    let dot01 = lm_dot2(v0, v1);
    let dot02 = lm_dot2(v0, v2);
    let dot11 = lm_dot2(v1, v1);
    let dot12 = lm_dot2(v1, v2);
    let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01);
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;
    lm_v2(u, v)
}
