// © 2026 Massachusetts Institute of Technology
// MIT License

#ifndef STATICDAG_H_
#define STATICDAG_H_

#include <stdbool.h>

int static_update(bool update, int new_value);
void path_mult(int update);
void path_add(int update);
void path_subtract(int update);
void driver(int val, int iterations);

#endif //STATICDAG_H_
