/// Direct translation of `void driver(double f)`, whose C body is
/// `printf("%llx %a %.4f\n", u.x, f, f)` where `u.x` is `f`'s raw IEEE
/// 754 bit pattern (via a `{ uint64_t x; double f; }` union).
pub fn driver(f: f64) {
    let bits = f.to_bits();

    let mut line = String::new();
    write_hex_u64(&mut line, bits);
    line.push(' ');
    write_hex_float(&mut line, f);
    line.push(' ');
    write_fixed4(&mut line, f);
    line.push('\n');

    for b in line.as_bytes() {
        crate::ffi::put_byte(*b);
    }
}

/// Matches C's `%llx`: lowercase hex, no `0x` prefix, no padding.
fn write_hex_u64(out: &mut String, v: u64) {
    out.push_str(&format!("{:x}", v));
}

/// Matches C's `%a`: C99/glibc hexadecimal floating-point notation,
/// lowercase, with the shortest fraction that exactly represents the
/// value (trailing zero hex digits dropped, and the decimal point
/// dropped too when the fraction is empty). The sign is taken from the
/// IEEE sign bit for finite values and for infinities (so negative zero
/// and `-inf` still print a leading `-`), but NaN is always printed
/// unsigned ("nan", never "-nan"), matching glibc's observed behavior.
/// Subnormal values are normalized so their highest set mantissa bit
/// becomes the implicit leading one, with the exponent adjusted to
/// match (e.g. the smallest subnormal prints as `0x1p-1074`, not
/// `0x0.0000000000001p-1022`).
fn write_hex_float(out: &mut String, f: f64) {
    let bits = f.to_bits();
    let sign_negative = (bits >> 63) & 1 == 1;
    let exp_bits = (bits >> 52) & 0x7FF;
    let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;

    let is_nan = exp_bits == 0x7FF && mantissa != 0;

    if sign_negative && !is_nan {
        out.push('-');
    }

    if exp_bits == 0x7FF {
        out.push_str(if mantissa == 0 { "inf" } else { "nan" });
        return;
    }

    out.push_str("0x");

    if exp_bits == 0 && mantissa == 0 {
        out.push_str("0p+0");
        return;
    }

    let (lead, exponent, frac_mantissa): (u32, i64, u64) = if exp_bits == 0 {
        // Subnormal: normalize so the highest set mantissa bit becomes
        // the implicit leading 1, matching glibc's `%a` output.
        let p = 63 - mantissa.leading_zeros();
        let new_mantissa = (mantissa & ((1u64 << p) - 1)) << (52 - p);
        (1, (p as i64) - 1074, new_mantissa)
    } else {
        (1, exp_bits as i64 - 1023, mantissa)
    };
    out.push(std::char::from_digit(lead, 16).unwrap());

    let mut nibbles = [0u8; 13];
    let mut m = frac_mantissa;
    for i in (0..13).rev() {
        nibbles[i] = (m & 0xF) as u8;
        m >>= 4;
    }
    if let Some(last) = (0..13).rev().find(|&i| nibbles[i] != 0) {
        out.push('.');
        for &n in &nibbles[..=last] {
            out.push(std::char::from_digit(n as u32, 16).unwrap());
        }
    }

    out.push('p');
    out.push(if exponent < 0 { '-' } else { '+' });
    out.push_str(&exponent.abs().to_string());
}

/// Matches C's `%.4f`: fixed-point decimal with exactly 4 digits after
/// the point. The sign is taken from the IEEE sign bit for finite
/// values and for infinities (so `-0.0` still prints a leading `-`),
/// but NaN is always printed unsigned ("nan", never "-nan"), matching
/// glibc's observed behavior.
fn write_fixed4(out: &mut String, f: f64) {
    if f.is_sign_negative() && !f.is_nan() {
        out.push('-');
    }
    if f.is_nan() {
        out.push_str("nan");
    } else if f.is_infinite() {
        out.push_str("inf");
    } else {
        out.push_str(&format!("{:.4}", f.abs()));
    }
}
