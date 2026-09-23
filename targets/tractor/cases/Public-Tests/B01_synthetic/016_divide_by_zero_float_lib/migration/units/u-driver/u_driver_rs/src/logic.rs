pub fn printLine(line: &str) {
    crate::ffi::printf_string(line);
}

pub fn printIntLine(intNumber: i32) {
    crate::ffi::printf_int(intNumber);
}

pub fn bad(data: f32) {
    let result = (100.0 / data) as i32;
    printIntLine(result);
}

fn goodG2B() {
    let data = 2.0f32;
    let result = (100.0 / data) as i32;
    printIntLine(result);
}

fn goodB2G(data: f32) {
    if data.abs() > 0.000001f32 {
        let result = (100.0 / data) as i32;
        printIntLine(result);
    } else {
        printLine("This would result in a divide by zero");
    }
}

pub fn good(data: f32) {
    goodG2B();
    goodB2G(data);
}

pub fn driver(goodData: f32, badData: f32) {
    printLine("Calling good()...");
    good(goodData);
    printLine("Finished good()");
    printLine("Calling bad()...");
    bad(badData);
    printLine("Finished bad()");
}
