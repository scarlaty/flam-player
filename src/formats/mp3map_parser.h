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
 * Point de depart d'un seek : derniere entree dont la position est <= au
 * temps demande, ou le debut des donnees (id3_offset, position 0) si le
 * temps precede entries[0]. Ecrit l'offset (octets) et la position
 * (unites 88200 Hz) de ce point. Retourne 0 si OK, -1 si pas de table.
 * L'appelant peut ensuite avancer trame par trame jusqu'a la cible.
 */
int mp3map_seek_floor(const mp3map_t *map, float seconds,
                      uint32_t *byte_offset, uint32_t *unit_pos);

/**
 * Cherche le byte_offset correspondant a un temps en secondes.
 * Retourne le byte_offset du point le plus proche (entrees de la table,
 * plus le debut des donnees id3_offset pour t=0), ou 0 si pas de table.
 */
uint32_t mp3map_seek(const mp3map_t *map, float seconds);

#endif /* MP3MAP_PARSER_H */
