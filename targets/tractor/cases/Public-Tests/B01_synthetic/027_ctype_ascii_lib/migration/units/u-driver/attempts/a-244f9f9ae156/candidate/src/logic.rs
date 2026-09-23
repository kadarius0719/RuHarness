pub struct ClassificationResults {
    pub alnum: i32,
    pub alpha: i32,
    pub lower: i32,
    pub upper: i32,
    pub digit: i32,
    pub xdigit: i32,
    pub cntrl: i32,
    pub graph: i32,
    pub space: i32,
    pub blank: i32,
    pub print: i32,
    pub punct: i32,
    pub to_lower: i8,
    pub to_upper: i8,
}

pub fn classify_char(c: i8) -> ClassificationResults {
    let byte = c as u8;
    let c_char = byte as char;

    let is_cntrl = byte < 32 || byte == 127;
    let is_space = byte == 9 || byte == 10 || byte == 11 || byte == 12 || byte == 13 || byte == 32;
    let is_blank = byte == 9 || byte == 32;
    let is_print = byte >= 32 && byte <= 126;
    let is_graph = is_print && !is_space;

    ClassificationResults {
        alnum: if byte >= 48 && byte <= 57 || byte >= 65 && byte <= 90 || byte >= 97 && byte <= 122 { 1 } else { 0 },
        alpha: if byte >= 65 && byte <= 90 || byte >= 97 && byte <= 122 { 1 } else { 0 },
        lower: if byte >= 97 && byte <= 122 { 1 } else { 0 },
        upper: if byte >= 65 && byte <= 90 { 1 } else { 0 },
        digit: if byte >= 48 && byte <= 57 { 1 } else { 0 },
        xdigit: if byte >= 48 && byte <= 57 || byte >= 65 && byte <= 70 || byte >= 97 && byte <= 102 { 1 } else { 0 },
        cntrl: if is_cntrl { 1 } else { 0 },
        graph: if is_graph { 1 } else { 0 },
        space: if is_space { 1 } else { 0 },
        blank: if is_blank { 1 } else { 0 },
        print: if is_print { 1 } else { 0 },
        punct: if is_print && !is_space && !(byte >= 48 && byte <= 57) && !(byte >= 65 && byte <= 90) && !(byte >= 97 && byte <= 122) { 1 } else { 0 },
        to_lower: if byte >= 65 && byte <= 90 { (byte + 32) as i8 } else { c },
        to_upper: if byte >= 97 && byte <= 122 { (byte - 32) as i8 } else { c },
    }
}
