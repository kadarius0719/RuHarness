// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <string.h>

#include "slicing.h"

/*
Index into a passed string
and print the substring indexed by [*start_ptr, *stop_ptr).
If there is no start, use 0.
If there is no stop, use the end of the string. 
*/

int slice(char *mystr, int *start_ptr, int *stop_ptr) {

    size_t len = strlen(mystr);

    char *end;
    int start, stop;

    if (start_ptr) {
        start = *start_ptr;
        if (start > len) {
            printf("Error: start is off the end of the string!\n");
            return 1;
        }
    } else {
        start = 0;
    }

    if (stop_ptr) {
        stop = *stop_ptr;
        if (stop > len) {
            printf("Error: stop is off the end of the string!\n");
            return 1;
        }
        if (stop <= start) {
            printf("Error: stop must come after start!\n");
            return 1;
        }
    // single-line else statement just to make style checking sad
    } else stop = len;

    /* char arithmetic: skip ahead `start` characters in the array */
    printf("%.*s\n", stop - start, mystr + start);

    return 0;
}
