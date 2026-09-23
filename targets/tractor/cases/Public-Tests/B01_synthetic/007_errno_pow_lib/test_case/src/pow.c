// © 2026 Massachusetts Institute of Technology
// MIT License

#include "pow.h"

#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

// Takes two arguments, a base and an exponent, and returns base^exponent
double my_pow(double base, double exponent) {
  // Calculate power
  errno = 0;
  double result = pow(base, exponent);
  if (errno == EDOM) {
    fprintf(stderr,
            "Domain error: pow(%.2f, %.2f) is undefined in the real number "
            "domain.\n",
            base, exponent);
    return -1;
  } else if (errno == ERANGE) {
    fprintf(stderr,
            "Range error: pow(%.2f, %.2f) caused overflow or underflow.\n",
            base, exponent);
    return -1;
  }

  return result;
}
