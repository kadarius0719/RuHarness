pub fn encode_quant(uni: i32, step: i32, pred: i32, tgt: i32, tgt2: i32, lsbit: i32) -> i32 {
    let mut uni = uni;
    let mut uni1 = uni + 1;
    let mut uni2 = uni - 1;

    if ((uni ^ uni1) & (!7)) != 0 {
        uni1 = uni;
    }
    if ((uni ^ uni2) & (!7)) != 0 {
        uni2 = uni;
    }

    if lsbit != 0 {
        if lsbit == 4 {
            uni &= !1;
            uni1 &= !1;
            uni2 &= !1;
            uni |= ((uni >> 1) & (uni >> 2)) & 1;
            uni1 |= ((uni1 >> 1) & (uni1 >> 2)) & 1;
            uni2 |= ((uni2 >> 1) & (uni2 >> 2)) & 1;
        } else if (lsbit & 1) != 0 {
            uni |= 1;
            uni1 |= 1;
            uni2 |= 1;
        } else {
            uni &= !1;
            uni1 &= !1;
            uni2 &= !1;
        }
    }

    let mut diff = ((2 * (uni & 7) + 1) * step) / 8;
    if (uni & 8) != 0 {
        diff = -diff;
    }
    let p0 = pred + diff;
    let mut d0 = tgt - p0;
    d0 = d0 ^ (d0 >> 31);

    diff = ((2 * (uni1 & 7) + 1) * step) / 8;
    if (uni1 & 8) != 0 {
        diff = -diff;
    }
    let p1 = pred + diff;
    let mut d1 = tgt - p1;
    d1 = d1 ^ (d1 >> 31);

    diff = ((2 * (uni2 & 7) + 1) * step) / 8;
    if (uni2 & 8) != 0 {
        diff = -diff;
    }
    let p2 = pred + diff;
    let mut d2 = tgt - p2;
    d2 = d2 ^ (d2 >> 31);

    let mut d3 = tgt2 - p0;
    d3 = d3 ^ (d3 >> 31);
    d0 += d3 >> 5;

    d3 = tgt2 - p1;
    d3 = d3 ^ (d3 >> 31);
    d1 += d3 >> 5;

    d3 = tgt2 - p2;
    d3 = d3 ^ (d3 >> 31);
    d2 += d3 >> 5;

    let mut result = uni;
    if d1 < d0 {
        result = uni1;
    }
    if d2 < d0 {
        result = uni2;
    }

    result
}
