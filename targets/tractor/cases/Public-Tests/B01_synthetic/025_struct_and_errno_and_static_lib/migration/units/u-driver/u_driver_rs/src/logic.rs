#[repr(C)]
pub struct HouseT {
    pub floors: i32,
    pub bedrooms: i32,
    pub bathrooms: f64,
}

pub fn add_floor(house: &mut HouseT) {
    house.floors += 1;
}

pub fn add_bedrooms(house: &mut HouseT, extra_bedrooms: i32) {
    house.bedrooms += extra_bedrooms;
}

pub fn print_house(house: &HouseT) {
    crate::ffi::printf_house(house.floors, house.bedrooms, house.bathrooms);
}

pub fn run_logic(house: &mut HouseT, extra_bedrooms: i32) {
    print_house(house);
    add_floor(house);
    print_house(house);
    house.bathrooms += 1.0;
    print_house(house);
    add_bedrooms(house, extra_bedrooms);
    print_house(house);
}

pub fn parse_val(s: &str) -> Option<i32> {
    let s = s.trim_start();

    let bytes = s.as_bytes();
    let mut i = 0;

    if i < bytes.len() && (bytes[i] as char == '+' || bytes[i] as char == '-') {
        i += 1;
    }

    let start = i;
    while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
        i += 1;
    }

    if i == start {
        return None;
    }

    s[..i].parse::<i32>().ok()
}
