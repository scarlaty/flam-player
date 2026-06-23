/**
 * main.c — Point d'entree du Flam Player
 *
 * Etape 6 : Chargement d'histoire .plain
 * Usage : flam-player <chemin/vers/histoire.plain>
 *         flam-player <script.lua> [--img-dir ...] [--sounds-dir ...] [--save-dir ...]
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

/* Etat global Lua */
static lua_State *g_lua = NULL;

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
 * Cherche dans script/ et a la racine de l'histoire.
 */
static char g_story_dir[1024] = "";

static int custom_lua_searcher(lua_State *L)
{
    const char *modname = luaL_checkstring(L, 1);
    char path[1024];
    const char *dirs[] = { "script", "." };

    for (int d = 0; d < 2; d++) {
        snprintf(path, sizeof(path), "%s/%s/%s.lua", g_story_dir, dirs[d], modname);
        /* Normaliser les slashes */
        for (char *p = path; *p; p++) {
            if (*p == '\\') *p = '/';
        }

        FILE *f = fopen(path, "rb");
        if (!f) continue;

        fseek(f, 0, SEEK_END);
        long fsize = ftell(f);
        fseek(f, 0, SEEK_SET);
        char *buf = (char *)malloc((size_t)fsize);
        fread(buf, 1, (size_t)fsize, f);
        fclose(f);

        /* Strip trailing null bytes */
        while (fsize > 0 && buf[fsize - 1] == '\0') fsize--;

        int err = luaL_loadbuffer(L, buf, (size_t)fsize, path);
        free(buf);
        if (err != LUA_OK) {
            return lua_error(L);
        }
        return 1;  /* retourner la fonction chargee */
    }

    lua_pushfstring(L, "\n\tno file '%s/script/%s.lua'\n\tno file '%s/%s.lua'",
                    g_story_dir, modname, g_story_dir, modname);
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
    fseek(f, 0, SEEK_END);
    long fsize = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = (char *)malloc((size_t)fsize);
    fread(buf, 1, (size_t)fsize, f);
    fclose(f);

    /* Retirer les trailing null bytes */
    while (fsize > 0 && buf[fsize - 1] == '\0') fsize--;

    int err = luaL_loadbuffer(g_lua, buf, (size_t)fsize, path);
    free(buf);
    if (err != LUA_OK) {
        const char *errmsg = lua_tostring(g_lua, -1);
        fprintf(stderr, "Erreur Lua: %s\n", errmsg ? errmsg : "erreur inconnue");
        lua_pop(g_lua, 1);
        return -1;
    }
    if (lua_pcall(g_lua, 0, 0, 0) != LUA_OK) {
        const char *errmsg = lua_tostring(g_lua, -1);
        fprintf(stderr, "Erreur Lua: %s\n", errmsg ? errmsg : "erreur inconnue");
        lua_pop(g_lua, 1);
        return -1;
    }

    /* Appeler setup() si elle existe */
    lua_getglobal(g_lua, "setup");
    if (lua_isfunction(g_lua, -1)) {
        if (lua_pcall(g_lua, 0, 0, 0) != LUA_OK) {
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

/* Add a story entry (directory or .pk archive) */
static void add_story_entry(const char *search_dir, const char *filename, int is_pk)
{
    if (g_story_count >= MAX_STORIES) return;

    char full[1024];
    snprintf(full, sizeof(full), "%s/%s", search_dir, filename);

    if (is_pk) {
        if (!pk_has_entry(full, "main.lua")) return;
    } else {
        if (!is_story_dir(full)) return;
    }

    /* Skip .pk if we already have the extracted .plain version */
    if (is_pk) {
        for (int i = 0; i < g_story_count; i++) {
            /* Compare titles to avoid duplicates */
            char title[256] = "";
            read_story_title(full, is_pk, title, sizeof(title));
            if (title[0] && strcmp(g_stories[i].title, title) == 0) return;
        }
    }

    story_entry_t *e = &g_stories[g_story_count];
    strncpy(e->path, full, sizeof(e->path) - 1);
    e->title[0] = '\0';
    e->thumbnail = NULL;
    e->is_pk = is_pk;
    read_story_title(full, is_pk, e->title, sizeof(e->title));
    e->thumbnail = load_thumbnail(full, is_pk);
    g_story_count++;
}

/* Scan a directory for .plain dirs and .plain.pk archives */
static void scan_for_stories(const char *search_dir)
{
#ifdef _WIN32
    /* First scan for .plain directories */
    {
        char pattern[1024];
        snprintf(pattern, sizeof(pattern), "%s\\*.plain", search_dir);
        WIN32_FIND_DATAA fd;
        HANDLE h = FindFirstFileA(pattern, &fd);
        if (h != INVALID_HANDLE_VALUE) {
            do {
                if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
                    add_story_entry(search_dir, fd.cFileName, 0);
                }
            } while (FindNextFileA(h, &fd) && g_story_count < MAX_STORIES);
            FindClose(h);
        }
    }

    /* Then scan for .plain.pk archives (files, not directories) */
    {
        char pattern[1024];
        snprintf(pattern, sizeof(pattern), "%s\\*.plain.pk", search_dir);
        WIN32_FIND_DATAA fd;
        HANDLE h = FindFirstFileA(pattern, &fd);
        if (h != INVALID_HANDLE_VALUE) {
            do {
                if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)) {
                    add_story_entry(search_dir, fd.cFileName, 1);
                }
            } while (FindNextFileA(h, &fd) && g_story_count < MAX_STORIES);
            FindClose(h);
        }
    }
#else
    (void)search_dir;
#endif
}

/* Temporary extraction directory for .pk stories */
static char g_pk_extract_dir[1024] = "";

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
        strncpy(g_pk_extract_dir, story_path, sizeof(g_pk_extract_dir) - 1);
        /* Remove .pk extension to get .plain path */
        size_t len = strlen(g_pk_extract_dir);
        if (len > 3 && strcmp(g_pk_extract_dir + len - 3, ".pk") == 0) {
            g_pk_extract_dir[len - 3] = '\0';
        }

        /* Check if already extracted */
        if (!is_story_dir(g_pk_extract_dir)) {
            fprintf(stderr, "Extracting %s ...\n", story_path);
            int n = pk_extract_all(story_path, g_pk_extract_dir);
            fprintf(stderr, "Extracted %d files to %s\n", n, g_pk_extract_dir);
        }
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
static int pick_folder(char *out, size_t out_sz)
{
    BROWSEINFOA bi = {0};
    bi.lpszTitle = "Choisir le dossier contenant les histoires (.plain)";
    bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;

    LPITEMIDLIST pidl = SHBrowseForFolderA(&bi);
    if (!pidl) return 0;

    int ok = SHGetPathFromIDListA(pidl, out);
    CoTaskMemFree(pidl);
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

    /* Thumbnail (in a fixed-size container for proper centering) */
    if (story->thumbnail) {
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

int main(int argc, char *argv[])
{
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
        } else {
            target_path = argv[i];
        }
    }

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

        if (!is_story_dir(g_pk_extract_dir)) {
            fprintf(stderr, "Extracting %s ...\n", target_path);
            int n = pk_extract_all(target_path, g_pk_extract_dir);
            fprintf(stderr, "Extracted %d files to %s\n", n, g_pk_extract_dir);
        }
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
