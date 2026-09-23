#include "driver.h"

#include <stdio.h>
#include <limits.h>

extern void run(int extra_bedrooms);

static void call_run(int case_num, int extra_bedrooms) {
    printf("case %d run(extra_bedrooms=%d)\n", case_num, extra_bedrooms);
    run(extra_bedrooms);
}

static void call_driver(int case_num, const char *in) {
    printf("case %d driver(in=\"%s\")\n", case_num, in);
    driver(in);
}

int main(void) {
    int case_num = 1;

    /* Direct run() calls. Running total of the_house.bedrooms starts at 5
       (the unit's static initializer) and is tracked in the comments so
       every extra_bedrooms value keeps house->bedrooms += extra_bedrooms
       within int range, avoiding signed overflow: 5 */
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

    /* driver(in) parses in with strtol and, on success, calls run(x)
       twice (effective delta 2*x); on failure it prints an error and
       leaves the house state untouched. First exercise every rejection
       path (no state change from any of these): empty string, blank
       string, non-numeric text, a sign with no digits, a magnitude that
       overflows long (ERANGE), and magnitudes that parse fine as long
       but fall outside [INT_MIN, INT_MAX]. */
    call_driver(case_num++, "");
    call_driver(case_num++, "   ");
    call_driver(case_num++, "abc");
    call_driver(case_num++, "+");
    call_driver(case_num++, "-");
    call_driver(case_num++, "99999999999999999999");
    call_driver(case_num++, "-99999999999999999999");
    call_driver(case_num++, "2147483648");
    call_driver(case_num++, "-2147483649");

    /* Now the accepted paths, each followed by two run() calls inside
       driver(). Running total resumes at 0 (left there by the direct
       run() calls above) and is tracked the same way. */
    call_driver(case_num++, "0");             /* x=0:   0,0 -> 0 */
    call_driver(case_num++, "1");              /* x=1:   1,2 -> 2 */
    call_driver(case_num++, "-1");             /* x=-1:  1,0 -> 0 */
    call_driver(case_num++, "5");              /* x=5:   5,10 -> 10 */
    call_driver(case_num++, "-5");             /* x=-5:  5,0 -> 0 */
    call_driver(case_num++, "42abc");          /* x=42:  42,84 -> 84 (trailing garbage ignored) */
    call_driver(case_num++, " 100");           /* x=100: 184,284 -> 284 (leading whitespace skipped) */
    call_driver(case_num++, "-142");           /* x=-142: 142,0 -> 0 */
    call_driver(case_num++, "0x10");           /* x=0:   0,0 -> 0 (base-10 parse stops at 'x') */
    call_driver(case_num++, "+7");             /* x=7:   7,14 -> 14 (leading plus sign) */
    call_driver(case_num++, "-14");            /* x=-14: 0,-14 -> -14 */
    call_driver(case_num++, "14");             /* x=14:  0,14 -> 14 */
    call_driver(case_num++, "-14");            /* x=-14: 0,-14 -> -14 */
    call_driver(case_num++, "1000000000");     /* x=1000000000: 999999986,1999999986 -> 1999999986 */
    call_driver(case_num++, "-1000000000");    /* x=-1000000000: 999999986,-14 -> -14 */
    call_driver(case_num++, "14");             /* x=14:  0,14 -> 14 */
    call_driver(case_num++, "-14");            /* x=-14: 0,-14 -> -14 */
    call_driver(case_num++, "1073741823");     /* x=1073741823: 1073741809,2147483632 -> 2147483632 */
    call_driver(case_num++, "-1073741823");    /* x=-1073741823: 1073741809,-14 -> -14 */

    return 0;
}
