#include "lib.h"

typedef signed short ima_s16_t;

static ima_u16_t ima_bswap16(ima_u16_t v) {
    return (v << 0x08 & 0xff00u) | (v >> 0x08 & 0x00ffu);
}

static ima_u16_t ima_btoh16(ima_u16_t v) { return ima_bswap16(v); }

static const int ima_index_table[16] = {-1, -1, -1, -1, 2, 4, 6, 8,
                                        -1, -1, -1, -1, 2, 4, 6, 8};

static const int ima_step_table[89] = {
    7,     8,     9,     10,    11,    12,    13,    14,    16,    17,
    19,    21,    23,    25,    28,    31,    34,    37,    41,    45,
    50,    55,    60,    66,    73,    80,    88,    97,    107,   118,
    130,   143,   157,   173,   190,   209,   230,   253,   279,   307,
    337,   371,   408,   449,   494,   544,   598,   658,   724,   796,
    876,   963,   1060,  1166,  1282,  1411,  1552,  1707,  1878,  2066,
    2272,  2499,  2749,  3024,  3327,  3660,  4026,  4428,  4871,  5358,
    5894,  6484,  7132,  7845,  8630,  9493,  10442, 11487, 12635, 13899,
    15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767};

static int ima_clamp_index(int index) {
    if (!!(index < 0))
        index = 0;
    else if (!!(index > 88))
        index = 88;
    return index;
}

static int ima_clamp_predict(int predict) {
    if (!!(predict < -32768))
        predict = -32768;
    else if (!!(predict > 32767))
        predict = 32767;
    return predict;
}

void ima_decode(ima_output_t *output, unsigned channel_count,
                      const struct ima_block *block, ima_u64_t decode_count,
                      struct ima_channel_state *state) {
    int index, predict, step, diff, nibble;
    ima_u64_t i;
    index = ima_btoh16(block->preamble) & 0x7f;
    predict = (ima_s16_t)ima_btoh16(block->preamble) & ~0x7f;
    if (index == state->index) {
        if ((diff = predict - state->predict) < 0)
            diff = -diff;
        if (diff <= 0x7f)
            predict = state->predict;
    }
    step = ima_step_table[index];
    for (i = 0; i < (decode_count >> 1); ++i) {
        do {
            nibble = (block->data[i] & 0xf);
            index = ima_clamp_index(index + ima_index_table[nibble]);
            diff = step >> 3;
            if (nibble & 4)
                diff += step;
            if (nibble & 2)
                diff += step >> 1;
            if (nibble & 1)
                diff += step >> 2;
            if (nibble & 8)
                predict -= diff;
            else
                predict += diff;
            step = ima_step_table[index];
            predict = ima_clamp_predict(predict);
            *output += ((ima_f32_t)(predict) * 0.0000305185f);
            *output += channel_count;
        } while (0);
        do {
            nibble = (block->data[i] >> 4);
            index = ima_clamp_index(index + ima_index_table[nibble]);
            diff = step >> 3;
            if (nibble & 4)
                diff += step;
            if (nibble & 2)
                diff += step >> 1;
            if (nibble & 1)
                diff += step >> 2;
            if (nibble & 8)
                predict -= diff;
            else
                predict += diff;
            step = ima_step_table[index];
            predict = ima_clamp_predict(predict);
            *output += ((ima_f32_t)(predict) * 0.0000305185f);
            *output += channel_count;
        } while (0);
    }
    if (!!((decode_count & 1) != 0))
        do {
            nibble = (block->data[decode_count >> 1] & 0xf);
            index = ima_clamp_index(index + ima_index_table[nibble]);
            diff = step >> 3;
            if (nibble & 4)
                diff += step;
            if (nibble & 2)
                diff += step >> 1;
            if (nibble & 1)
                diff += step >> 2;
            if (nibble & 8)
                predict -= diff;
            else
                predict += diff;
            step = ima_step_table[index];
            predict = ima_clamp_predict(predict);
            *output += ((ima_f32_t)(predict) * 0.0000305185f);
            *output += channel_count;
        } while (0);
    state->index = index;
    state->predict = predict;
}
