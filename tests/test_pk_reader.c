/**
 * Tests de src/formats/pk_reader.c (#575) : programme autonome, sans SDL ni Lua.
 *
 *   mkdir -p build && cc -std=gnu11 -Wall -Wextra -Isrc/formats tests/test_pk_reader.c
 *       src/formats/pk_reader.c -o build/test_pk_reader && build/test_pk_reader
 *   (Windows : mkdir build, puis
 *    cl /Fobuild\ /Febuild\test_pk_reader.exe /I src\formats tests\test_pk_reader.c src\formats\pk_reader.c)
 * Les sorties vont sous build/ (ignore par git) : rien a la racine du depot.
 *
 * Compile et execute sous Linux par le job `pk-reader-posix` de
 * .github/workflows/player.yml : c'est la seule compilation du code POSIX de
 * pk_reader.c (realpath, lstat, rmdir, liens symboliques). Les tests qui ont
 * besoin de liens ou de chemins longs ne tournent que sous POSIX.
 * Les e2e de tests/e2e/e2e_pk_extract.py couvrent le meme module sous Windows
 * (jonctions, alias 8.3) a travers flam-player.
 * Code de sortie : 0 si tout passe, 1 sinon.
 */
#include "pk_reader.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#ifdef _WIN32
#include <direct.h>
#include <process.h>
#include <windows.h>
#else
#include <unistd.h>
#endif

static int g_fail = 0;
static int g_checks = 0;

#define CHECK(cond)                                                              \
    do {                                                                         \
        g_checks++;                                                              \
        if (!(cond)) {                                                           \
            g_fail++;                                                            \
            printf("[ECHEC] %s:%d : %s\n", __FILE__, __LINE__, #cond);           \
        }                                                                        \
    } while (0)

/* ------------------------------------------------------------------ */
/* Outils                                                              */
/* ------------------------------------------------------------------ */

static char g_root[512];

static int make_dir(const char *path)
{
#ifdef _WIN32
    return _mkdir(path);
#else
    return mkdir(path, 0755);
#endif
}

static void make_root(void)
{
#ifdef _WIN32
    char tmp[300];
    DWORD n = GetTempPathA((DWORD)sizeof(tmp), tmp);
    if (n == 0 || n >= sizeof(tmp)) { fprintf(stderr, "pas de dossier temporaire\n"); exit(2); }
    snprintf(g_root, sizeof(g_root), "%sflam-pkt-%d-%lu", tmp, (int)_getpid(),
             (unsigned long)GetTickCount());
    for (char *p = g_root; *p; p++) if (*p == '\\') *p = '/';
#else
    const char *t = getenv("TMPDIR");
    snprintf(g_root, sizeof(g_root), "%s/flam-pkt-%d", (t && t[0]) ? t : "/tmp", (int)getpid());
#endif
    if (make_dir(g_root) != 0) { fprintf(stderr, "mkdir %s impossible\n", g_root); exit(2); }
}

static void drop_root(void)
{
    char cmd[640];
#ifdef _WIN32
    char w[512];
    snprintf(w, sizeof(w), "%s", g_root);
    for (char *p = w; *p; p++) if (*p == '/') *p = '\\';
    snprintf(cmd, sizeof(cmd), "rmdir /s /q \"%s\"", w);
#else
    snprintf(cmd, sizeof(cmd), "rm -rf '%s'", g_root);
#endif
    if (system(cmd) != 0) { /* nettoyage au mieux */ }
}

/* Chemin sous g_root (tampon tournant de 16 : ne pas le garder au-dela de quelques appels,
   copier dans un tableau local pour un chemin reutilise) */
static char *path_of(const char *rel)
{
    static char bufs[16][1100];
    static int next = 0;
    char *b = bufs[next++ % 16];
    snprintf(b, sizeof(bufs[0]), "%s/%s", g_root, rel);
    return b;
}

static void write_file(const char *path, const void *data, size_t n)
{
    /* cree les dossiers intermediaires sous g_root */
    char tmp[1100];
    snprintf(tmp, sizeof(tmp), "%s", path);
    for (char *p = tmp + strlen(g_root) + 1; *p; p++) {
        if (*p == '/') { *p = '\0'; make_dir(tmp); *p = '/'; }
    }
    FILE *f = fopen(path, "wb");
    if (!f) { fprintf(stderr, "ecriture impossible : %s\n", path); exit(2); }
    if (n) fwrite(data, 1, n, f);
    fclose(f);
}

static void write_text(const char *path, const char *text)
{
    write_file(path, text, strlen(text));
}

/* Contenu (malloc) ou NULL si absent */
static char *read_text(const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    char *buf = (char *)malloc(1 << 16);
    size_t n = buf ? fread(buf, 1, (1 << 16) - 1, f) : 0;
    fclose(f);
    if (!buf) return NULL;
    buf[n] = '\0';
    return buf;
}

static int has_text(const char *path, const char *want)
{
    char *s = read_text(path);
    int ok = s && strcmp(s, want) == 0;
    free(s);
    return ok;
}

static int exists(const char *path)
{
    struct stat st;
    return stat(path, &st) == 0;
}

/* ------------------------------------------------------------------ */
/* Fabrication d'archives ZIP (stockees)                               */
/* ------------------------------------------------------------------ */

typedef struct {
    const char *name;
    const char *data;
    uint16_t flags;
    uint16_t method;
} zent_t;

static void put16(FILE *f, uint16_t v) { fputc(v & 0xFF, f); fputc(v >> 8, f); }
static void put32(FILE *f, uint32_t v) { put16(f, (uint16_t)(v & 0xFFFF)); put16(f, (uint16_t)(v >> 16)); }

/* with_end = 0 : pas de repertoire central (archive sans fin normale). */
static void make_zip(const char *path, const zent_t *e, int n, int with_end)
{
    FILE *f = fopen(path, "wb");
    if (!f) { fprintf(stderr, "ecriture impossible : %s\n", path); exit(2); }
    for (int i = 0; i < n; i++) {
        uint32_t sz = (uint32_t)strlen(e[i].data);
        put32(f, 0x04034b50);
        put16(f, 20);
        put16(f, e[i].flags);
        put16(f, e[i].method);
        put16(f, 0); put16(f, 0);      /* heure, date */
        put32(f, 0);                   /* crc32 : non verifie par le lecteur */
        put32(f, sz); put32(f, sz);
        put16(f, (uint16_t)strlen(e[i].name));
        put16(f, 0);
        fwrite(e[i].name, 1, strlen(e[i].name), f);
        fwrite(e[i].data, 1, sz, f);
    }
    if (with_end) {
        put32(f, 0x02014b50);          /* repertoire central : fin normale */
        for (int i = 0; i < 42; i++) fputc(0, f);
    }
    fclose(f);
}

static void truncate_file(const char *path, long keep)
{
    char *buf = (char *)malloc((size_t)keep);
    FILE *f = fopen(path, "rb");
    size_t got = f ? fread(buf, 1, (size_t)keep, f) : 0;
    if (f) fclose(f);
    write_file(path, buf, got);
    free(buf);
}

#define Z(arr) (arr), (int)(sizeof(arr) / sizeof((arr)[0]))

static const char MAIN_V1[] = "-- v1";

/* ------------------------------------------------------------------ */
/* Tests                                                               */
/* ------------------------------------------------------------------ */

static void test_names(void)
{
    puts("-- pk_name_is_safe");
    CHECK(!pk_name_is_safe(NULL));
    CHECK(!pk_name_is_safe(""));
    CHECK(pk_name_is_safe("main.lua"));
    CHECK(pk_name_is_safe("img/thumbnail.lif"));
    CHECK(pk_name_is_safe("a/b/c.txt"));
    CHECK(!pk_name_is_safe("/abs"));
    CHECK(!pk_name_is_safe("\\abs"));
    CHECK(!pk_name_is_safe("../x"));
    CHECK(!pk_name_is_safe("a/../../x"));
    CHECK(!pk_name_is_safe("a\\..\\..\\x"));
    CHECK(!pk_name_is_safe("./x"));
    CHECK(!pk_name_is_safe("a/./b"));
    CHECK(!pk_name_is_safe("a//b"));
    CHECK(!pk_name_is_safe("C:/x"));
    CHECK(!pk_name_is_safe("a:b"));
    CHECK(!pk_name_is_safe("a."));
    CHECK(!pk_name_is_safe("a "));
    CHECK(!pk_name_is_safe(".. "));
    CHECK(!pk_name_is_safe("a?b"));
    CHECK(!pk_name_is_safe("a*b"));
    CHECK(!pk_name_is_safe("a\x01" "b"));
    CHECK(!pk_name_is_safe("CON"));
    CHECK(!pk_name_is_safe("nul.txt"));
    CHECK(!pk_name_is_safe("sub/aux"));
    CHECK(!pk_name_is_safe("COM1"));
    CHECK(!pk_name_is_safe("lpt9.log"));
    CHECK(pk_name_is_safe("com10"));
    CHECK(pk_name_is_safe("console.txt"));
}

static void test_read_entry(void)
{
    puts("-- pk_read_entry / pk_has_entry");
    zent_t z[] = { { "main.lua", MAIN_V1, 0, 0 }, { "info.plain", "titre", 0, 0 } };
    make_zip(path_of("read.pk"), z, 2, 1);
    size_t sz = 0;
    char *d = (char *)pk_read_entry(path_of("read.pk"), "info.plain", &sz);
    CHECK(d && sz == 5 && strcmp(d, "titre") == 0);
    free(d);
    CHECK(pk_has_entry(path_of("read.pk"), "main.lua"));
    CHECK(!pk_has_entry(path_of("read.pk"), "absent"));
    CHECK(pk_read_entry(path_of("absent.pk"), "main.lua", &sz) == NULL && sz == 0);
    CHECK(pk_read_entry(NULL, "x", NULL) == NULL);
}

static void test_zip_slip(void)
{
    puts("-- entrees dangereuses : rien hors du dossier, temoins non forges");
    make_dir(path_of("slip"));
    zent_t z[] = {
        { "main.lua", MAIN_V1, 0, 0 },
        { "../evil.txt", "E", 0, 0 },
        { "sub/../../evil2.txt", "E", 0, 0 },
        { "/abs-evil.txt", "E", 0, 0 },
        { "C:/evil3.txt", "E", 0, 0 },
        { ".extracting", "FORGE", 0, 0 },
        { ".EXTRACTED.", "FORGE", 0, 0 },
        { "EXTRAC~1", "FORGE", 0, 0 },
        { "CON", "FORGE", 0, 0 },
        { "ok.txt", "ok", 0, 0 },
    };
    make_zip(path_of("slip/s.plain.pk"), z, 10, 1);
    int complete = 0;
    int n = pk_extract_all_ex(path_of("slip/s.plain.pk"), path_of("slip/s.plain"), &complete);
    CHECK(n == 2 && complete == 1);   /* main.lua et ok.txt */
    CHECK(!exists(path_of("slip/evil.txt")) && !exists(path_of("slip/evil2.txt")) &&
          !exists(path_of("evil2.txt")));
    CHECK(!exists(path_of("slip/s.plain/EXTRAC~1")) && !exists(path_of("slip/s.plain/CON")));
    CHECK(!exists(path_of("slip/s.plain/.extracting")));
    CHECK(has_text(path_of("slip/s.plain/ok.txt"), "ok"));

    /* Meme chose par pk_extract_if_stale : le temoin est celui du player */
    CHECK(pk_extract_if_stale(path_of("slip/s.plain.pk"), path_of("slip/t.plain")) == 2);
    char *st = read_text(path_of("slip/t.plain/.extracted"));
    CHECK(st && strstr(st, "\nmain.lua\nok.txt\n") != NULL && !strstr(st, "FORGE"));
    free(st);
}

static void test_extract_and_purge(void)
{
    puts("-- extraction, etat, purge de l'ancienne version");
    char pk[1100];
    snprintf(pk, sizeof(pk), "%s/%s", g_root, "p/foo.plain.pk");
    char dir[1100];
    snprintf(dir, sizeof(dir), "%s/%s", g_root, "p/foo.plain");
    make_dir(path_of("p"));
    zent_t v1[] = { { "main.lua", MAIN_V1, 0, 0 }, { "old.txt", "v1", 0, 0 },
                    { "sub/old2.txt", "v1", 0, 0 }, { "keep/shared.txt", "v1", 0, 0 } };
    make_zip(pk, v1, 4, 1);
    CHECK(pk_extract_state(pk, dir) == PK_EXTRACT_STALE);
    CHECK(pk_extract_if_stale(pk, dir) == 4);
    CHECK(has_text(path_of("p/foo.plain/old.txt"), "v1"));
    CHECK(exists(path_of("p/foo.plain/.extracted")) && !exists(path_of("p/foo.plain/.extracting")));
    CHECK(pk_extract_state(pk, dir) == PK_EXTRACT_CURRENT);
    CHECK(pk_extract_if_stale(pk, dir) == 0);   /* a jour : rien a faire */

    write_text(path_of("p/foo.plain/mine.txt"), "a moi");
    zent_t v2[] = { { "main.lua", "-- v2 plus long", 0, 0 }, { "keep/shared.txt", "v2", 0, 0 } };
    make_zip(pk, v2, 2, 1);
    CHECK(pk_extract_state(pk, dir) == PK_EXTRACT_STALE);
    CHECK(pk_extract_if_stale(pk, dir) == 2);
    CHECK(!exists(path_of("p/foo.plain/old.txt")));
    CHECK(!exists(path_of("p/foo.plain/sub")));
    CHECK(has_text(path_of("p/foo.plain/keep/shared.txt"), "v2"));
    CHECK(has_text(path_of("p/foo.plain/mine.txt"), "a moi"));
    CHECK(pk_extract_state(pk, dir) == PK_EXTRACT_CURRENT);
}

static void test_states(void)
{
    puts("-- dossier d'auteur, .extracted sans main.lua, .extracting");
    make_dir(path_of("s"));
    zent_t z[] = { { "main.lua", MAIN_V1, 0, 0 } };
    char pk[1100];
    snprintf(pk, sizeof(pk), "%s/%s", g_root, "s/a.plain.pk");
    make_zip(pk, z, 1, 1);

    /* Dossier d'auteur : jamais touche, meme avec un .pk plus recent */
    write_text(path_of("s/a.plain/main.lua"), "AUTEUR");
    write_text(path_of("s/a.plain/notes.txt"), "a moi");
    CHECK(pk_extract_state(pk, path_of("s/a.plain")) == PK_EXTRACT_AUTHOR);
    CHECK(pk_extract_if_stale(pk, path_of("s/a.plain")) == 0);
    CHECK(has_text(path_of("s/a.plain/main.lua"), "AUTEUR"));
    CHECK(!exists(path_of("s/a.plain/.extracted")) && !exists(path_of("s/a.plain/.extracting")));

    /* .extracted sans main.lua : a (re)extraire */
    write_text(path_of("s/b.plain/.extracted"), "0 0\n");
    CHECK(pk_extract_state(pk, path_of("s/b.plain")) == PK_EXTRACT_STALE);
    CHECK(pk_extract_if_stale(pk, path_of("s/b.plain")) == 1);
    CHECK(has_text(path_of("s/b.plain/main.lua"), MAIN_V1));
    CHECK(pk_extract_state(pk, path_of("s/b.plain")) == PK_EXTRACT_CURRENT);

    /* .extracting present : extraction interrompue, a refaire */
    write_text(path_of("s/c.plain/main.lua"), "PARTIEL");
    write_text(path_of("s/c.plain/.extracting"), "");
    CHECK(pk_extract_state(pk, path_of("s/c.plain")) == PK_EXTRACT_STALE);
    CHECK(pk_extract_if_stale(pk, path_of("s/c.plain")) == 1);
    CHECK(has_text(path_of("s/c.plain/main.lua"), MAIN_V1));
    CHECK(!exists(path_of("s/c.plain/.extracting")) && exists(path_of("s/c.plain/.extracted")));

    /* .pk absent : l'extraction existante reste jouable (CURRENT), rien a purger */
    CHECK(pk_extract_state(path_of("s/absent.pk"), path_of("s/c.plain")) == PK_EXTRACT_CURRENT);
    CHECK(pk_extract_if_stale(path_of("s/absent.pk"), path_of("s/c.plain")) == 0);
}

static void test_stamp_not_written(void)
{
    puts("-- .extracted impossible a ecrire : .extracting reste");
    make_dir(path_of("w"));
    zent_t z[] = { { "main.lua", MAIN_V1, 0, 0 } };
    char pk[1100];
    snprintf(pk, sizeof(pk), "%s/%s", g_root, "w/a.plain.pk");
    make_zip(pk, z, 1, 1);
    /* .extracted est un dossier non vide : ni remove() ni fopen("wb") n'y arrivent */
    write_text(path_of("w/a.plain/.extracted/x"), "x");
    int n = pk_extract_if_stale(pk, path_of("w/a.plain"));
    CHECK(n == 1);                                    /* extraite, mais non datee */
    CHECK(exists(path_of("w/a.plain/.extracting")));  /* l'extraction reste a refaire */
    CHECK(pk_extract_state(pk, path_of("w/a.plain")) == PK_EXTRACT_STALE);
}

static void test_dry_run_refusals(void)
{
    puts("-- passe a blanc : une archive douteuse ne detruit pas l'extraction");
    make_dir(path_of("d"));
    char pk[1100];
    snprintf(pk, sizeof(pk), "%s/%s", g_root, "d/a.plain.pk");
    char dir[1100];
    snprintf(dir, sizeof(dir), "%s/%s", g_root, "d/a.plain");
    zent_t v1[] = { { "main.lua", MAIN_V1, 0, 0 }, { "old.txt", "v1", 0, 0 },
                    { "data.bin", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", 0, 0 } };
    make_zip(pk, v1, 3, 1);
    CHECK(pk_extract_if_stale(pk, dir) == 3);
    char *before = read_text(path_of("d/a.plain/.extracted"));
    CHECK(before != NULL);

    /* tronque dans data.bin */
    zent_t v2[] = { { "main.lua", "-- v2 !", 0, 0 },
                    { "data.bin", "yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy", 0, 0 } };
    make_zip(pk, v2, 2, 1);
    truncate_file(pk, 30 + 8 + 7 + 30 + 8 + 10);
    CHECK(pk_extract_if_stale(pk, dir) == -1);
    CHECK(has_text(path_of("d/a.plain/main.lua"), MAIN_V1) && exists(path_of("d/a.plain/old.txt")));
    char *after = read_text(path_of("d/a.plain/.extracted"));
    CHECK(after && strcmp(before, after) == 0 && !exists(path_of("d/a.plain/.extracting")));
    free(after);

    /* data descriptor */
    zent_t v3[] = { { "main.lua", "-- v3 !!", 0, 0 }, { "data.bin", "z", 0x0008, 0 } };
    make_zip(pk, v3, 2, 1);
    CHECK(pk_extract_if_stale(pk, dir) == -1);
    CHECK(has_text(path_of("d/a.plain/main.lua"), MAIN_V1) && exists(path_of("d/a.plain/old.txt")));

    /* main.lua de taille 0, puis compresse */
    zent_t v4[] = { { "main.lua", "", 0, 0 }, { "data.bin", "z", 0, 0 } };
    make_zip(pk, v4, 2, 1);
    CHECK(pk_extract_if_stale(pk, dir) == -1);
    zent_t v5[] = { { "main.lua", "-- v5 compresse", 0, 8 } };
    make_zip(pk, v5, 1, 1);
    CHECK(pk_extract_if_stale(pk, dir) == -1);

    /* sans fin normale (ni repertoire central ni fin d'archive) */
    zent_t v6[] = { { "main.lua", "-- v6 sans fin", 0, 0 } };
    make_zip(pk, v6, 1, 0);
    CHECK(pk_extract_if_stale(pk, dir) == -1);

    CHECK(has_text(path_of("d/a.plain/main.lua"), MAIN_V1) && exists(path_of("d/a.plain/old.txt")));
    after = read_text(path_of("d/a.plain/.extracted"));
    CHECK(after && strcmp(before, after) == 0 && !exists(path_of("d/a.plain/.extracting")));
    free(after);
    free(before);

    /* Sans extraction existante : sans fin normale, extraite mais incomplete */
    int complete = 1;
    CHECK(pk_extract_all_ex(pk, path_of("d/neuf"), &complete) == 1 && complete == 0);
    CHECK(pk_extract_if_stale(pk, path_of("d/neuf2")) == 1);
    CHECK(exists(path_of("d/neuf2/.extracting")) && !exists(path_of("d/neuf2/.extracted")));
}

#ifndef _WIN32
static void test_links_posix(void)
{
    puts("-- liens symboliques : la purge ne suit rien (POSIX)");
    make_dir(path_of("l"));
    char pk[1100];
    snprintf(pk, sizeof(pk), "%s/%s", g_root, "l/a.plain.pk");

    /* a) dossier intermediaire remplace par un lien vers l'exterieur */
    zent_t v1[] = { { "main.lua", MAIN_V1, 0, 0 }, { "sub/old2.txt", "v1", 0, 0 },
                    { "f.txt", "f1", 0, 0 } };
    make_zip(pk, v1, 3, 1);
    CHECK(pk_extract_if_stale(pk, path_of("l/a.plain")) == 3);
    write_text(path_of("l/dehors/old2.txt"), "precieux");
    write_text(path_of("l/dehors2.txt"), "precieux2");
    remove(path_of("l/a.plain/sub/old2.txt"));
    rmdir(path_of("l/a.plain/sub"));
    CHECK(symlink(path_of("l/dehors"), path_of("l/a.plain/sub")) == 0);
    /* c) fichier final remplace par un lien vers un fichier exterieur */
    remove(path_of("l/a.plain/f.txt"));
    CHECK(symlink(path_of("l/dehors2.txt"), path_of("l/a.plain/f.txt")) == 0);
    zent_t v2[] = { { "main.lua", "-- v2 plus long", 0, 0 } };
    make_zip(pk, v2, 1, 1);
    CHECK(pk_extract_if_stale(pk, path_of("l/a.plain")) == 1);
    CHECK(has_text(path_of("l/dehors/old2.txt"), "precieux"));
    CHECK(has_text(path_of("l/dehors2.txt"), "precieux2"));
    struct stat st;
    CHECK(lstat(path_of("l/a.plain/sub"), &st) == 0 && S_ISLNK(st.st_mode));
    CHECK(lstat(path_of("l/a.plain/f.txt"), &st) == 0 && S_ISLNK(st.st_mode));

    /* b) le dossier d'extraction lui-meme est un lien : rien n'est purge */
    make_zip(pk, v1, 3, 1);
    CHECK(pk_extract_if_stale(pk, path_of("l/reel.plain")) == 3);
    CHECK(symlink(path_of("l/reel.plain"), path_of("l/lien.plain")) == 0);
    CHECK(pk_extract_state(pk, path_of("l/lien.plain")) == PK_EXTRACT_CURRENT);
    make_zip(pk, v2, 1, 1);
    CHECK(pk_extract_state(pk, path_of("l/lien.plain")) == PK_EXTRACT_STALE);
    CHECK(pk_extract_if_stale(pk, path_of("l/lien.plain")) == 1);
    CHECK(exists(path_of("l/reel.plain/sub/old2.txt")));   /* ancienne extraction non purgee */
    CHECK(has_text(path_of("l/reel.plain/main.lua"), "-- v2 plus long"));
}

static void test_long_path_posix(void)
{
    puts("-- chemin trop long pour les temoins : abandon sans rien ecrire (POSIX)");
    make_dir(path_of("long"));
    char rel[1100] = "long";
    char comp[220];
    memset(comp, 'a', 200);
    comp[200] = '\0';
    for (int i = 0; i < 5; i++) {
        strcat(rel, "/");
        strcat(rel, comp);
        make_dir(path_of(rel));
    }
    zent_t z[] = { { "main.lua", MAIN_V1, 0, 0 } };
    make_zip(path_of("long/a.pk"), z, 1, 1);
    char dir[1700];
    int dn = snprintf(dir, sizeof(dir), "%s/%s", g_root, rel);   /* > 1024 octets */
    CHECK(dn > 1024 && dn < (int)sizeof(dir));
    CHECK(pk_extract_if_stale(path_of("long/a.pk"), dir) == -1);
    char probe[1800];
    snprintf(probe, sizeof(probe), "%s/main.lua", dir);
    CHECK(!exists(probe));
    snprintf(probe, sizeof(probe), "%s/.extracting", dir);
    CHECK(!exists(probe));
    snprintf(probe, sizeof(probe), "%s/.extracted", dir);
    CHECK(!exists(probe));
    /* Aucun fichier parasite : ni dans le dossier, ni dans son parent, qui ne doit contenir
       que le sous-dossier attendu (l'ancien code y creait un temoin au nom tronque). */
    char cmd[1900], parent[1700];
    snprintf(cmd, sizeof(cmd), "test -z \"$(ls -A '%s')\"", dir);
    CHECK(system(cmd) == 0);
    snprintf(parent, sizeof(parent), "%s", dir);
    char *slash = strrchr(parent, '/');
    CHECK(slash != NULL);
    if (slash) {
        *slash = '\0';
        snprintf(cmd, sizeof(cmd), "test \"$(ls -A '%s')\" = '%s'", parent, comp);
        CHECK(system(cmd) == 0);
    }
}

/* Dossier de sortie de longueur exacte len, sous g_root (composants de 200 octets au plus). */
static void make_dir_of_len(char *out, size_t sz, int tag, size_t len)
{
    snprintf(out, sz, "%s/lg%d", g_root, tag);
    make_dir(out);
    while (strlen(out) < len) {
        size_t r = len - strlen(out);          /* r >= 2 : "/" et au moins une lettre */
        size_t chunk = r - 1 > 200 ? 200 : r - 1;
        if (r - 1 > 200 && r - 1 - 200 < 2) chunk = 199;   /* ne pas laisser 1 octet */
        size_t at = strlen(out);
        out[at] = '/';
        memset(out + at + 1, 'b', chunk);
        out[at + 1 + chunk] = '\0';
        make_dir(out);
    }
}

static void test_state_near_limit_posix(void)
{
    puts("-- etat : un temoin consulte dont le chemin deborde n'est pas 'absent' (POSIX)");
    zent_t z[] = { { "main.lua", MAIN_V1, 0, 0 } };
    char pk[1100];
    snprintf(pk, sizeof(pk), "%s/near.pk", g_root);
    make_zip(pk, z, 1, 1);
    /* ".extracting" deborde des 1012 octets (1024 - 12), ".extracted" des 1013 : a 1012
       exactement, seul .extracting deborde et .extracted tient. */
    for (size_t len = 1006; len <= 1018; len++) {
        char dir[1700], f[1800];
        make_dir_of_len(dir, sizeof(dir), (int)len, len);
        CHECK(strlen(dir) == len);
        snprintf(f, sizeof(f), "%s/main.lua", dir);
        write_text(f, "PARTIEL");
        snprintf(f, sizeof(f), "%s/.extracting", dir);
        write_text(f, "");
        /* extraction interrompue : jamais un dossier d'auteur, quelle que soit la longueur */
        CHECK(pk_extract_state(pk, dir) == PK_EXTRACT_STALE);
    }
}
#endif

int main(void)
{
    make_root();
    test_names();
    test_read_entry();
    test_zip_slip();
    test_extract_and_purge();
    test_states();
    test_stamp_not_written();
    test_dry_run_refusals();
#ifndef _WIN32
    test_links_posix();
    test_long_path_posix();
    test_state_near_limit_posix();
#endif
    drop_root();
    printf("test_pk_reader : %d verifications, %d echec(s)\n", g_checks, g_fail);
    return g_fail ? 1 : 0;
}
