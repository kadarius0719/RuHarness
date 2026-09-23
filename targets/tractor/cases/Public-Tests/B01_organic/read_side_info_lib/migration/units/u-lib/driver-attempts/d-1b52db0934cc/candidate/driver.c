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

#define BUF_BYTES 96
#define GR_MAX 4

static uint64_t g_rng_state = 0x1F2E3D4C5B6A7988ULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static int g_case = 0;

static uint32_t sfbtab_hash(const uint8_t *tab, int count) {
    uint32_t acc = 2166136261u;
    int j;
    for (j = 0; j < count; j++) {
        acc = (acc ^ (uint32_t)tab[j]) * 16777619u;
    }
    return acc;
}

static void print_gr(const L3_gr_info_t *gr) {
    int k;
    int sfbcount;
    uint32_t sfbhash;

    printf(" part23=%" PRIu16 " big=%" PRIu16 " scfcomp=%" PRIu16
           " gain=%" PRIu8 " btype=%" PRIu8 " mixed=%" PRIu8
           " nlong=%" PRIu8 " nshort=%" PRIu8,
           gr->part_23_length, gr->big_values, gr->scalefac_compress,
           gr->global_gain, gr->block_type, gr->mixed_block_flag,
           gr->n_long_sfb, gr->n_short_sfb);
    printf(" tsel=[");
    for (k = 0; k < 3; k++) {
        if (k != 0) {
            printf(",");
        }
        printf("%" PRIu8, gr->table_select[k]);
    }
    printf("] rcnt=[");
    for (k = 0; k < 3; k++) {
        if (k != 0) {
            printf(",");
        }
        printf("%" PRIu8, gr->region_count[k]);
    }
    printf("] sbg=[");
    for (k = 0; k < 3; k++) {
        if (k != 0) {
            printf(",");
        }
        printf("%" PRIu8, gr->subblock_gain[k]);
    }
    printf("] preflag=%" PRIu8 " sfscale=%" PRIu8 " c1tab=%" PRIu8
           " scfsi=%" PRIu8,
           gr->preflag, gr->scalefac_scale, gr->count1_table, gr->scfsi);

    /* gr->sfbtab points into one of the unit's static const scale-factor
       band tables (g_scf_long / g_scf_short / g_scf_mixed); it is only
       ever assigned together with n_long_sfb/n_short_sfb, so whenever it
       is non-NULL those two fields correctly bound how many entries of
       the pointed-to row are valid to read. Folding those entries into a
       hash makes any single mutated table constant change this line,
       without ever printing (or being sensitive to) the pointer's own
       address. */
    if (gr->sfbtab != NULL) {
        sfbcount = (int)gr->n_long_sfb + (int)gr->n_short_sfb;
        sfbhash = sfbtab_hash(gr->sfbtab, sfbcount);
        printf(" sfbn=%d sfbhash=%08" PRIx32, sfbcount, sfbhash);
    } else {
        printf(" sfbn=0 sfbhash=00000000");
    }
}

static void run_case(uint8_t hdr1, uint8_t hdr2, uint8_t hdr3, int pos_start,
                      int limit_bits) {
    static uint8_t data[BUF_BYTES];
    uint8_t hdr[4];
    bs_t bs;
    L3_gr_info_t gr[GR_MAX];
    int ret, i;

    for (i = 0; i < BUF_BYTES; i++) {
        data[i] = (uint8_t)(xorshift64() & 0xFFu);
    }

    hdr[0] = 0xFF;
    hdr[1] = hdr1;
    hdr[2] = hdr2;
    hdr[3] = hdr3;

    bs.buf = data;
    bs.pos = pos_start;
    bs.limit = limit_bits;

    memset(gr, 0, sizeof(gr));

    ret = read_side_info(&bs, gr, hdr);

    printf("case %d hdr1=%" PRIu8 " hdr2=%" PRIu8 " hdr3=%" PRIu8
           " pos_start=%d limit=%d ret=%d bs_pos=%d\n",
           g_case, hdr1, hdr2, hdr3, pos_start, limit_bits, ret, bs.pos);
    for (i = 0; i < GR_MAX; i++) {
        printf("case %d gr%d:", g_case, i);
        print_gr(&gr[i]);
        printf("\n");
    }
    g_case++;
}

int main(void) {
    static const uint8_t hdr1_vals[] = { 0x00, 0x08 };
    static const uint8_t hdr2_vals[] = { 0x00, 0x04, 0x0C, 0xFC };
    static const uint8_t hdr3_vals[] = { 0x00, 0x40, 0xC0 };
    static const int small_limits[] = { 0, 8, 16, 32, 64, 100 };
    size_t n1 = sizeof(hdr1_vals) / sizeof(hdr1_vals[0]);
    size_t n2 = sizeof(hdr2_vals) / sizeof(hdr2_vals[0]);
    size_t n3 = sizeof(hdr3_vals) / sizeof(hdr3_vals[0]);
    size_t nl = sizeof(small_limits) / sizeof(small_limits[0]);
    size_t i, j, k, r, li;

    for (i = 0; i < n1; i++) {
        for (j = 0; j < n2; j++) {
            for (k = 0; k < n3; k++) {
                for (r = 0; r < 3; r++) {
                    run_case(hdr1_vals[i], hdr2_vals[j], hdr3_vals[k], 0,
                             512);
                }
            }
        }
    }

    for (r = 0; r < 8; r++) {
        run_case(0x08, 0x0C, 0xC0, (int)r, 512);
    }

    for (li = 0; li < nl; li++) {
        run_case(0x08, 0x0C, 0xC0, 0, small_limits[li]);
        run_case(0x00, 0x04, 0x00, 0, small_limits[li]);
        run_case(0x08, 0xFC, 0x40, 3, small_limits[li]);
    }

    /* Targeted headers that reach every sample-rate-index row (sr_idx
       0..7) of g_scf_long / g_scf_short / g_scf_mixed. sr_idx is derived
       from hdr[1] bits 3-4 and hdr[2] bits 2-3 only, so these eight
       (hdr1,hdr2) pairs were picked by solving that formula for each
       sr_idx value while staying inside the table's 8 rows (avoiding the
       hdr1=0x18,hdr2>>2&3==3 combination, which the unit itself does not
       guard and would index sr_idx==8, one past the table). Each is
       paired with a mono and a stereo hdr3 and repeated with fresh random
       payload bytes so the window-switching/block-type/mixed bits (which
       are read from the bitstream, not the header) end up exercising the
       long-block and short/mixed-block table paths for every row; the
       sfbhash printed by print_gr then exposes any mutated table
       constant reached this way. */
    {
        static const uint8_t sr_hdr1[8] = {
            0x00, 0x00, 0x08, 0x08, 0x08, 0x08, 0x18, 0x18
        };
        static const uint8_t sr_hdr2[8] = {
            0x00, 0x08, 0x00, 0x04, 0x08, 0x0C, 0x04, 0x08
        };
        static const uint8_t sr_hdr3[2] = { 0x00, 0xC0 };
        int si, hi;
        size_t rr;

        for (si = 0; si < 8; si++) {
            for (hi = 0; hi < 2; hi++) {
                for (rr = 0; rr < 8; rr++) {
                    run_case(sr_hdr1[si], sr_hdr2[si], sr_hdr3[hi], 0, 512);
                }
            }
        }
    }

    printf("total_cases=%d\n", g_case);
    return 0;
}
