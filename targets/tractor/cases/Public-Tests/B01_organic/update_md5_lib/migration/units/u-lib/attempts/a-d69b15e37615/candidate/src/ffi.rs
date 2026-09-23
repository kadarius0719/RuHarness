#[repr(C)]
pub struct TflacMd5 {
    pub pos: u32,
    pub total: u64,
    pub buffer: [u8; 72],
}

#[repr(C)]
pub struct Tflac {
    pub md5_ctx: TflacMd5,
    pub cur_blocksize: u32,
    pub channels: u32,
}

#[no_mangle]
pub unsafe extern "C" fn tflac_md5_addsample(m: *mut TflacMd5, bits: u32, val: u64) {
    let m_ref = &mut *m;
    crate::logic::md5_addsample(&mut m_ref.pos, &mut m_ref.total, &mut m_ref.buffer, bits, val);
}

#[no_mangle]
pub unsafe extern "C" fn tflac_pack_u64le(d: *mut u8, n: u64) {
    let slice = core::slice::from_raw_parts_mut(d, 8);
    crate::logic::pack_u64le(slice, n);
}

#[no_mangle]
pub unsafe extern "C" fn update_md5(t: *mut Tflac, samples: *const i32) -> u32 {
    let t_ref = &mut *t;
    // The C loop body always runs a fixed 5 iterations, reading 8
    // tflac_s32 samples per iteration starting at a stride of 32
    // elements (`samples += 8 * sizeof(tflac_s32)`, pointer arithmetic
    // in tflac_s32 units): offsets 0, 32, 64, 96, 128, each read up to
    // index+7. The highest index ever touched is 128 + 7 = 135, so the
    // function always needs 136 contiguous samples, regardless of
    // `cur_blocksize * channels` (that product only feeds the returned
    // "remaining" bookkeeping value, not how many samples are read).
    const SAMPLES_NEEDED: usize = 136;
    let samples_slice = core::slice::from_raw_parts(samples, SAMPLES_NEEDED);
    crate::logic::update_md5(
        &mut t_ref.md5_ctx.pos,
        &mut t_ref.md5_ctx.total,
        &mut t_ref.md5_ctx.buffer,
        t_ref.cur_blocksize,
        t_ref.channels,
        samples_slice,
    )
}
