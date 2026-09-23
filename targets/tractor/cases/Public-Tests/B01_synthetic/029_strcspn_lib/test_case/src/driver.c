// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <string.h>

void driver(const char *s1, const char *s2) {
    printf("%zu\n", strcspn(s1, s2));
}
