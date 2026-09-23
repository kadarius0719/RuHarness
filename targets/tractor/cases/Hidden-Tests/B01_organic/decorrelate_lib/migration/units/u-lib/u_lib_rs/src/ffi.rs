use crate::logic::Tflac;

#[no_mangle]
pub unsafe extern "C" fn decorrelate(
    t: *mut Tflac,
    channel: u32,
    stride: u32,
) {
    crate::logic::decorrelate(&mut *t, channel, stride);
}
