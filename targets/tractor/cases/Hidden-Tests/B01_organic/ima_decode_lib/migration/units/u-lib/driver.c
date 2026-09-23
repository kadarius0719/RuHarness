#include "lib.h"
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>
#include <limits.h>
#include <float.h>
#include <math.h>
#include <stdbool.h>
#include <ctype.h>
#include <errno.h>

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static uint16_t host_bswap16(uint16_t v) {
    return (uint16_t)(((uint32_t)(v << 8) & 0xff00u) | (((uint32_t)v >> 8) & 0x00ffu));
}

static int case_id = 0;

static void run_case(const char *label, uint16_t sw_preamble,
                      const ima_u8_t *data32, unsigned channel_count,
                      ima_u64_t decode_count,
                      ima_s32_t init_index, ima_s32_t init_predict) {
    struct ima_block block;
    struct ima_channel_state state;
    ima_output_t output;
    int i;

    block.preamble = host_bswap16(sw_preamble);
    for (i = 0; i < 32; i++) block.data[i] = data32[i];
    state.index = init_index;
    state.predict = init_predict;
    output = 0.0f;

    ima_decode(&output, channel_count, &block, decode_count, &state);

    printf("case %d %s sw=%04x channel_count=%u decode_count=%" PRIu64
           " init_index=%" PRId32 " init_predict=%" PRId32
           " out=%a final_index=%" PRId32 " final_predict=%" PRId32 "\n",
           case_id++, label, (unsigned)sw_preamble, channel_count,
           (uint64_t)decode_count, init_index, init_predict,
           (double)output, state.index, state.predict);
}

int main(void) {
    uint32_t rng = 314159265u;
    ima_u8_t data_zero[32];
    ima_u8_t data_max[32];
    ima_u8_t data_ramp[32];
    ima_u8_t data_alt[32];
    ima_u8_t data_rand[32];
    int i, r;

    memset(data_zero, 0x00, sizeof data_zero);
    memset(data_max, 0xFF, sizeof data_max);
    for (i = 0; i < 32; i++) data_ramp[i] = (ima_u8_t)((i * 7) & 0xFF);
    for (i = 0; i < 32; i++) data_alt[i] = (ima_u8_t)((i & 1) ? 0xAA : 0x55);

    run_case("zero_data_idx0", 0x0000, data_zero, 1, 0, 0, 0);
    run_case("zero_data_idx0_dc1", 0x0000, data_zero, 1, 1, 0, 0);
    run_case("zero_data_idx0_dc2", 0x0000, data_zero, 1, 2, 0, 0);
    run_case("zero_data_idx0_dc63", 0x0000, data_zero, 1, 63, 0, 0);
    run_case("zero_data_idx0_dc64", 0x0000, data_zero, 1, 64, 0, 0);
    run_case("max_idx0_dc64", 0x0000, data_max, 1, 64, 0, 0);
    run_case("max_idx88_dc64", 0x0058, data_max, 1, 64, 88, 0);
    run_case("ramp_idx44_dc64", 0x2C, data_ramp, 2, 64, 44, 0);
    run_case("alt_idx44_dc31", 0x2C, data_alt, 3, 31, 44, 0);
    run_case("alt_idx44_dc7", 0x2C, data_alt, 3, 7, 44, 0);

    /* predict from preamble negative, index 0 */
    run_case("predict_neg_idx0", 0x8000, data_ramp, 1, 64, 0, 0);
    /* predict from preamble large positive, index 0 */
    run_case("predict_pos_idx0", 0x7F80, data_ramp, 1, 64, 0, 0);
    /* predict encoded with index 44 and positive high bits */
    run_case("predict_pos_idx44", 0x552C, data_ramp, 1, 64, 44, 0);
    /* predict encoded with index 44 and negative high bits */
    run_case("predict_neg_idx44", 0xAA2C, data_ramp, 1, 64, 44, 0);

    /* state.index equals computed index, diff small (<=0x7f): predict overridden */
    run_case("same_index_small_diff", 0x2C, data_ramp, 1, 64, 44, 10);
    /* state.index equals computed index, diff large (>0x7f): predict kept as decoded */
    run_case("same_index_large_diff", 0x552C, data_ramp, 1, 64, 44, -30000);
    /* state.index differs from computed index: branch skipped */
    run_case("diff_index", 0x2C, data_ramp, 1, 64, 10, 500);

    /* channel_count zero and large */
    run_case("channel_zero", 0x0000, data_ramp, 0, 64, 0, 0);
    run_case("channel_large", 0x0000, data_ramp, 4000000000u, 8, 0, 0);

    /* odd decode counts at various sizes */
    run_case("dc1_max", 0x0000, data_max, 1, 1, 0, 0);
    run_case("dc3_max", 0x0000, data_max, 1, 3, 0, 0);
    run_case("dc9_max", 0x0000, data_max, 1, 9, 0, 0);

    /* initial predict at extremes to exercise clamp */
    run_case("predict_clamp_hi", 0x0000, data_max, 1, 64, 0, 32760);
    run_case("predict_clamp_lo", 0x0000, data_max, 1, 64, 0, -32760);

    for (r = 0; r < 24; r++) {
        for (i = 0; i < 32; i++) data_rand[i] = (ima_u8_t)(xorshift32(&rng) & 0xFFu);
        uint16_t idx = (uint16_t)(xorshift32(&rng) % 89u);
        uint16_t hi = (uint16_t)(xorshift32(&rng) & 0x1FFu);
        uint16_t sw = (uint16_t)((hi << 7) | idx);
        unsigned ch = xorshift32(&rng) % 5u;
        ima_u64_t dc = (ima_u64_t)(xorshift32(&rng) % 65u);
        ima_s32_t si = (ima_s32_t)(xorshift32(&rng) % 89u);
        ima_s32_t sp = (ima_s32_t)((int32_t)(xorshift32(&rng) % 65536u) - 32768);
        run_case("random", sw, data_rand, ch, dc, si, sp);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
