use crate::logic::encode_quant as encode_quant_logic;

#[no_mangle]
pub unsafe extern "C" fn encode_quant(
    uni: i32,
    step: i32,
    pred: i32,
    tgt: i32,
    tgt2: i32,
    lsbit: i32,
) -> i32 {
    encode_quant_logic(uni, step, pred, tgt, tgt2, lsbit)
}
