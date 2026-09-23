use crate::logic::Vec2;

#[no_mangle]
pub unsafe extern "C" fn convex_clip(
    poly: *mut Vec2,
    n_poly: i32,
    clip: *const Vec2,
    n_clip: i32,
    res: *mut Vec2,
) -> i32 {
    let n_poly_usize = n_poly as usize;
    let n_clip_usize = n_clip as usize;
    let max_size = n_poly_usize.saturating_add(n_clip_usize);
    let poly_slice = core::slice::from_raw_parts_mut(poly, max_size);
    let clip_slice = core::slice::from_raw_parts(clip, n_clip_usize);
    let res_slice = core::slice::from_raw_parts_mut(res, max_size);

    crate::logic::convex_clip(poly_slice, n_poly_usize, clip_slice, res_slice) as i32
}
