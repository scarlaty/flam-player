/**
 * pk_reader.c — Minimal ZIP reader for .plain.pk archives (store-only)
 *
 * Flam .plain.pk files are ZIP archives with compression method = store (0).
 * We parse local file headers (PK\x03\x04) sequentially to find entries.
 */

#include "pk_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <sys/stat.h>
#ifdef _WIN32
#include <direct.h>  /* _mkdir */
#endif

/* ZIP local file header signature */
#define ZIP_LOCAL_SIG  0x04034b50

#pragma pack(push, 1)
typedef struct {
    uint32_t signature;
    uint16_t version;
    uint16_t flags;
    uint16_t compression;
    uint16_t mod_time;
    uint16_t mod_date;
    uint32_t crc32;
    uint32_t compressed_size;
    uint32_t uncompressed_size;
    uint16_t name_len;
    uint16_t extra_len;
} zip_local_header_t;
#pragma pack(pop)

void *pk_read_entry(const char *pk_path, const char *entry_name, size_t *out_size)
{
    if (out_size) *out_size = 0;

    FILE *f = fopen(pk_path, "rb");
    if (!f) return NULL;

    size_t target_len = strlen(entry_name);
    char name_buf[512];

    while (1) {
        zip_local_header_t hdr;
        if (fread(&hdr, sizeof(hdr), 1, f) != 1) break;
        if (hdr.signature != ZIP_LOCAL_SIG) break;

        /* Read filename */
        size_t name_len = hdr.name_len;
        if (name_len >= sizeof(name_buf)) {
            /* Skip this entry */
            fseek(f, (long)(name_len + hdr.extra_len + hdr.compressed_size), SEEK_CUR);
            continue;
        }
        if (fread(name_buf, 1, name_len, f) != name_len) break;
        name_buf[name_len] = '\0';

        /* Skip extra field */
        if (hdr.extra_len > 0) {
            fseek(f, hdr.extra_len, SEEK_CUR);
        }

        /* Check if this is the entry we want */
        if (name_len == target_len && memcmp(name_buf, entry_name, target_len) == 0) {
            if (hdr.compression != 0) {
                /* Not store — we only support uncompressed */
                fclose(f);
                return NULL;
            }
            uint32_t size = hdr.uncompressed_size;
            void *data = malloc(size + 1);  /* +1 for optional null terminator */
            if (!data) { fclose(f); return NULL; }
            if (fread(data, 1, size, f) != size) {
                free(data);
                fclose(f);
                return NULL;
            }
            ((char *)data)[size] = '\0';  /* null terminate for text files */
            if (out_size) *out_size = size;
            fclose(f);
            return data;
        }

        /* Skip file data */
        if (hdr.compressed_size > 0) {
            fseek(f, (long)hdr.compressed_size, SEEK_CUR);
        }
    }

    fclose(f);
    return NULL;
}

int pk_has_entry(const char *pk_path, const char *entry_name)
{
    size_t sz;
    void *data = pk_read_entry(pk_path, entry_name, &sz);
    if (data) {
        free(data);
        return 1;
    }
    return 0;
}

/* Ensure parent directories exist for a file path */
static void ensure_parent_dirs(const char *filepath)
{
    char tmp[1024];
    strncpy(tmp, filepath, sizeof(tmp) - 1);
    tmp[sizeof(tmp) - 1] = '\0';

    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/' || *p == '\\') {
            char c = *p;
            *p = '\0';
#ifdef _WIN32
            _mkdir(tmp);
#else
            mkdir(tmp, 0755);
#endif
            *p = c;
        }
    }
}

int pk_extract_all(const char *pk_path, const char *out_dir)
{
    FILE *f = fopen(pk_path, "rb");
    if (!f) return -1;

    char name_buf[512];
    int count = 0;

    while (1) {
        zip_local_header_t hdr;
        if (fread(&hdr, sizeof(hdr), 1, f) != 1) break;
        if (hdr.signature != ZIP_LOCAL_SIG) break;

        size_t name_len = hdr.name_len;
        if (name_len >= sizeof(name_buf)) {
            fseek(f, (long)(name_len + hdr.extra_len + hdr.compressed_size), SEEK_CUR);
            continue;
        }
        if (fread(name_buf, 1, name_len, f) != name_len) break;
        name_buf[name_len] = '\0';

        if (hdr.extra_len > 0) {
            fseek(f, hdr.extra_len, SEEK_CUR);
        }

        /* Only extract stored (uncompressed) entries */
        if (hdr.compression != 0 || hdr.uncompressed_size == 0) {
            if (hdr.compressed_size > 0)
                fseek(f, (long)hdr.compressed_size, SEEK_CUR);
            continue;
        }

        /* Build output path */
        char out_path[1280];
        snprintf(out_path, sizeof(out_path), "%s/%s", out_dir, name_buf);

        /* Normalize slashes */
        for (char *p = out_path; *p; p++) {
            if (*p == '\\') *p = '/';
        }

        ensure_parent_dirs(out_path);

        /* Write file */
        FILE *out = fopen(out_path, "wb");
        if (out) {
            uint32_t remaining = hdr.uncompressed_size;
            char buf[8192];
            while (remaining > 0) {
                size_t chunk = remaining < sizeof(buf) ? remaining : sizeof(buf);
                size_t read = fread(buf, 1, chunk, f);
                if (read == 0) break;
                fwrite(buf, 1, read, out);
                remaining -= (uint32_t)read;
            }
            fclose(out);
            count++;
        } else {
            fseek(f, (long)hdr.uncompressed_size, SEEK_CUR);
        }
    }

    fclose(f);
    return count;
}
