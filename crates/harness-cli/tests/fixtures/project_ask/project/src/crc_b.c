#include "proj.h"

unsigned fast_crc(const char *s)
{
    unsigned c = 7;
    while (*s) c = c * 31u + (unsigned)*s++;
    return c;
}
