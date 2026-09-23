pub fn max_size_frame(blocksize: u32, channels: u32, bitdepth: u32) -> u32 {
    let sample_size_numerator = blocksize.wrapping_mul(bitdepth).wrapping_mul(channels.wrapping_mul((channels != 2) as u32)) +
                                blocksize.wrapping_mul(bitdepth).wrapping_mul((channels == 2) as u32) +
                                blocksize.wrapping_mul(bitdepth.wrapping_add((bitdepth != 32) as u32)).wrapping_mul((channels == 2) as u32) +
                                7;

    18u32.wrapping_add(channels).wrapping_add(sample_size_numerator / 8)
}
