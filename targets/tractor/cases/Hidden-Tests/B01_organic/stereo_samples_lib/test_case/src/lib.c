#include "lib.h"

typedef signed long long btac1c_s64;

static btac1c_s64 btac1c2_fakesqrt(btac1c_s64 val) {
    btac1c_s64 v, v1;
    int i, j;
    if (val < 0)
        return (-btac1c2_fakesqrt(-val));
    v = val;
    while (v > (1LL << 30)) {
        v = v >> 1;
    }
    while ((v * v) > val) {
        v = v >> 1;
    }
    for (i = 1; i < 17; i++) {
        v1 = v + (v >> i);
        j = 16;
        if (v1 == v)
            break;
        while (((v1 * v1) <= val) && (j--)) {
            v = v1;
            v1 = v + (v >> i);
        }
    }
    return (v);
}

int stereo_samples(btac1c_s16 *ibuf0, btac1c_s16 *ibuf1, int len) {
    len %= 16;
    int p0, p1, p2, p3, pc0, ps0, pc1, ps1;
    btac1c_s64 e, d, dc, ds;
    int i, j, k;
    e = 0;
    p0 = 0;
    p1 = 0;
    for (i = 0; i < len; i++) {
        p0 = ibuf0[i * 2 + 0];
        p1 = ibuf0[i * 2 + 1];
        p2 = ibuf1[i * 2 + 0];
        p3 = ibuf1[i * 2 + 1];
        pc0 = (p0 + p1) >> 1;
        ps0 = p0 - p1;
        pc1 = (p2 + p3) >> 1;
        ps1 = p2 - p3;
        dc = pc0 - pc1;
        ds = ps0 - ps1;
        ds = ds >> 2;
        d = dc * dc + ds * ds;
        e += d;
    }
    e = btac1c2_fakesqrt(e);
    if ((e < 0) || (e > (1 << 30)))
        e = (1 << 30);
    return (e);
}
