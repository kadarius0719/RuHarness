// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <errno.h>
#include <limits.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    int floors;
    int bedrooms;
    double bathrooms;
} house_t;

static house_t the_house = {.floors = 2, .bedrooms = 5, .bathrooms = 2.5};

static void add_floor(house_t *house) {
    house->floors++;
}

static void add_bedrooms(house_t *house, int extra_bedrooms) {
    house->bedrooms += extra_bedrooms;
}

static void add_floor_to_the_house() {
    add_floor(&the_house);
}

static void print_the_house() {
    printf("The house has %d floors, %d bedrooms, and %.1f bathrooms\n", the_house.floors, the_house.bedrooms, the_house.bathrooms);
}

void run(int extra_bedrooms) {
    print_the_house();
    add_floor_to_the_house();
    print_the_house();
    the_house.bathrooms += 1.0;
    print_the_house();
    add_bedrooms(&the_house, extra_bedrooms);
    print_the_house();
}

static bool parse_val(const char *str, int *val) {
    errno = 0;
    char *endp = (char *)str;
    long tmp = strtol(str, &endp, 10);
    if (endp != str && errno == 0 && tmp >= INT_MIN && tmp <= INT_MAX) {
        *val = tmp;
        return true;
    } else {
        return false;
    }
}

void driver(const char *in) {
    int x;
    if (parse_val(in, &x)) {
        run(x);
        run(x);
    } else {
        printf("An error occurred\n");
    }
}
