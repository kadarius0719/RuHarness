#include <stdint.h>

typedef uint8_t tflac_u8;
typedef uint32_t tflac_u32;
typedef uint64_t tflac_u64;

struct tflac_md5 {
    tflac_u32 a;
    tflac_u32 b;
    tflac_u32 c;
    tflac_u32 d;
    tflac_u32 pos;
    tflac_u64 total;
    tflac_u8 buffer[64 + 8];
};
typedef struct tflac_md5 tflac_md5;

void md5_transform(tflac_md5 *m);
