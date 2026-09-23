#include "driver.h"

#include <stdio.h>
#include <stdbool.h>
#include <limits.h>

/* foo_t and print_foo() are defined in the unit's own .c file, not
   exposed through driver.h, so this driver reconstructs the identical
   struct shape (same member order, types and bit-field widths) to call
   print_foo() directly, matching the [ABI CONTRACT] signature. */
typedef struct {
    unsigned int x : 2;
    unsigned int y : 3;
    bool b : 1;
    int z;
} foo_t;

extern void print_foo(const foo_t *foo);

static void call_driver(int case_num, unsigned int x, unsigned int y, bool b, int z) {
    printf("case %d driver(x=%u, y=%u, b=%d, z=%d)\n", case_num, x, y, (int)b, z);
    driver(x, y, b, z);
}

static void call_print_foo(int case_num, unsigned int x, unsigned int y, bool b, int z) {
    foo_t foo = {.x = x, .y = y, .b = b, .z = z};
    printf("case %d print_foo(x=%u, y=%u, b=%d, z=%d)\n", case_num, x, y, (int)b, z);
    print_foo(&foo);
}

int main(void) {
    int case_num = 1;

    /* driver() stores x into a 2-bit unsigned bit-field and y into a
       3-bit unsigned bit-field; assigning an out-of-range unsigned
       value to an unsigned bit-field is well-defined (it keeps the
       value modulo 2^width), so any unsigned int is safe to pass. Cover
       the exact bit-field range, the wrap-around just past it, and
       large/extreme unsigned magnitudes, crossed with both bool values
       and boundary int values for z (which is a plain, non-truncated
       member). */
    call_driver(case_num++, 0u, 0u, false, 0);
    call_driver(case_num++, 1u, 1u, true, 1);
    call_driver(case_num++, 2u, 2u, false, -1);
    call_driver(case_num++, 3u, 3u, true, INT_MAX);
    call_driver(case_num++, 3u, 7u, false, INT_MIN);
    call_driver(case_num++, 4u, 8u, true, 0);
    call_driver(case_num++, 5u, 9u, false, 5);
    call_driver(case_num++, 7u, 15u, true, -5);
    call_driver(case_num++, 8u, 16u, false, 100);
    call_driver(case_num++, 100u, 255u, true, -100);
    call_driver(case_num++, 255u, 256u, false, 12345);
    call_driver(case_num++, UINT_MAX, UINT_MAX, true, INT_MAX);
    call_driver(case_num++, UINT_MAX - 1u, UINT_MAX - 1u, false, INT_MIN);
    call_driver(case_num++, 0x80000000u, 0x80000000u, true, 0);
    call_driver(case_num++, 0x7FFFFFFFu, 0x7FFFFFFFu, false, -1);
    call_driver(case_num++, 2u, 5u, true, 7);
    call_driver(case_num++, 1u, 6u, false, -7);
    call_driver(case_num++, 3u, 4u, true, 42);
    call_driver(case_num++, 0u, 7u, false, -42);
    call_driver(case_num++, 6u, 2u, true, 1000000);
    call_driver(case_num++, 9u, 10u, false, -1000000);

    /* Direct print_foo() calls, exercising the same bit-field range and
       wrap-around behavior independently of driver(). */
    call_print_foo(case_num++, 0u, 0u, true, 0);
    call_print_foo(case_num++, 1u, 1u, false, -1);
    call_print_foo(case_num++, 2u, 2u, true, 1);
    call_print_foo(case_num++, 3u, 3u, false, INT_MAX);
    call_print_foo(case_num++, 3u, 7u, true, INT_MIN);
    call_print_foo(case_num++, 4u, 8u, false, 0);
    call_print_foo(case_num++, 5u, 9u, true, 5);
    call_print_foo(case_num++, 7u, 15u, false, -5);
    call_print_foo(case_num++, 8u, 16u, true, 100);
    call_print_foo(case_num++, 100u, 255u, false, -100);
    call_print_foo(case_num++, 255u, 256u, true, 12345);
    call_print_foo(case_num++, UINT_MAX, UINT_MAX, false, INT_MAX);
    call_print_foo(case_num++, UINT_MAX - 1u, UINT_MAX - 1u, true, INT_MIN);
    call_print_foo(case_num++, 0x80000000u, 0x80000000u, false, 0);
    call_print_foo(case_num++, 0x7FFFFFFFu, 0x7FFFFFFFu, true, -1);

    return 0;
}
