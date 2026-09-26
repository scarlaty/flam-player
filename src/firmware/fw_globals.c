/**
 * fw_globals.c — Objets firmware injectes dans Lua
 *
 * Implemente : state, progression, context_menu, back_callback,
 *              goto_library, screen, progress
 *              + flam_assert_crash (LV_ASSERT_HANDLER, voir lv_conf.h)
 */

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <io.h>
#else
#include <unistd.h>
#endif

#include "fw_globals.h"
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"
#include "lvgl/lvgl.h"
#include "lvgl/src/misc/lv_gc.h"
#include "bindings/lua_lv.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <sys/stat.h>
#include <errno.h>

/* ================================================================== */
/* Repertoire de sauvegarde                                            */
/* ================================================================== */

static char g_save_dir[1024] = "";
static int  g_want_quit = 0;
static int  g_want_context_menu = 0;
static int  g_want_back = 0;

/* Etat Lua courant (pour le traceback de flam_assert_crash) */
static lua_State *g_fw_L = NULL;

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

/* Profondeur maximale des tables sauvegardees */
#define SERIALIZE_MAX_DEPTH 32

/* Ecrit une chaine Lua litterale (valeurs ET cles) */
static void write_lua_string(FILE *f, const char *s, size_t len)
{
    fputc('"', f);
    for (size_t i = 0; i < len; i++) {
        unsigned char c = (unsigned char)s[i];
        switch (c) {
        case '"':  fputs("\\\"", f); break;
        case '\\': fputs("\\\\", f); break;
        case '\n': fputs("\\n", f);  break;
        case '\r': fputs("\\r", f);  break;
        case '\t': fputs("\\t", f);  break;
        default:
            /* \ddd sur 3 chiffres : pas d'ambiguite avec un chiffre suivant */
            if (c < 0x20 || c == 0x7f) fprintf(f, "\\%03u", (unsigned)c);
            else fputc((int)c, f);
            break;
        }
    }
    fputc('"', f);
}

/* Ecrit un nombre Lua en conservant son sous-type (entier / flottant) */
static void write_lua_number(lua_State *L, int idx, FILE *f)
{
    if (lua_isinteger(L, idx)) {
        lua_Integer v = lua_tointeger(L, idx);
        if (v == LUA_MININTEGER) {
            /* -9223372036854775808 serait relu comme un flottant */
            fprintf(f, "(%lld-1)", (long long)(v + 1));
        } else {
            fprintf(f, "%lld", (long long)v);
        }
        return;
    }
    lua_Number n = lua_tonumber(L, idx);
    if (n != n) { fputs("(0/0)", f); return; }
    if (isinf(n)) { fputs(n > 0 ? "(1/0)" : "(-1/0)", f); return; }
    char buf[64];
    snprintf(buf, sizeof(buf), "%.17g", (double)n);
    fputs(buf, f);
    /* "1" serait relu comme un entier : ajouter ".0" comme tostring() */
    if (buf[strspn(buf, "-0123456789")] == '\0') fputs(".0", f);
}

/* Serialise une valeur Lua sur la pile en texte Lua dans un FILE*.
   visited : index absolu d'une table des tables en cours d'ecriture
   (detection de cycles). */
static void serialize_value(lua_State *L, int idx, FILE *f, int indent,
                            int depth, int visited)
{
    int abs_idx = lua_absindex(L, idx);

    switch (lua_type(L, abs_idx)) {
    case LUA_TNIL:
        fprintf(f, "nil");
        break;
    case LUA_TBOOLEAN:
        fprintf(f, lua_toboolean(L, abs_idx) ? "true" : "false");
        break;
    case LUA_TNUMBER:
        write_lua_number(L, abs_idx, f);
        break;
    case LUA_TSTRING: {
        size_t len = 0;
        const char *s = lua_tolstring(L, abs_idx, &len);
        write_lua_string(f, s, len);
        break;
    }
    case LUA_TTABLE: {
        if (depth >= SERIALIZE_MAX_DEPTH) {
            fprintf(stderr, "fw: state trop profond (> %d), sous-table ignoree\n",
                    SERIALIZE_MAX_DEPTH);
            fprintf(f, "nil");
            break;
        }
        if (!lua_checkstack(L, 4)) {
            fprintf(stderr, "fw: pile Lua saturee, sous-table ignoree\n");
            fprintf(f, "nil");
            break;
        }
        /* Cycle : la table est deja en cours d'ecriture */
        lua_pushvalue(L, abs_idx);
        if (lua_rawget(L, visited) != LUA_TNIL) {
            lua_pop(L, 1);
            fprintf(stderr, "fw: cycle dans state, reference ignoree\n");
            fprintf(f, "nil");
            break;
        }
        lua_pop(L, 1);
        lua_pushvalue(L, abs_idx);
        lua_pushboolean(L, 1);
        lua_rawset(L, visited);

        fprintf(f, "{\n");
        lua_pushnil(L);
        int first = 1;
        while (lua_next(L, abs_idx) != 0) {
            int kt = lua_type(L, -2);
            if (kt != LUA_TSTRING && kt != LUA_TNUMBER && kt != LUA_TBOOLEAN) {
                /* Cle non serialisable (table, fonction...) : entree ignoree */
                lua_pop(L, 1);
                continue;
            }

            if (!first) fprintf(f, ",\n");
            first = 0;

            /* indentation */
            for (int i = 0; i < indent + 1; i++) fprintf(f, "  ");

            /* cle (sans lua_tolstring sur un nombre : casserait lua_next) */
            fputc('[', f);
            if (kt == LUA_TSTRING) {
                size_t klen = 0;
                const char *k = lua_tolstring(L, -2, &klen);
                write_lua_string(f, k, klen);
            } else if (kt == LUA_TNUMBER) {
                write_lua_number(L, -2, f);
            } else {
                fputs(lua_toboolean(L, -2) ? "true" : "false", f);
            }
            fputs("] = ", f);

            /* valeur */
            serialize_value(L, -1, f, indent + 1, depth + 1, visited);

            lua_pop(L, 1); /* pop valeur, garde cle */
        }
        fprintf(f, "\n");
        for (int i = 0; i < indent; i++) fprintf(f, "  ");
        fprintf(f, "}");

        /* Fin d'ecriture : une meme sous-table partagee (sans cycle)
           reste ecrite a chaque occurrence */
        lua_pushvalue(L, abs_idx);
        lua_pushnil(L);
        lua_rawset(L, visited);
        break;
    }
    default:
        /* Ignorer functions, userdata, etc. */
        fprintf(f, "nil");
        break;
    }
}

/* Corps protege de la serialisation : (FILE*, table) */
static int serialize_protected(lua_State *L)
{
    FILE *f = (FILE *)lua_touserdata(L, 1);
    lua_newtable(L); /* visited */
    int visited = lua_gettop(L);
    fprintf(f, "return ");
    serialize_value(L, 2, f, 0, 0, visited);
    fprintf(f, "\n");
    return 0;
}

/* Sauvegarde la table sur la pile dans un fichier Lua.
   Ecriture atomique : fichier .tmp puis remplacement, l'ancien fichier
   reste intact si l'ecriture echoue. */
static int save_lua_table(lua_State *L, int tbl_idx, const char *filepath)
{
    int abs_idx = lua_absindex(L, tbl_idx);
    char tmp_path[1300];
    snprintf(tmp_path, sizeof(tmp_path), "%s.tmp", filepath);

    if (!lua_checkstack(L, 4)) {
        fprintf(stderr, "fw: pile Lua saturee, '%s' non sauvegarde\n", filepath);
        return -1;
    }

    FILE *f = fopen(tmp_path, "wb");
    if (!f) {
        fprintf(stderr, "fw: cannot write '%s': %s\n", tmp_path, strerror(errno));
        return -1;
    }

    /* Protege : une erreur memoire Lua ne doit ni sauter hors d'ici
       (fichier ouvert) ni paniquer depuis la boucle principale */
    int err = 0;
    lua_pushcfunction(L, serialize_protected);
    lua_pushlightuserdata(L, f);
    lua_pushvalue(L, abs_idx);
    if (lua_pcall(L, 2, 0, 0) != LUA_OK) {
        fprintf(stderr, "fw: serialisation de '%s' echouee: %s\n", filepath,
                lua_tostring(L, -1));
        lua_pop(L, 1);
        err = 1;
    }

    if (fflush(f) != 0 || ferror(f)) err = 1;
#ifdef _WIN32
    if (!err && _commit(_fileno(f)) != 0) err = 1;
#else
    if (!err && fsync(fileno(f)) != 0) err = 1;
#endif
    if (fclose(f) != 0) err = 1;

    if (err) {
        fprintf(stderr, "fw: ecriture de '%s' echouee, ancien fichier conserve\n",
                filepath);
        remove(tmp_path);
        return -1;
    }

#ifdef _WIN32
    if (!MoveFileExA(tmp_path, filepath,
                     MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
        fprintf(stderr, "fw: remplacement de '%s' echoue (err %lu)\n",
                filepath, (unsigned long)GetLastError());
        remove(tmp_path);
        return -1;
    }
#else
    if (rename(tmp_path, filepath) != 0) {
        fprintf(stderr, "fw: remplacement de '%s' echoue: %s\n",
                filepath, strerror(errno));
        remove(tmp_path);
        return -1;
    }
#endif
    return 0;
}

/* Charge un fichier Lua qui retourne une table ; pousse la table (ou {} si absent) */
static void load_lua_table(lua_State *L, const char *filepath)
{
    /* Source texte uniquement (bytecode refuse, comme main.c), execute dans
       un _ENV vide : le fichier ne contient que des litteraux.
       Exactement 1 resultat : un fichier vide ("return" sans valeur) ne
       doit pas faire lire/depiler une valeur de l'appelant. */
    if (luaL_loadfilex(L, filepath, "t") == LUA_OK) {
        lua_newtable(L);
        if (!lua_setupvalue(L, -2, 1)) lua_pop(L, 1);
        if (lua_pcall(L, 0, 1, 0) == LUA_OK) {
            if (!lua_istable(L, -1)) {
                lua_pop(L, 1);
                lua_newtable(L);
            }
            return;
        }
    }
    lua_pop(L, 1); /* message d'erreur */
    lua_newtable(L);
}

/* Construit "<save_dir>/<name>" ; -1 si tronque (chemin refuse) */
static int build_save_path(char *out, size_t out_sz, const char *name)
{
    int n = snprintf(out, out_sz, "%s/%s", g_save_dir, name);
    if (n < 0 || (size_t)n >= out_sz) {
        fprintf(stderr, "fw: chemin de sauvegarde trop long ('%s/%s')\n",
                g_save_dir, name);
        return -1;
    }
    return 0;
}

/* ================================================================== */
/* state — table globale persistante                                   */
/* ================================================================== */

void fw_save_state(lua_State *L)
{
    if (g_save_dir[0] == '\0') return;
    ensure_dir_recursive(g_save_dir);

    char path[1280];
    if (build_save_path(path, sizeof(path), "state.lua") != 0) return;

    /* Lecture brute (sans __index) : hors pcall, un _G strict ferait PANIC */
    lua_pushglobaltable(L);
    lua_pushliteral(L, "state");
    lua_rawget(L, -2);
    lua_remove(L, -2);
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
    if (build_save_path(path, sizeof(path), "state.lua") != 0) {
        lua_newtable(L);
    } else {
        load_lua_table(L, path);
    }
    lua_setglobal(L, "state");
}

void fw_reload_state(lua_State *L)
{
    load_state(L);
}

/* ================================================================== */
/* progression — save(key, data) / load(key)                           */
/* ================================================================== */

/* Longueur max d'une cle de progression */
#define PROG_KEY_MAX 64

/* La cle devient un nom de fichier prog_<key>.lua sous save_dir :
   seuls [A-Za-z0-9_-] sont acceptes (pas de "/", "\", "..", ":",
   ni octet nul), 1 a PROG_KEY_MAX caracteres. Erreur Lua sinon. */
static const char *check_prog_key(lua_State *L, int arg)
{
    size_t len = 0;
    const char *key = luaL_checklstring(L, arg, &len);
    if (len == 0 || len > PROG_KEY_MAX) {
        luaL_argerror(L, arg, lua_pushfstring(L,
            "cle de progression invalide (1 a %d caracteres)", PROG_KEY_MAX));
    }
    for (size_t i = 0; i < len; i++) {
        unsigned char c = (unsigned char)key[i];
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
              (c >= '0' && c <= '9') || c == '_' || c == '-')) {
            luaL_argerror(L, arg,
                "cle de progression invalide (caracteres autorises : A-Z a-z 0-9 _ -)");
        }
    }
    return key;
}

/* Chemin prog_<key>.lua sous save_dir ; -1 si impossible */
static int build_prog_path(char *out, size_t out_sz, const char *key)
{
    char name[PROG_KEY_MAX + 16];
    snprintf(name, sizeof(name), "prog_%s.lua", key);
    return build_save_path(out, out_sz, name);
}

static int l_progression_save(lua_State *L)
{
    const char *key = check_prog_key(L, 1);
    luaL_checktype(L, 2, LUA_TTABLE);

    if (g_save_dir[0] == '\0') return 0;
    ensure_dir_recursive(g_save_dir);

    char path[1280];
    if (build_prog_path(path, sizeof(path), key) != 0) return 0;
    save_lua_table(L, 2, path);
    return 0;
}

static int l_progression_load(lua_State *L)
{
    const char *key = check_prog_key(L, 1);

    char path[1280];
    if (g_save_dir[0] == '\0' ||
        build_prog_path(path, sizeof(path), key) != 0) {
        lua_newtable(L);
        return 1;
    }

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

/*
 * Le menu a son propre groupe LVGL (g_ctx_group) : les touches ne vont
 * plus aux widgets de l'histoire derriere l'overlay, et le groupe de
 * l'histoire (focus + mode editing) n'est pas touche. A l'ouverture, les
 * indev clavier/encodeur rattaches au groupe de l'histoire basculent sur
 * g_ctx_group ; a la fermeture ils y reviennent.
 *
 * La remise en etat est faite dans le LV_EVENT_DELETE de l'overlay : elle
 * a lieu quelle que soit la facon dont l'overlay disparait (touche M/ESC,
 * clic sur une entree, lv_obj_clean de l'ecran au retour bibliotheque).
 *
 * Un clic ne detruit rien depuis le traitement indev : fermeture et appel
 * du callback Lua sont differes via lv_async_call (indev_encoder_proc
 * reutilise l'objet et le groupe apres l'envoi de LV_EVENT_CLICKED).
 * Le callback asynchrone ne capture aucun pointeur d'objet : il relit
 * l'etat global, donc sans risque si l'overlay a disparu entre temps.
 */

/* Donnees d'un bouton d'entree ayant un callback Lua */
typedef struct {
    lua_State *L;
    int func_ref;
} ctx_btn_data_t;

static lv_group_t *g_ctx_group        = NULL; /* groupe dedie au menu */
static lv_group_t *g_ctx_prev_group   = NULL; /* groupe de l'histoire */
static lv_obj_t   *g_ctx_prev_focus   = NULL; /* focus sauvegarde */
static bool        g_ctx_prev_editing = false;

/* Clic en attente de traitement asynchrone */
static int         g_ctx_click_pending = 0;
static lua_State  *g_ctx_click_L       = NULL;
static int         g_ctx_click_ref     = LUA_NOREF;

/* Le groupe existe-t-il encore ? (main.c peut l'avoir detruit) */
static bool group_is_alive(lv_group_t *grp)
{
    if (!grp) return false;
    lv_group_t *g;
    _LV_LL_READ(&LV_GC_ROOT(_lv_group_ll), g) {
        if (g == grp) return true;
    }
    return false;
}

static bool indev_uses_group(lv_indev_t *indev)
{
    lv_indev_type_t t = lv_indev_get_type(indev);
    return t == LV_INDEV_TYPE_KEYPAD || t == LV_INDEV_TYPE_ENCODER;
}

/* Bascule les indev clavier/encodeur rattaches a 'from' vers 'to' */
static void ctx_switch_indev_group(lv_group_t *from, lv_group_t *to)
{
    for (lv_indev_t *indev = lv_indev_get_next(NULL); indev;
         indev = lv_indev_get_next(indev)) {
        if (!indev_uses_group(indev)) continue;
        if (indev->group != from) continue;
        lv_indev_set_group(indev, to);
        /* Une touche tenue pendant la bascule ne doit pas cliquer de
           l'autre cote de l'overlay a son relachement (seulement si une
           touche est tenue : sinon le prochain appui serait avale) */
        if (indev->proc.types.keypad.last_state == LV_INDEV_STATE_PRESSED) {
            lv_indev_wait_release(indev);
        }
    }
}

static void ctx_clear_pending_click(void)
{
    g_ctx_click_pending = 0;
    g_ctx_click_L = NULL;
    g_ctx_click_ref = LUA_NOREF;
}

/* LV_EVENT_DELETE de l'overlay : restaure la navigation de l'histoire.
   Appele avant la suppression des boutons (encore valides ici). */
static void ctx_overlay_delete_cb(lv_event_t *e)
{
    if (lv_event_get_target(e) != g_ctx_overlay) return;
    g_ctx_overlay = NULL;

    /* Un clic non encore traite vise des refs liberees juste apres */
    ctx_clear_pending_click();

    lv_group_t *prev = group_is_alive(g_ctx_prev_group) ? g_ctx_prev_group : NULL;

    if (g_ctx_group) {
        ctx_switch_indev_group(g_ctx_group, prev);
        lv_group_del(g_ctx_group);
        g_ctx_group = NULL;
    }

    /* Restaurer le mode editing si le focus de l'histoire n'a pas bouge */
    if (prev && g_ctx_prev_focus &&
        lv_group_get_focused(prev) == g_ctx_prev_focus &&
        lv_group_get_editing(prev) != g_ctx_prev_editing) {
        lv_group_set_editing(prev, g_ctx_prev_editing);
    }

    g_ctx_prev_group = NULL;
    g_ctx_prev_focus = NULL;
    g_ctx_prev_editing = false;
}

static void close_context_menu(void)
{
    if (g_ctx_overlay) {
        lv_obj_del(g_ctx_overlay); /* -> ctx_overlay_delete_cb */
    }
}

/* Traitement differe d'un clic : fermer le menu puis appeler le cb Lua */
static void ctx_click_async_cb(void *user_data)
{
    (void)user_data;
    if (!g_ctx_click_pending) return;

    lua_State *L = g_ctx_click_L;
    int ref = g_ctx_click_ref;
    ctx_clear_pending_click();

    /* Pousser la fonction avant la fermeture : le DELETE des boutons
       libere les refs, la valeur reste ancree sur la pile */
    int has_fn = 0;
    if (L && ref != LUA_NOREF && ref != LUA_REFNIL) {
        lua_rawgeti(L, LUA_REGISTRYINDEX, ref);
        has_fn = lua_isfunction(L, -1);
        if (!has_fn) lua_pop(L, 1);
    }

    close_context_menu();

    if (has_fn) {
        if (lua_pcall(L, 0, 0, 0) != LUA_OK) {
            fprintf(stderr, "context_menu callback error: %s\n",
                    lua_tostring(L, -1));
            lua_pop(L, 1);
        }
    }
}

/* Evenements des boutons du menu (entrees et Fermer) */
static void ctx_btn_event_cb(lv_event_t *e)
{
    lv_event_code_t code = lv_event_get_code(e);
    ctx_btn_data_t *data = (ctx_btn_data_t *)lv_event_get_user_data(e);

    if (code == LV_EVENT_DELETE) {
        /* Unique point de liberation (aucun remove_event_cb ailleurs) */
        if (data) {
            if (data->func_ref != LUA_NOREF) {
                luaL_unref(data->L, LUA_REGISTRYINDEX, data->func_ref);
                data->func_ref = LUA_NOREF;
            }
            free(data);
        }
        return;
    }

    if (code != LV_EVENT_CLICKED) return;
    if (g_ctx_click_pending) return; /* un seul clic par ouverture */

    g_ctx_click_pending = 1;
    g_ctx_click_L   = data ? data->L : NULL;
    g_ctx_click_ref = data ? data->func_ref : LUA_NOREF;
    lv_async_call(ctx_click_async_cb, NULL);
}

/* Cout estime d'un bouton du menu dans le tas LVGL (instances bouton +
   label, attributs speciaux, styles locaux, texte, evenements) */
#define CTX_BTN_HEAP_COST  2048U

/* Garde du tas LVGL sans erreur Lua (meme regle que lua_lv_mem_check) :
   lv_obj_class_create_obj realloue le tableau des enfants du parent et
   lv_label_set_text alloue le texte sans tester NULL (LV_ASSERT_MALLOC). */
static bool ctx_heap_ok(size_t contig, size_t extra)
{
#if LV_MEM_CUSTOM == 0
    lv_mem_monitor_t mon;
    lv_mem_monitor(&mon);
    size_t need_contig = contig + LUA_LV_MEM_MARGIN;
    size_t need_total  = contig + extra + LUA_LV_MEM_MARGIN;
    return (size_t)mon.free_biggest_size >= need_contig &&
           (size_t)mon.free_size >= need_total;
#else
    (void)contig; (void)extra;
    return true;
#endif
}

/* Place pour un bouton de plus dans l'overlay (+ 'reserve' boutons) */
static bool ctx_heap_ok_for_btn(unsigned reserve)
{
    size_t cnt = g_ctx_overlay ? lv_obj_get_child_cnt(g_ctx_overlay) : 0;
    return ctx_heap_ok((cnt + 1 + reserve) * sizeof(lv_obj_t *),
                       (size_t)(1 + reserve) * CTX_BTN_HEAP_COST);
}

/* NULL si le bouton n'a pas pu etre cree ('data' reste alors a l'appelant) */
static lv_obj_t *ctx_create_btn(const char *text, uint32_t color,
                                ctx_btn_data_t *data)
{
    lv_obj_t *btn = lv_btn_create(g_ctx_overlay);
    if (!btn) return NULL;
    lv_obj_set_width(btn, 280);
    lv_obj_set_style_bg_color(btn, lv_color_hex(color), LV_PART_MAIN);
    lv_obj_set_style_radius(btn, 8, LV_PART_MAIN);

    lv_obj_t *lbl = lv_label_create(btn);
    if (!lbl) {
        lv_obj_del(btn); /* aucun evenement encore attache */
        return NULL;
    }
    lv_label_set_text(lbl, text);
    lv_obj_set_style_text_color(lbl, lv_color_white(), LV_PART_MAIN);
    lv_obj_center(lbl);

    /* Toute entree ferme le menu, avec ou sans callback */
    lv_obj_add_event_cb(btn, ctx_btn_event_cb, LV_EVENT_CLICKED, data);
    if (data) {
        lv_obj_add_event_cb(btn, ctx_btn_event_cb, LV_EVENT_DELETE, data);
    }
    return btn;
}

/* Resultat de ctx_build_entries */
typedef struct {
    lv_obj_t *first_btn;
    int       shown;      /* boutons d'entree crees */
    int       truncated;  /* 1 si le tas LVGL a manque */
} ctx_build_t;

/* Corps protege : (ctx_build_t*, entrees). Appele depuis fw_pump, hors de
   tout pcall : une erreur Lua (memoire) ne doit pas paniquer. Acces bruts
   uniquement (pas de __index), chaque bouton est precede d'une garde du tas
   qui reserve la place du bouton "Fermer". */
static int ctx_build_entries(lua_State *L)
{
    ctx_build_t *b = (ctx_build_t *)lua_touserdata(L, 1);
    int len = (int)lua_rawlen(L, 2);

    for (int i = 1; i <= len; i++) {
        if (lua_rawgeti(L, 2, i) != LUA_TTABLE) { lua_pop(L, 1); continue; }

        if (!ctx_heap_ok_for_btn(1)) {
            lua_pop(L, 1);
            b->truncated = 1;
            break;
        }

        /* Lire title (copie : la chaine est depilee avant usage) */
        lua_pushliteral(L, "title");
        lua_rawget(L, -2);
        const char *t = lua_tostring(L, -1);
        char entry_title[128];
        snprintf(entry_title, sizeof(entry_title), "%s", t ? t : "?");
        lua_pop(L, 1);

        /* Lire cb : ref avant malloc (luaL_ref peut lever une erreur) */
        ctx_btn_data_t *data = NULL;
        lua_pushliteral(L, "cb");
        lua_rawget(L, -2);
        if (lua_isfunction(L, -1)) {
            int ref = luaL_ref(L, LUA_REGISTRYINDEX);
            data = (ctx_btn_data_t *)malloc(sizeof(ctx_btn_data_t));
            if (data) {
                data->L = L;
                data->func_ref = ref;
            } else {
                luaL_unref(L, LUA_REGISTRYINDEX, ref);
            }
        } else {
            lua_pop(L, 1);
        }

        lua_pop(L, 1); /* pop entry table */

        lv_obj_t *btn = ctx_create_btn(entry_title, 0x16213e, data);
        if (!btn) {
            if (data) {
                luaL_unref(L, LUA_REGISTRYINDEX, data->func_ref);
                free(data);
            }
            b->truncated = 1;
            break;
        }
        if (!b->first_btn) b->first_btn = btn;
        b->shown++;
    }
    return 0;
}

static void show_context_menu(lua_State *L)
{
    if (g_ctx_entries_ref == LUA_NOREF) return;
    if (g_ctx_overlay) return; /* deja affiche */

    if (!lua_checkstack(L, 4)) return;
    lua_rawgeti(L, LUA_REGISTRYINDEX, g_ctx_entries_ref);
    if (!lua_istable(L, -1)) {
        lua_pop(L, 1);
        return;
    }
    int entries_idx = lua_gettop(L);

    /* Place pour groupe + overlay + titre + bouton Fermer */
    {
        lv_obj_t *scr = lv_scr_act();
        size_t cnt = scr ? lv_obj_get_child_cnt(scr) : 0;
        if (!scr || !ctx_heap_ok((cnt + 1) * sizeof(lv_obj_t *),
                                 4 * CTX_BTN_HEAP_COST)) {
            fprintf(stderr, "context_menu: tas LVGL epuise, menu non affiche\n");
            lua_pop(L, 1);
            return;
        }
    }

    /* Groupe de l'histoire : celui de l'indev clavier/encodeur, sinon le
       groupe par defaut. Sauvegarde focus + editing. */
    lv_group_t *def_grp = lv_group_get_default();
    lv_group_t *prev = NULL;
    for (lv_indev_t *indev = lv_indev_get_next(NULL); indev;
         indev = lv_indev_get_next(indev)) {
        if (indev_uses_group(indev) && indev->group) {
            prev = indev->group;
            break;
        }
    }
    if (!prev) prev = def_grp;
    g_ctx_prev_group   = prev;
    g_ctx_prev_focus   = prev ? lv_group_get_focused(prev) : NULL;
    g_ctx_prev_editing = prev ? lv_group_get_editing(prev) : false;

    /* Groupe dedie, rendu "par defaut" le temps de creer les boutons
       (sinon lv_btn_create les ajouterait au groupe de l'histoire) */
    g_ctx_group = lv_group_create();
    lv_group_set_wrap(g_ctx_group, true);
    lv_group_set_default(g_ctx_group);

    /* Creer l'overlay plein ecran */
    g_ctx_overlay = lv_obj_create(lv_scr_act());
    if (!g_ctx_overlay) {
        fprintf(stderr, "context_menu: tas LVGL epuise, menu non affiche\n");
        lv_group_set_default(def_grp);
        lv_group_del(g_ctx_group);
        g_ctx_group = NULL;
        g_ctx_prev_group = NULL;
        g_ctx_prev_focus = NULL;
        g_ctx_prev_editing = false;
        lua_pop(L, 1);
        return;
    }
    lv_obj_remove_style_all(g_ctx_overlay);
    lv_obj_set_size(g_ctx_overlay, 320, 240);
    lv_obj_set_style_bg_color(g_ctx_overlay, lv_color_hex(0x1a1a2e), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(g_ctx_overlay, LV_OPA_COVER, LV_PART_MAIN);
    lv_obj_set_flex_flow(g_ctx_overlay, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_flex_align(g_ctx_overlay, LV_FLEX_ALIGN_CENTER,
                          LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_row(g_ctx_overlay, 8, LV_PART_MAIN);
    lv_obj_add_event_cb(g_ctx_overlay, ctx_overlay_delete_cb, LV_EVENT_DELETE, NULL);

    /* Titre */
    lv_obj_t *title = lv_label_create(g_ctx_overlay);
    if (title) {
        lv_label_set_text(title, "Menu");
        lv_obj_set_style_text_color(title, lv_color_white(), LV_PART_MAIN);
    }

    /* Parcourir les entrees (mode protege) */
    ctx_build_t b = { NULL, 0, 0 };
    int ok = 1;
    lua_pushcfunction(L, ctx_build_entries);
    lua_pushlightuserdata(L, &b);
    lua_pushvalue(L, entries_idx);
    if (lua_pcall(L, 2, 0, 0) != LUA_OK) {
        fprintf(stderr, "context_menu: construction du menu echouee: %s\n",
                lua_tostring(L, -1));
        lua_pop(L, 1);
        ok = 0;
    }
    lua_pop(L, 1); /* pop entries table */

    if (b.truncated) {
        fprintf(stderr, "context_menu: tas LVGL insuffisant, menu tronque "
                "a %d entree(s)\n", b.shown);
    }

    /* Bouton fermer : pas de callback, ferme seulement le menu
       (sa place a ete reservee par la garde de chaque entree) */
    lv_obj_t *close_btn = ok ? ctx_create_btn("Fermer", 0x533483, NULL) : NULL;

    /* Rendre au groupe de l'histoire son statut de groupe par defaut */
    lv_group_set_default(def_grp);

    if (!ok || (!b.first_btn && !close_btn)) {
        /* Menu inutilisable : tout defaire (-> ctx_overlay_delete_cb) */
        close_context_menu();
        return;
    }

    /* Basculer la navigation clavier/encodeur sur le menu */
    ctx_switch_indev_group(prev, g_ctx_group);
    lv_group_focus_obj(b.first_btn ? b.first_btn : close_btn);
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
            /* Lecture brute (sans __index), comme pour setup() dans main.c */
            lua_pushglobaltable(L);
            lua_pushliteral(L, "back_callback");
            lua_rawget(L, -2);
            lua_remove(L, -2);
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
    g_fw_L = L;

    /* Nouvel etat Lua : toute ref registre d'un etat precedent est caduque
       (flam-test enchaine plusieurs fichiers sans fw_reset). La liberer
       ici ferait luaL_unref d'un slot vivant du nouveau registre. */
    g_ctx_entries_ref = LUA_NOREF;

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
    g_fw_L = NULL;

    /* Normalement deja fait par le LV_EVENT_DELETE de l'overlay
       (lv_obj_clean de l'ecran avant lua_close). Sinon, ne pas supprimer
       l'overlay ici : ses boutons feraient luaL_unref sur un etat ferme. */
    lv_async_call_cancel(ctx_click_async_cb, NULL);
    ctx_clear_pending_click();
    if (g_ctx_group) {
        lv_group_del(g_ctx_group);
        g_ctx_group = NULL;
    }
    g_ctx_prev_group = NULL;
    g_ctx_prev_focus = NULL;
    g_ctx_prev_editing = false;
    g_ctx_overlay = NULL;
    g_ctx_entries_ref = LUA_NOREF;
}

/* ================================================================== */
/* LV_ASSERT_HANDLER (voir libs/lv_conf.h)                             */
/* ================================================================== */

/* Traceback Lua calcule en mode protege (pas de longjmp hors du handler) */
static int assert_traceback_protected(lua_State *L)
{
    luaL_traceback(L, L, "[LVGL ASSERT] traceback Lua :", 1);
    fprintf(stderr, "%s\n", lua_tostring(L, -1));
    return 0;
}

void flam_assert_crash(const char *file, int line)
{
    static volatile int in_assert = 0;

#ifdef _MSC_VER
    /* Pas de boite modale "abort() has been called" ni de rapport WER */
    _set_abort_behavior(0, _WRITE_ABORT_MSG | _CALL_REPORTFAULT);
#endif

    if (in_assert) {
        /* Assert pendant le diagnostic (tas corrompu...) : sortir */
        fflush(stderr);
        abort();
    }
    in_assert = 1;

    fprintf(stderr, "\n[LVGL ASSERT] %s:%d\n", file ? file : "?", line);
    fflush(stderr);

    lua_State *L = g_fw_L;
    if (L && lua_checkstack(L, 4)) {
        lua_pushcfunction(L, assert_traceback_protected);
        if (lua_pcall(L, 0, 0, 0) != LUA_OK) {
            fprintf(stderr, "[LVGL ASSERT] traceback Lua indisponible\n");
            lua_pop(L, 1);
        }
    } else {
        fprintf(stderr, "[LVGL ASSERT] pas d'etat Lua actif\n");
    }
    fflush(stderr);

    lv_mem_monitor_t mon;
    memset(&mon, 0, sizeof(mon));
    lv_mem_monitor(&mon);
    fprintf(stderr,
            "[LVGL ASSERT] tas LVGL : total=%u libre=%u (plus grand bloc %u) "
            "utilise=%u%% frag=%u%% max_utilise=%u\n",
            (unsigned)mon.total_size, (unsigned)mon.free_size,
            (unsigned)mon.free_biggest_size, (unsigned)mon.used_pct,
            (unsigned)mon.frag_pct, (unsigned)mon.max_used);

    fflush(stderr);
    fflush(stdout);
    abort();
}
