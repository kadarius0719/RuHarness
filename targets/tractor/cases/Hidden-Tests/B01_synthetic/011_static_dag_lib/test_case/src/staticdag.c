// © 2026 Massachusetts Institute of Technology
// MIT License

#include "staticdag.h"

#include <stdbool.h>
#include <stdio.h>

int
static_update(bool update, int new_value) {
  static int run = 0;
  if(update) {
    run = new_value;
  }
  return run;
}

void
path_mult(int update) {
  int run = static_update(false, 0);
  run = run * update;
  int _updated = static_update(true, run);
  return;
}

void
path_add(int update) {
  int run = static_update(false, 0);
  run = run + update;
  int _updated = static_update(true, run);
  return;
}

void
path_subtract(int update) {
  int run = static_update(false, 0);
  run = run - update;
  int _updated = static_update(true, run);
  return;
}

/*
  Carry through static state across multiple function calls 
 */
void
driver(int val, int iterations) {
  for (int i = 0; i < iterations; i++) {
    path_add(val);
    printf("%d\n", static_update(false, 0));
    path_mult(val);
    printf("%d\n", static_update(false, 0));
    path_subtract(val);
    printf("%d\n", static_update(false, 0));
  }
  return;
}
