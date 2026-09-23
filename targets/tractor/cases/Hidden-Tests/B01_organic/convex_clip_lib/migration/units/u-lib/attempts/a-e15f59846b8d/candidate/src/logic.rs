#[derive(Clone, Copy)]
#[repr(C)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

fn v2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

fn sub2(a: Vec2, b: Vec2) -> Vec2 {
    v2(a.x - b.x, a.y - b.y)
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn left_of(a: Vec2, b: Vec2, c: Vec2) -> i32 {
    let x = cross2(sub2(b, a), sub2(c, b));
    if x < 0.0 {
        -1
    } else {
        if x > 0.0 {
            1
        } else {
            0
        }
    }
}

fn line_intersection(x0: Vec2, x1: Vec2, y0: Vec2, y1: Vec2) -> Option<Vec2> {
    let dx = sub2(x1, x0);
    let dy = sub2(y1, y0);
    let d = sub2(x0, y0);
    let dyx = cross2(dy, dx);
    if dyx == 0.0 {
        return None;
    }
    let t = cross2(d, dx) / dyx;
    if t <= 0.0 || t >= 1.0 {
        return None;
    }
    Some(v2(y0.x + t * dy.x, y0.y + t * dy.y))
}

pub fn convex_clip(poly: &mut [Vec2], initial_n_poly: usize, clip: &[Vec2], res: &mut [Vec2]) -> usize {
    let mut n_poly = initial_n_poly;
    let mut n_res = n_poly;
    let dir = left_of(clip[0], clip[1], clip[2]);

    let n_clip = clip.len();
    let mut j = n_clip - 1;

    for i in 0..n_clip {
        if n_res == 0 {
            break;
        }

        if i != 0 {
            for k in 0..n_res {
                poly[k] = res[k];
            }
            n_poly = n_res;
        }

        n_res = 0;
        if n_poly > 0 {
            let mut v0 = poly[n_poly - 1];
            let mut side0 = left_of(clip[j], clip[i], v0);

            if side0 != -dir {
                res[n_res] = v0;
                n_res += 1;
            }

            for k in 0..n_poly {
                let v1 = poly[k];
                let side1 = left_of(clip[j], clip[i], v1);

                if side0 + side1 == 0 && side0 != 0 {
                    if let Some(x) = line_intersection(clip[j], clip[i], v0, v1) {
                        res[n_res] = x;
                        n_res += 1;
                    }
                }

                if k == n_poly - 1 {
                    break;
                }

                if side1 != -dir {
                    res[n_res] = v1;
                    n_res += 1;
                }

                v0 = v1;
                side0 = side1;
            }
        }

        j = i;
    }

    n_res
}
