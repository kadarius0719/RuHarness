//! Safe logic for parsing a CAF/IMA4 container header, mirroring the
//! original C `ima_parse` byte-for-byte, including the C compiler's
//! struct padding (relied on by the original pointer arithmetic) and
//! its exact, non-obvious handling of the audio-description sample
//! rate field.

/// Mirrors `struct ima_block` from the C ABI. Only used to type the
/// `blocks` pointer stored in `ImaInfo`; this unit never reads through
/// it, so its own layout does not affect correctness.
#[repr(C)]
pub struct ImaBlock {
    pub preamble: u16,
    pub data: [u8; 32],
}

/// Mirrors `struct ima_info` from the C ABI exactly (field order and
/// types), so `src/ffi.rs` can reinterpret the caller's raw
/// `struct ima_info *` as a `&mut ImaInfo`.
#[repr(C)]
pub struct ImaInfo {
    pub blocks: *const ImaBlock,
    pub size: u64,
    pub sample_rate: f64,
    pub frame_count: u64,
    pub channel_count: u32,
}

// Sizes of the CAF structures exactly as the C compiler lays them out
// (with its usual alignment padding), since the original code walks
// the buffer via pointer arithmetic on those struct types.
const CAF_HEADER_SIZE: usize = 8; // { u32 type; u16 version; u16 flags; }
const CAF_CHUNK_HEADER_SIZE: usize = 16; // { u32 type; <4 pad>; i64 size; }
const CAF_DATA_SIZE: usize = 4; // { u32 edit_count; }

fn tag(bytes: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*bytes)
}

fn read_be_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([buf[off], buf[off + 1]])
}

fn read_be_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn read_be_u64(buf: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&buf[off..off + 8]);
    u64::from_be_bytes(a)
}

/// Reads the 8 bytes at `off` as a host-native (little-endian) `f64`,
/// exactly like the C code's direct `desc->sample_rate` struct field
/// access (no endian swap is applied to that particular field).
fn read_native_f64(buf: &[u8], off: usize) -> f64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&buf[off..off + 8]);
    f64::from_le_bytes(a)
}

/// Parses the CAF/IMA4 container in `data` and, on success, fills
/// `info` exactly like the C `ima_parse`. Returns the same error codes
/// the C code returns (-1, -2, -3, or 0 on success); `info` is left
/// untouched on every error path, matching the C original, which never
/// writes through `info` before those `return`s.
pub fn ima_parse(info: &mut ImaInfo, data: &[u8]) -> i32 {
    let header_type = read_be_u32(data, 0);
    if header_type != tag(b"caff") {
        return -1;
    }
    let header_version = read_be_u16(data, 4);
    if header_version != 1 {
        return -2;
    }

    let mut chunk_offset: usize = CAF_HEADER_SIZE;
    let mut desc_offset: usize = 0;
    let mut pakt_offset: usize = 0;

    let blocks_offset: usize;
    let data_chunk_size: u64;

    loop {
        let chunk_type = read_be_u32(data, chunk_offset);
        let chunk_size = read_be_u64(data, chunk_offset + 8);

        if chunk_type == tag(b"desc") {
            desc_offset = chunk_offset + CAF_CHUNK_HEADER_SIZE;
        } else if chunk_type == tag(b"pakt") {
            pakt_offset = chunk_offset + CAF_CHUNK_HEADER_SIZE;
        } else if chunk_type == tag(b"data") {
            blocks_offset = chunk_offset + CAF_CHUNK_HEADER_SIZE + CAF_DATA_SIZE;
            data_chunk_size = chunk_size;
            break;
        }

        let advance = (CAF_CHUNK_HEADER_SIZE as u64).wrapping_add(chunk_size);
        chunk_offset = (chunk_offset as u64).wrapping_add(advance) as usize;
    }

    let format_id = read_be_u32(data, desc_offset + 8);
    if format_id != tag(b"ima4") {
        return -3;
    }

    let frame_count = read_be_u64(data, pakt_offset + 8);
    let channel_count = read_be_u32(data, desc_offset + 24);

    // Replicates the original's exact (non-bit-reinterpreting) sequence:
    // read the raw sample_rate bytes as a native-endian double, convert
    // that numeric value to u64 (C's double-to-integer conversion,
    // truncating toward zero for in-range values), byte-swap the u64,
    // then reinterpret the swapped bits as a double.
    let raw_sample_rate = read_native_f64(data, desc_offset);
    let as_u64 = raw_sample_rate as u64;
    let swapped = as_u64.swap_bytes();
    let sample_rate = f64::from_bits(swapped);

    info.blocks = data.as_ptr().wrapping_add(blocks_offset) as *const ImaBlock;
    info.size = data_chunk_size;
    info.sample_rate = sample_rate;
    info.frame_count = frame_count;
    info.channel_count = channel_count;

    0
}
