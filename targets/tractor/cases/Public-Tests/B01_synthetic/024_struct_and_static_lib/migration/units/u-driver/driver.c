#include "driver.h"

#include <stdio.h>
#include <limits.h>

extern void run(int extra_bedrooms);

static void call_run(int case_num, int extra_bedrooms) {
    printf("case %d run(extra_bedrooms=%d)\n", case_num, extra_bedrooms);
    run(extra_bedrooms);
}

static void call_driver(int case_num, int x) {
    printf("case %d driver(x=%d)\n", case_num, x);
    driver(x);
}

int main(void) {
    int case_num = 1;

    /* Direct run() calls. Running total of the_house.bedrooms starts at 5
       (the unit's static initializer) and is tracked below in comments so
       every extra_bedrooms value stays in range for the int addition the
       unit performs (house->bedrooms += extra_bedrooms), avoiding signed
       overflow: 5 */
    call_run(case_num++, 0);                 /* 5+0=5 */
    call_run(case_num++, 1);                 /* 5+1=6 */
    call_run(case_num++, -1);                /* 6-1=5 */
    call_run(case_num++, 2);                 /* 5+2=7 */
    call_run(case_num++, -7);                /* 7-7=0 */
    call_run(case_num++, INT_MAX);           /* 0+INT_MAX=INT_MAX */
    call_run(case_num++, -2147483647);       /* INT_MAX-2147483647=0 */
    call_run(case_num++, INT_MIN);           /* 0+INT_MIN=INT_MIN */
    call_run(case_num++, 2147483647);        /* INT_MIN+2147483647=-1 */
    call_run(case_num++, 1);                 /* -1+1=0 */
    call_run(case_num++, 100);               /* 0+100=100 */
    call_run(case_num++, -200);              /* 100-200=-100 */
    call_run(case_num++, 300);               /* -100+300=200 */
    call_run(case_num++, -200);              /* 200-200=0 */
    call_run(case_num++, 1000000);           /* 0+1000000=1000000 */
    call_run(case_num++, -1000000);          /* 1000000-1000000=0 */
    call_run(case_num++, 7);                 /* 0+7=7 */
    call_run(case_num++, -7);                /* 7-7=0 */

    /* driver(x) calls run(x) twice, so the effective delta to bedrooms is
       2*x; the running total (currently 0) is tracked the same way and
       stays well within int range for every value chosen below. */
    call_driver(case_num++, 0);              /* 0,0 -> 0 */
    call_driver(case_num++, 1);              /* 1,2 -> 2 */
    call_driver(case_num++, -1);             /* 1,0 -> 0 */
    call_driver(case_num++, 5);              /* 5,10 -> 10 */
    call_driver(case_num++, -5);             /* 5,0 -> 0 */
    call_driver(case_num++, 100000);         /* 100000,200000 -> 200000 */
    call_driver(case_num++, -100000);        /* 100000,0 -> 0 */
    call_driver(case_num++, 1000000000);     /* 1000000000,2000000000 -> 2000000000 */
    call_driver(case_num++, -1000000000);    /* 1000000000,0 -> 0 */
    call_driver(case_num++, 1073741823);     /* 1073741823,2147483646 -> 2147483646 */
    call_driver(case_num++, -1073741823);    /* 1073741823,0 -> 0 */

    return 0;
}
