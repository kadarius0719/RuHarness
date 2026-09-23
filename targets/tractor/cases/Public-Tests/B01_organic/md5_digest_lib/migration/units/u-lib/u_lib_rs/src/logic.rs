#[repr(C)]
pub struct Md5 {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

pub fn md5_digest(m: &Md5, out: &mut [u8]) {
    if out.len() < 16 {
        return;
    }

    out[0] = m.a as u8;
    out[1] = (m.a >> 8) as u8;
    out[2] = (m.a >> 16) as u8;
    out[3] = (m.a >> 24) as u8;
    out[4] = m.b as u8;
    out[5] = (m.b >> 8) as u8;
    out[6] = (m.b >> 16) as u8;
    out[7] = (m.b >> 24) as u8;
    out[8] = m.c as u8;
    out[9] = (m.c >> 8) as u8;
    out[10] = (m.c >> 16) as u8;
    out[11] = (m.c >> 24) as u8;
    out[12] = m.d as u8;
    out[13] = (m.d >> 8) as u8;
    out[14] = (m.d >> 16) as u8;
    out[15] = (m.d >> 24) as u8;
}
