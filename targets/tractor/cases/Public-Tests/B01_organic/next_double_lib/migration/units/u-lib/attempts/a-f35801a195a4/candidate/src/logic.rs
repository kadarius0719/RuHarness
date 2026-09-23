#[repr(C)]
pub struct Rnd {
    pub state: [u64; 2],
}

fn rnd_next(rnd: &mut Rnd) -> u64 {
    let x = rnd.state[0];
    let y = rnd.state[1];
    rnd.state[0] = y;
    let mut x = x;
    x ^= x << 23;
    x ^= x >> 17;
    x ^= y ^ (y >> 26);
    rnd.state[1] = x;
    x.wrapping_add(y)
}

pub fn next_double(rnd: &mut Rnd) -> f64 {
    let value = rnd_next(rnd);
    let exponent = 1023u64;
    let mantissa = value >> 12;
    let result = (exponent << 52) | mantissa;
    f64::from_bits(result) - 1.0
}
