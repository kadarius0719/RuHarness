typedef unsigned char ima_u8_t;
typedef unsigned short ima_u16_t;
typedef unsigned long long ima_u64_t;
typedef signed int ima_s32_t;
typedef float ima_f32_t;
typedef ima_f32_t ima_output_t;

struct ima_channel_state {
    ima_s32_t index;
    ima_s32_t predict;
};

struct ima_block {
    ima_u16_t preamble;
    ima_u8_t data[(32)];
};

void ima_decode(ima_output_t *output, unsigned channel_count,
                             const struct ima_block *block,
                             ima_u64_t decode_count,
                             struct ima_channel_state *state);
