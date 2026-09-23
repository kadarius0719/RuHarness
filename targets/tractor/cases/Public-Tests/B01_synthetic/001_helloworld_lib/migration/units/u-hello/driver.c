#include "hello.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>
#include <limits.h>
#include <float.h>
#include <math.h>
#include <stdbool.h>
#include <ctype.h>
#include <errno.h>

int main(void) {
    int case_no;
    for (case_no = 0; case_no < 5; case_no++) {
        int ret = helloworld();
        printf("case %d ret=%d\n", case_no, ret);
    }
    return 0;
}
