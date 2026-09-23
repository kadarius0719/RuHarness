#[no_mangle]
pub unsafe extern "C" fn to_barycentric(p1: crate::logic::LmVec2, p2: crate::logic::LmVec2, p3: crate::logic::LmVec2, p: crate::logic::LmVec2) -> crate::logic::LmVec2 {
    crate::logic::to_barycentric(p1, p2, p3, p)
}
