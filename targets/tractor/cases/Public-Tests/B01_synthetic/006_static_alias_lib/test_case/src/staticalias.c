// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include "staticalias.h"

int*
static_alias(int *outer) {
  static int inner = 1;
  if(*outer >= inner) {
    inner += *outer;
    return &inner;
  } else {
    *outer += inner;
    return outer;
  }
}

/*
  Maintain a sum leveraging multiple references to a static variable
 */
void
driver(int initial_value, int iterations) {
  int *running_sum = &initial_value;
  for (int i = 0; i < iterations; i++) {
    running_sum = static_alias(running_sum);
    printf("%d\n", *running_sum);
  }
  return;
}
