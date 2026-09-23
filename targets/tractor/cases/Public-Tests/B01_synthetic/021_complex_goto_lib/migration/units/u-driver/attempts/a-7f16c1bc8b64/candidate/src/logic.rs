pub fn driver(mut x: i32, mut y: i32) {
    while x > 0 || y > 0 {
        print_str("loop\n");

        let mut skip_x_check = x == 1 && y == 4;

        loop {
            if !skip_x_check && x > 0 {
                print_str("x\n");
                x -= 1;
            }

            if y == 0 {
                break;
            }

            print_str("y\n");
            y -= 1;

            if x < 3 {
                skip_x_check = false;
                continue;
            } else {
                break;
            }
        }
    }
}

fn print_str(s: &str) {
    for byte in s.bytes() {
        crate::ffi::put_byte(byte);
    }
}
