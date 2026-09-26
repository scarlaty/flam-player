#ifndef PK_READER_H
#define PK_READER_H

#include <stddef.h>

/* Taille maximale d'une entree lue en memoire ou extraite (64 Mo) */
#define PK_MAX_ENTRY_SIZE  (64u * 1024u * 1024u)

/**
 * Read a file from a .plain.pk ZIP archive (store-only, no compression).
 *
 * @param pk_path     Path to the .plain.pk file
 * @param entry_name  Name of the entry to extract (e.g. "info.plain", "img/thumbnail.lif")
 * @param out_size    Output: size of the extracted data
 * @return            malloc'd buffer with file contents, or NULL if not found.
 *                    Caller must free().
 */
void *pk_read_entry(const char *pk_path, const char *entry_name, size_t *out_size);

/**
 * Check if a .plain.pk archive contains a given entry.
 */
int pk_has_entry(const char *pk_path, const char *entry_name);

/**
 * Extract all entries from a .plain.pk archive to a directory.
 * Creates subdirectories as needed.
 * Les entrees dont le nom est dangereux (voir pk_name_is_safe) ou dont le
 * chemin resolu sort de out_dir sont ignorees (zip-slip).
 *
 * @param pk_path   Path to the .plain.pk file
 * @param out_dir   Directory to extract into
 * @return          Number of entries extracted, or -1 on error.
 */
int pk_extract_all(const char *pk_path, const char *out_dir);

/**
 * Comme pk_extract_all ; si complete != NULL, *complete vaut 1 quand
 * l'archive a ete parcourue jusqu'au bout sans erreur d'ecriture (les
 * entrees refusees ou compressees ne rendent pas l'extraction incomplete).
 */
int pk_extract_all_ex(const char *pk_path, const char *out_dir, int *complete);

/**
 * 1 si out_dir contient main.lua et un temoin .extracted correspondant a
 * la taille et a la date de modification actuelles de pk_path, 0 sinon.
 */
int pk_extract_is_current(const char *pk_path, const char *out_dir);

/**
 * Extrait pk_path dans out_dir sauf si le dossier est deja a jour
 * (pk_extract_is_current). Ecrit le temoin .extracted apres une
 * extraction complete uniquement.
 * @return 0 si deja a jour, nombre d'entrees extraites, ou -1 si erreur.
 */
int pk_extract_if_stale(const char *pk_path, const char *out_dir);

/**
 * Verifie qu'un nom d'entree ZIP est un chemin relatif sur.
 * Refuse : nom vide, '/' ou '\' en tete, ':' (lecteur ou flux NTFS),
 * caracteres de controle, composant "..", composant vide ("a//b").
 * @return 1 si le nom est acceptable, 0 sinon.
 */
int pk_name_is_safe(const char *name);

#endif /* PK_READER_H */
