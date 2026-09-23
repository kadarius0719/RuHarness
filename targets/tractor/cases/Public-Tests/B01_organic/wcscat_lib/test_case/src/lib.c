#include <stddef.h>

#include "lib.h"

int wcscat(wchar_t *dst, size_t numElem, const wchar_t *src) {
    wchar_t *ptr = dst;
    if (!dst || numElem == 0)
        return 22;
    if (!src) {
        dst[0] = 0;
        return 22;
    }
    while (ptr < dst + numElem && *ptr != 0)
        ptr++;
    while (ptr < dst + numElem) {
        if ((*ptr++ = *src++) == 0)
            return 0;
    }
    dst[0] = 0;
    return 34;
}
