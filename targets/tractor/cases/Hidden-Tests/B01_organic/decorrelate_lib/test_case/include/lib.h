#include <stdint.h>

typedef uint8_t tflac_u8;
typedef int16_t tflac_s16;
typedef int32_t tflac_s32;
typedef uint32_t tflac_u32;
typedef uint64_t tflac_u64;

struct tflac {
    tflac_u32 bitdepth;
    tflac_u32 cur_blocksize;
    tflac_u32 subframe_bitdepth;
    tflac_u8 constant;
    tflac_u64 residual_errors[5];
    tflac_s32 residuals[5];
};
typedef struct tflac tflac;

void decorrelate(tflac *t, tflac_u32 channel, tflac_u32 stride);
