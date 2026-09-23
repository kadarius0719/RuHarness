#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <limits.h>

#include "driver.h"

/* bad, good, printHexCharLine and printLine have external linkage in the
   unit but are not declared in driver.h (only driver is); the ABI
   contract gives their exact C signatures, so we declare them here
   ourselves. These are plain declarations, never definitions, macros,
   or address-of uses of the unit's symbols. */
void bad(void);
void good(void);
void printHexCharLine(char charHex);
void printLine(const char *line);

static uint32_t g_rng_state = 0x13572468u;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

static void fill_random_string(char *buf, size_t cap) {
    size_t len = 0;
    size_t i;
    if (cap > 0) {
        len = (size_t)(xorshift32() % cap);
    }
    for (i = 0; i < len; i++) {
        uint32_t r = xorshift32();
        buf[i] = (char)(32 + (r % 95));
    }
    if (cap > 0) {
        buf[len] = '\0';
    }
}

int main(void) {
    int case_no = 0;
    int v;
    int i;

    /* ---- printHexCharLine: every possible char value. The function
       only formats and prints its argument (printf("%02x\n", charHex)),
       so no char value can ever be unsafe to pass. ---- */
    for (v = CHAR_MIN; v <= CHAR_MAX; v++) {
        char c = (char)v;
        printf("case %d printHexCharLine charHex=%d\n", case_no, (int)c);
        printHexCharLine(c);
        case_no++;
    }

    /* ---- printLine: NULL, the input the unit's own
       `if (line != NULL)` check shows is an accepted, safe argument ---- */
    printf("case %d printLine line=(null)\n", case_no);
    printLine(NULL);
    case_no++;

    /* ---- printLine: fixed non-NULL vectors ---- */
    {
        static char empty_str[] = "";
        static char hello_str[] = "hello, world";
        static char special_str[] = "tab\tnewline\nquote\"backslash\\end";
        static char long_str[513];
        char *fixed[4];
        size_t n;
        size_t k;

        for (k = 0; k < sizeof(long_str) - 1; k++) {
            long_str[k] = (char)('A' + (k % 26));
        }
        long_str[sizeof(long_str) - 1] = '\0';

        fixed[0] = empty_str;
        fixed[1] = hello_str;
        fixed[2] = special_str;
        fixed[3] = long_str;

        n = sizeof(fixed) / sizeof(fixed[0]);
        for (k = 0; k < n; k++) {
            printf("case %d printLine line=\"%s\"\n", case_no, fixed[k]);
            printLine(fixed[k]);
            case_no++;
        }
    }

    /* ---- printLine: pseudo-random printable strings ---- */
    for (i = 0; i < 10; i++) {
        char rand_buf[64];
        fill_random_string(rand_buf, sizeof(rand_buf));
        printf("case %d printLine line=\"%s\"\n", case_no, rand_buf);
        printLine(rand_buf);
        case_no++;
    }

    /* ---- bad: deterministic, parameter-less; the unit's own
       CHAR_MIN < 0 / multiply-and-truncate path is what is being
       pinned here ---- */
    for (i = 0; i < 3; i++) {
        printf("case %d bad\n", case_no);
        bad();
        printf("case %d bad_end\n", case_no);
        case_no++;
    }

    /* ---- good: deterministic, parameter-less ---- */
    for (i = 0; i < 3; i++) {
        printf("case %d good\n", case_no);
        good();
        printf("case %d good_end\n", case_no);
        case_no++;
    }

    /* ---- driver: every kind of truthy/falsy useGood value ---- */
    {
        int use_good_vals[] = { 0, 1, -1, 2, 100, INT_MIN, INT_MAX };
        size_t n = sizeof(use_good_vals) / sizeof(use_good_vals[0]);
        size_t k;
        for (k = 0; k < n; k++) {
            printf("case %d driver useGood=%d\n", case_no, use_good_vals[k]);
            driver(use_good_vals[k]);
            printf("case %d driver_end\n", case_no);
            case_no++;
        }
    }

    return 0;
}
