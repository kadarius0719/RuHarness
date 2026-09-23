typedef struct lm_vec2 {
    float x, y;
} lm_vec2;

int convex_clip(lm_vec2 *poly, int nPoly, const lm_vec2 *clip,
                         int nClip, lm_vec2 *res);
