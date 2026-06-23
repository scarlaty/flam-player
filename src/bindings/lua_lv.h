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

/* --- Animation wrapper (userdata Lua) --- */
typedef struct {
    lua_State *L;
    int        var_ref;       /* référence au "var" Lua (objet animé) */
    int        exec_cb_ref;   /* référence à la callback exec Lua */
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
   lv_obj_t* converti en entier. Cela garantit qu'un meme lv_obj_t*
   retourne toujours le MEME userdata Lua, ce qui permet d'utiliser
   les userdatas comme clés de table. */
#define LV_OBJ_CACHE_KEY "lv.obj_cache"

static inline void lua_lv_push_obj(lua_State *L, lv_obj_t *obj) {
    if (!obj) { lua_pushnil(L); return; }

    /* Récupérer (ou créer) la table cache */
    lua_getfield(L, LUA_REGISTRYINDEX, LV_OBJ_CACHE_KEY);
    if (lua_isnil(L, -1)) {
        lua_pop(L, 1);
        /* Créer la table cache avec weak values */
        lua_newtable(L);
        lua_newtable(L);  /* metatable */
        lua_pushstring(L, "v");
        lua_setfield(L, -2, "__mode");
        lua_setmetatable(L, -2);
        lua_pushvalue(L, -1);
        lua_setfield(L, LUA_REGISTRYINDEX, LV_OBJ_CACHE_KEY);
    }
    /* Stack: cache_table */

    /* Chercher le userdata existant */
    lua_pushlightuserdata(L, obj);
    lua_rawget(L, -2);
    if (!lua_isnil(L, -1)) {
        /* Trouvé : retourner le userdata caché */
        lua_remove(L, -2);  /* enlever cache_table */
        return;
    }
    lua_pop(L, 1);  /* pop nil */

    /* Créer un nouveau userdata */
    lv_obj_t **ud = (lv_obj_t **)lua_newuserdata(L, sizeof(lv_obj_t *));
    *ud = obj;
    luaL_setmetatable(L, LV_MT_OBJ);

    /* L'ajouter au cache */
    lua_pushlightuserdata(L, obj);
    lua_pushvalue(L, -2);  /* copier le userdata */
    lua_rawset(L, -4);     /* cache[ptr] = userdata */

    lua_remove(L, -2);  /* enlever cache_table */
}

static inline lv_obj_t *lua_lv_check_obj(lua_State *L, int idx) {
    lv_obj_t **ud = (lv_obj_t **)luaL_checkudata(L, idx, LV_MT_OBJ);
    return *ud;
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

/* Configurer le chemin de base pour lv.img_src.load() */
void lua_lv_set_img_base_path(const char *path);

/* Supprimer tous les timers Lua actifs (avant lua_close) */
void lua_lv_cleanup_timers(void);

#endif /* LUA_LV_H */
