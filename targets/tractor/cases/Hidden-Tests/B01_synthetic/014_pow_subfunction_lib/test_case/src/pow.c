// © 2026 Massachusetts Institute of Technology
// MIT License

#include "pow.h"

#include <stdio.h>
#include <stdlib.h>
#include <math.h>

int my_err = 0;

//helper function because why not
double my_pow(double base, double exponent) {
    double result = pow(base, exponent);
    if (isnan(result)) {
        fprintf(stderr, "Domain error: pow(%.2f, %.2f) is undefined in the real number domain.\n", base, exponent);
        my_err = -1;
        result = 0;
    } else if (isinf(result)) {
        fprintf(stderr, "Range error: pow(%.2f, %.2f) caused overflow or underflow.\n", base, exponent);
        my_err =  -1;
        result = 0;
    }

    return result;
}

//Accepts a base and an exponent and prints base^exponent
double feel_the_power(double base, double exponent) {
    // Calculate power
    my_err = 0;
    double result = my_pow(base, exponent);

    if (my_err != 0)
        printf("Oh no, there was an error! How rude.\n");
    else
        printf("Result: %.2f\n", result);

    return result;
}
