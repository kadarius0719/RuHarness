#include "lib.h"

void flip_horizontal(cp_image_t *img) {
    cp_pixel_t *pix = img->pix;
    int w = img->w;
    int h = img->h;
    int flips = h / 2;
    for (int i = 0; i < flips; ++i) {
        cp_pixel_t *a = pix + w * i;
        cp_pixel_t *b = pix + w * (h - i - 1);
        for (int j = 0; j < w; ++j) {
            cp_pixel_t t = *a;
            *a = *b;
            *b = t;
            ++a;
            ++b;
        }
    }
}
