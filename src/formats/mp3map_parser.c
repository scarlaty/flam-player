/**
 * mp3map_parser.c — Parser de fichiers .mp3map (table de seek Lunii/Flam)
 *
 * Format : header 12 octets + N x 8 octets (byte_offset, unit_pos) LE
 * Taux interne firmware : 88200 Hz (2 x 44100)
 */

#include "mp3map_parser.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define INTERNAL_RATE 88200.0f

int mp3map_parse(const char *path, mp3map_t *map) {
    memset(map, 0, sizeof(mp3map_t));

    FILE *f = fopen(path, "rb");
    if (!f) return -1;

    long size = -1;
    if (fseek(f, 0, SEEK_END) == 0) size = ftell(f);
    if (size < 0 || fseek(f, 0, SEEK_SET) != 0) { fclose(f); return -1; }

    if (size < 12) { fclose(f); return -1; }

    /* Header */
    uint8_t hdr[12];
    if (fread(hdr, 1, 12, f) != 12) { fclose(f); return -1; }

    map->total_units = (uint32_t)hdr[0] | ((uint32_t)hdr[1]<<8) |
                       ((uint32_t)hdr[2]<<16) | ((uint32_t)hdr[3]<<24);
    map->id3_offset  = (uint32_t)hdr[4] | ((uint32_t)hdr[5]<<8) |
                       ((uint32_t)hdr[6]<<16) | ((uint32_t)hdr[7]<<24);
    map->reserved    = (uint32_t)hdr[8] | ((uint32_t)hdr[9]<<8) |
                       ((uint32_t)hdr[10]<<16) | ((uint32_t)hdr[11]<<24);

    map->duration_s = (float)map->total_units / INTERNAL_RATE;

    /* Entries */
    long payload = size - 12;
    if (payload <= 0 || payload % 8 != 0) {
        map->num_entries = 0;
        map->entries = NULL;
        fclose(f);
        return 0;
    }

    map->num_entries = (int)(payload / 8);
    map->entries = (mp3map_entry_t *)malloc((size_t)map->num_entries * sizeof(mp3map_entry_t));
    if (!map->entries) { fclose(f); return -1; }

    for (int i = 0; i < map->num_entries; i++) {
        uint8_t buf[8];
        if (fread(buf, 1, 8, f) != 8) {
            /* Fichier tronque pendant la lecture : table inutilisable */
            free(map->entries);
            map->entries = NULL;
            map->num_entries = 0;
            fclose(f);
            return -1;
        }
        map->entries[i].byte_offset = (uint32_t)buf[0] | ((uint32_t)buf[1]<<8) |
                                      ((uint32_t)buf[2]<<16) | ((uint32_t)buf[3]<<24);
        map->entries[i].unit_pos    = (uint32_t)buf[4] | ((uint32_t)buf[5]<<8) |
                                      ((uint32_t)buf[6]<<16) | ((uint32_t)buf[7]<<24);
    }

    fclose(f);
    return 0;
}

/* Debut des donnees audio (premiere trame) : id3_offset, sauf valeur
   incoherente (au-dela de la premiere entree) -> octet 0 */
static uint32_t mp3map_data_start(const mp3map_t *map) {
    if (map->num_entries > 0 && map->id3_offset > map->entries[0].byte_offset)
        return 0;
    return map->id3_offset;
}

int mp3map_seek_floor(const mp3map_t *map, float seconds,
                      uint32_t *byte_offset, uint32_t *unit_pos) {
    if (!map->entries || map->num_entries == 0) return -1;

    float target_units = seconds * INTERNAL_RATE;

    /* Avant la premiere entree (ou cible invalide) : debut reel des
       donnees, position 0. La table ne contient pas d'entree pour t=0
       (entries[0] est ~1 s apres le debut sur les paquets Lunii/Telmi). */
    if (!(target_units >= (float)map->entries[0].unit_pos)) {
        *byte_offset = mp3map_data_start(map);
        *unit_pos = 0;
        return 0;
    }

    /* Recherche binaire : derniere entree dont unit_pos <= cible */
    int lo = 0, hi = map->num_entries - 1;
    while (lo < hi) {
        int mid = (lo + hi + 1) / 2;
        if ((float)map->entries[mid].unit_pos <= target_units) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    *byte_offset = map->entries[lo].byte_offset;
    *unit_pos = map->entries[lo].unit_pos;
    return 0;
}

uint32_t mp3map_seek(const mp3map_t *map, float seconds) {
    uint32_t off, unit;
    if (mp3map_seek_floor(map, seconds, &off, &unit) != 0) return 0;

    /* Entree la plus proche : comparer avec la suivante (ou avec la
       premiere entree si on est parti du debut des donnees) */
    float target_units = seconds * INTERNAL_RATE;
    for (int i = 0; i < map->num_entries; i++) {
        if (map->entries[i].unit_pos <= unit) continue;
        float d_lo = target_units - (float)unit;
        float d_hi = (float)map->entries[i].unit_pos - target_units;
        if (d_hi < d_lo) off = map->entries[i].byte_offset;
        break;
    }
    return off;
}
