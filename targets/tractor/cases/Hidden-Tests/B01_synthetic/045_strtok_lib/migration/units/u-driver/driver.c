#include <stdio.h>
#include <stdint.h>
#include <stddef.h>

#include "driver.h"

static uint32_t g_rng_state = 0x9E3779B1u;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

static void fill_random_tokens(char *buf, size_t cap) {
    static const char alnum[] =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    static const char seps[] = ":/\n";
    size_t len;
    size_t i;
    if (cap == 0) {
        return;
    }
    len = (size_t)(xorshift32() % cap);
    for (i = 0; i < len; i++) {
        uint32_t r = xorshift32();
        if ((r % 5u) == 0u) {
            buf[i] = seps[(r / 5u) % 3u];
        } else {
            buf[i] = alnum[r % (sizeof(alnum) - 1)];
        }
    }
    buf[len] = '\0';
}

static void run_case(int case_no, char *buf) {
    printf("case %d input=\"%s\"\n", case_no, buf);
    driver(buf);
    printf("case %d end\n", case_no);
}

int main(void) {
    int case_no = 0;
    int i;

    {
        char b0[] = "";
        run_case(case_no, b0); case_no++;
    }
    {
        char b1[] = ":::";
        run_case(case_no, b1); case_no++;
    }
    {
        char b2[] = "/\n:";
        run_case(case_no, b2); case_no++;
    }
    {
        char b3[] = "hello";
        run_case(case_no, b3); case_no++;
    }
    {
        char b4[] = "a:b/c\nd";
        run_case(case_no, b4); case_no++;
    }
    {
        char b5[] = ":/a:b/\n";
        run_case(case_no, b5); case_no++;
    }
    {
        char b6[] = "a::b//c\n\nd";
        run_case(case_no, b6); case_no++;
    }
    {
        char b7[] = "hello world:foo bar";
        run_case(case_no, b7); case_no++;
    }
    {
        char b8[] = "a";
        run_case(case_no, b8); case_no++;
    }
    {
        char b9[] = ":";
        run_case(case_no, b9); case_no++;
    }
    {
        char b10[] = "\n";
        run_case(case_no, b10); case_no++;
    }
    {
        char b11[] = "/";
        run_case(case_no, b11); case_no++;
    }
    {
        char b12[] = "::/\n::/\n";
        run_case(case_no, b12); case_no++;
    }
    {
        char b13[] = "one:two:three:four:five:six:seven:eight:nine:ten";
        run_case(case_no, b13); case_no++;
    }

    /* pseudo-random buffers of varied capacity, mixing alphanumeric
       content with the unit's three separator characters, to exercise
       many different token counts, run lengths and boundary placements */
    for (i = 0; i < 10; i++) {
        char buf[8];
        fill_random_tokens(buf, sizeof(buf));
        run_case(case_no, buf); case_no++;
    }
    for (i = 0; i < 15; i++) {
        char buf[64];
        fill_random_tokens(buf, sizeof(buf));
        run_case(case_no, buf); case_no++;
    }
    for (i = 0; i < 10; i++) {
        char buf[256];
        fill_random_tokens(buf, sizeof(buf));
        run_case(case_no, buf); case_no++;
    }
    for (i = 0; i < 3; i++) {
        char buf[2048];
        fill_random_tokens(buf, sizeof(buf));
        run_case(case_no, buf); case_no++;
    }

    return 0;
}
