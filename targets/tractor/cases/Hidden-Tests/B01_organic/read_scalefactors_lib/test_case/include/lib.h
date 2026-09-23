#include <stdint.h>

typedef struct {
    const uint8_t *buf;
    int pos, limit;
} bs_t;

void read_scalefactors(bs_t *bs, uint8_t *pba, uint8_t *scfcod,
                                  int bands, float *scf);
