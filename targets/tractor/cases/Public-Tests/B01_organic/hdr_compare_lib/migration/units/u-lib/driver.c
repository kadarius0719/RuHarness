#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

static int g_case_id = 1;

/* hdr_compare only ever reads h1[1], h1[2], h2[0], h2[1] and h2[2] --
   all with constant indices, so any 3-byte buffers are safe regardless
   of content; there is no implicit precondition to honor here (unlike
   hdr_bitrate/hdr_valid in this same family, which this unit does not
   expose). */
static void run_case(uint8_t h1_0, uint8_t h1_1, uint8_t h1_2, uint8_t h2_0,
                      uint8_t h2_1, uint8_t h2_2) {
    uint8_t h1[3];
    uint8_t h2[3];
    int ret;
    h1[0] = h1_0;
    h1[1] = h1_1;
    h1[2] = h1_2;
    h2[0] = h2_0;
    h2[1] = h2_1;
    h2[2] = h2_2;
    ret = hdr_compare(h1, h2);
    printf("case %d h1=%" PRIu8 ",%" PRIu8 ",%" PRIu8 " h2=%" PRIu8
           ",%" PRIu8 ",%" PRIu8 " ret=%d\n",
           g_case_id, h1_0, h1_1, h1_2, h2_0, h2_1, h2_2, ret);
    g_case_id++;
}

int main(void) {
    static const uint8_t h2_0_vals[] = {0xFFu, 0x00u};
    static const uint8_t h2_1_vals[] = {0xFFu, 0xF0u, 0xE2u, 0x00u};
    static const uint8_t h2_2_vals[] = {0x00u, 0x10u, 0xF0u, 0x0Cu, 0x30u};
    size_t i0, i1, i2;
    int v;

    for (i0 = 0; i0 < sizeof(h2_0_vals) / sizeof(h2_0_vals[0]); ++i0) {
        for (i1 = 0; i1 < sizeof(h2_1_vals) / sizeof(h2_1_vals[0]); ++i1) {
            for (i2 = 0; i2 < sizeof(h2_2_vals) / sizeof(h2_2_vals[0]);
                 ++i2) {
                uint8_t g0 = h2_0_vals[i0];
                uint8_t g1 = h2_1_vals[i1];
                uint8_t g2 = h2_2_vals[i2];
                for (v = 0; v < 6; ++v) {
                    uint8_t a1 = g1;
                    uint8_t a2 = g2;
                    switch (v) {
                    case 0:
                        break; /* exact match */
                    case 1:
                        a1 = (uint8_t)(g1 ^ 0x01u); /* bit ignored by 0xFE mask */
                        break;
                    case 2:
                        a1 = (uint8_t)(g1 ^ 0x02u); /* breaks the 0xFE match */
                        break;
                    case 3:
                        a2 = (uint8_t)(g2 ^ 0x10u); /* outside the 0x0C mask */
                        break;
                    case 4:
                        a2 = (uint8_t)(g2 ^ 0x04u); /* breaks the 0x0C match */
                        break;
                    case 5:
                        a2 = (uint8_t)((g2 & 0xF0u) == 0 ? (g2 | 0x10u)
                                                          : (g2 & 0x0Fu));
                        break; /* flips top-nibble zero-ness only */
                    default:
                        break;
                    }
                    run_case(0xFFu, a1, a2, g0, g1, g2);
                }
            }
        }
    }

    /* h1[0] is never read by hdr_compare; confirm it has no effect. */
    run_case(0x00u, 0xFFu, 0x00u, 0xFFu, 0xFFu, 0x00u);
    run_case(0x12u, 0xFFu, 0x00u, 0xFFu, 0xFFu, 0x00u);

    return 0;
}
