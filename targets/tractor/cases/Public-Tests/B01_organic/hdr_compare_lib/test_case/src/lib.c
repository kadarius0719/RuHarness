#include "lib.h"

static int hdr_valid(const uint8_t *h) {
    return h[0] == 0xff && ((h[1] & 0xF0) == 0xf0 || (h[1] & 0xFE) == 0xe2) &&
           ((((h[1]) >> 1) & 3) != 0) && (((h[2]) >> 4) != 15) &&
           ((((h[2]) >> 2) & 3) != 3);
}

int hdr_compare(const uint8_t *h1, const uint8_t *h2) {
    return hdr_valid(h2) && ((h1[1] ^ h2[1]) & 0xFE) == 0 &&
           ((h1[2] ^ h2[2]) & 0x0C) == 0 &&
           !((((h1[2]) & 0xF0) == 0) ^ (((h2[2]) & 0xF0) == 0));
}
