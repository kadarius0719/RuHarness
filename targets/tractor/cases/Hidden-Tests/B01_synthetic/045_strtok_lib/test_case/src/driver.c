// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <string.h>

void driver(char *s_in) {
    const char *sep = ":/\n";

    for (char *s = strtok(s_in, sep); s; s = strtok(NULL, sep)) {
        printf("line %s\n", s);
    }
}
