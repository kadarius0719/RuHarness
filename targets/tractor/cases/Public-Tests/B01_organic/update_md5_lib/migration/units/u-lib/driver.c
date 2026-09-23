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

/* lib.h only declares update_md5(); tflac_pack_u64le and
   tflac_md5_addsample are defined with external linkage in lib.c but
   are not exposed by the header, so the driver must declare their
   prototypes itself using the unit's own fixed-width typedefs. */
void tflac_pack_u64le(tflac_u8 *d, tflac_u64 n);
void tflac_md5_addsample(tflac_md5 *m, tflac_u32 bits, tflac_u64 val);

static uint64_t xs64(uint64_t *state) {
    uint64_t x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    return x;
}

static int32_t rand_i32(uint64_t *state) {
    return (int32_t)(xs64(state) & 0xFFFFFFFFu);
}

static void print_hex_bytes(const tflac_u8 *buf, size_t len) {
    size_t i;
    for (i = 0; i < len; i++) {
        printf("%02" PRIx8, buf[i]);
    }
}

static void run_pack(int *case_no, tflac_u64 n) {
    tflac_u8 d[16];
    memset(d, 0, sizeof(d));
    tflac_pack_u64le(d, n);
    printf("case %d pack n=%" PRIu64 " bytes=", *case_no, n);
    print_hex_bytes(d, 8);
    printf("\n");
    (*case_no)++;
}

static void run_addsample_sequence(int *case_no, const tflac_u32 *bits_seq,
                                    size_t bits_seq_len, uint64_t seed) {
    tflac_md5 m;
    uint64_t state = seed;
    size_t i;
    memset(&m, 0, sizeof(m));
    for (i = 0; i < bits_seq_len; i++) {
        tflac_u64 val = xs64(&state);
        tflac_md5_addsample(&m, bits_seq[i], val);
        printf("case %d addsample bits=%" PRIu32 " val=%" PRIu64
               " pos=%" PRIu32 " total=%" PRIu64 " buf=",
               *case_no, bits_seq[i], val, m.pos, m.total);
        print_hex_bytes(m.buffer, sizeof(m.buffer));
        printf("\n");
        (*case_no)++;
    }
}

static void run_update_md5(int *case_no, tflac_u32 cur_blocksize,
                            tflac_u32 channels, const tflac_s32 *samples) {
    tflac t;
    tflac_u32 ret;
    memset(&t, 0, sizeof(t));
    t.cur_blocksize = cur_blocksize;
    t.channels = channels;
    ret = update_md5(&t, samples);
    printf("case %d update_md5 bs=%" PRIu32 " ch=%" PRIu32 " ret=%" PRIu32
           " pos=%" PRIu32 " total=%" PRIu64 " buf=",
           *case_no, cur_blocksize, channels, ret, t.md5_ctx.pos,
           t.md5_ctx.total);
    print_hex_bytes(t.md5_ctx.buffer, sizeof(t.md5_ctx.buffer));
    printf("\n");
    (*case_no)++;
}

int main(void) {
    int case_no = 0;

    static const tflac_u64 pack_values[] = {
        0ULL,
        1ULL,
        0xFFULL,
        0x0102030405060708ULL,
        0xFFFFFFFF00000000ULL,
        0x00000000FFFFFFFFULL,
        0x8000000000000001ULL,
        0x8000000000000000ULL,
        0x7FFFFFFFFFFFFFFFULL,
        0xFFFFFFFFFFFFFFFFULL,
        0xAAAAAAAAAAAAAAAAULL,
        0x5555555555555555ULL,
        1ULL << 8,
        1ULL << 16,
        1ULL << 24,
        1ULL << 32,
        1ULL << 40,
        1ULL << 48,
        1ULL << 56,
        1ULL << 63,
    };
    size_t n_pack = sizeof(pack_values) / sizeof(pack_values[0]);
    size_t i;
    for (i = 0; i < n_pack; i++) {
        run_pack(&case_no, pack_values[i]);
    }
    {
        uint64_t state = 88172645463325252ULL;
        for (i = 0; i < 60; i++) {
            run_pack(&case_no, xs64(&state));
        }
    }

    {
        static const tflac_u32 bits_seq_a[] = {
            64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64};
        static const tflac_u32 bits_seq_b[] = {
            8,  16, 24, 32, 40, 48, 56, 64, 8,  16, 24, 32,
            40, 48, 56, 64, 8,  16, 24, 32, 40, 48, 56, 64};
        static const tflac_u32 bits_seq_c[] = {
            0, 5, 13, 64, 0, 5, 13, 64, 64, 64, 64, 64, 1, 2, 3, 4,
            64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64};
        static const tflac_u32 bits_seq_d[] = {
            64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
            64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
            64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
            64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64};
        run_addsample_sequence(&case_no, bits_seq_a,
                                sizeof(bits_seq_a) / sizeof(bits_seq_a[0]),
                                111111111ULL);
        run_addsample_sequence(&case_no, bits_seq_b,
                                sizeof(bits_seq_b) / sizeof(bits_seq_b[0]),
                                222222222ULL);
        run_addsample_sequence(&case_no, bits_seq_c,
                                sizeof(bits_seq_c) / sizeof(bits_seq_c[0]),
                                333333333ULL);
        run_addsample_sequence(&case_no, bits_seq_d,
                                sizeof(bits_seq_d) / sizeof(bits_seq_d[0]),
                                444444444ULL);
    }

    {
#define SAMPLES_LEN 160
        tflac_s32 samples_zero[SAMPLES_LEN];
        tflac_s32 samples_max[SAMPLES_LEN];
        tflac_s32 samples_min[SAMPLES_LEN];
        tflac_s32 samples_alt[SAMPLES_LEN];
        tflac_s32 samples_rand[SAMPLES_LEN];
        uint64_t state = 999999999ULL;
        int k;
        for (k = 0; k < SAMPLES_LEN; k++) {
            samples_zero[k] = 0;
            samples_max[k] = INT32_MAX;
            samples_min[k] = INT32_MIN;
            samples_alt[k] = (k % 2 == 0) ? 127 : -128;
            samples_rand[k] = rand_i32(&state);
        }

        run_update_md5(&case_no, 4096, 2, samples_zero);
        run_update_md5(&case_no, 0, 0, samples_zero);
        run_update_md5(&case_no, 4294967295u, 4294967295u, samples_zero);
        run_update_md5(&case_no, 1, 1, samples_zero);
        run_update_md5(&case_no, 192, 6, samples_zero);

        run_update_md5(&case_no, 4096, 2, samples_max);
        run_update_md5(&case_no, 4096, 2, samples_min);
        run_update_md5(&case_no, 4096, 2, samples_alt);

        for (k = 0; k < 30; k++) {
            run_update_md5(&case_no, (tflac_u32)(1000 + k),
                            (tflac_u32)(1 + (k % 8)), samples_rand);
            for (int j = 0; j < SAMPLES_LEN; j++) {
                samples_rand[j] = rand_i32(&state);
            }
        }
#undef SAMPLES_LEN
    }

    return 0;
}
