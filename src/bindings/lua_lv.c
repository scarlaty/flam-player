/**
 * lua_lv.c — Enregistrement principal des bindings Lua ↔ LVGL
 *
 * Crée la table globale `lv` avec :
 *   - Sous-tables : obj, btn, label, img, slider, img_src, style,
 *     anim, anim_var, timer, group, event, color, area, font,
 *     mem (emulateur uniquement)
 *   - Constantes : EVENT_*, KEY_*, ALIGN_*, FLEX_*, STATE_*, OPA_*, etc.
 */

#include "lua_lv.h"
#include <stdlib.h>
#include <string.h>

/* ================================================================== */
/* lv.color                                                            */
/* ================================================================== */

static int l_color_hex(lua_State *L) {
    uint32_t hex = (uint32_t)luaL_checkinteger(L, 1);
    lua_lv_push_color(L, lv_color_hex(hex));
    return 1;
}

static int l_color_black(lua_State *L) {
    lua_lv_push_color(L, lv_color_black());
    return 1;
}

static int l_color_white(lua_State *L) {
    lua_lv_push_color(L, lv_color_white());
    return 1;
}

static int l_color_make(lua_State *L) {
    uint8_t r = (uint8_t)luaL_checkinteger(L, 1);
    uint8_t g = (uint8_t)luaL_checkinteger(L, 2);
    uint8_t b = (uint8_t)luaL_checkinteger(L, 3);
    lua_lv_push_color(L, lv_color_make(r, g, b));
    return 1;
}

static const luaL_Reg color_funcs[] = {
    {"hex",   l_color_hex},
    {"make",  l_color_make},
    {"black", l_color_black},
    {"white", l_color_white},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.area                                                             */
/* ================================================================== */

static int l_area_new(lua_State *L) {
    lv_area_t *a = (lv_area_t *)lua_newuserdata(L, sizeof(lv_area_t));
    memset(a, 0, sizeof(lv_area_t));
    luaL_setmetatable(L, LV_MT_AREA);
    return 1;
}

static lv_area_t *check_area(lua_State *L, int idx) {
    return (lv_area_t *)luaL_checkudata(L, idx, LV_MT_AREA);
}

static int l_area_get_x1(lua_State *L) {
    lua_pushinteger(L, check_area(L, 1)->x1);
    return 1;
}
static int l_area_get_x2(lua_State *L) {
    lua_pushinteger(L, check_area(L, 1)->x2);
    return 1;
}
static int l_area_get_y1(lua_State *L) {
    lua_pushinteger(L, check_area(L, 1)->y1);
    return 1;
}
static int l_area_get_y2(lua_State *L) {
    lua_pushinteger(L, check_area(L, 1)->y2);
    return 1;
}

static const luaL_Reg area_funcs[] = {
    {"new",    l_area_new},
    {"get_x1", l_area_get_x1},
    {"get_x2", l_area_get_x2},
    {"get_y1", l_area_get_y1},
    {"get_y2", l_area_get_y2},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.mem — API EMULATEUR UNIQUEMENT                                   */
/* ================================================================== */
/* ATTENTION : lv.mem n'existe PAS dans le firmware officiel FLAM. Reserve
   aux tests de l'emulateur (tests/lua/fuzz_lvgl.lua) ; aucun script
   d'histoire ni code genere par telmi2flam ne doit l'utiliser.
   lv.mem.monitor() -> { total_size, free_size, free_biggest_size,
   used_pct, frag_pct, max_used } (lecture seule, lv_mem_monitor). */
static int l_mem_monitor(lua_State *L) {
    lv_mem_monitor_t mon;
    lv_memset_00(&mon, sizeof(mon));
    lv_mem_monitor(&mon);
    lua_createtable(L, 0, 6);
    lua_pushinteger(L, mon.total_size);        lua_setfield(L, -2, "total_size");
    lua_pushinteger(L, mon.free_size);         lua_setfield(L, -2, "free_size");
    lua_pushinteger(L, mon.free_biggest_size); lua_setfield(L, -2, "free_biggest_size");
    lua_pushinteger(L, mon.used_pct);          lua_setfield(L, -2, "used_pct");
    lua_pushinteger(L, mon.frag_pct);          lua_setfield(L, -2, "frag_pct");
    lua_pushinteger(L, mon.max_used);          lua_setfield(L, -2, "max_used");
    return 1;
}

static const luaL_Reg mem_funcs[] = {
    {"monitor", l_mem_monitor},
    {NULL, NULL}
};

/* ================================================================== */
/* Garde du tas LVGL (X1a, voir lua_lv.h)                              */
/* ================================================================== */

void lua_lv_mem_check(lua_State *L, size_t contig, size_t extra) {
#if LV_MEM_CUSTOM == 0
    lv_mem_monitor_t mon;
    lv_mem_monitor(&mon);
    /* Surcouts TLSF (en-tetes, alignement) couverts par la marge */
    size_t need_contig = contig + LUA_LV_MEM_MARGIN;
    size_t need_total  = contig + extra + LUA_LV_MEM_MARGIN;
    if ((size_t)mon.free_biggest_size < need_contig || (size_t)mon.free_size < need_total)
        luaL_error(L, "tas LVGL epuise (libre=%d, plus grand bloc=%d, requis=%d/%d)",
                   (int)mon.free_size, (int)mon.free_biggest_size,
                   (int)need_contig, (int)need_total);
#else
    (void)L; (void)contig; (void)extra;
#endif
}

static uint32_t class_instance_size(const lv_obj_class_t *cls) {
    while (cls && cls->instance_size == 0) cls = cls->base_class;
    return cls ? cls->instance_size : (uint32_t)sizeof(lv_obj_t);
}

void lua_lv_mem_check_create(lua_State *L, lv_obj_t *parent,
                             const lv_obj_class_t *cls) {
    /* lv_obj_class_create_obj : lv_mem_alloc(instance), puis
       lv_mem_realloc(children|screens, (n + 1) * sizeof(ptr)) sans test
       NULL (lv_obj_class.c) : il faut un bloc contigu de cette taille. */
    size_t cnt = 0;
    size_t extra = class_instance_size(cls);
    if (parent) {
        if (parent->spec_attr) cnt = parent->spec_attr->child_cnt;
        else extra += sizeof(_lv_obj_spec_attr_t);
    } else {
        lv_disp_t *disp = lv_disp_get_default();
        if (disp) cnt = disp->screen_cnt;
    }
    lua_lv_mem_check(L, (cnt + 1) * sizeof(lv_obj_t *), extra);
}

/* Upvalue 1 : la fonction gardee (sans upvalue propre) */
static int l_guarded(lua_State *L) {
    lua_CFunction f = lua_tocfunction(L, lua_upvalueindex(1));
    lua_lv_mem_check(L, 0, 0);
    return f(L);
}

void lua_lv_setfuncs_guarded(lua_State *L, const luaL_Reg *funcs,
                             const char *const *skip) {
    for (; funcs->name; funcs++) {
        int guarded = strncmp(funcs->name, "get_", 4) != 0;
        for (const char *const *s = skip; guarded && s && *s; s++)
            if (strcmp(funcs->name, *s) == 0) guarded = 0;
        if (guarded) {
            lua_pushcfunction(L, funcs->func);
            lua_pushcclosure(L, l_guarded, 1);
        } else {
            lua_pushcfunction(L, funcs->func);
        }
        lua_setfield(L, -2, funcs->name);
    }
}

/* ================================================================== */
/* Constantes                                                          */
/* ================================================================== */

static void register_constants(lua_State *L, int lv_idx) {
#define SET_INT(name, val) lua_pushinteger(L, val); lua_setfield(L, lv_idx, name)

    /* Événements */
    SET_INT("EVENT_CLICKED",     LV_EVENT_CLICKED);
    SET_INT("EVENT_PRESSED",     LV_EVENT_PRESSED);
    SET_INT("EVENT_RELEASED",    LV_EVENT_RELEASED);
    SET_INT("EVENT_FOCUSED",     LV_EVENT_FOCUSED);
    SET_INT("EVENT_DEFOCUSED",   LV_EVENT_DEFOCUSED);
    SET_INT("EVENT_KEY",         LV_EVENT_KEY);
    SET_INT("EVENT_SCROLL_END",  LV_EVENT_SCROLL_END);
    SET_INT("EVENT_VALUE_CHANGED", LV_EVENT_VALUE_CHANGED);
    SET_INT("EVENT_DELETE",       LV_EVENT_DELETE);
    SET_INT("EVENT_READY",        LV_EVENT_READY);
    SET_INT("EVENT_CANCEL",       LV_EVENT_CANCEL);
    SET_INT("EVENT_SCROLL_BEGIN", LV_EVENT_SCROLL_BEGIN);

    /* Touches */
    SET_INT("KEY_LEFT",  LV_KEY_LEFT);
    SET_INT("KEY_RIGHT", LV_KEY_RIGHT);
    SET_INT("KEY_ENTER", LV_KEY_ENTER);
    SET_INT("KEY_ESC",   LV_KEY_ESC);
    SET_INT("KEY_UP",    LV_KEY_UP);
    SET_INT("KEY_DOWN",  LV_KEY_DOWN);
    SET_INT("KEY_NEXT",  LV_KEY_NEXT);
    SET_INT("KEY_PREV",  LV_KEY_PREV);

    /* Alignements */
    SET_INT("ALIGN_DEFAULT",     LV_ALIGN_DEFAULT);
    SET_INT("ALIGN_TOP_LEFT",    LV_ALIGN_TOP_LEFT);
    SET_INT("ALIGN_TOP_MID",     LV_ALIGN_TOP_MID);
    SET_INT("ALIGN_TOP_RIGHT",   LV_ALIGN_TOP_RIGHT);
    SET_INT("ALIGN_BOTTOM_LEFT", LV_ALIGN_BOTTOM_LEFT);
    SET_INT("ALIGN_BOTTOM_MID",  LV_ALIGN_BOTTOM_MID);
    SET_INT("ALIGN_BOTTOM_RIGHT",LV_ALIGN_BOTTOM_RIGHT);
    SET_INT("ALIGN_LEFT_MID",    LV_ALIGN_LEFT_MID);
    SET_INT("ALIGN_RIGHT_MID",   LV_ALIGN_RIGHT_MID);
    SET_INT("ALIGN_CENTER",      LV_ALIGN_CENTER);

    /* Flex */
    SET_INT("FLEX_FLOW_ROW",        LV_FLEX_FLOW_ROW);
    SET_INT("FLEX_FLOW_COLUMN",     LV_FLEX_FLOW_COLUMN);
    SET_INT("FLEX_FLOW_ROW_WRAP",   LV_FLEX_FLOW_ROW_WRAP);
    SET_INT("FLEX_FLOW_COLUMN_WRAP",LV_FLEX_FLOW_COLUMN_WRAP);
    SET_INT("FLEX_ALIGN_START",        LV_FLEX_ALIGN_START);
    SET_INT("FLEX_ALIGN_END",          LV_FLEX_ALIGN_END);
    SET_INT("FLEX_ALIGN_CENTER",       LV_FLEX_ALIGN_CENTER);
    SET_INT("FLEX_ALIGN_SPACE_EVENLY", LV_FLEX_ALIGN_SPACE_EVENLY);
    SET_INT("FLEX_ALIGN_SPACE_AROUND", LV_FLEX_ALIGN_SPACE_AROUND);
    SET_INT("FLEX_ALIGN_SPACE_BETWEEN",LV_FLEX_ALIGN_SPACE_BETWEEN);

    /* États */
    SET_INT("STATE_DEFAULT",  LV_STATE_DEFAULT);
    SET_INT("STATE_FOCUSED",  LV_STATE_FOCUSED);
    SET_INT("STATE_PRESSED",  LV_STATE_PRESSED);
    SET_INT("STATE_CHECKED",  LV_STATE_CHECKED);
    SET_INT("STATE_DISABLED", LV_STATE_DISABLED);
    SET_INT("STATE_FOCUS_KEY", LV_STATE_FOCUS_KEY);
    SET_INT("STATE_USER_1",    LV_STATE_USER_1);
    SET_INT("STATE_USER_2",    LV_STATE_USER_2);
    SET_INT("STATE_USER_3",    LV_STATE_USER_3);
    SET_INT("STATE_USER_4",    LV_STATE_USER_4);

    /* Opacité */
    SET_INT("OPA_TRANSP", LV_OPA_TRANSP);
    SET_INT("OPA_COVER",  LV_OPA_COVER);
    SET_INT("OPA_10",  LV_OPA_10);
    SET_INT("OPA_20",  LV_OPA_20);
    SET_INT("OPA_30",  LV_OPA_30);
    SET_INT("OPA_40",  LV_OPA_40);
    SET_INT("OPA_50",  LV_OPA_50);
    SET_INT("OPA_60",  LV_OPA_60);
    SET_INT("OPA_70",  LV_OPA_70);
    SET_INT("OPA_80",  LV_OPA_80);
    SET_INT("OPA_90",  LV_OPA_90);

    /* Parts */
    SET_INT("PART_MAIN",      LV_PART_MAIN);
    SET_INT("PART_SCROLLBAR", LV_PART_SCROLLBAR);
    SET_INT("PART_INDICATOR", LV_PART_INDICATOR);
    SET_INT("PART_KNOB",      LV_PART_KNOB);

    /* Flags */
    SET_INT("OBJ_FLAG_HIDDEN",       LV_OBJ_FLAG_HIDDEN);
    SET_INT("OBJ_FLAG_CLICKABLE",    LV_OBJ_FLAG_CLICKABLE);
    SET_INT("OBJ_FLAG_SCROLLABLE",      LV_OBJ_FLAG_SCROLLABLE);
    SET_INT("OBJ_FLAG_CLICK_FOCUSABLE", LV_OBJ_FLAG_CLICK_FOCUSABLE);
    SET_INT("OBJ_FLAG_FLOATING",        LV_OBJ_FLAG_FLOATING);
    SET_INT("OBJ_FLAG_SCROLL_ONE",      LV_OBJ_FLAG_SCROLL_ONE);
    SET_INT("OBJ_FLAG_SNAPPABLE",       LV_OBJ_FLAG_SNAPPABLE);

    /* Animations */
    SET_INT("ANIM_ON",              LV_ANIM_ON);
    SET_INT("ANIM_OFF",             LV_ANIM_OFF);
    SET_INT("ANIM_REPEAT_INFINITE", LV_ANIM_REPEAT_INFINITE);

    /* Gradient */
    SET_INT("GRAD_DIR_NONE", LV_GRAD_DIR_NONE);
    SET_INT("GRAD_DIR_VER",  LV_GRAD_DIR_VER);
    SET_INT("GRAD_DIR_HOR",  LV_GRAD_DIR_HOR);

    /* Dither */
    SET_INT("DITHER_NONE",    LV_DITHER_NONE);
    SET_INT("DITHER_ORDERED", LV_DITHER_ORDERED);
    SET_INT("DITHER_ERR_DIFF", LV_DITHER_ERR_DIFF);

    /* Border side */
    SET_INT("BORDER_SIDE_NONE",   LV_BORDER_SIDE_NONE);
    SET_INT("BORDER_SIDE_BOTTOM", LV_BORDER_SIDE_BOTTOM);
    SET_INT("BORDER_SIDE_TOP",    LV_BORDER_SIDE_TOP);
    SET_INT("BORDER_SIDE_LEFT",   LV_BORDER_SIDE_LEFT);
    SET_INT("BORDER_SIDE_RIGHT",  LV_BORDER_SIDE_RIGHT);
    SET_INT("BORDER_SIDE_FULL",   LV_BORDER_SIDE_FULL);

    /* Scroll snap */
    SET_INT("SCROLL_SNAP_NONE",   LV_SCROLL_SNAP_NONE);
    SET_INT("SCROLL_SNAP_START",  LV_SCROLL_SNAP_START);
    SET_INT("SCROLL_SNAP_END",    LV_SCROLL_SNAP_END);
    SET_INT("SCROLL_SNAP_CENTER", LV_SCROLL_SNAP_CENTER);

    /* Radius */
    SET_INT("RADIUS_CIRCLE", LV_RADIUS_CIRCLE);

    /* Label */
    SET_INT("LABEL_LONG_WRAP",             LV_LABEL_LONG_WRAP);
    SET_INT("LABEL_LONG_DOT",              LV_LABEL_LONG_DOT);
    SET_INT("LABEL_LONG_SCROLL",           LV_LABEL_LONG_SCROLL);
    SET_INT("LABEL_LONG_SCROLL_CIRCULAR",  LV_LABEL_LONG_SCROLL_CIRCULAR);
    SET_INT("LABEL_LONG_CLIP",             LV_LABEL_LONG_CLIP);

    /* Text align */
    SET_INT("TEXT_ALIGN_LEFT",   LV_TEXT_ALIGN_LEFT);
    SET_INT("TEXT_ALIGN_CENTER", LV_TEXT_ALIGN_CENTER);
    SET_INT("TEXT_ALIGN_RIGHT",  LV_TEXT_ALIGN_RIGHT);
    SET_INT("TEXT_ALIGN_AUTO",   LV_TEXT_ALIGN_AUTO);

    /* Size special */
    SET_INT("SIZE_CONTENT", LV_SIZE_CONTENT);
    SET_INT("PCT_100",      LV_PCT(100));

#undef SET_INT
}

/* ================================================================== */
/* lv.font — Polices Nunito (Bold / ExtraBold, tailles 12-20)          */
/* ================================================================== */

extern lv_font_t nunito_bold_12;
extern lv_font_t nunito_bold_16;
extern lv_font_t nunito_bold_20;
extern lv_font_t nunito_extrabold_12;
extern lv_font_t nunito_extrabold_14;
extern lv_font_t nunito_extrabold_16;
extern lv_font_t nunito_extrabold_20;

static const struct { const char *name; const lv_font_t *font; } g_fonts[] = {
    {"nunito_bold_12",      &nunito_bold_12},
    {"nunito_bold_16",      &nunito_bold_16},
    {"nunito_bold_20",      &nunito_bold_20},
    {"nunito_extrabold_12", &nunito_extrabold_12},
    {"nunito_extrabold_14", &nunito_extrabold_14},
    {"nunito_extrabold_16", &nunito_extrabold_16},
    {"nunito_extrabold_20", &nunito_extrabold_20},
    {NULL, NULL}
};

/* Seuls les pointeurs de lv.font.* sont des polices : un autre light
   userdata (path d'anim, cbd, evenement) serait lu comme lv_font_t et
   LVGL appellerait ses "pointeurs de fonction" */
const lv_font_t *lua_lv_opt_font(lua_State *L, int idx) {
    if (lua_isnoneornil(L, idx)) return NULL;
    if (lua_islightuserdata(L, idx)) {
        const void *p = lua_touserdata(L, idx);
        for (int i = 0; g_fonts[i].name; i++)
            if (p == (const void *)g_fonts[i].font) return g_fonts[i].font;
    }
    luaL_argerror(L, idx, "lv.font attendu");
    return NULL;
}

static void register_fonts(lua_State *L, int lv_idx) {
    lua_newtable(L);

    for (int i = 0; g_fonts[i].name; i++) {
        lua_pushlightuserdata(L, (void *)g_fonts[i].font);
        lua_setfield(L, -2, g_fonts[i].name);
    }

    lua_setfield(L, lv_idx, "font");
}

/* ================================================================== */
/* Création des metatables                                             */
/* ================================================================== */

/* __eq pour les userdatas qui wrappent un pointeur :
   deux userdatas sont egales si elles pointent vers le meme objet C
   (jamais pour un pointeur NULL : deux objets supprimes different). */
static int l_ptr_eq(lua_State *L) {
    void **a = (void **)lua_touserdata(L, 1);
    void **b = (void **)lua_touserdata(L, 2);
    lua_pushboolean(L, a && b && *a && *a == *b);
    return 1;
}

static void create_metatables(lua_State *L) {
    const char *names[] = {
        LV_MT_OBJ, LV_MT_ANIM,
        LV_MT_GROUP, LV_MT_AREA, LV_MT_TIMER,
        NULL
    };
    /* Note: LV_MT_IMGDSC is NOT pre-created here — l_img_src_load()
       creates it with __gc for proper cleanup of decoded LIF images. */
    for (int i = 0; names[i]; i++) {
        luaL_newmetatable(L, names[i]);
        lua_pushcfunction(L, l_ptr_eq);
        lua_setfield(L, -2, "__eq");
        lua_pop(L, 1);
    }
    /* LV_MT_STYLE : __eq + __gc pour liberer les valeurs de style
       allouees dans le pool LVGL (lv_mem, 256KB). Sans ce __gc, chaque
       module qui cree des styles fuiterait de la memoire LVGL. */
    luaL_newmetatable(L, LV_MT_STYLE);
    lua_pushcfunction(L, l_ptr_eq);
    lua_setfield(L, -2, "__eq");
    lua_pushcfunction(L, lua_lv_style_gc);
    lua_setfield(L, -2, "__gc");
    lua_pop(L, 1);
}

/* ================================================================== */
/* Helper : enregistrer une sous-table de fonctions                    */
/* ================================================================== */

static void set_subtable(lua_State *L, int lv_idx, const char *name,
                         const luaL_Reg *funcs) {
    lua_newtable(L);
    luaL_setfuncs(L, funcs, 0);
    lua_setfield(L, lv_idx, name);
}

/* ================================================================== */
/* Point d'entrée                                                      */
/* ================================================================== */

static int l_pct(lua_State *L) {
    int32_t v = (int32_t)luaL_checkinteger(L, 1);
    lua_pushinteger(L, (lua_Integer)LV_PCT(v));
    return 1;
}

void lua_lv_after_close(void) {
    lua_lv_event_after_close();   /* timers et anims */
    lua_lv_obj_after_close();     /* handlers d'objets */
}

int luaopen_lv(lua_State *L) {
    /* Restes d'un etat precedent (voir lua_lv.h) */
    lua_lv_after_close();

    create_metatables(L);

    lua_newtable(L);  /* la table `lv` */
    int lv_idx = lua_gettop(L);

    /* Fonction lv.pct(x) */
    lua_pushcfunction(L, l_pct);
    lua_setfield(L, lv_idx, "pct");

    /* Sous-modules simples */
    set_subtable(L, lv_idx, "color", color_funcs);
    set_subtable(L, lv_idx, "area",  area_funcs);

    /* Polices */
    register_fonts(L, lv_idx);

    /* Constantes */
    register_constants(L, lv_idx);

    /* Sous-modules complexes (implémentés dans d'autres fichiers) */
    lua_lv_register_obj(L, lv_idx);     /* obj, btn, label, img, slider, img_src */
    lua_lv_register_style(L, lv_idx);   /* style */
    lua_lv_register_event(L, lv_idx);   /* event, group, anim, anim_var, timer */

    /* lv.mem : API emulateur uniquement (absent du firmware officiel) */
    set_subtable(L, lv_idx, "mem", mem_funcs);

    /* Publier comme globale */
    lua_setglobal(L, "lv");

    return 0;
}
