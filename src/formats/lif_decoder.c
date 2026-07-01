/**
 * lif_decoder.c — Decodeur LIF (Lunii Image File Format)
 *
 * Porte depuis liff_codec.py (Seph29/liff-viewer).
 * Decode un fichier .lif en buffer RGBA8888 pour LVGL.
 *
 * Format :
 *   [4] magic "liff"/"LIFF"
 *   [4] width  (uint32 BE)
 *   [4] height (uint32 BE)
 *   [1] channels (0xA2)
 *   [N] payload compresse (opcodes)
 *   [8] end marker (7x 0x00 + 0x01)
 */

#include "lif_decoder.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ================================================================== */
/* Constantes                                                          */
/* ================================================================== */

#define LIF_HEADER_SIZE  13
#define LIF_END_SIZE      8
#define LIF_CHANNEL_RGBA  0xA2

/* ================================================================== */
/* Cache couleur 64 entrees                                            */
/* ================================================================== */

typedef struct {
    int8_t r, g, b;
    uint8_t a;
} lif_color_t;

static inline uint8_t lif_hash(int r, int g, int b, int a) {
    (void)a;
    return (uint8_t)((r * 7 + g * 5 + b * 3) & 0x3F);
}

/* Modulo positif pour les deltas wrapping */
static inline int mod32(int v) { return ((v % 32) + 32) % 32; }
static inline int mod64(int v) { return ((v % 64) + 64) % 64; }

/* Expansion RGB565 -> 8 bits */
static inline uint8_t expand5(int v) { return (uint8_t)((v * 256) / 32); }
static inline uint8_t expand6(int v) { return (uint8_t)((v * 256) / 64); }

/* ================================================================== */
/* Score de coherence (pour detection row vs column major)              */
/* ================================================================== */

static uint64_t score_coherence(const uint8_t *rgba, int w, int h) {
    uint64_t score = 0;
    int stride = w * 4;
    for (int y = 0; y < h; y++) {
        const uint8_t *row = rgba + y * stride;
        for (int x = 1; x < w; x++) {
            int idx = x * 4;
            int pr = idx - 4;
            score += (uint64_t)(
                abs((int)row[idx+0] - (int)row[pr+0]) +
                abs((int)row[idx+1] - (int)row[pr+1]) +
                abs((int)row[idx+2] - (int)row[pr+2])
            );
        }
    }
    return score;
}

/* Transpose : row-major (w x h) -> column-major interpretation */
static void transpose_pixels(const uint8_t *src, uint8_t *dst, int w, int h) {
    for (int y = 0; y < h; y++) {
        for (int x = 0; x < w; x++) {
            /* src est stocke col-major : pixel[i] = (i % h, i / h)
               dst est row-major : pixel(x, y) */
            int src_idx = (x * h + y) * 4;
            int dst_idx = (y * w + x) * 4;
            memcpy(dst + dst_idx, src + src_idx, 4);
        }
    }
}

/* ================================================================== */
/* Decodeur principal                                                  */
/* ================================================================== */

lv_img_dsc_t *lif_decode_mem(const uint8_t *data, size_t data_size) {
    if (!data || data_size < LIF_HEADER_SIZE + LIF_END_SIZE) {
        return NULL;
    }

    /* Verifier le magic */
    if (!(data[0]=='l'&&data[1]=='i'&&data[2]=='f'&&data[3]=='f') &&
        !(data[0]=='L'&&data[1]=='I'&&data[2]=='F'&&data[3]=='F')) {
        return NULL;
    }

    /* Header : width/height big-endian */
    uint32_t w = ((uint32_t)data[4]<<24) | ((uint32_t)data[5]<<16) |
                 ((uint32_t)data[6]<<8)  |  (uint32_t)data[7];
    uint32_t h = ((uint32_t)data[8]<<24) | ((uint32_t)data[9]<<16) |
                 ((uint32_t)data[10]<<8) |  (uint32_t)data[11];

    if (w == 0 || h == 0 || w > 4096 || h > 4096) {
        return NULL;
    }

    size_t file_size = data_size;

    /* Payload : entre header et end marker */
    size_t payload_start = LIF_HEADER_SIZE;
    size_t payload_end   = (size_t)file_size - LIF_END_SIZE;
    if (payload_end <= payload_start) {
        LV_LOG_ERROR("LIF: no payload");
        return NULL;
    }

    const uint8_t *payload = data + payload_start;
    size_t payload_len = payload_end - payload_start;

    uint32_t total_pixels = w * h;

    /* Buffer de sortie RGBA8888 */
    size_t buf_size = (size_t)total_pixels * 4;
    fprintf(stderr, "[LIF] decode_mem: %ux%u, buf_size=%zu\n", w, h, buf_size); fflush(stderr);
    uint8_t *rgba = (uint8_t *)malloc(buf_size);
    if (!rgba) { fprintf(stderr, "[LIF] malloc FAILED rgba\n"); fflush(stderr); return NULL; }
    memset(rgba, 0, buf_size);

    /* Etat du decodeur */
    int cr = 0, cg = 0, cb = 0;
    int ca = 255;
    int run = 0;

    /* Cache couleur */
    lif_color_t cache[64];
    memset(cache, 0, sizeof(cache));

    size_t p = 0;      /* position dans le payload */
    uint32_t px = 0;   /* pixel courant */

    while (px < total_pixels) {
        if (run > 0) {
            run--;
        } else if (p < payload_len) {
            uint8_t b1 = payload[p++];

            if (b1 == 0xFE) {
                /* RGB565 sans alpha (2 octets) */
                if (p + 2 > payload_len) break;
                uint8_t lo = payload[p++];
                uint8_t hi = payload[p++];
                cb = lo & 0x1F;
                cg = ((lo >> 5) & 0x07) | ((hi << 3) & 0x38);
                cr = (hi >> 3) & 0x1F;

            } else if (b1 == 0xFF) {
                /* RGB565 avec alpha (3 octets) */
                if (p + 3 > payload_len) break;
                uint8_t lo = payload[p++];
                uint8_t hi = payload[p++];
                cb = lo & 0x1F;
                cg = ((lo >> 5) & 0x07) | ((hi << 3) & 0x38);
                cr = (hi >> 3) & 0x1F;
                ca = payload[p++];

            } else {
                uint8_t tag = b1 & 0xC0;

                if (tag == 0x00) {
                    /* Index cache */
                    uint8_t idx = b1 & 0x3F;
                    cr = cache[idx].r;
                    cg = cache[idx].g;
                    cb = cache[idx].b;
                    ca = cache[idx].a;

                } else if (tag == 0x40) {
                    /* Delta petit */
                    int db = ((b1 >> 4) & 0x03) - 2;
                    int dg = ((b1 >> 2) & 0x03) - 2;
                    int dr = (b1 & 0x03) - 2;
                    cr = mod32(cr + dr);
                    cg = mod64(cg + dg);
                    cb = mod32(cb + db);

                } else if (tag == 0x80) {
                    /* Delta vert etendu (1 octet supplementaire) */
                    if (p >= payload_len) break;
                    uint8_t b2 = payload[p++];
                    int vg = (b1 & 0x3F) - 32;
                    cb = mod32(cb + vg - 8 + ((b2 >> 4) & 0x0F));
                    cg = mod64(cg + vg);
                    cr = mod32(cr + vg - 8 + (b2 & 0x0F));

                } else {
                    /* Run-length (tag == 0xC0) */
                    run = b1 & 0x3F;
                }
            }
        } else {
            break;
        }

        /* Mettre a jour le cache */
        uint8_t h_idx = lif_hash(cr, cg, cb, ca);
        cache[h_idx].r = (int8_t)cr;
        cache[h_idx].g = (int8_t)cg;
        cache[h_idx].b = (int8_t)cb;
        cache[h_idx].a = (uint8_t)ca;

        /* Ecrire le pixel en RGBA8888 */
        size_t off = (size_t)px * 4;
        rgba[off + 0] = expand5(cr);   /* R */
        rgba[off + 1] = expand6(cg);   /* G */
        rgba[off + 2] = expand5(cb);   /* B */
        rgba[off + 3] = (uint8_t)ca;   /* A */

        px++;
    }

    /* ---- Detection row-major vs column-major ---- */
    /* Pour les grandes images, tester si le stockage est colonne-major */
    int max_dim = (w > h) ? w : h;
    if (max_dim > 64 && h > w) {
        /* Calculer les scores de coherence */
        uint64_t score_row = score_coherence(rgba, w, h);

        /* Creer un buffer transpose (interprete col-major -> row-major) */
        uint8_t *transposed = (uint8_t *)malloc(buf_size);
        if (transposed) {
            transpose_pixels(rgba, transposed, w, h);
            uint64_t score_col = score_coherence(transposed, w, h);

            /* Si col-major est >= 12% plus coherent, utiliser la transposition.
               Attention : score_row et score_col sont uint64_t, donc on
               evite la soustraction non-signee qui deborderait. */
            if (score_row > 0 && score_col < score_row) {
                double improvement = 1.0 - (double)score_col / (double)score_row;
                if (improvement >= 0.12) {
                    /* Utiliser le buffer transpose */
                    free(rgba);
                    rgba = transposed;
                } else {
                    free(transposed);
                }
            } else {
                free(transposed);
            }
        }
    }

    /* ---- Creer le descripteur LVGL ---- */
    /* LVGL 8.x attend du ARGB8888 (ou XRGB8888).
       Notre buffer est RGBA8888. Convertir en ARGB8888. */
    uint8_t *argb = (uint8_t *)malloc(buf_size);
    if (!argb) { free(rgba); return NULL; }

    for (uint32_t i = 0; i < total_pixels; i++) {
        size_t s = (size_t)i * 4;
        argb[s + 0] = rgba[s + 2];  /* B */
        argb[s + 1] = rgba[s + 1];  /* G */
        argb[s + 2] = rgba[s + 0];  /* R */
        argb[s + 3] = rgba[s + 3];  /* A */
    }
    free(rgba);

    lv_img_dsc_t *dsc = (lv_img_dsc_t *)malloc(sizeof(lv_img_dsc_t));
    if (!dsc) { free(argb); return NULL; }

    dsc->header.always_zero = 0;
    dsc->header.w = w;
    dsc->header.h = h;
    dsc->header.cf = LV_IMG_CF_TRUE_COLOR_ALPHA;
    dsc->data_size = (uint32_t)buf_size;
    dsc->data = argb;

    return dsc;
}

lv_img_dsc_t *lif_decode_file(const char *path) {
    fprintf(stderr, "[LIF] decode_file: %s\n", path ? path : "(null)"); fflush(stderr);
    FILE *f = fopen(path, "rb");
    if (!f) { fprintf(stderr, "[LIF] fopen FAILED\n"); fflush(stderr); return NULL; }

    fseek(f, 0, SEEK_END);
    long file_size = ftell(f);
    fseek(f, 0, SEEK_SET);

    if (file_size < LIF_HEADER_SIZE + LIF_END_SIZE) {
        fclose(f);
        fprintf(stderr, "[LIF] too small\n"); fflush(stderr);
        return NULL;
    }

    uint8_t *data = (uint8_t *)malloc((size_t)file_size);
    if (!data) { fclose(f); fprintf(stderr, "[LIF] malloc FAILED file_buf\n"); fflush(stderr); return NULL; }
    fread(data, 1, (size_t)file_size, f);
    fclose(f);
    fprintf(stderr, "[LIF] file loaded %ld bytes, decoding...\n", file_size); fflush(stderr);

    lv_img_dsc_t *dsc = lif_decode_mem(data, (size_t)file_size);
    free(data);
    fprintf(stderr, "[LIF] decode_mem -> %s\n", dsc ? "OK" : "NULL"); fflush(stderr);
    return dsc;
}

void lif_free(lv_img_dsc_t *dsc) {
    if (!dsc) return;
    fprintf(stderr, "[LIF] free dsc=%p data=%p\n", (void*)dsc, (void*)dsc->data); fflush(stderr);
    if (dsc->data) free((void *)dsc->data);
    free(dsc);
}
