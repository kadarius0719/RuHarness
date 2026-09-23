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

static float g_grbuf[4096];
static uint8_t g_bitbuf[4096];
static uint32_t xr_state;
static int g_case = 0;

static uint32_t xr32(void) {
    uint32_t x = xr_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    xr_state = x;
    return x;
}

/* Mirrors the unit's own address walk (grbuf + group_size*j, then += choff,
   choff = 18 - choff, persisting across j) purely to know which output
   floats were actually written, so only real observable output is printed. */
static void dump_touched(const char *tag, const float *grbuf, int group_size,
                          int total_bands, const uint8_t *bitalloc) {
    int choff = 576;
    for (int j = 0; j < 4; j++) {
        int pos = group_size * j;
        for (int i = 0; i < 2 * total_bands; i++) {
            if (bitalloc[i] != 0) {
                for (int k = 0; k < group_size; k++) {
                    g_case++;
                    printf("case %d %s j=%d i=%d k=%d idx=%d val=%a\n",
                           g_case, tag, j, i, k, pos + k, (double)grbuf[pos + k]);
                }
            }
            pos += choff;
            choff = 18 - choff;
        }
    }
}

static void run(const char *tag, int group_size, int total_bands,
                 const uint8_t *bitalloc_src, uint32_t bits_limit) {
    L12_scale_info sci;
    memset(&sci, 0, sizeof(sci));
    sci.total_bands = (uint8_t)total_bands;
    for (int i = 0; i < 2 * total_bands && i < 64; i++) {
        sci.bitalloc[i] = bitalloc_src[i];
    }

    memset(g_grbuf, 0, sizeof(g_grbuf));

    bs_t bs;
    bs.buf = g_bitbuf;
    bs.pos = 0;
    bs.limit = (int)bits_limit;

    int ret = dequantize_granule(g_grbuf, &bs, &sci, group_size);
    g_case++;
    printf("case %d %s ret=%d group_size=%d total_bands=%d bs_pos=%d\n",
           g_case, tag, ret, group_size, total_bands, bs.pos);
    dump_touched(tag, g_grbuf, group_size, total_bands, sci.bitalloc);
}

int main(void) {
    xr_state = 0x9E3779B1u;

    for (size_t i = 0; i < sizeof(g_bitbuf); i++) {
        g_bitbuf[i] = (uint8_t)(xr32() & 0xFFu);
    }

    /* Case 1: total_bands = 0 is a no-op (no reads, no writes). */
    {
        uint8_t ba[64];
        memset(ba, 0, sizeof(ba));
        run("no_bands", 5, 0, ba, 32000);
    }

    /* Case 2: bitalloc all zero -> ba==0 branch only, still walks addresses. */
    {
        uint8_t ba[2] = {0, 0};
        run("all_skip", 1, 1, ba, 32000);
    }

    /* Case 3: small case, low-bit-count (<17) branch only, group_size spans several k. */
    {
        uint8_t ba[2] = {9, 3};
        run("low_branch", 4, 1, ba, 32000);
    }

    /* Case 4: mix of skip / low-branch / high-branch (>=17). */
    {
        uint8_t ba[4] = {17, 20, 0, 16};
        run("mixed_small", 2, 2, ba, 32000);
    }

    /* Case 5: wider spread across all three branch kinds. */
    {
        uint8_t ba[8] = {1, 16, 17, 21, 0, 8, 19, 2};
        run("mixed_wide", 1, 4, ba, 32000);
    }

    /* Case 6: total_bands at its bitalloc[64]-imposed safe maximum (32), high branch only. */
    {
        uint8_t ba[64];
        for (int i = 0; i < 64; i++) {
            ba[i] = 17;
        }
        run("max_bands_high", 1, 32, ba, 32000);
    }

    /* Case 7: max total_bands again, alternating skip/low branch, larger group_size. */
    {
        uint8_t ba[64];
        for (int i = 0; i < 64; i++) {
            ba[i] = (uint8_t)((i % 2 == 0) ? 0 : 16);
        }
        run("max_bands_alt", 3, 32, ba, 32000);
    }

    /* Case 8: deliberately small bit limit so get_bits() hits its truncated-read
       (returns 0 without reading past the limit) path partway through. */
    {
        uint8_t ba[16];
        for (int i = 0; i < 16; i++) {
            static const uint8_t pool[] = {16, 20, 1, 17, 8, 21, 4, 2};
            ba[i] = pool[i % 8];
        }
        run("truncated_limit", 6, 8, ba, 40);
    }

    /* Case 9: group_size = 0 suppresses all writes (and all reads in the
       high-branch path too, since its k-loop also becomes empty), but the
       address walk and bitalloc dispatch still execute. */
    {
        uint8_t ba[8] = {1, 16, 17, 21, 0, 8, 19, 2};
        run("zero_group_size", 0, 4, ba, 32000);
    }

    /* Case 10: minimal total_bands with a large group_size. */
    {
        uint8_t ba[2] = {16, 1};
        run("min_bands_max_group", 12, 1, ba, 32000);
    }

    /* Cases 11-13: pseudo-random parameter combinations within the verified-safe
       envelope (total_bands <= 32, group_size <= 12, bitalloc drawn from the
       set of values known not to trigger undefined shifts in the unit). */
    {
        static const uint8_t safe_ba[] = {0, 1, 2, 4, 8, 16, 17, 18, 19, 20, 21};
        size_t nsafe = sizeof(safe_ba) / sizeof(safe_ba[0]);
        static const int tb_choices[] = {1, 2, 4, 8, 16, 32};
        static const int gs_choices[] = {0, 1, 3, 6, 12};
        for (int r = 0; r < 3; r++) {
            int total_bands = tb_choices[xr32() % (sizeof(tb_choices) / sizeof(tb_choices[0]))];
            int group_size = gs_choices[xr32() % (sizeof(gs_choices) / sizeof(gs_choices[0]))];
            uint8_t ba[64];
            for (int i = 0; i < 2 * total_bands && i < 64; i++) {
                ba[i] = safe_ba[xr32() % nsafe];
            }
            run("random", group_size, total_bands, ba, 32000);
        }
    }

    return 0;
}
