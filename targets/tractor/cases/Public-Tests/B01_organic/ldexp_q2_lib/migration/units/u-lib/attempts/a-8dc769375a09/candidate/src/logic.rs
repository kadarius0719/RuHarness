pub fn ldexp_q2(y: f32, exp_q2: i32) -> f32 {
    const G_EXPFRAC: [f32; 4] = [
        9.31322575e-10,
        7.83145814e-10,
        6.58544508e-10,
        5.53767716e-10,
    ];

    let mut y = y;
    let mut exp_q2_remaining = exp_q2;

    loop {
        let e = if 30 * 4 > exp_q2_remaining {
            exp_q2_remaining
        } else {
            30 * 4
        };

        let shift_amount = (e >> 2) as u32;
        let power_of_two = ((1i32 << 30) >> shift_amount) as f32;

        y *= G_EXPFRAC[(e & 3) as usize] * power_of_two;

        exp_q2_remaining -= e;

        if exp_q2_remaining <= 0 {
            break;
        }
    }

    y
}
