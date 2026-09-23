#[derive(Clone, Copy)]
#[repr(C)]
pub struct Sprite {
    pub texture_id: u64,
    pub sort_bits: i32,
}

fn sprite_less_than_or_equal(a: &Sprite, b: &Sprite) -> bool {
    if a.sort_bits <= b.sort_bits {
        return true;
    }
    if a.sort_bits == b.sort_bits && a.texture_id <= b.texture_id {
        return true;
    }
    false
}

fn merge_sort_iteration(src: &[Sprite], lo: usize, split: usize, hi: usize, dest: &mut [Sprite]) {
    let mut i = lo;
    let mut j = split;

    for k in lo..hi {
        if i < split && (j >= hi || sprite_less_than_or_equal(&src[i], &src[j])) {
            dest[k] = src[i];
            i += 1;
        } else {
            dest[k] = src[j];
            j += 1;
        }
    }
}

fn merge_sort_recurse(src: &mut [Sprite], lo: usize, hi: usize, dest: &mut [Sprite]) {
    if hi - lo <= 1 {
        return;
    }

    let split = (lo + hi) / 2;
    merge_sort_recurse(dest, lo, split, src);
    merge_sort_recurse(dest, split, hi, src);
    merge_sort_iteration(src, lo, split, hi, dest);
}

pub fn merge_sort(a: &mut [Sprite], b: &mut [Sprite]) {
    if a.len() != b.len() {
        return;
    }

    let size = a.len();
    b.copy_from_slice(a);
    merge_sort_recurse(b, 0, size, a);
}
