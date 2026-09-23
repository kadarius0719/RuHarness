#include "driver.h"

#include <stdio.h>

/* Exercise driver(char c) over every distinct bit pattern a char can
   hold, independent of whether plain char is signed or unsigned on the
   compiling platform. That gives complete coverage of the parameter's
   domain: zero, one, the negative range (if char is signed), and both
   the minimum and maximum representable values, and it reaches every
   branch of every ctype classification and conversion the unit calls. */
static void call_driver(int case_num, unsigned char byte) {
    char c = (char)byte;
    printf("case %d driver(byte=%u, c=%d)\n", case_num, (unsigned)byte, (int)c);
    driver(c);
}

int main(void) {
    int case_num = 1;

    for (int byte = 0; byte <= 255; byte++) {
        call_driver(case_num++, (unsigned char)byte);
    }

    return 0;
}
