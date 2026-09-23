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

static uint32_t xr_state;

static uint32_t xr32(void) {
    uint32_t x = xr_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    xr_state = x;
    return x;
}

static int g_case = 0;

static void run_case(const uint8_t *bin, size_t bin_len, size_t extra_slack) {
    char hexbuf[600];
    size_t hex_maxlen = bin_len * 2U + 1U + extra_slack;
    if (hex_maxlen > sizeof(hexbuf)) {
        hex_maxlen = sizeof(hexbuf);
    }
    memset(hexbuf, 'Z', sizeof(hexbuf));
    char *ret = bin2hex(hexbuf, hex_maxlen, bin, bin_len);
    g_case++;
    printf("case %d bin_len=%" PRIu64 " hex_maxlen=%" PRIu64 " ret_eq_hex=%d out=%s\n",
           g_case, (uint64_t)bin_len, (uint64_t)hex_maxlen,
           (ret == hexbuf) ? 1 : 0, hexbuf);
}

int main(void) {
    xr_state = 0x9E3779B9u;

    /* empty buffer */
    {
        uint8_t bin0[1];
        bin0[0] = 0;
        run_case(bin0, 0, 0);
    }

    /* single byte, min and max */
    {
        uint8_t b0[1] = {0x00};
        run_case(b0, 1, 0);
    }
    {
        uint8_t bff[1] = {0xFF};
        run_case(bff, 1, 0);
    }

    /* every byte value individually, exercising every nibble mapping */
    for (int v = 0; v < 256; v++) {
        uint8_t b[1];
        b[0] = (uint8_t)v;
        run_case(b, 1, 0);
    }

    /* two-byte combos covering low/high nibble boundaries */
    {
        uint8_t bin2[2] = {0x0F, 0xF0};
        run_case(bin2, 2, 0);
    }
    {
        uint8_t bin2[2] = {0x9A, 0x39};
        run_case(bin2, 2, 0);
    }

    /* sequential 16 bytes */
    {
        uint8_t bin16[16];
        for (int i = 0; i < 16; i++) {
            bin16[i] = (uint8_t)(i * 17);
        }
        run_case(bin16, 16, 0);
    }

    /* 32 bytes from xorshift PRNG */
    {
        uint8_t bin32[32];
        for (int i = 0; i < 32; i++) {
            bin32[i] = (uint8_t)(xr32() & 0xFFu);
        }
        run_case(bin32, 32, 0);
    }

    /* full 256-byte sweep in one call */
    {
        uint8_t bin256[256];
        for (int i = 0; i < 256; i++) {
            bin256[i] = (uint8_t)i;
        }
        run_case(bin256, 256, 0);
    }

    /* extra slack in hex_maxlen beyond the minimum required */
    {
        uint8_t bin4[4] = {0x12, 0x34, 0xAB, 0xCD};
        run_case(bin4, 4, 10);
    }

    /* more xorshift-driven random vectors of varying length */
    for (int c = 0; c < 10; c++) {
        uint8_t binr[20];
        size_t len = (size_t)(1u + (xr32() % 20u));
        for (size_t i = 0; i < len; i++) {
            binr[i] = (uint8_t)(xr32() & 0xFFu);
        }
        run_case(binr, len, 0);
    }

    return 0;
}
