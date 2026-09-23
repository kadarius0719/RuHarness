// © 2026 Massachusetts Institute of Technology
// MIT License

#include "simplestruct.h"
#include <stdbool.h>

bool isItMay (struct Date date) {
    return date.month == 5;
}

int whatYearIsIt (struct Date date) {
    return date.year;
}