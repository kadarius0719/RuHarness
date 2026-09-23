#include "driver.h"

#include <stdio.h>
#include <limits.h>

/* driver(x) loops while i < x, starting i at 0 and adding 2 to j every
   iteration, printing one line per iteration. A non-positive x makes
   the loop body run zero times (0 < x is false), which is completely
   safe for any negative value including INT_MIN. A large positive x,
   however, would print an impractical number of lines (violating the
   256 KiB total output budget) and would eventually make j overflow
   int (undefined behavior) long before finishing, so x is kept to a
   bounded, practical range on the positive side. */
static void call_driver(int case_num, int x) {
    printf("case %d driver(x=%d)\n", case_num, x);
    driver(x);
}

int main(void) {
    int case_num = 1;

    call_driver(case_num++, INT_MIN);
    call_driver(case_num++, -1000000);
    call_driver(case_num++, -2);
    call_driver(case_num++, -1);
    call_driver(case_num++, 0);
    call_driver(case_num++, 1);
    call_driver(case_num++, 2);
    call_driver(case_num++, 3);
    call_driver(case_num++, 10);
    call_driver(case_num++, 100);
    call_driver(case_num++, 1000);

    return 0;
}
