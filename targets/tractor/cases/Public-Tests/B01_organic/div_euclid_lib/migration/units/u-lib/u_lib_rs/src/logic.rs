pub fn div_euclid(v1: i32, v2: i32) -> i32 {
    if v2 == 0 {
        return 0;
    }

    let (q, r) = if v1 >= 0 {
        if v2 >= 0 {
            (v1 / v2, v1 % v2)
        } else if v2 != -0x7fffffff - 1 {
            let q = -(v1 / (-v2));
            let r = v1 % (-v2);
            (q, r)
        } else {
            (0, v1)
        }
    } else if v1 != -0x7fffffff - 1 {
        if v2 >= 0 {
            let q = -((-v1) / v2);
            let r = -((-v1) % v2);
            (q, r)
        } else if v2 != -0x7fffffff - 1 {
            let q = ((-v1) / (-v2));
            let r = -((-v1) % (-v2));
            (q, r)
        } else {
            let q = 1;
            let r = v1 - q * v2;
            (q, r)
        }
    } else if v2 >= 0 {
        let q = -((-((v1 + v2))) / v2) - 1;
        let r = -((-((v1 + v2))) % v2);
        (q, r)
    } else if v2 != -0x7fffffff - 1 {
        let q = ((-(v1 - v2)) / (-v2)) + 1;
        let r = -((-((v1 - v2))) % (-v2));
        (q, r)
    } else {
        (1, 0)
    };

    if r >= 0 {
        q
    } else {
        q + (if v2 > 0 { -1 } else { 1 })
    }
}
