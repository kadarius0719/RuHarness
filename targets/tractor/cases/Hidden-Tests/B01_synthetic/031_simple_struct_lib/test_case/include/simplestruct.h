// © 2026 Massachusetts Institute of Technology
// MIT License

#ifndef SIMPLESTRUCT_H_
#define SIMPLESTRUCT_H_

#include <stdbool.h>

struct Date {
    int month;
    int day;
    int year;
};

bool isItMay (struct Date date);
int whatYearIsIt (struct Date date);

#endif //SIMPLESTRUCT_H_