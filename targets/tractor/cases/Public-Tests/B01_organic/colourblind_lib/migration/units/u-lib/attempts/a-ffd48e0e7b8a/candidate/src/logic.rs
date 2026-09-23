#[repr(C)]
pub enum CbImpairment {
    CbProtanopia = 0,
    CbDeuteranopia = 1,
    CbTritanopia = 2,
}

fn protanopia(r: &mut f32, g: &mut f32, b: &mut f32) {
    let red = *r;
    let green = *g;
    let blue = *b;
    *r = 0.17055699213417f32 * red + 0.82944301379913f32 * green + 2.91188E-9f32 * blue;
    *g = 0.17055699092998f32 * red + 0.82944300785005f32 * green - 5.98679E-10f32 * blue;
    *b = -0.00451714424166f32 * red + 0.00451714427397f32 * green + blue;
}

fn deuteranopia(r: &mut f32, g: &mut f32, b: &mut f32) {
    let red = *r;
    let green = *g;
    let blue = *b;
    *r = 0.33066007266046f32 * red + 0.66933992517563f32 * green + 3.559314E-9f32 * blue;
    *g = 0.33066007387760f32 * red + 0.66933992719147f32 * green - 1.758327E-9f32 * blue;
    *b = -0.02785538261323f32 * red + 0.02785538252318f32 * green + blue;
}

fn tritanopia(r: &mut f32, g: &mut f32, b: &mut f32) {
    let red = *r;
    let green = *g;
    let blue = *b;
    *r = red + 0.12739886310880f32 * green - 0.12739886341072f32 * blue;
    *g = -4.486E-11f32 * red + 0.87390929928361f32 * green + 0.12609070101523f32 * blue;
    *b = 3.1113E-10f32 * red + 0.87390929725848f32 * green + 0.12609070067115f32 * blue;
}

pub fn colourblind(impairment: CbImpairment, r: &mut f32, g: &mut f32, b: &mut f32) {
    match impairment {
        CbImpairment::CbProtanopia => protanopia(r, g, b),
        CbImpairment::CbDeuteranopia => deuteranopia(r, g, b),
        CbImpairment::CbTritanopia => tritanopia(r, g, b),
    }
}
