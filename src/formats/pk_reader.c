/**
 * pk_reader.c — Minimal ZIP reader for .plain.pk archives (store-only)
 *
 * Flam .plain.pk files are ZIP archives with compression method = store (0).
 * We parse local file headers (PK\x03\x04) sequentially to find entries.
 *
 * Securite : les archives viennent de l'exterieur (histoires telechargees).
 * - les tailles lues dans les en-tetes sont bornees par la taille reelle du
 *   fichier et par PK_MAX_ENTRY_SIZE (pas de debordement de malloc/fread) ;
 * - pk_extract_all refuse les noms d'entree dangereux (zip-slip : "..",
 *   chemins absolus, lecteur "C:", flux NTFS "a:b") et verifie que le
 *   chemin final reste sous le dossier cible.
 */

#include "pk_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <sys/stat.h>
#ifdef _WIN32
#include <direct.h>  /* _mkdir */
#include <windows.h> /* GetFileAttributesExA (mtime a 100 ns) */
#else
#include <limits.h>  /* PATH_MAX */
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

/* Taille totale du fichier ouvert (position remise au debut), -1 si erreur */
static long pk_file_size(FILE *f)
{
    if (fseek(f, 0, SEEK_END) != 0) return -1;
    long sz = ftell(f);
    if (sz < 0) return -1;
    if (fseek(f, 0, SEEK_SET) != 0) return -1;
    return sz;
}

/* Nombre d'octets restants apres la position courante (0 si erreur) */
static uint64_t pk_remaining(FILE *f, long file_size)
{
    long pos = ftell(f);
    if (pos < 0 || pos > file_size) return 0;
    return (uint64_t)(file_size - pos);
}

/* Avance de n octets sans depasser la fin du fichier.
   Retourne 0 si ok, -1 si n depasse ce qui reste (archive tronquee/forgee). */
static int pk_skip(FILE *f, uint64_t n, long file_size)
{
    if (n == 0) return 0;
    if (n > pk_remaining(f, file_size)) return -1;
    return fseek(f, (long)n, SEEK_CUR) == 0 ? 0 : -1;
}

void *pk_read_entry(const char *pk_path, const char *entry_name, size_t *out_size)
{
    if (out_size) *out_size = 0;
    if (!pk_path || !entry_name) return NULL;

    FILE *f = fopen(pk_path, "rb");
    if (!f) return NULL;

    long file_size = pk_file_size(f);
    if (file_size < 0) { fclose(f); return NULL; }

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
            if (pk_skip(f, (uint64_t)name_len + hdr.extra_len + hdr.compressed_size,
                        file_size) != 0) break;
            continue;
        }
        if (fread(name_buf, 1, name_len, f) != name_len) break;
        name_buf[name_len] = '\0';

        /* Skip extra field */
        if (pk_skip(f, hdr.extra_len, file_size) != 0) break;

        /* Check if this is the entry we want */
        if (name_len == target_len && memcmp(name_buf, entry_name, target_len) == 0) {
            if (hdr.compression != 0) {
                /* Not store — we only support uncompressed */
                fclose(f);
                return NULL;
            }
            /* Borner la taille annoncee : limite fixe et donnees reellement
               presentes dans le fichier (evite malloc(0xFFFFFFFF + 1) == 0) */
            size_t size = (size_t)hdr.uncompressed_size;
            if (size > PK_MAX_ENTRY_SIZE ||
                (uint64_t)size > pk_remaining(f, file_size)) {
                fclose(f);
                return NULL;
            }
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
        if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
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

int pk_name_is_safe(const char *name)
{
    if (!name || !name[0]) return 0;
    if (name[0] == '/' || name[0] == '\\') return 0;

    const char *comp = name;
    for (const char *p = name; ; p++) {
        unsigned char c = (unsigned char)*p;
        if (c == '/' || c == '\\' || c == '\0') {
            size_t len = (size_t)(p - comp);
            if (len == 0 && c != '\0') return 0;              /* "a//b" */
            if (len == 2 && comp[0] == '.' && comp[1] == '.') return 0;
            if (c == '\0') break;
            comp = p + 1;
            continue;
        }
        if (c == ':' || c < 0x20) return 0;
    }
    return 1;
}

/**
 * Controle final : le chemin resolu de out_path doit commencer par le
 * chemin resolu de out_dir suivi d'un separateur.
 * Sur POSIX, realpath() exige que le chemin existe : on resout donc le
 * dossier parent (deja cree par ensure_parent_dirs).
 */
static int pk_path_is_inside(const char *out_dir, const char *out_path)
{
#ifdef _WIN32
    char base[1024], full[1024];
    if (!_fullpath(base, out_dir, sizeof(base))) return 0;
    if (!_fullpath(full, out_path, sizeof(full))) return 0;
    size_t blen = strlen(base);
    while (blen > 0 && (base[blen - 1] == '\\' || base[blen - 1] == '/')) blen--;
    if (_strnicmp(base, full, blen) != 0) return 0;
    return full[blen] == '\\' || full[blen] == '/';
#else
    char base[PATH_MAX], parent[PATH_MAX], full[PATH_MAX];
    if (!realpath(out_dir, base)) return 0;
    strncpy(parent, out_path, sizeof(parent) - 1);
    parent[sizeof(parent) - 1] = '\0';
    char *slash = strrchr(parent, '/');
    if (!slash) return 0;
    *slash = '\0';
    if (!realpath(parent, full)) return 0;
    size_t blen = strlen(base);
    if (strncmp(base, full, blen) != 0) return 0;
    return full[blen] == '\0' || full[blen] == '/';
#endif
}

int pk_extract_all(const char *pk_path, const char *out_dir)
{
    return pk_extract_all_ex(pk_path, out_dir, NULL);
}

int pk_extract_all_ex(const char *pk_path, const char *out_dir, int *complete)
{
    if (complete) *complete = 0;
    if (!pk_path || !out_dir || !out_dir[0]) return -1;

    FILE *f = fopen(pk_path, "rb");
    if (!f) return -1;

    long file_size = pk_file_size(f);
    if (file_size < 0) { fclose(f); return -1; }

    char name_buf[512];
    int count = 0;
    int done = 0;      /* 1 : fin normale (repertoire central ou fin de fichier) */
    int failed = 0;    /* 1 : une entree valide n'a pas pu etre ecrite */

    while (1) {
        zip_local_header_t hdr;
        if (ftell(f) == file_size) { done = 1; break; }   /* fin exacte */
        if (fread(&hdr, sizeof(hdr), 1, f) != 1) break;   /* en-tete tronque */
        if (hdr.signature != ZIP_LOCAL_SIG) { done = 1; break; }

        size_t name_len = hdr.name_len;
        if (name_len >= sizeof(name_buf)) {
            if (pk_skip(f, (uint64_t)name_len + hdr.extra_len + hdr.compressed_size,
                        file_size) != 0) break;
            continue;
        }
        if (fread(name_buf, 1, name_len, f) != name_len) break;
        name_buf[name_len] = '\0';

        if (pk_skip(f, hdr.extra_len, file_size) != 0) break;

        /* Only extract stored (uncompressed) entries, de taille coherente */
        if (hdr.compression != 0 || hdr.uncompressed_size == 0 ||
            hdr.uncompressed_size != hdr.compressed_size) {
            if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
            continue;
        }

        /* Taille bornee par la limite et par les donnees presentes */
        if (hdr.uncompressed_size > PK_MAX_ENTRY_SIZE ||
            (uint64_t)hdr.uncompressed_size > pk_remaining(f, file_size)) {
            fprintf(stderr, "[PK] entree '%s' ignoree : taille invalide (%lu)\n",
                    name_buf, (unsigned long)hdr.uncompressed_size);
            break;  /* archive tronquee ou forgee : on arrete */
        }

        /* Zip-slip : refuser les noms qui sortiraient du dossier cible */
        if (!pk_name_is_safe(name_buf)) {
            fprintf(stderr, "[PK] entree refusee (chemin dangereux) : '%s'\n", name_buf);
            if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
            continue;
        }

        /* Build output path */
        char out_path[1280];
        int n = snprintf(out_path, sizeof(out_path), "%s/%s", out_dir, name_buf);
        if (n < 0 || (size_t)n >= sizeof(out_path)) {
            if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
            continue;
        }

        /* Normalize slashes */
        for (char *p = out_path; *p; p++) {
            if (*p == '\\') *p = '/';
        }

        ensure_parent_dirs(out_path);

        if (!pk_path_is_inside(out_dir, out_path)) {
            fprintf(stderr, "[PK] entree refusee (hors du dossier cible) : '%s'\n", name_buf);
            if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
            continue;
        }

        /* Write file */
        FILE *out = fopen(out_path, "wb");
        if (out) {
            uint32_t remaining = hdr.uncompressed_size;
            int write_err = 0;
            char buf[8192];
            while (remaining > 0) {
                size_t chunk = remaining < sizeof(buf) ? remaining : sizeof(buf);
                size_t got = fread(buf, 1, chunk, f);
                if (got == 0) break;
                if (fwrite(buf, 1, got, out) != got) { write_err = 1; break; }
                remaining -= (uint32_t)got;
            }
            fclose(out);
            if (write_err || remaining > 0) {
                /* Fichier incomplet : ne pas le laisser en place */
                remove(out_path);
                break;
            }
            count++;
        } else {
            failed = 1;
            if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
        }
    }

    fclose(f);
    if (complete) *complete = (done && !failed) ? 1 : 0;
    return count;
}

/* ------------------------------------------------------------------ */
/* Temoin d'extraction : <out_dir>/.extracted = "taille mtime" du .pk,  */
/* ecrit seulement apres une extraction complete.                       */
/* ------------------------------------------------------------------ */

#define PK_STAMP_NAME ".extracted"

/* Empreinte du .pk (taille + date de modification). 0 si ok, -1 sinon. */
static int pk_stamp(const char *pk_path, char *out, size_t out_sz)
{
#ifdef _WIN32
    WIN32_FILE_ATTRIBUTE_DATA fa;
    if (!GetFileAttributesExA(pk_path, GetFileExInfoStandard, &fa)) return -1;
    unsigned long long size = ((unsigned long long)fa.nFileSizeHigh << 32) | fa.nFileSizeLow;
    unsigned long long mt = ((unsigned long long)fa.ftLastWriteTime.dwHighDateTime << 32) |
                            fa.ftLastWriteTime.dwLowDateTime;
    snprintf(out, out_sz, "%llu %llu", size, mt);
#else
    struct stat st;
    if (stat(pk_path, &st) != 0) return -1;
    snprintf(out, out_sz, "%llu %lld", (unsigned long long)st.st_size,
             (long long)st.st_mtime);
#endif
    return 0;
}

int pk_extract_is_current(const char *pk_path, const char *out_dir)
{
    if (!pk_path || !out_dir || !out_dir[0]) return 0;

    char path[1024];
    struct stat st;
    snprintf(path, sizeof(path), "%s/main.lua", out_dir);
    if (stat(path, &st) != 0) return 0;

    char want[64];
    if (pk_stamp(pk_path, want, sizeof(want)) != 0) return 0;

    snprintf(path, sizeof(path), "%s/" PK_STAMP_NAME, out_dir);
    FILE *f = fopen(path, "rb");
    if (!f) return 0;
    char got[64];
    size_t n = fread(got, 1, sizeof(got) - 1, f);
    fclose(f);
    got[n] = '\0';
    while (n > 0 && (got[n - 1] == '\n' || got[n - 1] == '\r')) got[--n] = '\0';
    return strcmp(got, want) == 0;
}

int pk_extract_if_stale(const char *pk_path, const char *out_dir)
{
    if (!pk_path || !out_dir || !out_dir[0]) return -1;
    if (pk_extract_is_current(pk_path, out_dir)) return 0;

    /* Retirer l'ancien temoin d'abord : une extraction interrompue ne
       doit pas laisser croire que le dossier est a jour */
    char path[1024];
    snprintf(path, sizeof(path), "%s/" PK_STAMP_NAME, out_dir);
    remove(path);

    int complete = 0;
    int n = pk_extract_all_ex(pk_path, out_dir, &complete);
    if (n <= 0) return -1;
    if (!complete) {
        fprintf(stderr, "[PK] extraction incomplete : %s\n", pk_path);
        return n;
    }

    char stamp[64];
    if (pk_stamp(pk_path, stamp, sizeof(stamp)) == 0) {
        FILE *f = fopen(path, "wb");
        if (f) {
            fprintf(f, "%s\n", stamp);
            fclose(f);
        }
    }
    return n;
}
