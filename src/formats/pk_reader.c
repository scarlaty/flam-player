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
#include <ctype.h>
#include <stdint.h>
#include <sys/stat.h>
#ifdef _WIN32
#include <direct.h>  /* _mkdir */
#include <windows.h> /* GetFileAttributesExA (mtime a 100 ns) */
#include <share.h>   /* _fsopen, _SH_DENYWR */
#else
#include <limits.h>  /* PATH_MAX */
#include <unistd.h>  /* rmdir */
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

static int pk_ieq(const char *a, size_t alen, const char *b);

/* Noms de peripheriques Windows (CON, NUL, COM1...) : "NUL.txt" reste le
   peripherique, et ecrire dans "CON" ecrit sur la console. */
static int pk_is_device_name(const char *comp, size_t len)
{
    static const char *const fixed[] = { "con", "prn", "aux", "nul", "conin$", "conout$" };
    size_t base = 0;
    while (base < len && comp[base] != '.') base++;   /* le nom sans extension */
    while (base > 0 && comp[base - 1] == ' ') base--;
    for (size_t i = 0; i < sizeof(fixed) / sizeof(fixed[0]); i++) {
        if (pk_ieq(comp, base, fixed[i])) return 1;
    }
    /* COM1-9 / LPT1-9, aussi avec un chiffre en exposant (1, 2, 3 de cp1252 /
       Latin-1 : octet 0xB9, 0xB2, 0xB3 ; ou leur forme UTF-8 : 0xC2 puis cet
       octet). fopen recoit les octets du nom tels quels (page de code ANSI) ;
       les deux formes sont couvertes. */
    int c3 = tolower((unsigned char)comp[0]) == 'c' && tolower((unsigned char)comp[1]) == 'o' &&
             tolower((unsigned char)comp[2]) == 'm';
    int l3 = tolower((unsigned char)comp[0]) == 'l' && tolower((unsigned char)comp[1]) == 'p' &&
             tolower((unsigned char)comp[2]) == 't';
    if (base >= 4 && (c3 || l3)) {
        unsigned char d = (unsigned char)comp[3];
        if (base == 4 && ((d >= '1' && d <= '9') || d == 0xB9 || d == 0xB2 || d == 0xB3))
            return 1;
        if (base == 5 && d == 0xC2) {
            unsigned char e = (unsigned char)comp[4];
            if (e == 0xB9 || e == 0xB2 || e == 0xB3) return 1;
        }
    }
    return 0;
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
            if (len == 1 && comp[0] == '.') return 0;         /* "./x", "a/./b" */
            /* Windows retire points et espaces finaux : "..", ".. " et "a." ne
               designent pas ce qu'ils annoncent */
            if (len > 0 && (comp[len - 1] == '.' || comp[len - 1] == ' ')) return 0;
            if (len > 0 && pk_is_device_name(comp, len)) return 0;
            if (c == '\0') break;
            comp = p + 1;
            continue;
        }
        /* ':' (lecteur, flux NTFS), controles et caracteres que Windows ne
           peut pas ecrire dans un nom : sinon l'extraction echouerait apres
           la purge */
        if (c == ':' || c < 0x20 || c == '<' || c == '>' || c == '"' || c == '|' ||
            c == '?' || c == '*') return 0;
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

/* Signatures ZIP marquant la fin normale de la suite d'entrees locales */
#define ZIP_CENTRAL_SIG  0x02014b50   /* repertoire central */
#define ZIP_END_SIG      0x06054b50   /* fin d'archive */
#define ZIP_FLAG_DESCRIPTOR 0x0008    /* tailles apres les donnees (data descriptor) */

/* ------------------------------------------------------------------ */
/* Temoins d'extraction (poses par le player, absents d'un dossier      */
/* ecrit a la main) :                                                   */
/*  - <out_dir>/.extracting : extraction en cours ou interrompue ; il   */
/*    liste (un nom par ligne) les fichiers deja extraits ;             */
/*  - <out_dir>/.extracted  : "taille mtime" du .pk, puis la liste des  */
/*    fichiers extraits ; ecrit seulement apres une extraction          */
/*    complete (il remplace .extracting).                               */
/* Un dossier avec main.lua et aucun temoin est un dossier d'auteur :   */
/* il n'est jamais ecrase par le .pk voisin, ni purge.                  */
/* ------------------------------------------------------------------ */

#define PK_STAMP_NAME ".extracted"
#define PK_BUSY_NAME  ".extracting"

static int pk_ieq(const char *a, size_t alen, const char *b)
{
    if (alen != strlen(b)) return 0;
    for (size_t i = 0; i < alen; i++) {
        if (tolower((unsigned char)a[i]) != tolower((unsigned char)b[i])) return 0;
    }
    return 1;
}

/* Un composant est-il un temoin (ou un alias du temoin) ? Prive de ses points
   et espaces finaux (que Windows retire), compare sans tenir compte de la
   casse, sur toutes les plateformes. Alias : le nom court 8.3 de ".extracted"
   / ".extracting" ("EXTRAC~1", "EXTRAC~2"...), que les volumes NTFS et FAT
   resolvent vers le temoin. */
static int pk_comp_is_witness(const char *comp, size_t len)
{
    while (len > 0 && (comp[len - 1] == '.' || comp[len - 1] == ' ')) len--;
    if (pk_ieq(comp, len, PK_STAMP_NAME) || pk_ieq(comp, len, PK_BUSY_NAME)) return 1;
    if (len >= 8) {
        static const char alias[] = "extrac~";
        size_t i = 0;
        for (; i < sizeof(alias) - 1; i++) {
            if (tolower((unsigned char)comp[i]) != alias[i]) return 0;
        }
        size_t d = i;
        while (d < len && comp[d] >= '0' && comp[d] <= '9') d++;
        if (d > i && (d == len || comp[d] == '.')) return 1;
    }
    return 0;
}

/* Nom reserve aux temoins : un .pk ne doit pas pouvoir les ecrire ni les
   forger. Tous les composants sont controles (".extracted/x" creerait un
   dossier .extracted) : ".extracted.", ".EXTRACTED ", "sub/.extracting",
   "EXTRAC~1"... */
static int pk_name_is_witness(const char *name)
{
    const char *comp = name;
    for (const char *p = name; ; p++) {
        if (*p == '/' || *p == '\\' || *p == '\0') {
            if (pk_comp_is_witness(comp, (size_t)(p - comp))) return 1;
            if (*p == '\0') break;
            comp = p + 1;
        }
    }
    return 0;
}

#define PK_MANIFEST_MAX (8u * 1024u * 1024u)

/* Liste des fichiers extraits : ecrite au fil de l'eau dans .extracting
   (pour reprendre une purge apres interruption) ET gardee en memoire (c'est
   elle, et non une relecture du disque, qui alimente .extracted). */
typedef struct {
    FILE *f;
    char *buf;
    size_t len, cap;
    int overflow;     /* liste trop grosse : non conservee en memoire */
} pk_list_t;

static void pk_list_add(pk_list_t *l, const char *name)
{
    if (l->f) {
        fprintf(l->f, "%s\n", name);
        fflush(l->f);
    }
    if (l->overflow) return;
    size_t n = strlen(name) + 1;
    if (l->len + n + 1 > PK_MANIFEST_MAX) { l->overflow = 1; return; }
    if (l->len + n + 1 > l->cap) {
        size_t cap = l->cap ? l->cap * 2 : 4096;
        while (cap < l->len + n + 1) cap *= 2;
        char *nb = (char *)realloc(l->buf, cap);
        if (!nb) { l->overflow = 1; return; }
        l->buf = nb;
        l->cap = cap;
    }
    memcpy(l->buf + l->len, name, n - 1);
    l->buf[l->len + n - 1] = '\n';
    l->len += n;
    l->buf[l->len] = '\0';
}

/* Extraction ; si list != NULL, chaque fichier ecrit y est ajoute
   (un nom par ligne, '/' comme separateur).
   dry != 0 : passe a blanc, meme parcours et memes refus de noms et de
   tailles mais rien n'est ecrit (ni dossier, ni fichier) ; count = entrees
   qui auraient ete extraites. Elle ne peut PAS verifier : la longueur du
   chemin final, pk_path_is_inside (le dossier n'existe pas encore) ni
   l'ouverture reelle des fichiers (disque plein, droits, nom refuse par le
   systeme de fichiers).
   *has_main = 1 si "main.lua" aurait ete extrait (stocke, taille > 0). */
static int pk_extract_core(const char *pk_path, const char *out_dir,
                           int *complete, pk_list_t *list,
                           int dry, int *has_main)
{
    if (has_main) *has_main = 0;
    if (complete) *complete = 0;
    if (!pk_path || !out_dir || !out_dir[0]) return -1;

    FILE *f = fopen(pk_path, "rb");
    if (!f) return -1;

    long file_size = pk_file_size(f);
    if (file_size < 0) { fclose(f); return -1; }

    char name_buf[512];
    int count = 0;
    int done = 0;      /* 1 : fin normale (repertoire central ou fin d'archive) */
    int failed = 0;    /* 1 : une entree valide n'a pas pu etre ecrite */

    while (1) {
        zip_local_header_t hdr;
        uint32_t sig;
        if (fread(&sig, sizeof(sig), 1, f) != 1) {
            fprintf(stderr, "[PK] archive tronquee (pas de repertoire central) : %s\n",
                    pk_path);
            break;
        }
        if (sig == ZIP_CENTRAL_SIG || sig == ZIP_END_SIG) { done = 1; break; }
        if (sig != ZIP_LOCAL_SIG) {
            /* Signature inconnue : ce n'est pas une fin normale */
            fprintf(stderr, "[PK] signature inconnue 0x%08lx : extraction incomplete (%s)\n",
                    (unsigned long)sig, pk_path);
            break;
        }
        hdr.signature = sig;
        if (fread((char *)&hdr + sizeof(sig), sizeof(hdr) - sizeof(sig), 1, f) != 1) {
            fprintf(stderr, "[PK] en-tete tronque : extraction incomplete (%s)\n", pk_path);
            break;
        }

        size_t name_len = hdr.name_len;
        if (hdr.flags & ZIP_FLAG_DESCRIPTOR) {
            /* Les tailles de l'en-tete ne sont pas fiables (data descriptor) :
               impossible de delimiter l'entree, donc extraction incomplete */
            fprintf(stderr, "[PK] entree a data descriptor non prise en charge : "
                            "extraction incomplete (%s)\n", pk_path);
            failed = 1;
            break;
        }
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
        if (pk_name_is_witness(name_buf)) {
            fprintf(stderr, "[PK] entree refusee (nom reserve) : '%s'\n", name_buf);
            if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
            continue;
        }

        if (dry) {
            if (strcmp(name_buf, "main.lua") == 0 && has_main) *has_main = 1;
            count++;
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
            if (list) {
                /* nom relatif normalise ('/'), apres "out_dir/" */
                pk_list_add(list, out_path + strlen(out_dir) + 1);
            }
        } else {
            fprintf(stderr, "[PK] entree '%s' : ouverture impossible, extraction incomplete (%s)%s",
                    name_buf, pk_path, "\n");
            failed = 1;
            if (pk_skip(f, hdr.compressed_size, file_size) != 0) break;
        }
    }

    fclose(f);
    if (complete) *complete = (done && !failed) ? 1 : 0;
    return count;
}

int pk_extract_all_ex(const char *pk_path, const char *out_dir, int *complete)
{
    return pk_extract_core(pk_path, out_dir, complete, NULL, 0, NULL);
}

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

/* "dir/name" dans buf ; -1 si le resultat serait tronque (un chemin tronque
   designerait un autre fichier : on abandonne au lieu de s'en servir). Ne peut
   arriver que sous POSIX (PATH_MAX > 1024) : sous Windows, MAX_PATH est de 260. */
static int pk_join(char *buf, size_t sz, const char *dir, const char *name)
{
    int n = snprintf(buf, sz, "%s/%s", dir, name);
    return (n < 0 || (size_t)n >= sz) ? -1 : 0;
}

static int pk_file_exists(const char *dir, const char *name)
{
    char path[1024];
    struct stat st;
    if (pk_join(path, sizeof(path), dir, name) != 0) return 0;
    return stat(path, &st) == 0;
}

int pk_extract_state(const char *pk_path, const char *out_dir)
{
    if (!pk_path || !out_dir || !out_dir[0]) return PK_EXTRACT_STALE;

    if (!pk_file_exists(out_dir, "main.lua")) return PK_EXTRACT_STALE;
    if (pk_file_exists(out_dir, PK_BUSY_NAME)) return PK_EXTRACT_STALE;

    char path[1024];
    if (pk_join(path, sizeof(path), out_dir, PK_STAMP_NAME) != 0) return PK_EXTRACT_STALE;
    FILE *f = fopen(path, "rb");
    if (!f) return PK_EXTRACT_AUTHOR;   /* aucun temoin : dossier d'auteur */

    char want[64];
    if (pk_stamp(pk_path, want, sizeof(want)) != 0) {
        /* .pk absent ou illisible : l'extraction existante reste jouable,
           rien a re-extraire (et surtout rien a purger) */
        fclose(f);
        return PK_EXTRACT_CURRENT;
    }
    /* Premiere ligne seulement : la suite est la liste des fichiers extraits */
    char got[64];
    size_t n = fread(got, 1, sizeof(got) - 1, f);
    fclose(f);
    got[n] = '\0';
    char *eol = strpbrk(got, "\r\n");
    if (eol) *eol = '\0';
    return strcmp(got, want) == 0 ? PK_EXTRACT_CURRENT : PK_EXTRACT_STALE;
}

/* ------------------------------------------------------------------ */
/* Purge de l'ancienne extraction                                       */
/* ------------------------------------------------------------------ */

/* Contenu d'un petit fichier texte (malloc, NUL final), NULL si absent. */
static char *pk_read_text(const char *dir, const char *name)
{
    char path[1024];
    if (pk_join(path, sizeof(path), dir, name) != 0) return NULL;
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    long sz = pk_file_size(f);
    if (sz < 0 || (unsigned long)sz > PK_MANIFEST_MAX + 64) { fclose(f); return NULL; }
    char *buf = (char *)malloc((size_t)sz + 1);
    if (!buf) { fclose(f); return NULL; }
    size_t got = fread(buf, 1, (size_t)sz, f);
    fclose(f);
    buf[got] = '\0';
    return buf;
}

/* Classe un chemin sans suivre les liens : 0 absent, 1 fichier ordinaire,
   2 dossier ordinaire, -1 lien symbolique / jonction / autre. */
static int pk_classify(const char *path)
{
#ifdef _WIN32
    DWORD a = GetFileAttributesA(path);
    if (a == INVALID_FILE_ATTRIBUTES) return 0;
    if (a & FILE_ATTRIBUTE_REPARSE_POINT) return -1;
    return (a & FILE_ATTRIBUTE_DIRECTORY) ? 2 : 1;
#else
    struct stat st;
    if (lstat(path, &st) != 0) return 0;
    if (S_ISLNK(st.st_mode)) return -1;
    if (S_ISDIR(st.st_mode)) return 2;
    return S_ISREG(st.st_mode) ? 1 : -1;
#endif
}

static int pk_rmdir(const char *path)
{
#ifdef _WIN32
    return _rmdir(path);
#else
    return rmdir(path);
#endif
}

/* Supprime un fichier de l'ancienne extraction. Refuse tout ce qui n'est pas
   un fichier ordinaire atteint par des dossiers ordinaires sous out_dir. */
static void pk_purge_one(const char *out_dir, const char *name)
{
    if (!pk_name_is_safe(name) || pk_name_is_witness(name)) return;

    char path[1280];
    int n = snprintf(path, sizeof(path), "%s/%s", out_dir, name);
    if (n < 0 || (size_t)n >= sizeof(path)) return;
    for (char *q = path; *q; q++) {
        if (*q == '\\') *q = '/';
    }

    /* Chaque composant intermediaire doit etre un dossier ordinaire */
    size_t base = strlen(out_dir);
    for (size_t i = base + 1; path[i]; i++) {
        if (path[i] != '/' && path[i] != '\\') continue;
        char c = path[i];
        path[i] = '\0';
        int k = pk_classify(path);
        path[i] = c;
        if (k != 2) return;
    }
    if (pk_classify(path) != 1) return;
    if (!pk_path_is_inside(out_dir, path)) return;   /* meme regle qu'a l'extraction */
    if (remove(path) != 0) return;

    /* Retirer les dossiers devenus vides (rmdir echoue s'ils ne le sont pas) */
    for (size_t i = strlen(path); i > base + 1; i--) {
        if (path[i] != '/' && path[i] != '\\') continue;
        path[i] = '\0';
        if (pk_rmdir(path) != 0) break;
    }
}

/* Supprime les fichiers listes dans text (un nom par ligne). */
static void pk_purge_listed(const char *out_dir, char *text)
{
    /* out_dir lui-meme ne doit pas etre un lien */
    if (pk_classify(out_dir) != 2) return;
    char *line = text;
    while (line && *line) {
        char *eol = strpbrk(line, "\r\n");
        char *next = NULL;
        if (eol) {
            next = eol + 1;
            *eol = '\0';
        }
        if (line[0]) pk_purge_one(out_dir, line);
        line = next;
    }
}

int pk_extract_if_stale(const char *pk_path, const char *out_dir)
{
    if (!pk_path || !out_dir || !out_dir[0]) return -1;

    /* Chemins des temoins : abandon si le dossier est trop profond pour eux. */
    char path[1024];
    char busy[1024];
    if (pk_join(busy, sizeof(busy), out_dir, PK_BUSY_NAME) != 0 ||
        pk_join(path, sizeof(path), out_dir, PK_STAMP_NAME) != 0) {
        fprintf(stderr, "[PK] extraction annulee : chemin trop long : %s\n", out_dir);
        return -1;
    }
    if (pk_extract_state(pk_path, out_dir) != PK_EXTRACT_STALE) return 0;

    /* Dossier reconnu comme extraction du player (temoin present) : seul cas
       ou une purge aura lieu. */
    int recognized = pk_file_exists(out_dir, PK_STAMP_NAME) ||
                     pk_file_exists(out_dir, PK_BUSY_NAME);

    /* Avant toute lecture ou ecriture des temoins et avant la purge : passe a
       blanc du .pk (meme parcours que l'extraction, rien d'ecrit). Il doit
       s'ouvrir et donner un main.lua extractible ; si une extraction existe
       deja (donc une purge), il doit en plus etre parcouru jusqu'a une fin
       normale sans data descriptor ni signature inconnue : une archive
       tronquee ou forgee ne detruit pas une extraction valide. Sinon rien
       n'est touche (le dossier existant reste jouable tel quel). */
    char stamp[64];
    int complete_dry = 0, has_main = 0;
    if (pk_stamp(pk_path, stamp, sizeof(stamp)) != 0 ||
        pk_extract_core(pk_path, out_dir, &complete_dry, NULL, 1, &has_main) < 0 ||
        !has_main || (recognized && !complete_dry)) {
        fprintf(stderr, "[PK] %s illisible, incomplet ou sans main.lua : %s laisse tel quel\n",
                pk_path, out_dir);
        return -1;
    }

    /* Temoin present : lire la liste de l'ancienne extraction avant de
       toucher aux temoins. Sans temoin (dossier absent, dossier d'auteur
       incomplet) : rien a purger, jamais. */
    char *old_stamp = pk_read_text(out_dir, PK_STAMP_NAME);
    char *old_busy = pk_read_text(out_dir, PK_BUSY_NAME);

    /* Poser .extracting avant tout (sans le vider : il garde la liste de
       l'ancienne extraction tant qu'elle n'est pas purgee), puis retirer
       l'ancien temoin : une extraction interrompue ne doit pas laisser
       croire que le dossier est a jour, ni passer pour un dossier d'auteur */
    ensure_parent_dirs(busy);
    FILE *bf = fopen(busy, "ab");
    if (!bf) {
        fprintf(stderr, "[PK] extraction annulee : %s non inscriptible\n", out_dir);
        free(old_stamp);
        free(old_busy);
        return -1;
    }
    fclose(bf);

    if (recognized) {
        if (old_stamp) {
            char *rest = strchr(old_stamp, '\n');   /* apres la ligne d'empreinte */
            if (rest) pk_purge_listed(out_dir, rest + 1);
        }
        if (old_busy) pk_purge_listed(out_dir, old_busy);
    }
    free(old_stamp);
    free(old_busy);

    remove(path);
    /* Repart d'une liste vide pour cette extraction. Sous Windows, ecriture
       refusee aux autres ouvertures pendant l'extraction : une entree qui
       designerait .extracting par un autre nom echoue au lieu de l'ecraser. */
#ifdef _WIN32
    bf = _fsopen(busy, "wb", _SH_DENYWR);
#else
    bf = fopen(busy, "wb");
#endif
    if (!bf) {
        fprintf(stderr, "[PK] extraction annulee : %s non inscriptible\n", out_dir);
        return -1;
    }

    int complete = 0;
    pk_list_t list = { bf, NULL, 0, 0, 0 };
    int n = pk_extract_core(pk_path, out_dir, &complete, &list, 0, NULL);
    fclose(bf);
    if (n <= 0) { free(list.buf); return -1; }
    if (!complete) {
        free(list.buf);
        fprintf(stderr, "[PK] extraction incomplete : %s\n", pk_path);
        return n;
    }

    /* .extracted : empreinte puis liste des fichiers. .extracting n'est
       retire que si .extracted a vraiment ete ecrit. */
    int ok = 0;
    FILE *f = fopen(path, "wb");
    if (f) {
        int w = fprintf(f, "%s\n", stamp);
        if (w >= 0 && list.buf && !list.overflow && fputs(list.buf, f) < 0) w = -1;
        if (fclose(f) == 0 && w >= 0) ok = 1;
        else remove(path);
    }
    free(list.buf);
    if (!ok) {
        fprintf(stderr, "[PK] temoin %s non ecrit dans %s : %s laisse en place, "
                        "extraction a refaire\n", PK_STAMP_NAME, out_dir, PK_BUSY_NAME);
        return n;
    }
    remove(busy);   /* .extracted remplace .extracting */
    return n;
}
