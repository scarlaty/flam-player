#ifndef LIF_DECODER_H
#define LIF_DECODER_H

#include "lvgl/lvgl.h"
#include <stdint.h>

/**
 * Decode un fichier .lif et retourne un lv_img_dsc_t pret pour LVGL.
 *
 * Le buffer RGBA est alloue avec lv_mem_alloc (ou malloc).
 * L'appelant doit liberer avec lif_free().
 *
 * @param path  Chemin du fichier .lif
 * @return      Pointeur vers un lv_img_dsc_t alloue, ou NULL si erreur.
 */
lv_img_dsc_t *lif_decode_file(const char *path);

/**
 * Decode un buffer LIF en memoire.
 *
 * @param data  Pointeur vers les donnees LIF
 * @param size  Taille des donnees
 * @return      Pointeur vers un lv_img_dsc_t alloue, ou NULL si erreur.
 */
lv_img_dsc_t *lif_decode_mem(const uint8_t *data, size_t size);

/**
 * Libere un lv_img_dsc_t alloue par lif_decode_file/lif_decode_mem.
 */
void lif_free(lv_img_dsc_t *dsc);

#endif /* LIF_DECODER_H */
