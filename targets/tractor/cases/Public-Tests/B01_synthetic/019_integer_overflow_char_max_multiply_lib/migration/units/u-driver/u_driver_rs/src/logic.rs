pub fn printLine(line: &str) {
    crate::ffi::printf_string(line);
}

pub fn printHexCharLine(charHex: i8) {
    crate::ffi::printf_hex_byte(charHex);
}

pub fn bad() {
    let data: i8 = i8::MAX;
    if data > 0 {
        let result = data.wrapping_mul(2);
        printHexCharLine(result);
    }
}

fn goodG2B() {
    let data: i8 = 2;
    if data > 0 {
        let result = data.wrapping_mul(2);
        printHexCharLine(result);
    }
}

fn goodB2G() {
    let mut data: i8 = i8::MAX;
    if data > 0 {
        if data < (i8::MAX / 2) {
            let result = data.wrapping_mul(2);
            printHexCharLine(result);
        } else {
            printLine("data value is too large to perform arithmetic safely.");
        }
    }
}

pub fn good() {
    goodG2B();
    goodB2G();
}

pub fn driver(useGood: i32) {
    if useGood != 0 {
        good();
    } else {
        bad();
    }
}
