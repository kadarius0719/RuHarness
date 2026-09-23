// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <string.h>

typedef struct {
    int floors;
    int bedrooms;
    double bathrooms;
} house_t;

static void print_hex(unsigned char *p, int len) {
    for (int i = 0; i < len; i++) {
        printf("%02x", p[i]);
    }
    printf("\n");
}

void driver(int floors) {
    house_t house = {0};
    house.floors = floors;
    house.bedrooms = 3;
    house.bathrooms = 2.;
    char raw[sizeof(house)];
    memcpy(raw, &house, sizeof(house));
    print_hex((unsigned char *)&raw, sizeof(raw));
}
