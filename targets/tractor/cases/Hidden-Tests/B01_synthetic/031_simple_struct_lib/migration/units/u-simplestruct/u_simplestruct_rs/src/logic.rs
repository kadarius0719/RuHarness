#[repr(C)]
#[derive(Copy, Clone)]
pub struct Date {
    pub month: i32,
    pub day: i32,
    pub year: i32,
}

pub fn is_it_may(date: Date) -> bool {
    date.month == 5
}

pub fn what_year_is_it(date: Date) -> i32 {
    date.year
}
