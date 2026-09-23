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

void stb__CompressAlphaBlock(unsigned char *dest, unsigned char *src, int stride);

static uint32_t xorshift32(uint32_t *state) {
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

static void print_bytes(const char *label, int case_id, const unsigned char *buf, size_t n) {
    printf("case %d %s", case_id, label);
    for (size_t i = 0; i < n; i++) {
        printf(" %02x", (unsigned)buf[i]);
    }
    printf("\n");
}

int main(void) {
    unsigned char src32[32];
    unsigned char dest16[16];
    unsigned char src_big[64];
    unsigned char dest8[8];
    uint32_t rng = 2463534242u;
    int case_id = 0;
    int i, r;

    memset(src32, 0, sizeof src32);
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_all_zero", case_id++, dest16, 16);

    memset(src32, 0xFF, sizeof src32);
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_all_max", case_id++, dest16, 16);

    for (i = 0; i < 32; i++) src32[i] = (unsigned char)i;
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_ascending", case_id++, dest16, 16);

    for (i = 0; i < 32; i++) src32[i] = (unsigned char)(31 - i);
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_descending", case_id++, dest16, 16);

    for (i = 0; i < 32; i++) src32[i] = (unsigned char)((i & 1) ? 0xFF : 0x00);
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_alt_01", case_id++, dest16, 16);

    for (i = 0; i < 32; i++) src32[i] = (unsigned char)((i & 1) ? 0x00 : 0xFF);
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_alt_10", case_id++, dest16, 16);

    memset(src32, 128, sizeof src32);
    src32[0] = 0;
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_outlier_low", case_id++, dest16, 16);

    memset(src32, 128, sizeof src32);
    src32[2] = 255;
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_outlier_high", case_id++, dest16, 16);

    for (i = 0; i < 32; i++) src32[i] = (unsigned char)(10 + (i % 5));
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_small_range", case_id++, dest16, 16);

    for (i = 0; i < 32; i++) src32[i] = (unsigned char)(50 + (i % 20));
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_mid_range", case_id++, dest16, 16);

    for (i = 0; i < 32; i++) src32[i] = 200;
    src32[31] = 201;
    compress_bc5(dest16, src32);
    print_bytes("compress_bc5_dist1", case_id++, dest16, 16);

    for (r = 0; r < 24; r++) {
        for (i = 0; i < 32; i++) src32[i] = (unsigned char)(xorshift32(&rng) & 0xFFu);
        compress_bc5(dest16, src32);
        print_bytes("compress_bc5_random", case_id++, dest16, 16);
    }

    memset(src_big, 0, sizeof src_big);
    stb__CompressAlphaBlock(dest8, src_big, 1);
    print_bytes("alpha_block_zero_stride1", case_id++, dest8, 8);

    memset(src_big, 0xFF, sizeof src_big);
    stb__CompressAlphaBlock(dest8, src_big, 1);
    print_bytes("alpha_block_max_stride1", case_id++, dest8, 8);

    for (i = 0; i < 64; i++) src_big[i] = (unsigned char)((i * 4) % 256);
    stb__CompressAlphaBlock(dest8, src_big, 1);
    print_bytes("alpha_block_ramp_stride1", case_id++, dest8, 8);

    for (i = 0; i < 64; i++) src_big[i] = (unsigned char)(((64 - i) * 4) % 256);
    stb__CompressAlphaBlock(dest8, src_big, 2);
    print_bytes("alpha_block_ramp_stride2", case_id++, dest8, 8);

    for (i = 0; i < 64; i++) src_big[i] = (unsigned char)(xorshift32(&rng) & 0xFFu);
    stb__CompressAlphaBlock(dest8, src_big, 3);
    print_bytes("alpha_block_random_stride3", case_id++, dest8, 8);

    for (i = 0; i < 64; i++) src_big[i] = (unsigned char)(xorshift32(&rng) & 0xFFu);
    stb__CompressAlphaBlock(dest8, src_big, 4);
    print_bytes("alpha_block_random_stride4", case_id++, dest8, 8);

    memset(src_big, 0, sizeof src_big);
    src_big[0] = 77;
    stb__CompressAlphaBlock(dest8, src_big, 0);
    print_bytes("alpha_block_stride0", case_id++, dest8, 8);

    for (r = 0; r < 12; r++) {
        for (i = 0; i < 64; i++) src_big[i] = (unsigned char)(xorshift32(&rng) & 0xFFu);
        stb__CompressAlphaBlock(dest8, src_big, 4);
        print_bytes("alpha_block_random4", case_id++, dest8, 8);
    }

    for (r = 0; r < 8; r++) {
        for (i = 0; i < 64; i++) src_big[i] = (unsigned char)(xorshift32(&rng) & 0xFFu);
        stb__CompressAlphaBlock(dest8, src_big, 2);
        print_bytes("alpha_block_random2", case_id++, dest8, 8);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
