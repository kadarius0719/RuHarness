#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

static int g_case_id = 1;

/* hdr_bitrate indexes halfrate[!!(h[1]&0x8)][((h[1]>>1)&3)-1][h[2]>>4].
   The middle index is only in bounds (0..2) when (h[1]>>1)&3 is 1, 2 or
   3 -- a raw value of 0 underflows the array (index -1), which the
   original header-validity check (hdr_valid, used elsewhere in this
   library) explicitly forbids. Likewise the last index (0..14 valid)
   overflows when h[2]>>4 is 15, which hdr_valid also forbids. This is
   therefore an implicit precondition of hdr_bitrate: it must only be
   called on a header for which those two checks already hold. The
   driver honors that precondition on every call while still sweeping
   every legal combination of the three indices, plus the header bits
   the function ignores, to exercise the whole table. */
static void run_case(uint8_t h0, uint8_t h1, uint8_t h2, uint8_t h3) {
    uint8_t hdr[4];
    unsigned ret;
    hdr[0] = h0;
    hdr[1] = h1;
    hdr[2] = h2;
    hdr[3] = h3;
    ret = hdr_bitrate(hdr);
    printf("case %d h1=%" PRIu8 " h2=%" PRIu8 " ret=%u\n", g_case_id, h1, h2,
           ret);
    g_case_id++;
}

int main(void) {
    int dim0, raw2, dim2, f1, f2;

    for (dim0 = 0; dim0 < 2; ++dim0) {
        for (raw2 = 1; raw2 <= 3; ++raw2) {
            for (dim2 = 0; dim2 <= 14; ++dim2) {
                for (f1 = 0; f1 < 2; ++f1) {
                    for (f2 = 0; f2 < 2; ++f2) {
                        uint8_t filler1 = f1 ? 0xF1u : 0x00u;
                        uint8_t filler2 = f2 ? 0x0Fu : 0x00u;
                        uint8_t h1 = (uint8_t)((dim0 << 3) | (raw2 << 1) |
                                                filler1);
                        uint8_t h2 = (uint8_t)((dim2 << 4) | filler2);
                        run_case(0xFFu, h1, h2, 0x00u);
                    }
                }
            }
        }
    }

    return 0;
}
