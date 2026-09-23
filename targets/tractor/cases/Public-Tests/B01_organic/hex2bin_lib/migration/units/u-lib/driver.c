#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

static uint32_t rng_state = 0x87654321u;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static int g_case_id = 1;

static void run_case(const char *hex, size_t hex_len, const char *ignore,
                      size_t bin_maxlen, int use_end_p) {
    uint8_t *bin;
    const char *end = NULL;
    const char **end_p = use_end_p ? &end : NULL;
    int ret;
    size_t i;
    size_t alloc_n = bin_maxlen > 0 ? bin_maxlen : 1;

    bin = (uint8_t *)malloc(alloc_n);
    for (i = 0; i < alloc_n; ++i) {
        bin[i] = 0;
    }

    ret = hex2bin(bin, bin_maxlen, hex, hex_len, ignore, end_p);

    printf("case %d hex_len=%zu bin_maxlen=%zu use_end_p=%d ret=%d",
           g_case_id, hex_len, bin_maxlen, use_end_p, ret);
    if (use_end_p) {
        ptrdiff_t off = end - hex;
        printf(" end_off=%td", off);
    }
    printf("\n");

    if (ret > 0) {
        for (i = 0; i < (size_t)ret; ++i) {
            printf("case %d bin[%zu]=%" PRIu8 "\n", g_case_id, i, bin[i]);
        }
    }

    free(bin);
    g_case_id++;
}

int main(void) {
    char rbuf[64];
    int i, j, k;

    run_case("", 0, NULL, 4, 0);
    run_case("", 0, NULL, 0, 1);

    run_case("48656c6c6f", 10, NULL, 5, 0);
    run_case("48656c6c6f", 10, NULL, 5, 1);
    run_case("48656c6c6f", 10, NULL, 4, 0);
    run_case("48656c6c6f", 10, NULL, 4, 1);

    run_case("ab", 2, NULL, 1, 0);
    run_case("a", 1, NULL, 4, 0);
    run_case("a", 1, NULL, 4, 1);
    run_case("abc", 3, NULL, 4, 0);
    run_case("abc", 3, NULL, 4, 1);

    run_case("de:ad:be:ef", 11, ":", 4, 0);
    run_case("de:ad:be:ef", 11, ":", 4, 1);
    run_case("de:ad:be:ef", 11, NULL, 4, 0);
    run_case("de:ad:be:ef", 11, NULL, 4, 1);

    run_case("d:eadbeef", 9, ":", 4, 1);

    run_case("deXf", 4, NULL, 4, 0);
    run_case("deXf", 4, NULL, 4, 1);

    run_case("deadbeef", 4, NULL, 4, 1);

    run_case("ab", 2, NULL, 0, 0);
    run_case("ab", 2, NULL, 0, 1);

    run_case("12 34:56-78", 11, " :-", 4, 0);
    run_case("12 34:56-78", 11, " :-", 4, 1);
    run_case("12 34:56-78", 11, " :-", 3, 0);
    run_case("12 34:56-78", 11, " :-", 3, 1);

    run_case("FFEEDDCC", 8, NULL, 4, 0);
    run_case("00112233445566778899AABBCCDDEEFF", 32, NULL, 16, 1);

    for (i = 0; i < 10; ++i) {
        int nbytes = (int)(xorshift32() % 16u) + 1;
        int nchars = nbytes * 2;
        for (j = 0; j < nchars; ++j) {
            int nib = (int)(xorshift32() & 0xFu);
            rbuf[j] = "0123456789abcdef"[nib];
        }
        for (k = 0; k < 3; ++k) {
            size_t maxlen;
            if (k == 0) {
                maxlen = (size_t)nbytes;
            } else if (k == 1) {
                maxlen = (nbytes > 0) ? (size_t)(nbytes - 1) : (size_t)0;
            } else {
                maxlen = (size_t)nbytes + 3;
            }
            run_case(rbuf, (size_t)nchars, NULL, maxlen, (k % 2));
        }
    }

    return 0;
}
