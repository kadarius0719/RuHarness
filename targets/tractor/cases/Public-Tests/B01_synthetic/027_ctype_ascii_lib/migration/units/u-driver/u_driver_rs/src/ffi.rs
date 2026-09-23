use crate::logic::{classify_char, ClassificationResults};

extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_results(results: &ClassificationResults) {
    unsafe {
        let fmt = b"alphanumeric: %d\n\0".as_ptr();
        printf(fmt, results.alnum);

        let fmt = b"alphabetic: %d\n\0".as_ptr();
        printf(fmt, results.alpha);

        let fmt = b"lowercase: %d\n\0".as_ptr();
        printf(fmt, results.lower);

        let fmt = b"uppercase: %d\n\0".as_ptr();
        printf(fmt, results.upper);

        let fmt = b"digit: %d\n\0".as_ptr();
        printf(fmt, results.digit);

        let fmt = b"hexadecimal: %d\n\0".as_ptr();
        printf(fmt, results.xdigit);

        let fmt = b"control: %d\n\0".as_ptr();
        printf(fmt, results.cntrl);

        let fmt = b"graphical: %d\n\0".as_ptr();
        printf(fmt, results.graph);

        let fmt = b"space: %d\n\0".as_ptr();
        printf(fmt, results.space);

        let fmt = b"blank: %d\n\0".as_ptr();
        printf(fmt, results.blank);

        let fmt = b"printing: %d\n\0".as_ptr();
        printf(fmt, results.print);

        let fmt = b"punctuation: %d\n\0".as_ptr();
        printf(fmt, results.punct);

        let fmt = b"to lower: %c\n\0".as_ptr();
        printf(fmt, results.to_lower as u8 as i32);

        let fmt = b"to upper: %c\n\0".as_ptr();
        printf(fmt, results.to_upper as u8 as i32);
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(c: i8) {
    let results = crate::logic::classify_char(c);
    printf_results(&results);
}
