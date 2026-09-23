#[no_mangle]
pub unsafe extern "C" fn update_frame_header(t: *mut crate::logic::tflac) {
    crate::logic::update_frame_header(&mut *t);
}
