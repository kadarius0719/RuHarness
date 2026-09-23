#include "lib.h"

void stb__CompressAlphaBlock(unsigned char *dest, unsigned char *src,
                                    int stride) {
    int i, dist, bias, dist4, dist2, bits, mask;
    int mn, mx;
    mn = mx = src[0];
    for (i = 1; i < 16; i++) {
        if (src[i * stride] < mn)
            mn = src[i * stride];
        else if (src[i * stride] > mx)
            mx = src[i * stride];
    }
    dest[0] = (unsigned char)mx;
    dest[1] = (unsigned char)mn;
    dest += 2;
    dist = mx - mn;
    dist4 = dist * 4;
    dist2 = dist * 2;
    bias = (dist < 8) ? (dist - 1) : (dist / 2 + 2);
    bias -= mn * 7;
    bits = 0, mask = 0;
    for (i = 0; i < 16; i++) {
        int a = src[i * stride] * 7 + bias;
        int ind, t;
        t = (a >= dist4) ? -1 : 0;
        ind = t & 4;
        a -= dist4 & t;
        t = (a >= dist2) ? -1 : 0;
        ind += t & 2;
        a -= dist2 & t;
        ind += (a >= dist);
        ind = -ind & 7;
        ind ^= (2 > ind);
        mask |= ind << bits;
        if ((bits += 3) >= 8) {
            *dest++ = (unsigned char)mask;
            mask >>= 8;
            bits -= 8;
        }
    }
}

// taking/putting return data into arrays; 64 bytes each
void compress_bc5(unsigned char *dest, const unsigned char *src) {
    stb__CompressAlphaBlock(dest, (unsigned char *)src, 2);
    stb__CompressAlphaBlock(dest + 8, (unsigned char *)src + 1, 2);
}
