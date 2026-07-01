/**
 * fw_globals.c — Objets firmware injectes dans Lua
 *
 * Implemente : state, progression, context_menu, back_callback,
 *              goto_library, screen, progress
 */

#include "fw_globals.h"
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"
#include "lvgl/lvgl.h"
#include "bindings/lua_lv.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <errno.h>

/* ================================================================== */
/* Repertoire de sauvegarde                                            */
/* ================================================================== */

static char g_save_dir[1024] = "";
static int  g_want_quit = 0;
static int  g_want_context_menu = 0;
static int  g_want_back = 0;

/* Overlay context menu LVGL */
static lv_obj_t *g_ctx_overlay = NULL;

void fw_set_save_dir(const char *dir)
{
    if (dir) {
        strncpy(g_save_dir, dir, sizeof(g_save_dir) - 1);
        g_save_dir[sizeof(g_save_dir) - 1] = '\0';
    }
}

/* ================================================================== */
/* Utilitaires : serialisation Lua <-> fichier texte (format Lua)      */
/* ================================================================== */

static void ensure_dir_recursive(const char *path)
{
    char tmp[1024];
    strncpy(tmp, path, sizeof(tmp) - 1);
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
#ifdef _WIN32
    _mkdir(tmp);
#else
    mkdir(tmp, 0755);
#endif
}

/* Serialise une valeur Lua sur la pile en texte Lua dans un FILE* */
static void serialize_value(lua_State *L, int idx, FILE *f, int indent)
{
    int abs_idx = (idx > 0) ? idx : lua_gettop(L) + idx + 1;

    switch (lua_type(L, abs_idx)) {
    case LUA_TNIL:
        fprintf(f, "nil");
        break;
    case LUA_TBOOLEAN:
        fprintf(f, lua_toboolean(L, abs_idx) ? "true" : "false");
        break;
    case LUA_TNUMBER:
        if (lua_isinteger(L, abs_idx)) {
            fprintf(f, "%lld", (long long)lua_tointeger(L, abs_idx));
        } else {
            fprintf(f, "%.17g", lua_tonumber(L, abs_idx));
        }
        break;
    case LUA_TSTRING: {
        const char *s = lua_tostring(L, abs_idx);
        fputc('"', f);
        for (; *s; s++) {
            if (*s == '"') fprintf(f, "\\\"");
            else if (*s == '\\') fprintf(f, "\\\\");
            else if (*s == '\n') fprintf(f, "\\n");
            else if (*s == '\r') fprintf(f, "\\r");
            else fputc(*s, f);
        }
        fputc('"', f);
        break;
    }
    case LUA_TTABLE: {
        fprintf(f, "{\n");
        lua_pushnil(L);
        int first = 1;
        while (lua_next(L, abs_idx) != 0) {
            if (!first) fprintf(f, ",\n");
            first = 0;

            /* indentation */
            for (int i = 0; i < indent + 1; i++) fprintf(f, "  ");

            /* cle */
            if (lua_type(L, -2) == LUA_TSTRING) {
                fprintf(f, "[\"");
                const char *k = lua_tostring(L, -2);
                for (; *k; k++) {
                    if (*k == '"') fprintf(f, "\\\"");
                    else fputc(*k, f);
                }
                fprintf(f, "\"] = ");
            } else if (lua_type(L, -2) == LUA_TNUMBER) {
                if (lua_isinteger(L, -2)) {
                    fprintf(f, "[%lld] = ", (long long)lua_tointeger(L, -2));
                } else {
                    fprintf(f, "[%.17g] = ", lua_tonumber(L, -2));
                }
            }

            /* valeur */
            serialize_value(L, -1, f, indent + 1);

            lua_pop(L, 1); /* pop valeur, garde cle */
        }
        fprintf(f, "\n");
        for (int i = 0; i < indent; i++) fprintf(f, "  ");
        fprintf(f, "}");
        break;
    }
    default:
        /* Ignorer functions, userdata, etc. */
        fprintf(f, "nil");
        break;
    }
}

/* Sauvegarde la table sur la pile dans un fichier Lua */
static int save_lua_table(lua_State *L, int tbl_idx, const char *filepath)
{
    FILE *f = fopen(filepath, "wb");
    if (!f) {
        fprintf(stderr, "fw: cannot write '%s': %s\n", filepath, strerror(errno));
        return -1;
    }
    fprintf(f, "return ");
    serialize_value(L, tbl_idx, f, 0);
    fprintf(f, "\n");
    fclose(f);
    return 0;
}

/* Charge un fichier Lua qui retourne une table ; pousse la table (ou {} si absent) */
static void load_lua_table(lua_State *L, const char *filepath)
{
    if (luaL_dofile(L, filepath) == LUA_OK) {
        if (!lua_istable(L, -1)) {
            lua_pop(L, 1);
            lua_newtable(L);
        }
    } else {
        lua_pop(L, 1); /* erreur message */
        lua_newtable(L);
    }
}

/* ================================================================== */
/* state — table globale persistante                                   */
/* ================================================================== */

void fw_save_state(lua_State *L)
{
    if (g_save_dir[0] == '\0') return;
    ensure_dir_recursive(g_save_dir);

    char path[1280];
    snprintf(path, sizeof(path), "%s/state.lua", g_save_dir);

    lua_getglobal(L, "state");
    if (lua_istable(L, -1)) {
        save_lua_table(L, -1, path);
    }
    lua_pop(L, 1);
}

static void load_state(lua_State *L)
{
    if (g_save_dir[0] == '\0') {
        lua_newtable(L);
        lua_setglobal(L, "state");
        return;
    }

    char path[1280];
    snprintf(path, sizeof(path), "%s/state.lua", g_save_dir);
    load_lua_table(L, path);
    lua_setglobal(L, "state");
}

void fw_reload_state(lua_State *L)
{
    load_state(L);
}

/* ================================================================== */
/* progression — save(key, data) / load(key)                           */
/* ================================================================== */

static int l_progression_save(lua_State *L)
{
    const char *key = luaL_checkstring(L, 1);
    luaL_checktype(L, 2, LUA_TTABLE);

    if (g_save_dir[0] == '\0') return 0;
    ensure_dir_recursive(g_save_dir);

    char path[1280];
    snprintf(path, sizeof(path), "%s/prog_%s.lua", g_save_dir, key);
    save_lua_table(L, 2, path);
    return 0;
}

static int l_progression_load(lua_State *L)
{
    const char *key = luaL_checkstring(L, 1);

    if (g_save_dir[0] == '\0') {
        lua_newtable(L);
        return 1;
    }

    char path[1280];
    snprintf(path, sizeof(path), "%s/prog_%s.lua", g_save_dir, key);
    load_lua_table(L, path);
    return 1;
}

static const luaL_Reg progression_funcs[] = {
    {"save", l_progression_save},
    {"load", l_progression_load},
    {NULL, NULL}
};

/* ================================================================== */
/* context_menu — set_entries(table)                                   */
/* ================================================================== */

/* Stocke la ref du tableau d'entrees dans le registry */
static int g_ctx_entries_ref = LUA_NOREF;

static int l_context_menu_set_entries(lua_State *L)
{
    luaL_checktype(L, 1, LUA_TTABLE);

    /* Liberer l'ancienne ref */
    if (g_ctx_entries_ref != LUA_NOREF) {
        luaL_unref(L, LUA_REGISTRYINDEX, g_ctx_entries_ref);
    }

    lua_pushvalue(L, 1);
    g_ctx_entries_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    return 0;
}

static const luaL_Reg context_menu_funcs[] = {
    {"set_entries", l_context_menu_set_entries},
    {NULL, NULL}
};

/* ================================================================== */
/* Context menu overlay LVGL                                           */
/* ================================================================== */

/* Callback quand un bouton du menu contextuel est clique */
typedef struct {
    lua_State *L;
    int func_ref;
} ctx_btn_data_t;

static void ctx_btn_event_cb(lv_event_t *e)
{
    ctx_btn_data_t *data = (ctx_btn_data_t *)lv_event_get_user_data(e);
    if (!data) return;

    /* Fermer le menu d'abord */
    if (g_ctx_overlay) {
        lv_obj_del(g_ctx_overlay);
        g_ctx_overlay = NULL;
    }

    /* Appeler le callback Lua */
    lua_rawgeti(data->L, LUA_REGISTRYINDEX, data->func_ref);
    if (lua_pcall(data->L, 0, 0, 0) != LUA_OK) {
        fprintf(stderr, "context_menu callback error: %s\n",
                lua_tostring(data->L, -1));
        lua_pop(data->L, 1);
    }

    /* Liberer */
    luaL_unref(data->L, LUA_REGISTRYINDEX, data->func_ref);
    free(data);
}

static void show_context_menu(lua_State *L)
{
    if (g_ctx_entries_ref == LUA_NOREF) return;
    if (g_ctx_overlay) return; /* deja affiche */

    lua_rawgeti(L, LUA_REGISTRYINDEX, g_ctx_entries_ref);
    if (!lua_istable(L, -1)) {
        lua_pop(L, 1);
        return;
    }

    /* Creer l'overlay plein ecran */
    g_ctx_overlay = lv_obj_create(lv_scr_act());
    lv_obj_remove_style_all(g_ctx_overlay);
    lv_obj_set_size(g_ctx_overlay, 320, 240);
    lv_obj_set_style_bg_color(g_ctx_overlay, lv_color_hex(0x1a1a2e), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(g_ctx_overlay, LV_OPA_COVER, LV_PART_MAIN);
    lv_obj_set_flex_flow(g_ctx_overlay, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_flex_align(g_ctx_overlay, LV_FLEX_ALIGN_CENTER,
                          LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_row(g_ctx_overlay, 8, LV_PART_MAIN);

    /* Titre */
    lv_obj_t *title = lv_label_create(g_ctx_overlay);
    lv_label_set_text(title, "Menu");
    lv_obj_set_style_text_color(title, lv_color_white(), LV_PART_MAIN);

    /* Parcourir les entrees */
    int len = (int)lua_rawlen(L, -1);
    lv_obj_t *first_btn = NULL;
    for (int i = 1; i <= len; i++) {
        lua_rawgeti(L, -1, i);
        if (!lua_istable(L, -1)) { lua_pop(L, 1); continue; }

        /* Lire title */
        lua_getfield(L, -1, "title");
        const char *entry_title = lua_tostring(L, -1);
        lua_pop(L, 1);

        /* Lire cb */
        lua_getfield(L, -1, "cb");
        int has_cb = lua_isfunction(L, -1);
        int func_ref = LUA_NOREF;
        if (has_cb) {
            func_ref = luaL_ref(L, LUA_REGISTRYINDEX);
        } else {
            lua_pop(L, 1);
        }

        lua_pop(L, 1); /* pop entry table */

        /* Creer un bouton */
        lv_obj_t *btn = lv_btn_create(g_ctx_overlay);
        lv_obj_set_width(btn, 280);
        lv_obj_set_style_bg_color(btn, lv_color_hex(0x16213e), LV_PART_MAIN);
        lv_obj_set_style_radius(btn, 8, LV_PART_MAIN);

        lv_obj_t *lbl = lv_label_create(btn);
        lv_label_set_text(lbl, entry_title ? entry_title : "?");
        lv_obj_set_style_text_color(lbl, lv_color_white(), LV_PART_MAIN);
        lv_obj_center(lbl);

        if (has_cb) {
            ctx_btn_data_t *data = (ctx_btn_data_t *)malloc(sizeof(ctx_btn_data_t));
            data->L = L;
            data->func_ref = func_ref;
            lv_obj_add_event_cb(btn, ctx_btn_event_cb, LV_EVENT_CLICKED, data);
        }

        if (!first_btn) first_btn = btn;
    }

    /* Bouton fermer */
    lv_obj_t *close_btn = lv_btn_create(g_ctx_overlay);
    lv_obj_set_width(close_btn, 280);
    lv_obj_set_style_bg_color(close_btn, lv_color_hex(0x533483), LV_PART_MAIN);
    lv_obj_set_style_radius(close_btn, 8, LV_PART_MAIN);

    lv_obj_t *close_lbl = lv_label_create(close_btn);
    lv_label_set_text(close_lbl, "Fermer");
    lv_obj_set_style_text_color(close_lbl, lv_color_white(), LV_PART_MAIN);
    lv_obj_center(close_lbl);

    /* Le callback fermer : pas de func_ref, juste supprimer l'overlay */
    ctx_btn_data_t *close_data = (ctx_btn_data_t *)malloc(sizeof(ctx_btn_data_t));
    close_data->L = L;
    close_data->func_ref = LUA_NOREF;
    lv_obj_add_event_cb(close_btn, ctx_btn_event_cb, LV_EVENT_CLICKED, close_data);

    lua_pop(L, 1); /* pop entries table */

    /* Forcer le focus sur le premier bouton du menu pour la navigation encodeur */
    lv_obj_t *focus_target = first_btn ? first_btn : close_btn;
    lv_group_t *def_grp = lv_group_get_default();
    if (def_grp) {
        lv_group_focus_obj(focus_target);
    }
}

static void close_context_menu(void)
{
    if (g_ctx_overlay) {
        lv_obj_del(g_ctx_overlay);
        g_ctx_overlay = NULL;
    }
}

/* ================================================================== */
/* goto_library — quitter l'histoire                                   */
/* ================================================================== */

static int g_want_library = 0;

static int l_goto_library(lua_State *L)
{
    fprintf(stderr, "[FW] goto_library() called — saving state, returning to browser.\n");
    fw_save_state(L);
    g_want_library = 1;
    return 0;
}

/* ================================================================== */
/* screen — stubs                                                      */
/* ================================================================== */

static int l_screen_set_state(lua_State *L)
{
    (void)L; /* no-op sur desktop */
    return 0;
}

static int l_screen_wake_up(lua_State *L)
{
    (void)L;
    return 0;
}

static int l_screen_set_brightness(lua_State *L)
{
    (void)L;
    return 0;
}

static int l_screen_on_state_changed(lua_State *L)
{
    (void)L; /* no-op */
    return 0;
}

static const luaL_Reg screen_funcs[] = {
    {"set_state",        l_screen_set_state},
    {"wake_up",          l_screen_wake_up},
    {"set_brightness",   l_screen_set_brightness},
    {"on_state_changed", l_screen_on_state_changed},
    {NULL, NULL}
};

/* ================================================================== */
/* Triggers depuis le driver SDL                                       */
/* ================================================================== */

void fw_trigger_context_menu(void)
{
    g_want_context_menu = 1;
}

void fw_trigger_back(void)
{
    g_want_back = 1;
}

/* ================================================================== */
/* Pump — gerer les evenements firmware                                */
/* ================================================================== */

int fw_pump(lua_State *L)
{
    /* Context menu : touche M */
    if (g_want_context_menu) {
        g_want_context_menu = 0;
        if (g_ctx_overlay) {
            close_context_menu();
        } else {
            show_context_menu(L);
        }
    }

    /* Back : touche ESC */
    if (g_want_back) {
        g_want_back = 0;
        fprintf(stderr, "[FW] ESC pressed (back)\n");
        if (g_ctx_overlay) {
            close_context_menu();
        } else {
            /* Appeler back_callback() si defini */
            lua_getglobal(L, "back_callback");
            if (lua_isfunction(L, -1)) {
                fprintf(stderr, "[FW] calling back_callback()\n");
                if (lua_pcall(L, 0, 0, 0) != LUA_OK) {
                    fprintf(stderr, "back_callback error: %s\n",
                            lua_tostring(L, -1));
                    lua_pop(L, 1);
                }
            } else {
                lua_pop(L, 1);
                fprintf(stderr, "[FW] no back_callback, calling goto_library\n");
                g_want_library = 1;
            }
        }
    }

    if (g_want_library) {
        g_want_library = 0;
        fprintf(stderr, "[FW] returning to story browser\n");
        return 2;  /* return to story browser */
    }
    return g_want_quit ? 1 : 0;
}

/* ================================================================== */
/* Enregistrement                                                      */
/* ================================================================== */

void fw_register_globals(lua_State *L)
{
    /* state — table persistante */
    load_state(L);

    /* progression — table avec save/load */
    lua_newtable(L);
    luaL_setfuncs(L, progression_funcs, 0);
    lua_setglobal(L, "progression");

    /* context_menu */
    lua_newtable(L);
    luaL_setfuncs(L, context_menu_funcs, 0);
    lua_setglobal(L, "context_menu");

    /* goto_library() */
    lua_pushcfunction(L, l_goto_library);
    lua_setglobal(L, "goto_library");

    /* back_callback — par defaut = goto_library */
    lua_pushcfunction(L, l_goto_library);
    lua_setglobal(L, "back_callback");

    /* screen — stubs */
    lua_newtable(L);
    luaL_setfuncs(L, screen_funcs, 0);
    lua_setglobal(L, "screen");

    /* progress — variable numerique globale */
    lua_pushinteger(L, 0);
    lua_setglobal(L, "progress");
}

void fw_reset(void)
{
    g_save_dir[0] = '\0';
    g_want_quit = 0;
    g_want_context_menu = 0;
    g_want_back = 0;
    g_want_library = 0;
    g_ctx_overlay = NULL;
    g_ctx_entries_ref = LUA_NOREF;
}
