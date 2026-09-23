#include "lib.h"

typedef int lm_bool;

static inline lm_vec2 lm_v2(float x, float y) {
    lm_vec2 v = {x, y};
    return v;
}

static inline lm_vec2 lm_sub2(lm_vec2 a, lm_vec2 b) {
    return lm_v2(a.x - b.x, a.y - b.y);
}

static inline float lm_cross2(lm_vec2 a, lm_vec2 b) {
    return a.x * b.y - a.y * b.x;
}

static inline int lm_leftOf(lm_vec2 a, lm_vec2 b, lm_vec2 c) {
    float x = lm_cross2(lm_sub2(b, a), lm_sub2(c, b));
    return x < 0 ? -1 : x > 0;
}

static lm_bool lm_lineIntersection(lm_vec2 x0, lm_vec2 x1, lm_vec2 y0,
                                   lm_vec2 y1, lm_vec2 *res) {
    lm_vec2 dx = lm_sub2(x1, x0);
    lm_vec2 dy = lm_sub2(y1, y0);
    lm_vec2 d = lm_sub2(x0, y0);
    float dyx = lm_cross2(dy, dx);
    if (dyx == 0.0f)
        return 0;
    dyx = lm_cross2(d, dx) / dyx;
    if (dyx <= 0 || dyx >= 1)
        return 0;
    res->x = y0.x + dyx * dy.x;
    res->y = y0.y + dyx * dy.y;
    return 1;
}

int convex_clip(lm_vec2 *poly, int nPoly, const lm_vec2 *clip,
                         int nClip, lm_vec2 *res) {
    int nRes = nPoly;
    int dir = lm_leftOf(clip[0], clip[1], clip[2]);
    for (int i = 0, j = nClip - 1; i < nClip && nRes; j = i++) {
        if (i != 0)
            for (nPoly = 0; nPoly < nRes; nPoly++)
                poly[nPoly] = res[nPoly];
        nRes = 0;
        lm_vec2 v0 = poly[nPoly - 1];
        int side0 = lm_leftOf(clip[j], clip[i], v0);
        if (side0 != -dir)
            res[nRes++] = v0;
        for (int k = 0; k < nPoly; k++) {
            lm_vec2 v1 = poly[k], x;
            int side1 = lm_leftOf(clip[j], clip[i], v1);
            if (side0 + side1 == 0 && side0 &&
                lm_lineIntersection(clip[j], clip[i], v0, v1, &x))
                res[nRes++] = x;
            if (k == nPoly - 1)
                break;
            if (side1 != -dir)
                res[nRes++] = v1;
            v0 = v1;
            side0 = side1;
        }
    }
    return nRes;
}
