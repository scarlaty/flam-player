/**
 * main.c — Point d'entree du Flam Player
 *
 * Etape 6 : Chargement d'histoire .plain
 * Usage : flam-player <chemin/vers/histoire.plain>
 *         flam-player <script.lua> [--img-dir ...] [--sounds-dir ...] [--save-dir ...]
 * Options : --strict        require() limite a script/ (comme le device)
 *           --watchdog <ms> delai max d'un script Lua sans rendre la main
 *                           (defaut 10000, 0 = desactive)
 *           --screenshot <chemin> fichier BMP des captures (touche S et
 *                           capture auto), prioritaire sur FLAM_SCREENSHOT
 */

#include "SDL.h"
#include "lvgl/lvgl.h"
#include "platform/sdl_driver.h"
#include "platform/sdl_audio.h"
#include "bindings/lua_lv.h"
#include "firmware/fw_globals.h"
#include "formats/lif_decoder.h"
#include "formats/pk_reader.h"

#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <stdlib.h>

#ifdef _WIN32
#include <windows.h>   /* FindFirstFile / FindNextFile */
#include <shlobj.h>    /* SHBrowseForFolder */
#endif

#ifdef _MSC_VER
/* Une fonction appelee sans prototype est une erreur, pas un warning */
#pragma warning(error: 4013)
#endif

/* Prototypes (fonctions definies plus bas, utilisees avant) */
static int is_story_dir(const char *path);
static int load_story(const char *story_dir);
#ifdef _WIN32
static char *wide_to_acp_exact(const wchar_t *w);
static char *path_acp_by_components(const wchar_t *w);
#endif

/* Etat global Lua */
static lua_State *g_lua = NULL;

/* Mode strict (--strict) : require() limite a script/, comme le device */
static int g_strict = 0;

/* Taille max d'un script Lua charge (main.lua ou module) */
#define MAX_SCRIPT_SIZE (16L * 1024L * 1024L)

/* ------------------------------------------------------------------ */
/* Watchdog Lua : interrompt un script qui ne rend pas la main          */
/* (boucle infinie) au lieu de figer le simulateur.                     */
/* Le compteur est rearme a chaque tour de boucle principale.           */
/* ------------------------------------------------------------------ */

#define WATCHDOG_DEFAULT_MS  10000u
#define WATCHDOG_HOOK_COUNT  100000

static uint32_t g_watchdog_ms    = WATCHDOG_DEFAULT_MS;  /* 0 = desactive */
static uint32_t g_watchdog_start = 0;

static void watchdog_rearm(void)
{
    g_watchdog_start = SDL_GetTicks();
}

static void watchdog_hook(lua_State *L, lua_Debug *ar)
{
    (void)ar;
    if (g_watchdog_ms == 0) return;
    uint32_t elapsed = SDL_GetTicks() - g_watchdog_start;
    if (elapsed > g_watchdog_ms) {
        /* Rearmer pour ne pas tuer aussi le code qui gere l'erreur */
        watchdog_rearm();
        luaL_error(L, "watchdog: script Lua bloque depuis plus de %d ms "
                      "(boucle infinie ?). Option --watchdog 0 pour desactiver.",
                   (int)elapsed);
    }
}

/* Gestionnaire d'erreur commun : ajoute la pile d'appels Lua au message */
static int lua_traceback_msgh(lua_State *L)
{
    const char *msg = lua_tostring(L, 1);
    if (!msg) {
        if (luaL_callmeta(L, 1, "__tostring") && lua_type(L, -1) == LUA_TSTRING)
            return 1;
        msg = lua_pushfstring(L, "(objet d'erreur de type %s)", luaL_typename(L, 1));
    }
    luaL_traceback(L, L, msg, 1);
    return 1;
}

/* lua_pcall avec traceback : la fonction et ses nargs arguments sont au sommet */
static int pcall_traceback(lua_State *L, int nargs, int nresults)
{
    int base = lua_gettop(L) - nargs;  /* index de la fonction */
    lua_pushcfunction(L, lua_traceback_msgh);
    lua_insert(L, base);
    int status = lua_pcall(L, nargs, nresults, base);
    lua_remove(L, base);
    return status;
}

/**
 * Lit un fichier entier en memoire (malloc). Retourne NULL si erreur.
 * Verifie ftell, malloc et le nombre d'octets lus.
 */
static char *read_whole_file(FILE *f, long *out_size)
{
    long fsize = -1;
    if (fseek(f, 0, SEEK_END) == 0) fsize = ftell(f);
    if (fsize < 0 || fsize > MAX_SCRIPT_SIZE || fseek(f, 0, SEEK_SET) != 0) return NULL;
    char *buf = (char *)malloc(fsize > 0 ? (size_t)fsize : 1);
    if (!buf) return NULL;
    if (fread(buf, 1, (size_t)fsize, f) != (size_t)fsize) {
        free(buf);
        return NULL;
    }
    *out_size = fsize;
    return buf;
}

/* Groupe de focus principal (= "document" dans le firmware) */
static lv_group_t *g_focus_group = NULL;

/* Header bar widgets */
static lv_obj_t  *g_header       = NULL;
static lv_obj_t  *g_header_title = NULL;

extern lv_font_t nunito_bold_12;

/**
 * Initialise le runtime Lua et enregistre les bindings.
 */
static int init_lua(void)
{
    g_lua = luaL_newstate();
    if (!g_lua) {
        fprintf(stderr, "Erreur: impossible de creer l'etat Lua\n");
        return -1;
    }

    /* Bibliotheques standard Lua */
    luaL_openlibs(g_lua);

    /* Watchdog anti boucle infinie */
    watchdog_rearm();
    if (g_watchdog_ms > 0) {
        lua_sethook(g_lua, watchdog_hook, LUA_MASKCOUNT, WATCHDOG_HOOK_COUNT);
    }

    /* Enregistrer les bindings lv.* */
    luaopen_lv(g_lua);

    /* Injecter les globales firmware :
       - window   = ecran LVGL actif
       - document = focus group principal (pour la navigation encodeur)
    */
    /* Ecran noir par defaut (comme le firmware Flam) */
    lv_obj_set_style_bg_color(lv_scr_act(), lv_color_black(), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(lv_scr_act(), LV_OPA_COVER, LV_PART_MAIN);

    /* ---- Header bar (28px, like real Flam firmware) ---- */
    g_header = lv_obj_create(lv_scr_act());
    lv_obj_remove_style_all(g_header);
    lv_obj_set_pos(g_header, 0, 0);
    lv_obj_set_size(g_header, FLAM_SCREEN_W, 28);
    lv_obj_set_style_bg_color(g_header, lv_color_hex(0x0D1117), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(g_header, LV_OPA_COVER, LV_PART_MAIN);
    lv_obj_set_style_pad_left(g_header, 8, LV_PART_MAIN);
    lv_obj_set_style_pad_right(g_header, 8, LV_PART_MAIN);
    lv_obj_clear_flag(g_header, LV_OBJ_FLAG_SCROLLABLE);

    /* Story title (centered) */
    g_header_title = lv_label_create(g_header);
    lv_label_set_text(g_header_title, "Flam Player");
    lv_label_set_long_mode(g_header_title, LV_LABEL_LONG_DOT);
    lv_obj_set_width(g_header_title, FLAM_SCREEN_W - 16);
    lv_obj_set_style_text_color(g_header_title, lv_color_hex(0xE0E4E8), LV_PART_MAIN);
    lv_obj_set_style_text_font(g_header_title, &nunito_bold_12, LV_PART_MAIN);
    lv_obj_set_style_text_align(g_header_title, LV_TEXT_ALIGN_CENTER, LV_PART_MAIN);
    lv_obj_align(g_header_title, LV_ALIGN_CENTER, 0, 0);

    /* ---- Content window below header ---- */
    lv_obj_t *content_window = lv_obj_create(lv_scr_act());
    lv_obj_remove_style_all(content_window);
    lv_obj_set_pos(content_window, 0, 28);
    lv_obj_set_size(content_window, FLAM_SCREEN_W, FLAM_SCREEN_H - 28);
    lv_obj_set_style_bg_color(content_window, lv_color_black(), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(content_window, LV_OPA_COVER, LV_PART_MAIN);

    lua_lv_push_obj(g_lua, content_window);
    lua_setglobal(g_lua, "window");

    g_focus_group = lv_group_create();
    lv_group_set_default(g_focus_group);
    lua_lv_push_group(g_lua, g_focus_group);
    lua_setglobal(g_lua, "document");

    /* Associer le focus group a l'input device */
    extern lv_indev_t *g_indev;  /* defini dans sdl_driver.c */
    if (g_indev) {
        lv_indev_set_group(g_indev, g_focus_group);
    }

    /* Enregistrer l'API audio Lua */
    sdl_audio_register_lua(g_lua);

    /* Enregistrer les objets firmware (state, progression, context_menu, etc.) */
    fw_register_globals(g_lua);

    return 0;
}

/**
 * Custom Lua searcher qui charge les fichiers en strippant les trailing null bytes.
 * Cherche dans script/ et a la racine de l'histoire (script/ seul en --strict).
 * Seul le source texte est accepte (mode "t") : pas de bytecode precompile.
 */
static char g_story_dir[1024] = "";

static int custom_lua_searcher(lua_State *L)
{
    const char *modname = luaL_checkstring(L, 1);
    char path[1024];
    const char *dirs[] = { "script", "." };
    int ndirs = g_strict ? 1 : 2;

    /* Pas de sortie du dossier de l'histoire : "..", chemin absolu,
       lecteur "C:" ou octet nul dans le nom de module sont refuses */
    size_t modlen = 0;
    lua_tolstring(L, 1, &modlen);
    if (modlen != strlen(modname) || modname[0] == '/' || modname[0] == '\\' ||
        strchr(modname, ':') || strstr(modname, "..")) {
        lua_pushfstring(L, "\n\tmodule '%s' refuse (chemin hors de l'histoire)",
                        modname);
        return 1;
    }

    for (int d = 0; d < ndirs; d++) {
        int n = snprintf(path, sizeof(path), "%s/%s/%s.lua", g_story_dir, dirs[d], modname);
        if (n < 0 || (size_t)n >= sizeof(path)) continue;  /* tronque : ignore */
        /* Normaliser les slashes */
        for (char *p = path; *p; p++) {
            if (*p == '\\') *p = '/';
        }

        FILE *f = fopen(path, "rb");
        if (!f) continue;

        long fsize = 0;
        char *buf = read_whole_file(f, &fsize);
        fclose(f);
        if (!buf) {
            return luaL_error(L, "impossible de lire le module '%s'", path);
        }

        /* Strip trailing null bytes */
        while (fsize > 0 && buf[fsize - 1] == '\0') fsize--;

        /* "@chemin" : messages d'erreur et traceback avec le nom du fichier */
        char chunkname[1100];
        snprintf(chunkname, sizeof(chunkname), "@%s", path);
        int err = luaL_loadbufferx(L, buf, (size_t)fsize, chunkname, "t");
        free(buf);
        if (err != LUA_OK) {
            return lua_error(L);
        }
        return 1;  /* retourner la fonction chargee */
    }

    if (g_strict) {
        lua_pushfstring(L, "\n\tno file '%s/script/%s.lua' (--strict)",
                        g_story_dir, modname);
    } else {
        lua_pushfstring(L, "\n\tno file '%s/script/%s.lua'\n\tno file '%s/%s.lua'",
                        g_story_dir, modname, g_story_dir, modname);
    }
    return 1;  /* retourner le message d'erreur */
}

/**
 * Configure le require() Lua pour chercher dans le dossier de l'histoire.
 */
static void set_lua_package_path(const char *story_dir)
{
    strncpy(g_story_dir, story_dir, sizeof(g_story_dir) - 1);
    g_story_dir[sizeof(g_story_dir) - 1] = '\0';

    /* Inserer notre searcher en position 2 (avant le searcher fichier par defaut) */
    lua_getglobal(g_lua, "package");
    lua_getfield(g_lua, -1, "searchers");

    /* Decaler les searchers existants d'une position */
    int len = (int)lua_rawlen(g_lua, -1);
    for (int i = len; i >= 2; i--) {
        lua_rawgeti(g_lua, -1, i);
        lua_rawseti(g_lua, -2, i + 1);
    }

    /* Inserer notre searcher en position 2 */
    lua_pushcfunction(g_lua, custom_lua_searcher);
    lua_rawseti(g_lua, -2, 2);

    /* Mode strict : ne garder que preload + notre searcher (pas de
       recherche via package.path/cpath depuis le dossier courant) */
    if (g_strict) {
        int n = (int)lua_rawlen(g_lua, -1);
        for (int i = n; i >= 3; i--) {
            lua_pushnil(g_lua);
            lua_rawseti(g_lua, -2, i);
        }
    }

    lua_pop(g_lua, 2); /* pop searchers + package */
}

/**
 * Charge et execute un script Lua. Appelle setup() si elle existe.
 * Gere les fichiers avec des trailing null bytes (courant dans les .plain).
 */
static int load_script(const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "Erreur: impossible d'ouvrir '%s'\n", path);
        return -1;
    }
    long fsize = 0;
    char *buf = read_whole_file(f, &fsize);
    fclose(f);
    if (!buf) {
        fprintf(stderr, "Erreur: lecture impossible de '%s'\n", path);
        return -1;
    }

    /* Retirer les trailing null bytes */
    while (fsize > 0 && buf[fsize - 1] == '\0') fsize--;

    /* Source texte uniquement : le bytecode precompile est refuse */
    char chunkname[1100];
    snprintf(chunkname, sizeof(chunkname), "@%s", path);
    int err = luaL_loadbufferx(g_lua, buf, (size_t)fsize, chunkname, "t");
    free(buf);
    if (err != LUA_OK) {
        const char *errmsg = lua_tostring(g_lua, -1);
        fprintf(stderr, "Erreur Lua: %s\n", errmsg ? errmsg : "erreur inconnue");
        lua_pop(g_lua, 1);
        return -1;
    }
    watchdog_rearm();
    if (pcall_traceback(g_lua, 0, 0) != LUA_OK) {
        const char *errmsg = lua_tostring(g_lua, -1);
        fprintf(stderr, "Erreur Lua: %s\n", errmsg ? errmsg : "erreur inconnue");
        lua_pop(g_lua, 1);
        return -1;
    }

    /* Appeler setup() si elle existe. Lecture brute (sans __index) : avec
       un _G "strict" dont __index leve une erreur, lua_getglobal hors
       pcall partait en PANIC puis abort si setup() n'etait pas definie. */
    lua_pushglobaltable(g_lua);
    lua_pushliteral(g_lua, "setup");
    lua_rawget(g_lua, -2);
    lua_remove(g_lua, -2);
    if (lua_isfunction(g_lua, -1)) {
        watchdog_rearm();
        if (pcall_traceback(g_lua, 0, 0) != LUA_OK) {
            const char *err = lua_tostring(g_lua, -1);
            fprintf(stderr, "Erreur dans setup(): %s\n", err ? err : "erreur inconnue");
            lua_pop(g_lua, 1);
            return -1;
        }
    } else {
        lua_pop(g_lua, 1);
    }

    return 0;
}

/**
 * Cree un ecran d'erreur LVGL avec le message.
 */
static void show_error_screen(const char *msg)
{
    lv_obj_t *scr = lv_scr_act();
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x800000), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, LV_PART_MAIN);

    lv_obj_t *label = lv_label_create(scr);
    lv_label_set_text(label, msg);
    lv_obj_set_style_text_color(label, lv_color_white(), LV_PART_MAIN);
    lv_label_set_long_mode(label, LV_LABEL_LONG_WRAP);
    lv_obj_set_width(label, 300);
    lv_obj_align(label, LV_ALIGN_CENTER, 0, 0);
}

/* ------------------------------------------------------------------ */
/* Story browser — scans for .plain directories and lets user pick one */
/* ------------------------------------------------------------------ */

#define MAX_STORIES 32
#define THUMB_W 64
#define THUMB_H 64

typedef struct {
    char path[1024];
    char title[256];
    lv_img_dsc_t *thumbnail;  /* decoded from img/thumbnail.lif, or NULL */
    int is_pk;                /* 1 if .plain.pk archive, 0 if directory */
    /* .pk : dossier d'extraction .plain (ANSI). Vide si son nom n'est pas
       representable en ANSI et que le dossier n'existe pas encore : il est
       alors cree au clic depuis pk_wdir, puis designe par son nom court. */
    char pk_dir[1024];
#ifdef _WIN32
    wchar_t pk_wdir[1024];
#endif
} story_entry_t;

static story_entry_t g_stories[MAX_STORIES];
static int g_story_count = 0;
static int g_story_browser_active = 0;
static char g_current_scan_dir[1024] = ".";

/* Read first line of info.plain as story title */
static void read_story_title(const char *path, int is_pk, char *out, size_t out_sz)
{
    if (is_pk) {
        size_t sz;
        char *data = (char *)pk_read_entry(path, "info.plain", &sz);
        if (data && sz > 0) {
            /* Copy first line */
            size_t i;
            for (i = 0; i < sz && i < out_sz - 1 && data[i] != '\n' && data[i] != '\0'; i++) {
                out[i] = data[i];
            }
            out[i] = '\0';
        }
        free(data);
    } else {
        char info[1024];
        snprintf(info, sizeof(info), "%s/info.plain", path);
        FILE *f = fopen(info, "r");
        if (f) {
            if (fgets(out, (int)out_sz, f)) {
                size_t len = strlen(out);
                if (len > 0 && out[len-1] == '\n') out[len-1] = '\0';
            }
            fclose(f);
        }
    }
    if (out[0] == '\0') {
        /* Fallback: use filename/directory name */
        const char *p = path, *last = path;
        for (; *p; p++) { if (*p == '/' || *p == '\\') last = p + 1; }
        strncpy(out, last, out_sz - 1);
        out[out_sz - 1] = '\0';
    }
}

/* Load thumbnail from img/thumbnail.lif */
static lv_img_dsc_t *load_thumbnail(const char *path, int is_pk)
{
    if (is_pk) {
        size_t sz;
        void *data = pk_read_entry(path, "img/thumbnail.lif", &sz);
        if (!data) return NULL;
        lv_img_dsc_t *dsc = lif_decode_mem((const uint8_t *)data, sz);
        free(data);
        return dsc;
    } else {
        char full[1024];
        snprintf(full, sizeof(full), "%s/img/thumbnail.lif", path);
        return lif_decode_file(full);
    }
}

/* Free all loaded thumbnails */
static void free_thumbnails(void)
{
    for (int i = 0; i < g_story_count; i++) {
        if (g_stories[i].thumbnail) {
            lif_free(g_stories[i].thumbnail);
            g_stories[i].thumbnail = NULL;
        }
    }
}

/* Add a story entry (directory or .pk archive).
   pk_dirname : nom (ANSI) du dossier d'extraction d'un .pk, "" s'il n'existe
   pas encore sous un nom ANSI, ou NULL pour le deduire en retirant ".pk".
   Retourne l'entree ajoutee ou mise a jour, NULL si ignoree. */
static story_entry_t *add_story_entry(const char *search_dir, const char *filename,
                                      int is_pk, const char *pk_dirname)
{
    if (g_story_count >= MAX_STORIES) return NULL;

    char full[1024];
    snprintf(full, sizeof(full), "%s/%s", search_dir, filename);

    if (is_pk) {
        if (!pk_has_entry(full, "main.lua")) return NULL;
    } else {
        if (!is_story_dir(full)) return NULL;
    }

    /* Skip .pk if we already have the extracted .plain version.
       Si ce dossier .plain est le dossier d'extraction du .pk mais qu'il est
       perime (ou incomplet), proposer le .pk a sa place : le clic
       re-extraira l'archive au lieu de rejouer l'ancienne version. */
    char dir[1024] = "";
    if (is_pk) {
        if (pk_dirname) {
            if (pk_dirname[0]) snprintf(dir, sizeof(dir), "%s/%s", search_dir, pk_dirname);
        } else {
            strncpy(dir, full, sizeof(dir) - 1);
            dir[sizeof(dir) - 1] = '\0';
            size_t dlen = strlen(dir);
            if (dlen > 3) dir[dlen - 3] = '\0';   /* retirer ".pk" */
        }
        for (int i = 0; dir[0] && i < g_story_count; i++) {
            if (g_stories[i].is_pk || strcmp(g_stories[i].path, dir) != 0) continue;
            if (pk_extract_is_current(full, dir)) return NULL;
            story_entry_t *old_e = &g_stories[i];
            if (old_e->thumbnail) lif_free(old_e->thumbnail);
            strncpy(old_e->path, full, sizeof(old_e->path) - 1);
            old_e->path[sizeof(old_e->path) - 1] = '\0';
            strncpy(old_e->pk_dir, dir, sizeof(old_e->pk_dir) - 1);
            old_e->pk_dir[sizeof(old_e->pk_dir) - 1] = '\0';
#ifdef _WIN32
            old_e->pk_wdir[0] = L'\0';
#endif
            old_e->title[0] = '\0';
            old_e->is_pk = 1;
            read_story_title(full, 1, old_e->title, sizeof(old_e->title));
            old_e->thumbnail = load_thumbnail(full, 1);
            return old_e;
        }
        /* Pas de deduplication par titre : deux archives distinctes peuvent
           porter le meme titre (versions V1/V2, titre par defaut). Le seul
           vrai doublon (.pk + son dossier d'extraction) est traite ci-dessus
           par comparaison de chemins. */
    }

    story_entry_t *e = &g_stories[g_story_count];
    strncpy(e->path, full, sizeof(e->path) - 1);
    e->path[sizeof(e->path) - 1] = '\0';
    e->title[0] = '\0';
    e->thumbnail = NULL;
    e->is_pk = is_pk;
    read_story_title(full, is_pk, e->title, sizeof(e->title));
    e->thumbnail = load_thumbnail(full, is_pk);
    e->pk_dir[0] = '\0';
    if (is_pk) {
        strncpy(e->pk_dir, dir, sizeof(e->pk_dir) - 1);
        e->pk_dir[sizeof(e->pk_dir) - 1] = '\0';
    }
#ifdef _WIN32
    e->pk_wdir[0] = L'\0';
#endif
    g_story_count++;
    return e;
}

#ifdef _WIN32
/* Nom ANSI d'une entree de dossier : le nom long s'il est representable
   dans la code page ANSI, sinon son nom court 8.3 (vide si les noms courts
   sont desactives sur le volume). Retourne 1 si out est rempli. */
static int entry_name_acp(const wchar_t *wfull, const wchar_t *wname,
                          char *out, size_t out_sz)
{
    char *c = wide_to_acp_exact(wname);
    if (!c) {
        WIN32_FIND_DATAW fd;
        HANDLE h = FindFirstFileW(wfull, &fd);
        if (h != INVALID_HANDLE_VALUE) {
            FindClose(h);
            if (fd.cAlternateFileName[0]) c = wide_to_acp_exact(fd.cAlternateFileName);
        }
    }
    if (!c) return 0;
    int ok = strlen(c) < out_sz;
    if (ok) memcpy(out, c, strlen(c) + 1);
    free(c);
    return ok;
}
#endif

/* Scan a directory for .plain dirs and .plain.pk archives.
   Enumeration en UTF-16 (FindFirstFileW) : avec FindFirstFileA, un nom
   contenant un caractere hors de la code page ANSI (polonais, grec...)
   revenait avec des '?' et l'histoire etait ignoree sans message. */
static void scan_for_stories(const char *search_dir)
{
#ifdef _WIN32
    wchar_t wdir[1024];
    if (MultiByteToWideChar(CP_ACP, 0, search_dir, -1, wdir, 1024) <= 0) return;

    for (int pass = 0; pass < 2; pass++) {
        int is_pk = (pass == 1);   /* d'abord les dossiers .plain, puis les .plain.pk */
        wchar_t pattern[1100];
        if (swprintf(pattern, 1100, is_pk ? L"%ls\\*.plain.pk" : L"%ls\\*.plain", wdir) < 0) return;
        WIN32_FIND_DATAW fd;
        HANDLE h = FindFirstFileW(pattern, &fd);
        if (h == INVALID_HANDLE_VALUE) continue;
        do {
            int is_dir = (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
            if (is_dir == is_pk) continue;
            wchar_t wfull[1400];
            if (swprintf(wfull, 1400, L"%ls\\%ls", wdir, fd.cFileName) < 0) continue;
            char name[MAX_PATH];
            if (!entry_name_acp(wfull, fd.cFileName, name, sizeof(name))) {
                fprintf(stderr, "Histoire ignoree : nom non representable en ANSI "
                                "et sans nom court 8.3 (%ls)\n", wfull);
                continue;
            }
            if (!is_pk) {
                add_story_entry(search_dir, name, 0, NULL);
                continue;
            }
            char *exact = wide_to_acp_exact(fd.cFileName);
            if (exact) {
                /* Nom representable : dossier d'extraction = nom sans ".pk" */
                free(exact);
                add_story_entry(search_dir, name, 1, NULL);
                continue;
            }
            /* Nom court d'un .plain.pk : il ne finit plus par ".plain.pk",
               le dossier d'extraction se deduit du nom long (UTF-16) */
            wchar_t wext[1400];
            size_t wl = wcslen(wfull);
            if (wl <= 3 || wl >= 1400) continue;
            memcpy(wext, wfull, (wl - 3) * sizeof(wchar_t));   /* retirer ".pk" */
            wext[wl - 3] = L'\0';
            char dname[MAX_PATH] = "";
            DWORD attr = GetFileAttributesW(wext);
            if (attr != INVALID_FILE_ATTRIBUTES && (attr & FILE_ATTRIBUTE_DIRECTORY)) {
                const wchar_t *wbase = wext + wcslen(wdir) + 1;
                if (!entry_name_acp(wext, wbase, dname, sizeof(dname))) dname[0] = '\0';
            }
            story_entry_t *e = add_story_entry(search_dir, name, 1, dname);
            if (e && !e->pk_dir[0]) {
                if (wl - 3 < sizeof(e->pk_wdir) / sizeof(e->pk_wdir[0])) {
                    memcpy(e->pk_wdir, wext, (wl - 2) * sizeof(wchar_t));
                } else {
                    e->pk_wdir[0] = L'\0';
                }
            }
        } while (FindNextFileW(h, &fd) && g_story_count < MAX_STORIES);
        FindClose(h);
    }
#else
    (void)search_dir;
#endif
}

/* Temporary extraction directory for .pk stories */
static char g_pk_extract_dir[1024] = "";

/* Extrait un .plain.pk dans out_dir si le dossier est absent, perime
   (.pk modifie depuis) ou issu d'une extraction interrompue.
   Partage par la ligne de commande et le navigateur d'histoires. */
static void extract_pk_if_needed(const char *pk_path, const char *out_dir)
{
    if (pk_extract_is_current(pk_path, out_dir)) return;
    if (is_story_dir(out_dir)) {
        fprintf(stderr, "%s perime ou incomplet : nouvelle extraction\n", out_dir);
    }
    fprintf(stderr, "Extracting %s ...\n", pk_path);
    int n = pk_extract_if_stale(pk_path, out_dir);
    fprintf(stderr, "Extracted %d files to %s\n", n, out_dir);
}

/* Callback when a story button is clicked */
static void story_btn_clicked(lv_event_t *ev)
{
    int idx = (int)(intptr_t)lv_event_get_user_data(ev);
    if (idx < 0 || idx >= g_story_count) return;

    /* Clean the browser UI */
    lv_obj_t *win = lv_obj_get_child(lv_scr_act(), 1); /* content_window */
    lv_obj_clean(win);
    lv_group_remove_all_objs(g_focus_group);

    g_story_browser_active = 0;

    const char *story_path = g_stories[idx].path;

    /* If .pk archive, extract to temp directory first */
    if (g_stories[idx].is_pk) {
        /* Extract next to the .pk file in a .plain directory */
        if (g_stories[idx].pk_dir[0]) {
            strncpy(g_pk_extract_dir, g_stories[idx].pk_dir, sizeof(g_pk_extract_dir) - 1);
            g_pk_extract_dir[sizeof(g_pk_extract_dir) - 1] = '\0';
        }
#ifdef _WIN32
        else if (g_stories[idx].pk_wdir[0]) {
            /* Nom long du dossier non representable en ANSI : le creer en
               UTF-16 puis le designer par son nom court 8.3 */
            const wchar_t *wext = g_stories[idx].pk_wdir;
            char *acp = NULL;
            if (CreateDirectoryW(wext, NULL) || GetLastError() == ERROR_ALREADY_EXISTS) {
                acp = path_acp_by_components(wext);
            }
            if (!acp || strlen(acp) >= sizeof(g_pk_extract_dir)) {
                fprintf(stderr, "Dossier d'extraction sans nom ANSI utilisable : %ls\n", wext);
                free(acp);
                show_error_screen("Erreur de chargement.\nVoir la console.");
                return;
            }
            memcpy(g_pk_extract_dir, acp, strlen(acp) + 1);
            free(acp);
        }
#endif
        else {
            strncpy(g_pk_extract_dir, story_path, sizeof(g_pk_extract_dir) - 1);
            g_pk_extract_dir[sizeof(g_pk_extract_dir) - 1] = '\0';
            /* Remove .pk extension to get .plain path */
            size_t len = strlen(g_pk_extract_dir);
            if (len > 3 && strcmp(g_pk_extract_dir + len - 3, ".pk") == 0) {
                g_pk_extract_dir[len - 3] = '\0';
            }
        }

        /* Extraire si absent, perime ou incomplet */
        extract_pk_if_needed(story_path, g_pk_extract_dir);
        story_path = g_pk_extract_dir;
    }

    /* Load the story */
    if (load_story(story_path) != 0) {
        show_error_screen("Erreur de chargement.\nVoir la console.");
    }
}

extern lv_font_t nunito_extrabold_16;

/* Forward declaration */
static void create_story_browser(const char *scan_dir);

#ifdef _WIN32
/* Open native Windows folder picker dialog */
/* Selection en UTF-16 puis conversion ANSI (noms courts 8.3 pour les
   composants non representables) : SHGetPathFromIDListA rendait des '?'
   pour un dossier au nom grec, polonais... */
static int pick_folder(char *out, size_t out_sz)
{
    BROWSEINFOW bi = {0};
    bi.lpszTitle = L"Choisir le dossier contenant les histoires (.plain)";
    bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;

    LPITEMIDLIST pidl = SHBrowseForFolderW(&bi);
    if (!pidl) return 0;

    wchar_t wpath[MAX_PATH];
    int ok = SHGetPathFromIDListW(pidl, wpath);
    CoTaskMemFree(pidl);
    if (!ok) return 0;

    char *s = wide_to_acp_exact(wpath);
    if (!s) s = path_acp_by_components(wpath);
    if (!s) {
        fprintf(stderr, "Dossier sans chemin ANSI utilisable : %ls\n", wpath);
        return 0;
    }
    ok = strlen(s) < out_sz;
    if (ok) memcpy(out, s, strlen(s) + 1);
    free(s);
    return ok;
}
#endif

/* Callback for "Choisir un dossier..." button */
static void browse_btn_clicked(lv_event_t *ev)
{
    (void)ev;
#ifdef _WIN32
    char folder[1024] = "";
    if (pick_folder(folder, sizeof(folder))) {
        strncpy(g_current_scan_dir, folder, sizeof(g_current_scan_dir) - 1);
        g_current_scan_dir[sizeof(g_current_scan_dir) - 1] = '\0';
        create_story_browser(g_current_scan_dir);
    }
#endif
}

/* Helper: create a styled button for the story browser (text only) */
static lv_obj_t *create_browser_btn(lv_obj_t *parent, const char *text,
                                     lv_color_t bg, lv_color_t fg,
                                     const lv_font_t *font)
{
    lv_obj_t *btn = lv_btn_create(parent);
    lv_obj_remove_style_all(btn);
    lv_obj_set_size(btn, 288, LV_SIZE_CONTENT);
    lv_obj_set_style_pad_all(btn, 10, LV_PART_MAIN);
    lv_obj_set_style_radius(btn, 6, LV_PART_MAIN);
    lv_obj_set_style_bg_color(btn, bg, LV_PART_MAIN);
    lv_obj_set_style_bg_opa(btn, LV_OPA_COVER, LV_PART_MAIN);
    lv_obj_set_style_bg_color(btn, lv_color_hex(0x2A3040), LV_PART_MAIN | LV_STATE_FOCUSED);
    lv_obj_set_style_border_width(btn, 1, LV_PART_MAIN | LV_STATE_FOCUSED);
    lv_obj_set_style_border_color(btn, lv_color_hex(0xFBBD2A), LV_PART_MAIN | LV_STATE_FOCUSED);

    lv_obj_t *lbl = lv_label_create(btn);
    lv_label_set_text(lbl, text);
    lv_obj_set_style_text_color(lbl, fg, LV_PART_MAIN);
    lv_obj_set_style_text_font(lbl, font, LV_PART_MAIN);
    lv_label_set_long_mode(lbl, LV_LABEL_LONG_WRAP);
    lv_obj_set_width(lbl, 268);
    return btn;
}

/* Helper: create a story card button with thumbnail + title */
static lv_obj_t *create_story_card(lv_obj_t *parent, story_entry_t *story)
{
    lv_obj_t *btn = lv_btn_create(parent);
    lv_obj_remove_style_all(btn);
    lv_obj_set_size(btn, 288, LV_SIZE_CONTENT);
    lv_obj_set_style_pad_all(btn, 8, LV_PART_MAIN);
    lv_obj_set_style_radius(btn, 8, LV_PART_MAIN);
    lv_obj_set_style_bg_color(btn, lv_color_hex(0x1A1D23), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(btn, LV_OPA_COVER, LV_PART_MAIN);
    lv_obj_set_style_bg_color(btn, lv_color_hex(0x2A3040), LV_PART_MAIN | LV_STATE_FOCUSED);
    lv_obj_set_style_border_width(btn, 1, LV_PART_MAIN | LV_STATE_FOCUSED);
    lv_obj_set_style_border_color(btn, lv_color_hex(0xFBBD2A), LV_PART_MAIN | LV_STATE_FOCUSED);

    /* Row layout: thumbnail | title */
    lv_obj_set_flex_flow(btn, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(btn, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_column(btn, 10, LV_PART_MAIN);

    /* Thumbnail (in a fixed-size container for proper centering).
       Une vignette de dimension nulle provoquerait une division par zero. */
    if (story->thumbnail &&
        story->thumbnail->header.w > 0 && story->thumbnail->header.h > 0) {
        lv_obj_t *img_cont = lv_obj_create(btn);
        lv_obj_remove_style_all(img_cont);
        lv_obj_set_size(img_cont, THUMB_W, THUMB_H);
        lv_obj_clear_flag(img_cont, LV_OBJ_FLAG_SCROLLABLE);
        lv_obj_set_style_radius(img_cont, 4, LV_PART_MAIN);
        lv_obj_set_style_clip_corner(img_cont, true, LV_PART_MAIN);

        lv_obj_t *img = lv_img_create(img_cont);
        lv_img_set_src(img, story->thumbnail);
        /* Scale to fit THUMB_W x THUMB_H (keep aspect ratio) */
        uint16_t zoom_w = (uint16_t)(256 * THUMB_W / story->thumbnail->header.w);
        uint16_t zoom_h = (uint16_t)(256 * THUMB_H / story->thumbnail->header.h);
        uint16_t zoom = zoom_w < zoom_h ? zoom_w : zoom_h;
        lv_img_set_zoom(img, zoom);
        lv_img_set_pivot(img, 0, 0);
        /* Center the scaled image in the container */
        int scaled_w = (int)(story->thumbnail->header.w * zoom / 256);
        int scaled_h = (int)(story->thumbnail->header.h * zoom / 256);
        lv_obj_set_pos(img, (THUMB_W - scaled_w) / 2, (THUMB_H - scaled_h) / 2);
    }

    /* Title */
    lv_obj_t *lbl = lv_label_create(btn);
    lv_label_set_text(lbl, story->title);
    lv_obj_set_style_text_color(lbl, lv_color_hex(0xE0E4E8), LV_PART_MAIN);
    lv_obj_set_style_text_font(lbl, &nunito_extrabold_16, LV_PART_MAIN);
    lv_label_set_long_mode(lbl, LV_LABEL_LONG_WRAP);
    lv_obj_set_width(lbl, story->thumbnail ? 196 : 268);

    return btn;
}

static void create_story_browser(const char *scan_dir)
{
    /* Remember scan dir for refresh */
    strncpy(g_current_scan_dir, scan_dir, sizeof(g_current_scan_dir) - 1);
    g_current_scan_dir[sizeof(g_current_scan_dir) - 1] = '\0';

    free_thumbnails();
    g_story_count = 0;
    scan_for_stories(scan_dir);

    if (g_header_title) {
        lv_label_set_text(g_header_title, "Flam Player");
    }
    /* Titre de la fenetre aussi : sinon il garde "Flam Player - <titre>"
       de la derniere histoire au retour a la bibliotheque. */
    sdl_driver_set_title("Flam Player");

    /* Get content window (child 1 of screen, after header) */
    lv_obj_t *win = lv_obj_get_child(lv_scr_act(), 1);
    lv_obj_clean(win);
    lv_group_remove_all_objs(g_focus_group);

    /* Scrollable list container */
    lv_obj_t *list = lv_obj_create(win);
    lv_obj_remove_style_all(list);
    lv_obj_set_size(list, FLAM_SCREEN_W, FLAM_SCREEN_H - 28);
    lv_obj_set_flex_flow(list, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_flex_align(list, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_top(list, 16, LV_PART_MAIN);
    lv_obj_set_style_pad_row(list, 12, LV_PART_MAIN);
    lv_obj_set_style_pad_left(list, 16, LV_PART_MAIN);
    lv_obj_set_style_pad_right(list, 16, LV_PART_MAIN);
    lv_obj_set_style_pad_bottom(list, 40, LV_PART_MAIN);

    if (g_story_count == 0) {
        lv_obj_t *msg = lv_label_create(list);
        lv_label_set_text(msg, "Aucune histoire trouvee.");
        lv_obj_set_style_text_color(msg, lv_color_hex(0x888888), LV_PART_MAIN);
        lv_obj_set_style_text_font(msg, &nunito_bold_12, LV_PART_MAIN);
        lv_obj_set_style_text_align(msg, LV_TEXT_ALIGN_CENTER, LV_PART_MAIN);
        lv_obj_set_width(msg, 280);
    }

    /* Story cards */
    for (int i = 0; i < g_story_count; i++) {
        lv_obj_t *btn = create_story_card(list, &g_stories[i]);
        lv_obj_add_event_cb(btn, story_btn_clicked, LV_EVENT_CLICKED,
                            (void *)(intptr_t)i);
    }

    /* "Choisir un dossier..." button */
#ifdef _WIN32
    {
        lv_obj_t *browse = create_browser_btn(list, "Choisir un dossier...",
            lv_color_hex(0x16213E), lv_color_hex(0x8899AA), &nunito_bold_12);
        lv_obj_set_style_text_align(lv_obj_get_child(browse, 0), LV_TEXT_ALIGN_CENTER, LV_PART_MAIN);
        lv_obj_add_event_cb(browse, browse_btn_clicked, LV_EVENT_CLICKED, NULL);
    }
#endif

    /* Focus first focusable button */
    lv_obj_t *first = lv_obj_get_child(list, g_story_count == 0 ? 1 : 0);
    if (first) {
        lv_group_focus_obj(first);
    }

    g_story_browser_active = 1;
}

/**
 * Detecte si un chemin est un dossier .plain (contient main.lua).
 */
static int is_story_dir(const char *path)
{
    char check[1024];
    struct stat st;
    snprintf(check, sizeof(check), "%s/main.lua", path);
    return (stat(check, &st) == 0);
}

/**
 * Charge une histoire depuis un dossier .plain.
 * Configure automatiquement img/, sounds/, script/, save.
 */
static int load_story(const char *story_dir)
{
    char path_buf[1024];

    /* Images : img/ */
    snprintf(path_buf, sizeof(path_buf), "%s/img", story_dir);
    lua_lv_set_img_base_path(path_buf);

    /* Audio : sounds/ */
    snprintf(path_buf, sizeof(path_buf), "%s/sounds", story_dir);
    sdl_audio_set_base_path(path_buf);

    /* Sauvegarde : saves/ a cote du dossier .plain */
    /* Extraire le nom du dossier pour le save dir */
    const char *dirname = story_dir;
    const char *p;
    /* Trouver le dernier / ou \ */
    for (p = story_dir; *p; p++) {
        if (*p == '/' || *p == '\\') dirname = p + 1;
    }
    /* Creer saves/{dirname}/ a cote du dossier story */
    {
        /* Calculer le parent dir */
        size_t parent_len = (size_t)(dirname - story_dir);
        char save_dir[1024];
        if (parent_len > 0) {
            snprintf(save_dir, sizeof(save_dir), "%.*ssaves/%s",
                     (int)parent_len, story_dir, dirname);
        } else {
            snprintf(save_dir, sizeof(save_dir), "saves/%s", dirname);
        }
        fw_set_save_dir(save_dir);
        fw_reload_state(g_lua);
    }

    /* Package path Lua : script/ et racine */
    set_lua_package_path(story_dir);

    /* Titre fenetre + header */
    {
        char info_path[1024];
        snprintf(info_path, sizeof(info_path), "%s/info.plain", story_dir);
        FILE *f = fopen(info_path, "r");
        if (f) {
            char title[256] = "";
            if (fgets(title, sizeof(title), f)) {
                /* Retirer le \n */
                size_t len = strlen(title);
                if (len > 0 && title[len-1] == '\n') title[len-1] = '\0';

                char win_title[300];
                snprintf(win_title, sizeof(win_title), "Flam Player - %s", title);
                sdl_driver_set_title(win_title);

                /* Update header bar title */
                if (g_header_title) {
                    lv_label_set_text(g_header_title, title);
                }
            }
            fclose(f);
        }
    }

    /* Charger main.lua */
    snprintf(path_buf, sizeof(path_buf), "%s/main.lua", story_dir);
    printf("Loading story: %s\n", story_dir);

    return load_script(path_buf);
}

#ifdef _WIN32
/* Convertit une chaine large en code page ANSI (CP_ACP). Retourne une
   chaine allouee, ou NULL si un caractere n'est pas representable. */
static char *wide_to_acp_exact(const wchar_t *w)
{
    BOOL lossy = FALSE;
    int n = WideCharToMultiByte(CP_ACP, WC_NO_BEST_FIT_CHARS, w, -1,
                                NULL, 0, NULL, &lossy);
    if (n <= 0 || lossy) return NULL;
    char *s = (char *)malloc((size_t)n);
    if (!s) return NULL;
    lossy = FALSE;
    if (WideCharToMultiByte(CP_ACP, WC_NO_BEST_FIT_CHARS, w, -1,
                            s, n, NULL, &lossy) <= 0 || lossy) {
        free(s);
        return NULL;
    }
    return s;
}

/* Reconstruit un chemin en ANSI composant par composant : un composant
   representable est garde tel quel, sinon on prend son nom court 8.3
   (cAlternateFileName ; GetShortPathNameW garde les noms deja courts
   comme "Omega" en grec meme s'ils ne sont pas representables en ANSI).
   Retourne une chaine allouee, ou NULL si impossible. */
static char *path_acp_by_components(const wchar_t *w)
{
    char out[2048];
    size_t olen = 0;
    size_t wlen = wcslen(w);
    wchar_t *pre = (wchar_t *)malloc((wlen + 1) * sizeof(wchar_t));
    if (!pre) return NULL;
    size_t start = 0;
    while (1) {
        size_t end = start;
        while (w[end] && w[end] != L'\\' && w[end] != L'/') end++;
        /* composant w[start..end) */
        memcpy(pre, w, end * sizeof(wchar_t));
        pre[end] = L'\0';
        char *c = wide_to_acp_exact(pre + start);
        if (!c) {
            int wild = 0;
            for (size_t k = start; k < end; k++) {
                if (w[k] == L'*' || w[k] == L'?') wild = 1;
            }
            WIN32_FIND_DATAW fd;
            HANDLE h = wild ? INVALID_HANDLE_VALUE : FindFirstFileW(pre, &fd);
            if (h != INVALID_HANDLE_VALUE) {
                FindClose(h);
                if (fd.cAlternateFileName[0]) c = wide_to_acp_exact(fd.cAlternateFileName);
            }
        }
        if (!c) { free(pre); return NULL; }
        size_t cl = strlen(c);
        if (olen + cl + 2 > sizeof(out)) { free(c); free(pre); return NULL; }
        memcpy(out + olen, c, cl);
        olen += cl;
        free(c);
        if (!w[end]) break;
        out[olen++] = (char)w[end];   /* separateur '\' ou '/' */
        start = end + 1;
    }
    free(pre);
    out[olen] = '\0';
    return _strdup(out);
}

/* SDL2main fournit argv en UTF-8 alors que tout le player (stat, fopen,
   FindFirstFileA, luaL_loadfile) utilise les API ANSI : un chemin accentue
   passe en argument etait introuvable. On reconvertit donc chaque argument
   UTF-8 -> ANSI. Si un caractere n'existe pas dans la code page ANSI, les
   composants concernes sont remplaces par leur nom court 8.3 (les autres,
   dont le nom final d'un .plain.pk dont derive le dossier d'extraction,
   sont gardes tels quels). */
static void argv_utf8_to_acp(int argc, char *argv[])
{
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
        int ascii = 1;
        for (const char *p = a; *p; p++) {
            if ((unsigned char)*p >= 0x80) { ascii = 0; break; }
        }
        if (ascii) continue;

        int wn = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, a, -1, NULL, 0);
        if (wn <= 0) continue;   /* pas de l'UTF-8 valide : laisser tel quel */
        wchar_t *w = (wchar_t *)malloc((size_t)wn * sizeof(wchar_t));
        if (!w) continue;
        MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, a, -1, w, wn);

        char *s = wide_to_acp_exact(w);
        if (!s) s = path_acp_by_components(w);
        free(w);
        if (s) argv[i] = s;   /* jamais libere : vit jusqu'a la fin */
    }
}
#endif

int main(int argc, char *argv[])
{
#ifdef _WIN32
    argv_utf8_to_acp(argc, argv);
#endif

    /* Parser les arguments CLI en premier */
    const char *target_path = NULL;
    const char *img_dir = NULL;
    const char *sounds_dir = NULL;
    const char *save_dir = NULL;
    const char *scan_dir = NULL;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--img-dir") == 0 && i + 1 < argc) {
            img_dir = argv[++i];
        } else if (strcmp(argv[i], "--sounds-dir") == 0 && i + 1 < argc) {
            sounds_dir = argv[++i];
        } else if (strcmp(argv[i], "--save-dir") == 0 && i + 1 < argc) {
            save_dir = argv[++i];
        } else if (strcmp(argv[i], "--scan-dir") == 0 && i + 1 < argc) {
            scan_dir = argv[++i];
        } else if (strcmp(argv[i], "--strict") == 0) {
            g_strict = 1;
        } else if (strcmp(argv[i], "--watchdog") == 0 && i + 1 < argc) {
            long ms = strtol(argv[++i], NULL, 10);
            g_watchdog_ms = ms > 0 ? (uint32_t)ms : 0;
        } else if (strcmp(argv[i], "--screenshot") == 0 && i + 1 < argc) {
            sdl_driver_set_screenshot_path(argv[++i]);
        } else {
            target_path = argv[i];
        }
    }

    /* --strict s'applique aussi aux bindings lv.* (img_src.load...) */
    lua_lv_set_strict(g_strict);

    /* Detecter le mode : dossier .plain, archive .plain.pk, ou script Lua */
    int is_story = 0;
    int is_pk = 0;
    if (target_path) {
        size_t tlen = strlen(target_path);
        if (tlen > 9 && strcmp(target_path + tlen - 9, ".plain.pk") == 0) {
            is_pk = 1;
            is_story = 1;
        } else if (is_story_dir(target_path)) {
            is_story = 1;
        }
    }

    /* Si .plain.pk, extraire dans un dossier .plain temporaire */
    if (is_pk && target_path) {
        strncpy(g_pk_extract_dir, target_path, sizeof(g_pk_extract_dir) - 1);
        g_pk_extract_dir[sizeof(g_pk_extract_dir) - 1] = '\0';
        /* Retirer .pk pour obtenir le chemin .plain */
        size_t elen = strlen(g_pk_extract_dir);
        if (elen > 3) g_pk_extract_dir[elen - 3] = '\0';

        extract_pk_if_needed(target_path, g_pk_extract_dir);
        target_path = g_pk_extract_dir;
    }

    /* En mode direct (script Lua), appliquer les overrides explicites */
    if (!is_story) {
        if (img_dir)     lua_lv_set_img_base_path(img_dir);
        if (sounds_dir)  sdl_audio_set_base_path(sounds_dir);
        if (save_dir)    fw_set_save_dir(save_dir);
    }

    /* Initialiser SDL2 + LVGL */
    if (sdl_driver_init() != 0) {
        fprintf(stderr, "Erreur initialisation SDL/LVGL\n");
        return 1;
    }

    /* Initialiser audio */
    if (sdl_audio_init() != 0) {
        fprintf(stderr, "Warning: audio init failed\n");
    }

    /* Initialiser Lua (charge state depuis save_dir) */
    if (init_lua() != 0) {
        sdl_audio_quit();
        sdl_driver_quit();
        return 1;
    }

    /* Charger le contenu */
    if (target_path) {
        int err;
        if (is_story) {
            err = load_story(target_path);
        } else {
            err = load_script(target_path);
        }
        if (err != 0) {
            show_error_screen("Erreur de chargement.\nVoir la console.");
        }
    } else {
        /* No argument: scan for .plain stories */
        char exe_dir[1024] = ".";
        if (scan_dir) {
            strncpy(exe_dir, scan_dir, sizeof(exe_dir) - 1);
        }
#ifdef _WIN32
        else {
            char exe_path[1024];
            DWORD len = GetModuleFileNameA(NULL, exe_path, sizeof(exe_path));
            if (len > 0) {
                char *last = strrchr(exe_path, '\\');
                if (!last) last = strrchr(exe_path, '/');
                if (last) { *last = '\0'; strncpy(exe_dir, exe_path, sizeof(exe_dir) - 1); }
            }
        }
#endif
        create_story_browser(exe_dir);
    }

    /* Boucle principale */
    while (1) {
        watchdog_rearm();
        if (sdl_driver_poll()) {
            break;
        }
        sdl_audio_pump(g_lua);
        int pump = fw_pump(g_lua);
        if (pump == 1) {
            break;  /* quit */
        }
        if (pump == 2) {
            /* Return to story browser */
            sdl_audio_stop_all();
            fw_save_state(g_lua);

            /* Clean up Lua timers and animations before closing Lua */
            lua_lv_cleanup_timers();
            lv_anim_del_all();

            /* Clean LVGL before closing Lua (avoid double-free via __gc) */
            lv_group_set_default(NULL);
            if (g_focus_group) {
                lv_group_remove_all_objs(g_focus_group);
                lv_group_del(g_focus_group);
                g_focus_group = NULL;
            }
            lv_obj_clean(lv_scr_act());
            g_header = NULL;
            g_header_title = NULL;

            /* Now safe to close Lua */
            lua_close(g_lua);
            g_lua = NULL;
            fw_reset();

            /* Re-init Lua + UI */
            if (init_lua() != 0) break;
            create_story_browser(g_current_scan_dir);
        }
        SDL_Delay(5);
    }

    /* Sauvegarder l'etat avant de quitter */
    if (g_lua) fw_save_state(g_lua);

    /* Cleanup */
    if (g_lua) lua_close(g_lua);
    sdl_audio_quit();
    sdl_driver_quit();
    return 0;
}
