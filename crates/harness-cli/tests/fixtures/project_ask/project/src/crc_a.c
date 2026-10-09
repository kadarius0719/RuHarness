#include "proj.h"

unsigned fast_crc(const char *s)
{
    unsigned c = 0;
    while (*s) c = (c << 1) ^ (unsigned)*s++;
    return c;
}
