#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <limits.h>

#include "echo.h"

static uint32_t g_rng_state = 0x9E3779B9u;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

/* Fill buf (capacity cap, including the terminating NUL) with a
   pseudo-random printable ASCII string of length in [0, cap-1),
   NUL-terminated. */
static void fill_random_string(char *buf, size_t cap) {
    size_t len = 0;
    size_t i;
    if (cap > 0) {
        len = (size_t)(xorshift32() % cap);
    }
    for (i = 0; i < len; i++) {
        uint32_t r = xorshift32();
        buf[i] = (char)(32 + (r % 95)); /* printable ASCII 32..126 */
    }
    if (cap > 0) {
        buf[len] = '\0';
    }
}

static char g_dummy_argv0[] = "prog";
static char g_hello[] = "hello";
static char g_empty[] = "";
static char g_fmt[] = "%s%d%%n";
static char g_normal[] = "normal";
static char g_long[1001];
static char g_special[] = "tab\tnewline\nend\\backslash\"quote";

#define N_MANY 16
#define N_MAX  100
#define BUF_CAP 24

static char g_many_bufs[N_MANY][BUF_CAP];
static char g_max_bufs[N_MAX][BUF_CAP];

int main(void) {
    int ret;

    /* case 0: argc == 0. argv must still be a valid pointer even though
       no element of it is ever dereferenced by the unit. */
    {
        char *argv0[1];
        argv0[0] = g_dummy_argv0;
        printf("case 0 argc=%d\n", 0);
        ret = echo(0, argv0);
        printf("case 0 ret=%d\n", ret);
    }

    /* case 1: argc == 1 (program name only); nothing printed by echo */
    {
        char *argv1[1];
        argv1[0] = g_dummy_argv0;
        printf("case 1 argc=%d\n", 1);
        ret = echo(1, argv1);
        printf("case 1 ret=%d\n", ret);
    }

    /* case 2: argc negative (-1) */
    {
        char *argv2[1];
        argv2[0] = g_dummy_argv0;
        printf("case 2 argc=%d\n", -1);
        ret = echo(-1, argv2);
        printf("case 2 ret=%d\n", ret);
    }

    /* case 3: argc == INT_MIN, minimum possible value */
    {
        char *argv3[1];
        argv3[0] = g_dummy_argv0;
        printf("case 3 argc=%d\n", INT_MIN);
        ret = echo(INT_MIN, argv3);
        printf("case 3 ret=%d\n", ret);
    }

    /* case 4: argc == 2, a single normal argument */
    {
        char *argv4[2];
        argv4[0] = g_dummy_argv0;
        argv4[1] = g_hello;
        printf("case 4 argc=%d\n", 2);
        ret = echo(2, argv4);
        printf("case 4 ret=%d\n", ret);
    }

    /* case 5: argc == 2, empty-string argument */
    {
        char *argv5[2];
        argv5[0] = g_dummy_argv0;
        argv5[1] = g_empty;
        printf("case 5 argc=%d\n", 2);
        ret = echo(2, argv5);
        printf("case 5 ret=%d\n", ret);
    }

    /* case 6: argc == 3, a format-specifier-looking argument followed by
       a normal one, to exercise that argv content is only ever printed
       via %s and never reinterpreted */
    {
        char *argv6[3];
        argv6[0] = g_dummy_argv0;
        argv6[1] = g_fmt;
        argv6[2] = g_normal;
        printf("case 6 argc=%d\n", 3);
        ret = echo(3, argv6);
        printf("case 6 ret=%d\n", ret);
    }

    /* case 7: argc == 2, a very long argument (1000 'A' characters) */
    {
        char *argv7[2];
        memset(g_long, 'A', sizeof(g_long) - 1);
        g_long[sizeof(g_long) - 1] = '\0';
        argv7[0] = g_dummy_argv0;
        argv7[1] = g_long;
        printf("case 7 argc=%d len=%d\n", 2, (int)strlen(g_long));
        ret = echo(2, argv7);
        printf("case 7 ret=%d\n", ret);
    }

    /* case 8: argc == 2, an argument containing embedded control and
       punctuation characters, to exercise byte-for-byte reproduction */
    {
        char *argv8[2];
        argv8[0] = g_dummy_argv0;
        argv8[1] = g_special;
        printf("case 8 argc=%d\n", 2);
        ret = echo(2, argv8);
        printf("case 8 ret=%d\n", ret);
    }

    /* case 9: many pseudo-random arguments of varying (including zero)
       length */
    {
        char *argv9[N_MANY];
        int i;
        argv9[0] = g_dummy_argv0;
        for (i = 1; i < N_MANY; i++) {
            fill_random_string(g_many_bufs[i], BUF_CAP);
            argv9[i] = g_many_bufs[i];
        }
        printf("case 9 argc=%d\n", N_MANY);
        ret = echo(N_MANY, argv9);
        printf("case 9 ret=%d\n", ret);
    }

    /* case 10: a larger number of pseudo-random arguments, exercising a
       bigger argument count while staying within the output budget */
    {
        char *argv10[N_MAX];
        int i;
        argv10[0] = g_dummy_argv0;
        for (i = 1; i < N_MAX; i++) {
            fill_random_string(g_max_bufs[i], BUF_CAP);
            argv10[i] = g_max_bufs[i];
        }
        printf("case 10 argc=%d\n", N_MAX);
        ret = echo(N_MAX, argv10);
        printf("case 10 ret=%d\n", ret);
    }

    /* case 11: another round of pseudo-random arguments, continuing the
       generator state, to call the unit "many times" with fresh data */
    {
        char *argv11[N_MANY];
        int i;
        argv11[0] = g_dummy_argv0;
        for (i = 1; i < N_MANY; i++) {
            fill_random_string(g_many_bufs[i], BUF_CAP);
            argv11[i] = g_many_bufs[i];
        }
        printf("case 11 argc=%d\n", N_MANY);
        ret = echo(N_MANY, argv11);
        printf("case 11 ret=%d\n", ret);
    }

    return 0;
}
