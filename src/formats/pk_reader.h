#ifndef PK_READER_H
#define PK_READER_H

#include <stddef.h>

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
 *
 * @param pk_path   Path to the .plain.pk file
 * @param out_dir   Directory to extract into
 * @return          Number of entries extracted, or -1 on error.
 */
int pk_extract_all(const char *pk_path, const char *out_dir);

#endif /* PK_READER_H */
