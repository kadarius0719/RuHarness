pub fn print_line(line: &str) {
    for byte in line.bytes() {
        crate::ffi::put_byte(byte);
    }
    crate::ffi::put_byte(b'\n');
}

fn helper_bad() {
    print_line("helperBad()");
}

pub fn bad() {
    print_line("bad()");
}

fn helper_good() {
    print_line("helperGood()");
}

pub fn good() {
    print_line("good()");
    helper_good();
}

pub fn driver() {
    print_line("Calling good()...");
    good();
    print_line("Finished good()");
    print_line("Calling bad()...");
    bad();
    print_line("Finished bad()");
}
