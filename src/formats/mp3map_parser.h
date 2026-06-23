#ifndef MP3MAP_PARSER_H
#define MP3MAP_PARSER_H

#include <stdint.h>

typedef struct {
    uint32_t byte_offset;
    uint32_t unit_pos;
} mp3map_entry_t;

typedef struct {
    uint32_t        total_units;
    uint32_t        id3_offset;
    uint32_t        reserved;
    float           duration_s;    /* total_units / 88200 */
    int             num_entries;
    mp3map_entry_t *entries;       /* tableau alloue, liberer avec free() */
} mp3map_t;

/**
 * Parse un fichier .mp3map. Retourne 0 si OK, -1 si erreur.
 * L'appelant doit liberer map->entries avec free().
 */
int mp3map_parse(const char *path, mp3map_t *map);

/**
 * Cherche le byte_offset correspondant a un temps en secondes.
 * Retourne le byte_offset le plus proche, ou 0 si pas de table.
 */
uint32_t mp3map_seek(const mp3map_t *map, float seconds);

#endif /* MP3MAP_PARSER_H */
