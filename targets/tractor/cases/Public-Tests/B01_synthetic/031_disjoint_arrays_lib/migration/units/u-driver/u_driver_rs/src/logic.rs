pub fn fma_array(out: &mut [i32], mul1: &[i32], mul2: &[i32], add: &[i32]) {
    for i in 0..out.len() {
        out[i] = mul1[i].wrapping_mul(mul2[i]).wrapping_add(add[i]);
    }
}

pub fn call_fma_impl(data: &[i32]) -> i32 {
    if data.is_empty() {
        return 0;
    }

    let len = data.len();
    let mut out = vec![0i32; len];
    let zeros = vec![0i32; len];
    let ones = vec![1i32; len];

    out[0] = 0;

    fma_array(&mut out, &ones, data, &zeros);

    out[len - 1]
}

fn parse_integers_bytes(bytes: &[u8]) -> Vec<i32> {
    let mut result = Vec::new();
    let mut i = 0;

    for _ in 0..100 {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }

        if i >= bytes.len() {
            break;
        }

        let mut num_str = String::new();
        if bytes[i] as char == '-' || bytes[i] as char == '+' {
            num_str.push(bytes[i] as char);
            i += 1;
        }

        let digit_start = num_str.len();
        while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
            num_str.push(bytes[i] as char);
            i += 1;
        }

        if num_str.len() == digit_start {
            break;
        }

        if let Ok(num) = num_str.parse::<i32>() {
            result.push(num);
        } else {
            break;
        }
    }

    result
}

pub fn driver_impl(bytes: &[u8]) {
    let data = parse_integers_bytes(bytes);
    let result = call_fma_impl(&data);
    crate::ffi::print_int(result);
}
