use crate::logic;

#[no_mangle]
pub extern "C" fn isItMay(date: logic::Date) -> bool {
    logic::is_it_may(date)
}

#[no_mangle]
pub extern "C" fn whatYearIsIt(date: logic::Date) -> i32 {
    logic::what_year_is_it(date)
}
