#include "lib.h"

void decorrelate(tflac *t, tflac_u32 channel, tflac_u32 stride) {
    tflac_s32 *residuals_0 = t->residuals;
    tflac_u32 i = 0;
    tflac_u32 l = 0;
    tflac_u32 r = 0;
    tflac_u32 non_constant = 0;
    tflac_u32 min_found = 0;
    if (channel == 0) {
        t->subframe_bitdepth = t->bitdepth;
        while (i < t->cur_blocksize && i <= 5) {
            residuals_0[i] = (tflac_s32)l;
            non_constant |=
                (tflac_u32)residuals_0[i] ^ (tflac_u32)residuals_0[0];
            min_found |= residuals_0[i] == (-2147483647 - 1);
            i++;
            l += stride;
        }
    } else {
        t->subframe_bitdepth = t->bitdepth + 1;
        while (i < t->cur_blocksize && i <= 5) {
            residuals_0[i] = ((tflac_s32)l) - ((tflac_s32)r);
            non_constant |=
                (tflac_u32)residuals_0[i] ^ (tflac_u32)residuals_0[0];
            min_found |= residuals_0[i] == (-2147483647 - 1);
            i++;
            l += stride;
            r += stride;
        }
    }
    t->constant = !non_constant;
    t->residual_errors[0] = min_found ? (18446744073709551615UL) : 0UL;
}
