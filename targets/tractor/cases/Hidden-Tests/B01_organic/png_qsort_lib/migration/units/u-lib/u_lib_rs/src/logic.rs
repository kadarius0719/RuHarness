#[derive(Clone, Copy)]
#[repr(C)]
pub struct CpV2i {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct CpIntegerImage {
    pub img_index: i32,
    pub size: CpV2i,
    pub min: CpV2i,
    pub max: CpV2i,
    pub fit: i32,
}

fn cp_perimeter_pred(a: &CpIntegerImage, b: &CpIntegerImage) -> bool {
    let perimeter_a = 2 * (a.size.x + a.size.y);
    let perimeter_b = 2 * (b.size.x + b.size.y);
    perimeter_b < perimeter_a
}

pub fn qsort(items: &mut [CpIntegerImage]) {
    if items.len() <= 1 {
        return;
    }

    let count = items.len();
    let pivot = items[count - 1];
    let mut low = 0;

    for i in 0..(count - 1) {
        if cp_perimeter_pred(&items[i], &pivot) {
            let tmp = items[i];
            items[i] = items[low];
            items[low] = tmp;
            low += 1;
        }
    }

    items[count - 1] = items[low];
    items[low] = pivot;

    if low > 0 {
        qsort(&mut items[0..low]);
    }
    if low + 1 < count {
        qsort(&mut items[low + 1..count]);
    }
}
