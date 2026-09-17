/*
Differential test driver for unit u001-katajainen.

Compiled twice by the M0 oracle: once linked against the original
katajainen.c, once against the Rust staticlib exposing the identical ABI.
Stdout of the two builds must be byte-identical.

Inputs are deterministic (fixed-seed xorshift64), covering: empty/one/two
symbol special cases, both error paths, dense/sparse distributions, and
large weights that exercise the C comparator's int-truncation behavior.
*/

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include "katajainen.h"

#define MAX_N 320
#define RANDOM_CASES 490

static uint64_t rng_state = UINT64_C(0x9E3779B97F4A7C15);

static uint64_t rnd(void) {
  rng_state ^= rng_state << 13;
  rng_state ^= rng_state >> 7;
  rng_state ^= rng_state << 17;
  return rng_state;
}

static void run_case(int case_id, const size_t* freq, int n, int maxbits,
                     unsigned* bits) {
  int i;
  int ret = ZopfliLengthLimitedCodeLengths(freq, n, maxbits, bits);
  printf("case %d n=%d maxbits=%d ret=%d\n", case_id, n, maxbits, ret);
  for (i = 0; i < n; i++) printf("%u ", bits[i]);
  printf("\n");
}

int main(void) {
  static size_t freq[MAX_N];
  static unsigned bits[MAX_N];
  int c, i;
  int case_id = 0;

  /* Fixed edge cases. */
  run_case(case_id++, freq, 0, 15, bits);              /* n = 0 */
  freq[0] = 0;
  run_case(case_id++, freq, 1, 15, bits);              /* all zero */
  freq[0] = 7;
  run_case(case_id++, freq, 1, 15, bits);              /* single symbol */
  freq[0] = 3; freq[1] = 9;
  run_case(case_id++, freq, 2, 15, bits);              /* two symbols */
  freq[0] = 1; freq[1] = 1; freq[2] = 1; freq[3] = 1;
  run_case(case_id++, freq, 4, 1, bits);               /* maxbits too small */
  freq[0] = ((size_t)1 << 55); freq[1] = 1; freq[2] = 2;
  run_case(case_id++, freq, 3, 15, bits);              /* weight >= 2^55 */
  freq[0] = 1; freq[1] = 1; freq[2] = 2; freq[3] = 4;
  run_case(case_id++, freq, 4, 15, bits);              /* tiny huffman */
  freq[0] = 1; freq[1] = 1; freq[2] = 2; freq[3] = 3; freq[4] = 5; freq[5] = 8;
  run_case(case_id++, freq, 6, 3, bits);               /* length limiting */
  for (i = 0; i < 288; i++) freq[i] = (size_t)(i * i % 97);
  run_case(case_id++, freq, 288, 15, bits);            /* DEFLATE-shaped */
  for (i = 0; i < 300; i++) freq[i] = 1;
  run_case(case_id++, freq, 300, 9, bits);             /* uniform, tight cap */

  /* Deterministic random cases. */
  for (c = 0; c < RANDOM_CASES; c++) {
    /* maxbits stays <= 15: ExtractBitLengths has a fixed counts[16] buffer,
    so larger values are outside the C function's implicit contract (the C
    baseline SIGBUSes at maxbits=20 — found by this driver, recorded in
    migration/DECISIONS.md hazard #4). */
    static const int maxbits_choices[8] = {1, 2, 3, 5, 7, 10, 12, 15};
    int n = 1 + (int)(rnd() % MAX_N);
    int maxbits = maxbits_choices[rnd() % 8];
    int pattern = (int)(rnd() % 5);
    for (i = 0; i < n; i++) {
      switch (pattern) {
        case 0: freq[i] = rnd() % 16; break;                  /* mostly small */
        case 1: freq[i] = (rnd() % 10 == 0) ? rnd() % 100000 : 0; /* sparse */
        case 2: freq[i] = rnd() % 100000 + 1; break;          /* dense */
        case 3: freq[i] = rnd() & ((UINT64_C(1) << 22) - 1); break; /* large,
                  but inside the C comparator's well-defined domain: weights
                  >= 2^22 make the int-truncated qsort comparator inconsistent,
                  which is UB in C (migration/DECISIONS.md hazard #1) */
        default: freq[i] = (rnd() % 50 == 0)
                     ? (((size_t)1 << 55) + rnd() % 1000)     /* error path */
                     : rnd() % 1000;
      }
    }
    run_case(case_id++, freq, n, maxbits, bits);
  }

  return 0;
}
