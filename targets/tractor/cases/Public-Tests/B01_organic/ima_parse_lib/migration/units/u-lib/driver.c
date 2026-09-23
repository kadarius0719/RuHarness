#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

static uint32_t rng_state = 0x1a2b3c4du;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static int g_case_id = 1;

static void put_u8v(uint8_t *buf, size_t *pos, uint8_t v) {
    buf[*pos] = v;
    *pos += 1;
}
static void put_be16(uint8_t *buf, size_t *pos, uint16_t v) {
    put_u8v(buf, pos, (uint8_t)(v >> 8));
    put_u8v(buf, pos, (uint8_t)(v & 0xffu));
}
static void put_be32(uint8_t *buf, size_t *pos, uint32_t v) {
    put_u8v(buf, pos, (uint8_t)(v >> 24));
    put_u8v(buf, pos, (uint8_t)(v >> 16));
    put_u8v(buf, pos, (uint8_t)(v >> 8));
    put_u8v(buf, pos, (uint8_t)(v));
}
static void put_be64(uint8_t *buf, size_t *pos, uint64_t v) {
    int i;
    for (i = 7; i >= 0; --i) {
        put_u8v(buf, pos, (uint8_t)(v >> (i * 8)));
    }
}
static void put_tag(uint8_t *buf, size_t *pos, const char *tag4) {
    int i;
    for (i = 0; i < 4; ++i) {
        put_u8v(buf, pos, (uint8_t)tag4[i]);
    }
}
/* Writes the double's native (host) byte representation directly. The
   unit reads desc->sample_rate as a plain struct member (no explicit
   byte swap is applied to that one field, unlike every other multi-byte
   field), so this is what the "sample_rate" bytes must hold. Callers
   must only pass finite, non-negative, u64-range values here: the unit
   itself does an unchecked (and otherwise UB-prone) double-to-u64
   conversion on this value before re-encoding it, so anything outside
   that range would make the ORIGINAL unit invoke undefined behavior --
   not something this driver may ever trigger. */
static void put_native_f64(uint8_t *buf, size_t *pos, double v) {
    union {
        double d;
        uint8_t b[8];
    } u;
    int i;
    u.d = v;
    for (i = 0; i < 8; ++i) {
        put_u8v(buf, pos, u.b[i]);
    }
}
static void put_zeros(uint8_t *buf, size_t *pos, size_t n) {
    size_t i;
    for (i = 0; i < n; ++i) {
        put_u8v(buf, pos, 0);
    }
}

static void bytes_to_hex(char *out, const uint8_t *in, size_t n) {
    static const char hexd[] = "0123456789abcdef";
    size_t i;
    for (i = 0; i < n; ++i) {
        out[2 * i] = hexd[(in[i] >> 4) & 0xFu];
        out[2 * i + 1] = hexd[in[i] & 0xFu];
    }
    out[2 * n] = '\0';
}

static void print_result(int ret, const struct ima_info *info) {
    printf("case %d ret=%d\n", g_case_id, ret);
    if (ret == 0) {
        size_t nblocks;
        size_t i;
        char hex[80];
        printf("case %d size=%" PRIu64 " sample_rate=%a frame_count=%" PRIu64
               " channel_count=%" PRIu32 "\n",
               g_case_id, (uint64_t)info->size, info->sample_rate,
               (uint64_t)info->frame_count, (uint32_t)info->channel_count);
        nblocks = (size_t)(info->size >= 4 ? (info->size - 4) / 34 : 0);
        if (nblocks > 8) {
            nblocks = 8;
        }
        for (i = 0; i < nblocks; ++i) {
            bytes_to_hex(hex, info->blocks[i].data, 32);
            printf("case %d block[%zu] preamble=%" PRIu16 " data=%s\n",
                   g_case_id, i, info->blocks[i].preamble, hex);
        }
    }
    g_case_id++;
}

/* Builds a synthetic CAF-style buffer: an 8-byte header, followed (in
   either order) by a "desc" chunk and a "pakt" chunk, optionally an
   unrecognized chunk the parser must skip over, and finally a "data"
   chunk holding n_blocks IMA blocks. The private struct layouts used
   internally by the unit (caf_header/caf_chunk/caf_audio_description/
   caf_packet_table/caf_data) are not visible from this driver; their
   field sizes and natural (unpacked) alignment were derived from the
   unit's own pointer arithmetic and byte-swap usage, and this buffer is
   built to match exactly what that compiled layout expects. desc and
   pakt must always both appear before data (the unit dereferences them
   unconditionally once data is found), so every case here supplies
   both. */
static void run_valid_case(uint16_t version, int pakt_first,
                            int insert_unknown_chunk, double sample_rate,
                            uint32_t channels, uint64_t frame_count,
                            int bad_format_id, int n_blocks,
                            uint32_t block_seed) {
    uint8_t buf[512];
    size_t pos = 0;
    int b, j, order;
    uint32_t seed = block_seed;
    struct ima_info info;
    int ret;

    put_tag(buf, &pos, "caff");
    put_be16(buf, &pos, version);
    put_be16(buf, &pos, 0x0000u);

    if (insert_unknown_chunk) {
        put_tag(buf, &pos, "free");
        put_zeros(buf, &pos, 4); /* caf_chunk internal padding */
        put_be64(buf, &pos, 8);  /* payload size */
        put_zeros(buf, &pos, 8); /* ignored payload */
    }

    for (order = 0; order < 2; ++order) {
        int want_pakt = pakt_first ? (order == 0) : (order == 1);
        if (want_pakt) {
            put_tag(buf, &pos, "pakt");
            put_zeros(buf, &pos, 4);
            put_be64(buf, &pos, 24);
            put_be64(buf, &pos, 0);           /* packet_count (unused) */
            put_be64(buf, &pos, frame_count); /* frame_count */
            put_be32(buf, &pos, 0);           /* priming_frames (unused) */
            put_be32(buf, &pos, 0);           /* remainder_frames (unused) */
        } else {
            put_tag(buf, &pos, "desc");
            put_zeros(buf, &pos, 4);
            put_be64(buf, &pos, 32);
            put_native_f64(buf, &pos, sample_rate);
            put_tag(buf, &pos, bad_format_id ? "xxxx" : "ima4");
            put_be32(buf, &pos, 0); /* format_flags (unused) */
            put_be32(buf, &pos, 0); /* bytes_per_packet (unused) */
            put_be32(buf, &pos, 0); /* frames_per_packet (unused) */
            put_be32(buf, &pos, channels);
            put_be32(buf, &pos, 0); /* bits_per_channel (unused) */
        }
    }

    put_tag(buf, &pos, "data");
    put_zeros(buf, &pos, 4);
    put_be64(buf, &pos, (uint64_t)(4 + n_blocks * 34));
    put_be32(buf, &pos, 0); /* edit_count (unused) */
    for (b = 0; b < n_blocks; ++b) {
        put_be16(buf, &pos, (uint16_t)(0x1000u + (uint32_t)b));
        for (j = 0; j < 32; ++j) {
            seed = seed * 1664525u + 1013904223u;
            put_u8v(buf, &pos, (uint8_t)(seed >> 24));
        }
    }

    memset(&info, 0, sizeof(info));
    ret = ima_parse(&info, buf);
    print_result(ret, &info);
}

static void run_short_case(int bad_magic, uint16_t version) {
    uint8_t buf[8];
    size_t pos = 0;
    struct ima_info info;
    int ret;

    put_tag(buf, &pos, bad_magic ? "CAFF" : "caff");
    put_be16(buf, &pos, version);
    put_be16(buf, &pos, 0);

    memset(&info, 0, sizeof(info));
    ret = ima_parse(&info, buf);
    print_result(ret, &info);
}

int main(void) {
    static const double rates[4] = {44100.0, 48000.0, 8000.0, 96000.0};
    int i;

    /* ret == -1: bad magic; the function returns before the chunk loop
       ever runs, so no chunk data is needed at all. */
    run_short_case(1, 1);
    run_short_case(1, 0);

    /* ret == -2: good magic, wrong version; same early-return shape. */
    run_short_case(0, 0);
    run_short_case(0, 2);
    run_short_case(0, 65535);

    /* ret == -3: full, well-formed chunk chain but format_id != "ima4". */
    run_valid_case(1, 0, 0, 44100.0, 2, 1000, 1, 1, 0x11111111u);
    run_valid_case(1, 1, 1, 48000.0, 1, 500, 1, 2, 0x22222222u);

    /* ret == 0: desc-before-pakt, no filler chunk, varying block counts,
       sample rates, channel counts and frame counts. */
    run_valid_case(1, 0, 0, 44100.0, 2, 1000, 0, 1, 0x33333333u);
    run_valid_case(1, 0, 0, 0.0, 1, 0, 0, 1, 0x44444444u);
    run_valid_case(1, 0, 0, 8000.0, 1, 1, 0, 0, 0x55555555u);
    run_valid_case(1, 0, 0, 96000.0, 6, 123456789u, 0, 3, 0x66666666u);
    run_valid_case(1, 0, 1, 16000.0, 3, 777, 0, 2, 0xAAAAAAAAu);

    /* ret == 0: pakt-before-desc, with an unrelated chunk skipped first
       in some cases. */
    run_valid_case(1, 1, 1, 48000.0, 2, 2000, 0, 2, 0x77777777u);
    run_valid_case(1, 1, 0, 22050.0, 4, 42, 0, 1, 0x88888888u);
    run_valid_case(1, 1, 1, 192000.0, 8, 99999, 0, 4, 0x99999999u);

    for (i = 0; i < 20; ++i) {
        uint16_t ver = 1;
        int pakt_first = (int)(xorshift32() & 1u);
        int unknown = (int)(xorshift32() & 1u);
        uint32_t channels = (xorshift32() % 8u) + 1u;
        uint64_t frames = xorshift32();
        int nblocks = (int)(xorshift32() % 4u);
        uint32_t seed = xorshift32();
        double rate = rates[xorshift32() % 4u];
        run_valid_case(ver, pakt_first, unknown, rate, channels, frames, 0,
                        nblocks, seed);
    }

    return 0;
}
