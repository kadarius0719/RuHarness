// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <ctype.h>
#include <locale.h>
#include <stdio.h>
#include <stdlib.h>

void driver(char c) {
    setlocale(LC_ALL, "C");
    
    printf("alphanumeric: %d\n", isalnum(c));
    printf("alphabetic: %d\n", isalpha(c));
    printf("lowercase: %d\n", islower(c));
    printf("uppercase: %d\n", isupper(c));
    printf("digit: %d\n", isdigit(c));
    printf("hexadecimal: %d\n", isxdigit(c));
    printf("control: %d\n", iscntrl(c));
    printf("graphical: %d\n", isgraph(c));
    printf("space: %d\n", isspace(c));
    printf("blank: %d\n", isblank(c));
    printf("printing: %d\n", isprint(c));
    printf("punctuation: %d\n", ispunct(c));
    printf("to lower: %c\n", tolower(c));
    printf("to upper: %c\n", toupper(c));
}
