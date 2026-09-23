#include "lib.h"

int predict_sample(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    pfcn %= 17;
    int pred, p0, p1;
    int i;
    i = idx;
    switch (pfcn) {
    case 0:
        pred = psamp[(i - 1) & 7];
        break;
    case 1:
        pred = 2 * psamp[(i - 1) & 7] - psamp[(i - 2) & 7];
        break;
    case 2:
        pred = (3 * psamp[(i - 1) & 7] - psamp[(i - 2) & 7]) >> 1;
        break;
    case 3:
        pred = (5 * psamp[(i - 1) & 7] - psamp[(i - 2) & 7]) >> 2;
        break;
    case 4:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7]);
        p1 = (psamp[(i - 2) & 7] + psamp[(i - 3) & 7]);
        pred = p0 - (p1 >> 1);
        break;
    case 5:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7]);
        p1 = (psamp[(i - 2) & 7] + psamp[(i - 3) & 7]);
        pred = (3 * p0 - p1) >> 2;
        break;
    case 6:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7]);
        p1 = (psamp[(i - 2) & 7] + psamp[(i - 3) & 7]);
        pred = (5 * p0 - p1) >> 3;
        break;
    case 7:
        pred = (18 * psamp[(i - 1) & 7] - 4 * psamp[(i - 2) & 7] +
                3 * psamp[(i - 3) & 7] - 2 * psamp[(i - 4) & 7] +
                1 * psamp[(i - 5) & 7]) /
               16;
        break;
    case 8:
        pred = (72 * psamp[(i - 1) & 7] - 16 * psamp[(i - 2) & 7] +
                12 * psamp[(i - 3) & 7] - 8 * psamp[(i - 4) & 7] +
                5 * psamp[(i - 5) & 7] - 3 * psamp[(i - 6) & 7] +
                3 * psamp[(i - 7) & 7] - 1 * psamp[(i - 8) & 7]) /
               64;
        break;
    case 9:
        pred = (76 * psamp[(i - 1) & 7] - 17 * psamp[(i - 2) & 7] +
                10 * psamp[(i - 3) & 7] - 7 * psamp[(i - 4) & 7] +
                5 * psamp[(i - 5) & 7] - 4 * psamp[(i - 6) & 7] +
                4 * psamp[(i - 7) & 7] - 3 * psamp[(i - 8) & 7]) /
               64;
        break;
    case 10:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7] + psamp[(i - 3) & 7] +
              psamp[(i - 4) & 7]);
        p1 = (psamp[(i - 5) & 7] + psamp[(i - 6) & 7] + psamp[(i - 7) & 7] +
              psamp[(i - 8) & 7]);
        pred = (5 * p0 - p1) >> 4;
        break;
    case 11:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7] + psamp[(i - 3) & 7] +
              psamp[(i - 4) & 7]);
        p1 = (psamp[(i - 5) & 7] + psamp[(i - 6) & 7] + psamp[(i - 7) & 7] +
              psamp[(i - 8) & 7]);
        pred = (p0 + p1) >> 3;
        break;
    case 12:
    case 13:
    case 14:
    case 15:
        pred = (ridx->firfx[pfcn - 12][0] * psamp[(i - 1) & 7] +
                ridx->firfx[pfcn - 12][1] * psamp[(i - 2) & 7] +
                ridx->firfx[pfcn - 12][2] * psamp[(i - 3) & 7] +
                ridx->firfx[pfcn - 12][3] * psamp[(i - 4) & 7] +
                ridx->firfx[pfcn - 12][4] * psamp[(i - 5) & 7] +
                ridx->firfx[pfcn - 12][5] * psamp[(i - 6) & 7] +
                ridx->firfx[pfcn - 12][6] * psamp[(i - 7) & 7] +
                ridx->firfx[pfcn - 12][7] * psamp[(i - 8) & 7]) /
               256;
        break;
    default:
        pred = 0;
        break;
    }
    return (pred);
}
