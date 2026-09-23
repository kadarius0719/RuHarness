// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

#include "sieve.h"

/*
Count from a starting point, 
stopping when the count ends in 9 (base 10).
*/
void sieve(int val) {
    while (1) {
        printf("%d\n", val);
        if (val % 10 == 9) {
            break;
        }
        val++;
    }
}
