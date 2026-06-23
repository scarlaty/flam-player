/**
 * lua_lv_event.c — Bindings pour lv.event, lv.group, lv.anim,
 *                  lv.anim_var, lv.timer
 */

#include "lua_lv.h"
#include <stdlib.h>

/* ================================================================== */
/* lv.event                                                            */
/* ================================================================== */

static int l_event_send(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_event_code_t code = (lv_event_code_t)luaL_checkinteger(L, 2);
    void *param = NULL;
    uint32_t key_val;
    if (!lua_isnoneornil(L, 3)) {
        key_val = (uint32_t)lua_tointeger(L, 3);
        param = &key_val;
    }
    lv_event_send(obj, code, param);
    return 0;
}

static int l_event_get_code(lua_State *L) {
    lv_event_t *e = (lv_event_t *)lua_touserdata(L, 1);
    if (!e) return luaL_error(L, "event is nil");
    lua_pushinteger(L, lv_event_get_code(e));
    return 1;
}

static int l_event_get_target(lua_State *L) {
    lv_event_t *e = (lv_event_t *)lua_touserdata(L, 1);
    if (!e) return luaL_error(L, "event is nil");
    lua_lv_push_obj(L, lv_event_get_target(e));
    return 1;
}

static int l_event_get_key_value(lua_State *L) {
    lv_event_t *e = (lv_event_t *)lua_touserdata(L, 1);
    if (!e) { char z[2] = {0,0}; lua_pushstring(L, z); return 1; }

    /* Lua scripts do: string.byte(lv.event.get_key_value(event))
       so we must return a 1-character string, not an integer. */
    void *param = lv_event_get_param(e);
    if (param) {
        char buf[2] = { (char)(*(uint32_t *)param), '\0' };
        lua_pushstring(L, buf);
    } else {
        char z[2] = {0,0};
        lua_pushstring(L, z);
    }
    return 1;
}

static const luaL_Reg event_funcs[] = {
    {"send",          l_event_send},
    {"get_code",      l_event_get_code},
    {"get_target",    l_event_get_target},
    {"get_key_value", l_event_get_key_value},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.group                                                            */
/* ================================================================== */

static int l_group_add_obj(lua_State *L) {
    lv_group_t *grp = lua_lv_check_group(L, 1);
    lv_obj_t *obj = lua_lv_check_obj(L, 2);
    lv_group_add_obj(grp, obj);
    return 0;
}

static int l_group_remove_obj(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_group_remove_obj(obj);
    return 0;
}

static int l_group_focus_obj(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_group_focus_obj(obj);
    return 0;
}

static int l_group_set_editing(lua_State *L) {
    lv_group_t *grp = lua_lv_check_group(L, 1);
    bool en = lua_toboolean(L, 2);
    lv_group_set_editing(grp, en);
    return 0;
}

static int l_group_set_wrap(lua_State *L) {
    lv_group_t *grp = lua_lv_check_group(L, 1);
    bool en = lua_toboolean(L, 2);
    lv_group_set_wrap(grp, en);
    return 0;
}

static int l_group_remove_all_objs(lua_State *L) {
    lv_group_t *grp = lua_lv_check_group(L, 1);
    lv_group_remove_all_objs(grp);
    return 0;
}

static const luaL_Reg group_funcs[] = {
    {"add_obj",          l_group_add_obj},
    {"remove_obj",       l_group_remove_obj},
    {"remove_all_objs",  l_group_remove_all_objs},
    {"focus_obj",        l_group_focus_obj},
    {"set_editing",      l_group_set_editing},
    {"set_wrap",         l_group_set_wrap},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.timer                                                            */
/* ================================================================== */

static void timer_cb_wrapper(lv_timer_t *timer);

void lua_lv_cleanup_timers(void) {
    lv_timer_t *t = lv_timer_get_next(NULL);
    while (t) {
        lv_timer_t *next = lv_timer_get_next(t);
        if (t->timer_cb == timer_cb_wrapper) {
            lua_lv_timer_ud_t *tud = (lua_lv_timer_ud_t *)t->user_data;
            if (tud) free(tud);
            lv_timer_del(t);
        }
        t = next;
    }
}

static void timer_cb_wrapper(lv_timer_t *timer) {
    lua_lv_timer_ud_t *tud = (lua_lv_timer_ud_t *)timer->user_data;
    if (!tud) return;
    lua_State *L = tud->L;

    lua_rawgeti(L, LUA_REGISTRYINDEX, tud->func_ref);
    if (lua_pcall(L, 0, 0, 0) != LUA_OK) {
        const char *err = lua_tostring(L, -1);
        fprintf(stderr, "[TIMER] callback error: %s\n", err ? err : "?");
        lua_pop(L, 1);
    }
}

static int l_timer_new(lua_State *L) {
    luaL_checktype(L, 1, LUA_TFUNCTION);
    uint32_t period = (uint32_t)luaL_checkinteger(L, 2);

    /* Allouer les données du callback */
    lua_lv_timer_ud_t *tud = (lua_lv_timer_ud_t *)malloc(sizeof(lua_lv_timer_ud_t));
    tud->L = L;
    lua_pushvalue(L, 1);
    tud->func_ref = luaL_ref(L, LUA_REGISTRYINDEX);

    lv_timer_t *timer = lv_timer_create(timer_cb_wrapper, period, tud);
    tud->timer = timer;

    /* Si repeat_count spécifié */
    if (!lua_isnoneornil(L, 3)) {
        int32_t repeat = (int32_t)lua_tointeger(L, 3);
        lv_timer_set_repeat_count(timer, repeat);
    }

    /* Retourner le timer comme light userdata pour pouvoir le supprimer */
    lua_pushlightuserdata(L, timer);
    return 1;
}

static int l_timer_del(lua_State *L) {
    lv_timer_t *timer = (lv_timer_t *)lua_touserdata(L, 1);
    if (timer) {
        lua_lv_timer_ud_t *tud = (lua_lv_timer_ud_t *)timer->user_data;
        if (tud) {
            luaL_unref(L, LUA_REGISTRYINDEX, tud->func_ref);
            free(tud);
        }
        lv_timer_del(timer);
    }
    return 0;
}

static int l_timer_reset(lua_State *L) {
    lv_timer_t *timer = (lv_timer_t *)lua_touserdata(L, 1);
    if (timer) lv_timer_reset(timer);
    return 0;
}

static int l_timer_set_repeat_count(lua_State *L) {
    lv_timer_t *timer = (lv_timer_t *)lua_touserdata(L, 1);
    if (timer) lv_timer_set_repeat_count(timer, (int32_t)luaL_checkinteger(L, 2));
    return 0;
}

static const luaL_Reg timer_funcs[] = {
    {"new",              l_timer_new},
    {"del",              l_timer_del},
    {"reset",            l_timer_reset},
    {"set_repeat_count", l_timer_set_repeat_count},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.anim                                                             */
/* ================================================================== */

static void anim_exec_cb_wrapper(void *var, int32_t value) {
    lua_lv_anim_ud_t *aud = (lua_lv_anim_ud_t *)var;
    if (!aud || aud->exec_cb_ref == LUA_NOREF) return;
    lua_State *L = aud->L;

    lua_rawgeti(L, LUA_REGISTRYINDEX, aud->exec_cb_ref);

    /* Pousser le "var" (l'objet animé) */
    if (aud->var_ref != LUA_NOREF) {
        lua_rawgeti(L, LUA_REGISTRYINDEX, aud->var_ref);
    } else {
        lua_pushnil(L);
    }

    lua_pushinteger(L, value);

    if (lua_pcall(L, 2, 0, 0) != LUA_OK) {
        const char *err = lua_tostring(L, -1);
        LV_LOG_ERROR("Lua anim exec callback error: %s", err ? err : "?");
        lua_pop(L, 1);
    }
}

static int l_anim_new(lua_State *L) {
    lua_lv_anim_ud_t *aud = (lua_lv_anim_ud_t *)lua_newuserdata(L, sizeof(lua_lv_anim_ud_t));
    lv_anim_init(&aud->anim);
    aud->L = L;
    aud->var_ref = LUA_NOREF;
    aud->exec_cb_ref = LUA_NOREF;

    /* La variable d'animation pointe vers le userdata lui-même
       (utilisé par le wrapper exec_cb pour retrouver les refs Lua) */
    lv_anim_set_var(&aud->anim, aud);

    luaL_setmetatable(L, LV_MT_ANIM);
    return 1;
}

static lua_lv_anim_ud_t *check_anim(lua_State *L, int idx) {
    return (lua_lv_anim_ud_t *)luaL_checkudata(L, idx, LV_MT_ANIM);
}

static int l_anim_set_var(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    /* Stocker la référence Lua au "var" */
    if (aud->var_ref != LUA_NOREF) luaL_unref(L, LUA_REGISTRYINDEX, aud->var_ref);
    lua_pushvalue(L, 2);
    aud->var_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    /* Retourner le userdata anim (permet animations[anim].var = lv.anim.set_var(anim, x)) */
    lua_pushvalue(L, 1);
    return 1;
}

static int l_anim_set_values(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    int32_t start = (int32_t)luaL_checkinteger(L, 2);
    int32_t end   = (int32_t)luaL_checkinteger(L, 3);
    lv_anim_set_values(&aud->anim, start, end);
    return 0;
}

static int l_anim_set_time(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    lv_anim_set_time(&aud->anim, (uint32_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_anim_set_duration(lua_State *L) {
    /* Alias pour set_time */
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    lv_anim_set_time(&aud->anim, (uint32_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_anim_set_exec_cb(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    luaL_checktype(L, 2, LUA_TFUNCTION);

    if (aud->exec_cb_ref != LUA_NOREF) luaL_unref(L, LUA_REGISTRYINDEX, aud->exec_cb_ref);
    lua_pushvalue(L, 2);
    aud->exec_cb_ref = luaL_ref(L, LUA_REGISTRYINDEX);

    lv_anim_set_exec_cb(&aud->anim, anim_exec_cb_wrapper);
    return 0;
}

static int l_anim_set_path_cb(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    if (lua_islightuserdata(L, 2)) {
        lv_anim_path_cb_t path_cb = (lv_anim_path_cb_t)lua_touserdata(L, 2);
        lv_anim_set_path_cb(&aud->anim, path_cb);
    }
    return 0;
}

static int l_anim_set_delay(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    lv_anim_set_delay(&aud->anim, (uint32_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_anim_set_early_apply(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    lv_anim_set_early_apply(&aud->anim, lua_toboolean(L, 2));
    return 0;
}

static int l_anim_set_playback_time(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    lv_anim_set_playback_time(&aud->anim, (uint32_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_anim_set_repeat_count(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    lv_anim_set_repeat_count(&aud->anim, (uint16_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_anim_start(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);

    /* Garder le userdata anim vivant dans le registry pour la durée de l'animation */
    lua_pushvalue(L, 1);
    int anim_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    (void)anim_ref;  /* TODO: libérer quand l'anim est finie */

    lv_anim_start(&aud->anim);
    return 0;
}

static const luaL_Reg anim_funcs[] = {
    {"new",          l_anim_new},
    {"set_var",      l_anim_set_var},
    {"set_values",   l_anim_set_values},
    {"set_time",     l_anim_set_time},
    {"set_duration", l_anim_set_duration},
    {"set_exec_cb",  l_anim_set_exec_cb},
    {"set_path_cb",  l_anim_set_path_cb},
    {"set_delay",          l_anim_set_delay},
    {"set_early_apply",    l_anim_set_early_apply},
    {"set_playback_time",  l_anim_set_playback_time},
    {"set_repeat_count",   l_anim_set_repeat_count},
    {"start",              l_anim_start},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.anim_var                                                         */
/* ================================================================== */

static int l_anim_var_del(lua_State *L) {
    /* En LVGL 8.x : lv_anim_del(var, NULL) pour supprimer par variable */
    if (lua_isuserdata(L, 1)) {
        lua_lv_anim_ud_t *aud = (lua_lv_anim_ud_t *)luaL_testudata(L, 1, LV_MT_ANIM);
        if (aud) {
            lv_anim_del(aud, NULL);
        }
    }
    return 0;
}

static const luaL_Reg anim_var_funcs[] = {
    {"del", l_anim_var_del},
    {NULL, NULL}
};

/* ================================================================== */
/* Registration                                                        */
/* ================================================================== */

static void set_subtable(lua_State *L, int lv_idx, const char *name,
                         const luaL_Reg *funcs) {
    lua_newtable(L);
    luaL_setfuncs(L, funcs, 0);
    lua_setfield(L, lv_idx, name);
}

void lua_lv_register_event(lua_State *L, int lv_idx) {
    set_subtable(L, lv_idx, "event",    event_funcs);
    set_subtable(L, lv_idx, "group",    group_funcs);
    set_subtable(L, lv_idx, "timer",    timer_funcs);

    /* anim — avec les path functions en plus des fonctions */
    lua_newtable(L);
    luaL_setfuncs(L, anim_funcs, 0);
    /* Ajouter les path functions comme light userdata */
    lua_pushlightuserdata(L, (void *)lv_anim_path_ease_in_out);
    lua_setfield(L, -2, "path_ease_in_out");
    lua_pushlightuserdata(L, (void *)lv_anim_path_linear);
    lua_setfield(L, -2, "path_linear");
    lua_pushlightuserdata(L, (void *)lv_anim_path_ease_out);
    lua_setfield(L, -2, "path_ease_out");
    lua_pushlightuserdata(L, (void *)lv_anim_path_ease_in);
    lua_setfield(L, -2, "path_ease_in");
    lua_pushlightuserdata(L, (void *)lv_anim_path_overshoot);
    lua_setfield(L, -2, "path_overshoot");
    lua_pushlightuserdata(L, (void *)lv_anim_path_bounce);
    lua_setfield(L, -2, "path_bounce");
    lua_setfield(L, lv_idx, "anim");

    set_subtable(L, lv_idx, "anim_var", anim_var_funcs);
}
