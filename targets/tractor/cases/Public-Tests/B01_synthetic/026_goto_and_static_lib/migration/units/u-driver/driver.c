#include "driver.h"

#include <stdio.h>
#include <limits.h>

static void call_driver(int case_num, int x, int local_y, int z) {
    printf("case %d driver(x=%d, local_y=%d, z=%d)\n", case_num, x, local_y, z);
    driver(x, local_y, z);
}

int main(void) {
    int case_num = 1;

    /* Success path: x==1, y==2 (after being set from local_y), z==3. */
    call_driver(case_num++, 1, 2, 3);

    /* Branch 1: x != 1 rejected before y or z are examined. Cover zero,
       one-off, negative and extreme values of x; local_y/z are irrelevant
       here since the function returns via goto before reading them for
       any comparison other than being copied into the static y. */
    call_driver(case_num++, 0, 0, 0);
    call_driver(case_num++, -1, 0, 0);
    call_driver(case_num++, 2, 0, 0);
    call_driver(case_num++, INT_MIN, 0, 0);
    call_driver(case_num++, INT_MAX, 0, 0);

    /* Branch 2: x == 1 but y (set from local_y) != 2. Cover zero,
       one-off (1 and 3), negative and extreme values of local_y. */
    call_driver(case_num++, 1, 0, 3);
    call_driver(case_num++, 1, 1, 3);
    call_driver(case_num++, 1, 3, 3);
    call_driver(case_num++, 1, -1, 3);
    call_driver(case_num++, 1, INT_MIN, 3);
    call_driver(case_num++, 1, INT_MAX, 3);

    /* Branch 3: x == 1, y == 2, but z != 3. Cover zero, one-off (2 and
       4), negative and extreme values of z. */
    call_driver(case_num++, 1, 2, 0);
    call_driver(case_num++, 1, 2, 2);
    call_driver(case_num++, 1, 2, 4);
    call_driver(case_num++, 1, 2, -1);
    call_driver(case_num++, 1, 2, INT_MIN);
    call_driver(case_num++, 1, 2, INT_MAX);

    /* Extreme combinations across all three parameters at once, and a
       second pass over the success path to confirm the static y is
       re-set correctly and determinism holds across repeated calls. */
    call_driver(case_num++, INT_MIN, INT_MIN, INT_MIN);
    call_driver(case_num++, INT_MAX, INT_MAX, INT_MAX);
    call_driver(case_num++, 1, INT_MIN, INT_MIN);
    call_driver(case_num++, 1, 2, INT_MAX);
    call_driver(case_num++, 1, 2, 3);

    return 0;
}
