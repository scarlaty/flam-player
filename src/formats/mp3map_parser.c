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

    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);

    if (size < 12) { fclose(f); return -1; }

    /* Header */
    uint8_t hdr[12];
    fread(hdr, 1, 12, f);

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
        fread(buf, 1, 8, f);
        map->entries[i].byte_offset = (uint32_t)buf[0] | ((uint32_t)buf[1]<<8) |
                                      ((uint32_t)buf[2]<<16) | ((uint32_t)buf[3]<<24);
        map->entries[i].unit_pos    = (uint32_t)buf[4] | ((uint32_t)buf[5]<<8) |
                                      ((uint32_t)buf[6]<<16) | ((uint32_t)buf[7]<<24);
    }

    fclose(f);
    return 0;
}

uint32_t mp3map_seek(const mp3map_t *map, float seconds) {
    if (!map->entries || map->num_entries == 0) return 0;

    float target_units = seconds * INTERNAL_RATE;

    /* Recherche binaire de l'entree la plus proche */
    int lo = 0, hi = map->num_entries - 1;
    while (lo < hi) {
        int mid = (lo + hi + 1) / 2;
        if ((float)map->entries[mid].unit_pos <= target_units) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    return map->entries[lo].byte_offset;
}
