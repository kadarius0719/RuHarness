// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include "staticloop.h"

int
static_sum(int update) {
  static int sum = 0;
  sum += update;
  return sum;
}

/*
  Maintain a running total using a static variable
 */
void
driver(int stride) {
  for (int i = 0; i < 10; i++) {
    printf("%d\n", static_sum(i * stride));
  }
  return;
}
