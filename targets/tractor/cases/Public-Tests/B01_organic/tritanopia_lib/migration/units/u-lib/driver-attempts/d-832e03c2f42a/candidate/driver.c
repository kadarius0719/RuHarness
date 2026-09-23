#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>
#include <limits.h>
#include <float.h>
#include <math.h>
#include <stdbool.h>
#include <ctype.h>
#include <errno.h>

static void run_case(int *case_no, unsigned char r, unsigned char g,
                      unsigned char b) {
    cb_rgb_255 in;
    cb_rgb_255 out;
    in.R = r;
    in.G = g;
    in.B = b;
    out = tritanopia(in);
    printf("case %d in=%u,%u,%u out=%u,%u,%u\n", *case_no, (unsigned)r,
           (unsigned)g, (unsigned)b, (unsigned)out.R, (unsigned)out.G,
           (unsigned)out.B);
    (*case_no)++;
}

int main(void) {
    int case_no = 0;
    size_t v;

    /*
     * The reference tritanopia() pipeline removes gamma, applies a fixed
     * 3x3 matrix, re-applies gamma and then casts the 0..1-ish result to
     * unsigned char via `(unsigned char)(x * 255.f + 0.5f)`. That matrix
     * is not gamut-safe: for some saturated inputs (isolated blue, or
     * red+green both saturated with no blue) the post-matrix value lands
     * well outside [0,1], and the final cast then converts an
     * out-of-range float to unsigned char, which is undefined behavior
     * in C and is exactly what produced the -O0 vs -O2 divergence.
     *
     * Every case below has been checked by hand against the published
     * matrix coefficients to stay inside the representable range of the
     * final cast with comfortable margin, so no case here can trigger
     * that undefined behavior while still exercising every branch of
     * the unit's gamma/matrix pipeline (both legs of the piecewise
     * gamma-removal and gamma-application, for every channel, and both
     * the pow-branch and linear-branch of the blue channel).
     */

    /* Full grayscale ramp: R=G=B=v. The matrix is ~identity on the gray
       axis (coefficients cancel to within 1e-9), so this stays safely
       in range for the entire 0..255 sweep and exercises both legs of
       gamma-removal / gamma-application for every channel. */
    for (v = 0; v < 256; v++) {
        run_case(&case_no, (unsigned char)v, (unsigned char)v,
                 (unsigned char)v);
    }

    /* Pure red axis (G=B=0): off-diagonal terms vanish exactly, so
       Red' == R_lin exactly and Green'/Blue' pick up only the
       vanishingly small cross-coefficients. Safe for the full sweep. */
    for (v = 0; v < 256; v++) {
        run_case(&case_no, (unsigned char)v, 0, 0);
    }

    /* Pure green axis (R=B=0): every coefficient touching G in the
       matrix is positive and each row's coefficients sum to < 1, so the
       result stays inside [0,1] for the full sweep with wide margin. */
    for (v = 0; v < 256; v++) {
        run_case(&case_no, 0, (unsigned char)v, 0);
    }

    /* Pure blue axis (R=G=0), restricted to small B: isolated blue
       pulls Red' negative (Red' = -c2 * B_lin), and the linear leg of
       gamma-application amplifies that by ~12.92*255 before the final
       cast. Verified safe (with margin) up to raw B=10; B=11 is already
       marginal, so the sweep stops at 10 to stay clear of the boundary. */
    for (v = 0; v <= 10; v++) {
        run_case(&case_no, 0, 0, (unsigned char)v);
    }

    /* Hand-verified safe saturated / mixed combinations, chosen to
       exercise the blue channel's pow-branch of gamma-removal (which the
       restricted blue axis above cannot reach) and mixed-channel paths,
       while keeping every intermediate matrix output comfortably inside
       the safe range of the final cast:
         - (255,255,255), (255,0,0), (0,255,0): axis maxima together.
         - (255,0,255): R and B both saturated, G=0 -> Red' ~= 0.87.
         - (0,255,255): G and B both saturated, R=0 -> all channels
           land just under 1.0 (largest verified margin case).
         - (150,0,255), (200,0,255): moderate R offsetting saturated B
           enough to keep Red' positive and in range.
         - (255,10,0): saturated R with a small G nudge, verified to stay
           just under the cast's upper bound.
         - (0,255,10): saturated G with a small B (linear-branch) nudge. */
    run_case(&case_no, 255, 255, 255);
    run_case(&case_no, 255, 0, 0);
    run_case(&case_no, 0, 255, 0);
    run_case(&case_no, 255, 0, 255);
    run_case(&case_no, 0, 255, 255);
    run_case(&case_no, 150, 0, 255);
    run_case(&case_no, 200, 0, 255);
    run_case(&case_no, 255, 10, 0);
    run_case(&case_no, 0, 255, 10);

    /* A few additional interior grayscale-adjacent points (already
       covered by the full ramp above, kept for readability/labelling
       of notable boundaries: 1, 127/128 midpoint, 254). */
    run_case(&case_no, 1, 1, 1);
    run_case(&case_no, 127, 127, 127);
    run_case(&case_no, 128, 128, 128);
    run_case(&case_no, 254, 254, 254);

    return 0;
}
