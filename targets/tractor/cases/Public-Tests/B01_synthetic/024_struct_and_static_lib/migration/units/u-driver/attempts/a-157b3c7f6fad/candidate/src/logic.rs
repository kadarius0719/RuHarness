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
