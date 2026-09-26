/**
 * lua_lv.h — Header commun pour les bindings Lua ↔ LVGL
 */
#ifndef LUA_LV_H
#define LUA_LV_H

#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"
#include "lvgl/lvgl.h"

/* Noms des metatables */
#define LV_MT_OBJ    "LvObj"
#define LV_MT_STYLE  "LvStyle"
#define LV_MT_ANIM   "LvAnim"
#define LV_MT_GROUP  "LvGroup"
#define LV_MT_AREA   "LvArea"
#define LV_MT_IMGDSC "LvImgDsc"
#define LV_MT_TIMER  "LvTimer"

/* --- Callback event (heap-allocated, passé comme user_data à LVGL) --- */
typedef struct {
    lua_State *L;
    int        func_ref;   /* référence dans le registry Lua */
} lua_lv_cb_data_t;

/* --- Animation wrapper (userdata Lua) ---
   Le "var" et la callback exec sont des user values du userdata (et non
   des refs registry) : une closure exec qui capture l'anim ne forme plus
   un cycle racine dans le registre, le GC peut la collecter (fuite X1c). */
#define LUA_LV_ANIM_UV_VAR   1
#define LUA_LV_ANIM_UV_EXEC  2
#define LUA_LV_ANIM_NUV      2
typedef struct {
    lua_State *L;             /* thread principal */
    int        has_var;       /* user value LUA_LV_ANIM_UV_VAR renseignee */
    int        has_exec;      /* user value LUA_LV_ANIM_UV_EXEC renseignee */
    lv_anim_t  anim;          /* DOIT etre apres les autres champs (cf. lv_anim_start self-ref) */
} lua_lv_anim_ud_t;

/* --- Timer wrapper (userdata Lua) --- */
typedef struct {
    lua_State  *L;
    int         func_ref;
    lv_timer_t *timer;
} lua_lv_timer_ud_t;

/* ================================================================== */
/* Helpers push/check inline                                           */
/* ================================================================== */

/* Cle unique dans le registry pour le cache de userdatas obj.
   Le cache est une table faible (weak values) indexee par le pointeur
   lv_obj_t* (light userdata). Cela garantit qu'un meme lv_obj_t*
   retourne toujours le MEME userdata Lua, ce qui permet d'utiliser
   les userdatas comme clés de table. */
#define LV_OBJ_CACHE_KEY "lv.obj_cache"

/* Pousse le userdata LvObj de obj (nil si obj est NULL ou en cours de
   suppression). Au premier push, l'objet recoit un enregistrement de
   duree de vie (lua_lv_obj.c) : handler LV_EVENT_DELETE unique qui met
   *ud = NULL, purge le cache et libere les ancres (styles, image,
   callbacks d'evenement). */
void lua_lv_push_obj(lua_State *L, lv_obj_t *obj);

/* Objet vivant ou erreur Lua (objet supprime : *ud == NULL) */
static inline lv_obj_t *lua_lv_check_obj(lua_State *L, int idx) {
    lv_obj_t **ud = (lv_obj_t **)luaL_checkudata(L, idx, LV_MT_OBJ);
    if (!*ud) luaL_error(L, "bad argument #%d (lv object deleted)", idx);
    return *ud;
}

/* Comme lua_lv_check_obj mais renvoie NULL pour un objet supprime
   (pour les operations idempotentes : del, clean, remove_event_cb) */
static inline lv_obj_t *lua_lv_check_obj_or_null(lua_State *L, int idx) {
    lv_obj_t **ud = (lv_obj_t **)luaL_checkudata(L, idx, LV_MT_OBJ);
    return *ud;
}

/* Objet vivant de la classe cls exacte, sinon erreur Lua. Les fonctions
   propres a un widget (lv_label_*, lv_slider_*...) convertissent lv_obj_t
   en lv_label_t, etc. : sur un objet d'une autre classe, elles liraient
   et ecriraient au-dela de l'allocation (LV_ASSERT_OBJ desactive). */
static inline lv_obj_t *lua_lv_check_obj_class(lua_State *L, int idx,
                                               const lv_obj_class_t *cls,
                                               const char *name) {
    lv_obj_t *obj = lua_lv_check_obj(L, idx);
    if (!lv_obj_check_type(obj, cls))
        luaL_error(L, "bad argument #%d (lv.%s attendu)", idx, name);
    return obj;
}

static inline lv_obj_t *lua_lv_opt_obj(lua_State *L, int idx) {
    if (lua_isnoneornil(L, idx)) return lv_scr_act();
    return lua_lv_check_obj(L, idx);
}

static inline lv_style_t *lua_lv_check_style(lua_State *L, int idx) {
    return (lv_style_t *)luaL_checkudata(L, idx, LV_MT_STYLE);
}

static inline void lua_lv_push_group(lua_State *L, lv_group_t *grp) {
    if (!grp) { lua_pushnil(L); return; }
    lv_group_t **ud = (lv_group_t **)lua_newuserdata(L, sizeof(lv_group_t *));
    *ud = grp;
    luaL_setmetatable(L, LV_MT_GROUP);
}

static inline lv_group_t *lua_lv_check_group(lua_State *L, int idx) {
    lv_group_t **ud = (lv_group_t **)luaL_checkudata(L, idx, LV_MT_GROUP);
    return *ud;
}

static inline lv_color_t lua_lv_check_color(lua_State *L, int idx) {
    lv_color_t c;
    c.full = (uint32_t)luaL_checkinteger(L, idx);
    return c;
}

static inline void lua_lv_push_color(lua_State *L, lv_color_t c) {
    lua_pushinteger(L, (lua_Integer)c.full);
}

/* ================================================================== */
/* Registration (appelé depuis lua_lv.c)                               */
/* ================================================================== */

/* Chaque fonction reçoit l'index de la table `lv` sur la pile Lua.    */
void lua_lv_register_obj(lua_State *L, int lv_idx);
void lua_lv_register_style(lua_State *L, int lv_idx);
void lua_lv_register_event(lua_State *L, int lv_idx);

/* Point d'entrée : crée la table globale `lv` avec tout le contenu.   */
int luaopen_lv(lua_State *L);

/* Finalizer pour les styles (appelle lv_style_reset avant libération) */
int lua_lv_style_gc(lua_State *L);

/* Police de lv.font argument idx : nil => NULL (ignore par l'appelant),
   pointeur hors de la liste lv.font.* => erreur Lua (lua_lv.c) */
const lv_font_t *lua_lv_opt_font(lua_State *L, int idx);

/* Vrai si e est un evenement dont le callback Lua est en cours
   (event_cb_wrapper, lua_lv_obj.c) : un lv_event_t garde apres son
   callback designe une zone de pile C morte. */
int lua_lv_event_is_active(const void *e);

/* Vrai si code est un lv.EVENT_* expose (DELETE compris) : seuls codes
   livres aux callbacks Lua (lua_lv_event.c). Les autres (DRAW_*,
   COVER_CHECK, HIT_TEST...) arrivent pendant le rendu. */
int lua_lv_event_code_exposed(lv_event_code_t code);

/* Vrai si un callback Lua LV_EVENT_DEFOCUSED est en cours : LVGL garde
   alors un noeud de la liste du groupe (lv_group_focus_obj,
   focus_next_core), la liste ne doit pas changer (lua_lv_obj.c). */
int lua_lv_in_defocus(void);

/* Imbrication des callbacks Lua appeles par LVGL (evenements, timers,
   anims) : lua_lv_cb_enter renvoie 0 (et n'entre pas) au-dela de
   LUA_LV_CB_MAX_DEPTH, sinon 1 ; lua_lv_cb_leave apres le callback.
   Borne la recursion Lua -> LVGL -> Lua avant le debordement de la
   pile C (0xC00000FD). */
#define LUA_LV_CB_MAX_DEPTH 32
int  lua_lv_cb_enter(const char *what);
void lua_lv_cb_leave(void);

/* Configurer le chemin de base pour lv.img_src.load() */
void lua_lv_set_img_base_path(const char *path);

/* Supprimer tous les timers Lua actifs (avant lua_close) */
void lua_lv_cleanup_timers(void);

/* Apres lua_close : retire de LVGL tout ce qui renvoie encore vers l'etat
   ferme (timers, anims, handlers d'objets crees par un __gc pendant
   lua_close, dont les userdata ne sont jamais finalises). Appele aussi au
   debut de luaopen_lv : un seul etat Lua a la fois utilise les bindings,
   et aucun lv_timer_handler ne tourne entre lua_close et l'etat suivant.
   L'hote peut l'appeler juste apres lua_close (idempotent). */
void lua_lv_after_close(void);
/* Parties internes (lua_lv_obj.c, lua_lv_event.c) */
void lua_lv_obj_after_close(void);
void lua_lv_event_after_close(void);
void lua_lv_anim_unhook_obj(lv_obj_t *obj);

/* Mode strict (--strict de main.c, F33) : lv.img_src.load sur un fichier
   absent leve une erreur Lua au lieu de renvoyer nil. */
void lua_lv_set_strict(int strict);
int  lua_lv_get_strict(void);

/* ================================================================== */
/* Garde du tas LVGL (X1a)                                             */
/* ================================================================== */
/* LVGL 8.3 ne teste pas tous ses retours d'allocation (lv_obj_class.c :
   tableau children/screens ; LV_ASSERT_MALLOC => flam_assert_crash).
   Les bindings verifient donc AVANT d'appeler LVGL qu'il reste assez de
   tas, sinon luaL_error("tas LVGL epuise ..."). La marge couvre les
   petites allocations internes (styles du theme, texte par defaut,
   spec_attr, handlers) et le rendu (LV_LAYER_SIMPLE_BUF_SIZE). */
#define LUA_LV_MEM_MARGIN  (32U * 1024U)

/* Erreur Lua si le tas LVGL n'offre pas un bloc contigu de contig octets
   ET contig + extra octets libres au total (marge incluse). */
void lua_lv_mem_check(lua_State *L, size_t contig, size_t extra);

/* Garde d'un constructeur : instance de cls + agrandissement du tableau
   children de parent (ou du tableau screens du display si parent NULL). */
void lua_lv_mem_check_create(lua_State *L, lv_obj_t *parent,
                             const lv_obj_class_t *cls);

/* Comme luaL_setfuncs(L, funcs, 0), mais chaque fonction est precedee de
   lua_lv_mem_check(L, 0, 0), sauf celles dont le nom est dans skip
   (tableau termine par NULL) ou commence par "get_". */
void lua_lv_setfuncs_guarded(lua_State *L, const luaL_Reg *funcs,
                             const char *const *skip);

#endif /* LUA_LV_H */
