/**
 * lua_lv_event.c — Bindings pour lv.event, lv.group, lv.anim,
 *                  lv.anim_var, lv.timer
 */

#include "lua_lv.h"
#include "lvgl/src/misc/lv_gc.h"   /* LV_GC_ROOT(_lv_anim_ll) */
#include <stdint.h>
#include <stdlib.h>

/* Thread principal : les callbacks LVGL ne doivent pas utiliser une
   coroutine qui peut mourir */
static lua_State *main_thread(lua_State *L) {
    lua_rawgeti(L, LUA_REGISTRYINDEX, LUA_RIDX_MAINTHREAD);
    lua_State *M = lua_tothread(L, -1);
    lua_pop(L, 1);
    return M ? M : L;
}

/* ================================================================== */
/* lv.event                                                            */
/* ================================================================== */

/* Codes acceptes par lv.event.send : les lv.EVENT_* exposes, sauf DELETE.
   Les autres codes LVGL attendent en param une structure (HIT_TEST,
   COVER_CHECK, REFR_EXT_DRAW_SIZE, DRAW_*, GET_SELF_SIZE...) que les
   handlers internes liraient ou ecriraient. */
static int event_send_allowed(lv_event_code_t code) {
    switch (code) {
    case LV_EVENT_CLICKED:
    case LV_EVENT_PRESSED:
    case LV_EVENT_RELEASED:
    case LV_EVENT_FOCUSED:
    case LV_EVENT_DEFOCUSED:
    case LV_EVENT_KEY:
    case LV_EVENT_SCROLL_BEGIN:
    case LV_EVENT_SCROLL_END:
    case LV_EVENT_VALUE_CHANGED:
    case LV_EVENT_READY:
    case LV_EVENT_CANCEL:
        return 1;
    default:
        return 0;
    }
}

int lua_lv_event_code_exposed(lv_event_code_t code) {
    return code == LV_EVENT_DELETE || event_send_allowed(code);
}

static int l_event_send(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_event_code_t code = (lv_event_code_t)luaL_checkinteger(L, 2);
    /* DELETE simule : obj_final_cb (lua_lv_obj.c) invaliderait le userdata,
       liberait les cbd et desancrerait styles/image d'un objet encore
       vivant (use-after-free au rendu suivant). Seul lv.obj.del le declenche. */
    if (code == LV_EVENT_DELETE)
        return luaL_error(L, "lv.event.send: LV_EVENT_DELETE interdit, utiliser lv.obj.del");
    if (!event_send_allowed(code))
        return luaL_error(L, "lv.event.send: code d'evenement %d non supporte", (int)code);
    /* KEY : lv_obj/slider/arc lisent *(char *)param sans tester NULL, on
       passe toujours la touche (0 si absente). Autres codes : param NULL,
       les handlers LVGL de PRESSED/RELEASED... y lisent un lv_indev_t. */
    void *param = NULL;
    uint32_t key_val = 0;
    if (code == LV_EVENT_KEY) {
        if (!lua_isnoneornil(L, 3)) key_val = (uint32_t)lua_tointeger(L, 3);
        param = &key_val;
    }
    lv_event_send(obj, code, param);
    return 0;
}

/* Evenement en argument idx : light userdata d'un callback en cours,
   sinon erreur (evenement garde apres son callback, autre userdata) */
static lv_event_t *check_event(lua_State *L, int idx) {
    if (lua_isnoneornil(L, idx)) luaL_error(L, "event is nil");
    lv_event_t *e = (lv_event_t *)lua_touserdata(L, idx);
    if (!lua_islightuserdata(L, idx) || !lua_lv_event_is_active(e))
        luaL_error(L, "bad argument #%d (evenement expire ou invalide)", idx);
    return e;
}

static int l_event_get_code(lua_State *L) {
    lv_event_t *e = check_event(L, 1);
    lua_pushinteger(L, lv_event_get_code(e));
    return 1;
}

static int l_event_get_target(lua_State *L) {
    lv_event_t *e = check_event(L, 1);
    lua_lv_push_obj(L, lv_event_get_target(e));
    return 1;
}

static int l_event_get_key_value(lua_State *L) {
    if (lua_isnoneornil(L, 1)) { char z[2] = {0,0}; lua_pushstring(L, z); return 1; }
    lv_event_t *e = check_event(L, 1);

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

/* Pendant un callback DEFOCUSED, LVGL (lv_group_focus_obj, focus_next_core)
   a deja mis de cote le noeud lv_obj_t** de la cible et le reutilise au
   retour sans le revalider : retirer un objet du groupe peut liberer ce
   noeud (use-after-free). lv.obj.del/clean y sont differes (lua_lv_obj.c). */
static void group_check_not_defocus(lua_State *L, const char *fn) {
    if (lua_lv_in_defocus())
        luaL_error(L, "lv.group.%s: interdit pendant LV_EVENT_DEFOCUSED", fn);
}

static int l_group_add_obj(lua_State *L) {
    lv_group_t *grp = lua_lv_check_group(L, 1);
    lv_obj_t *obj = lua_lv_check_obj(L, 2);
    /* lv_group_add_obj commence par lv_group_remove_obj(obj) */
    if (lv_obj_get_group(obj)) group_check_not_defocus(L, "add_obj");
    lua_lv_mem_check(L, sizeof(void *) * 4, 0);   /* _lv_ll_ins_tail (LV_ASSERT_MALLOC) */
    lv_group_add_obj(grp, obj);
    return 0;
}

static int l_group_remove_obj(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    if (lv_obj_get_group(obj)) group_check_not_defocus(L, "remove_obj");
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
    group_check_not_defocus(L, "remove_all_objs");
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

/* Le handle Lua est un vrai userdata LV_MT_TIMER. Tant que le timer LVGL
   existe, self_ref garde le userdata vivant (un timer non stocke continue
   de tourner). Quand LVGL supprime le timer (one-shot termine) ou sur
   lv.timer.del, timer_release coupe le lien : tud->timer = NULL,
   user_data = NULL, refs liberees. Les appels suivants sont des no-op. */
typedef struct {
    lua_lv_timer_ud_t base;
    int               self_ref;   /* ref registry du userdata lui-meme */
} timer_ud_t;

static void timer_cb_wrapper(lv_timer_t *timer);

static void timer_release(timer_ud_t *tud) {
    lua_State *L = tud->base.L;
    if (tud->base.timer) {
        tud->base.timer->user_data = NULL;
        tud->base.timer = NULL;
    }
    if (tud->base.func_ref != LUA_NOREF) {
        luaL_unref(L, LUA_REGISTRYINDEX, tud->base.func_ref);
        tud->base.func_ref = LUA_NOREF;
    }
    if (tud->self_ref != LUA_NOREF) {
        luaL_unref(L, LUA_REGISTRYINDEX, tud->self_ref);
        tud->self_ref = LUA_NOREF;
    }
}

/* Supprime le timer LVGL s'il existe encore (idempotent) */
static void timer_delete(timer_ud_t *tud) {
    lv_timer_t *timer = tud->base.timer;
    timer_release(tud);
    if (timer) lv_timer_del(timer);
}

static void timer_set_repeat(timer_ud_t *tud, int32_t repeat) {
    if (!tud->base.timer) return;
    /* repeat 0 : LVGL supprimerait le timer sans passer par le wrapper */
    if (repeat == 0) timer_delete(tud);
    else lv_timer_set_repeat_count(tud->base.timer, repeat);
}

void lua_lv_cleanup_timers(void) {
    lv_timer_t *t = lv_timer_get_next(NULL);
    while (t) {
        lv_timer_t *next = lv_timer_get_next(t);
        if (t->timer_cb == timer_cb_wrapper) {
            timer_ud_t *tud = (timer_ud_t *)t->user_data;
            if (tud) timer_release(tud);
            lv_timer_del(t);
        }
        t = next;
    }
}

/* Apres lua_close (lua_lv_after_close) : timers Lua restants, crees par
   un __gc pendant lua_close (leur userdata, libere, n'est jamais
   finalise). user_data designe la memoire liberee : ne pas la lire. */
static void timer_after_close(void) {
    lv_timer_t *t = lv_timer_get_next(NULL);
    while (t) {
        lv_timer_t *next = lv_timer_get_next(t);
        if (t->timer_cb == timer_cb_wrapper) {
            t->user_data = NULL;
            lv_timer_del(t);
        }
        t = next;
    }
}

static void timer_cb_wrapper(lv_timer_t *timer) {
    timer_ud_t *tud = (timer_ud_t *)timer->user_data;
    if (!tud) return;
    lua_State *L = tud->base.L;

    /* Ancrer le userdata pendant l'appel (la callback peut faire del) */
    lua_rawgeti(L, LUA_REGISTRYINDEX, tud->self_ref);
    if (lua_lv_cb_enter("TIMER")) {
        lua_rawgeti(L, LUA_REGISTRYINDEX, tud->base.func_ref);
        int rc = lua_pcall(L, 0, 0, 0);
        lua_lv_cb_leave();
        if (rc != LUA_OK) {
            const char *err = lua_tostring(L, -1);
            fprintf(stderr, "[TIMER] callback error: %s\n", err ? err : "?");
            lua_pop(L, 1);
        }
    }
    /* Dernier declenchement : LVGL va supprimer le timer au retour */
    if (tud->base.timer == timer && timer->repeat_count == 0)
        timer_release(tud);
    lua_pop(L, 1);
}

static timer_ud_t *test_timer(lua_State *L, int idx) {
    return (timer_ud_t *)luaL_testudata(L, idx, LV_MT_TIMER);
}

static int l_timer_gc(lua_State *L) {
    timer_ud_t *tud = test_timer(L, 1);
    /* Ne se produit timer vivant qu'a lua_close (self_ref) */
    if (tud && tud->base.timer) {
        lv_timer_t *timer = tud->base.timer;
        timer->user_data = NULL;
        tud->base.timer = NULL;
        lv_timer_del(timer);
    }
    return 0;
}

static int l_timer_new(lua_State *L) {
    luaL_checktype(L, 1, LUA_TFUNCTION);
    uint32_t period = (uint32_t)luaL_checkinteger(L, 2);
    lua_settop(L, 3);   /* fixe repeat_count optionnel en 3 ; le userdata sera en 4 */
    lua_lv_mem_check(L, sizeof(lv_timer_t), 0);   /* lv_timer_create : LV_ASSERT_MALLOC */

    timer_ud_t *tud = (timer_ud_t *)lua_newuserdatauv(L, sizeof(timer_ud_t), 0);
    tud->base.L = main_thread(L);   /* pas une coroutine qui peut mourir */
    tud->base.func_ref = LUA_NOREF;
    tud->base.timer = NULL;
    tud->self_ref = LUA_NOREF;
    luaL_setmetatable(L, LV_MT_TIMER);

    lv_timer_t *timer = lv_timer_create(timer_cb_wrapper, period, tud);
    if (!timer) return luaL_error(L, "lv.timer.new: allocation failed");
    tud->base.timer = timer;
    lua_pushvalue(L, 1);
    tud->base.func_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    lua_pushvalue(L, -1);
    tud->self_ref = luaL_ref(L, LUA_REGISTRYINDEX);

    /* Si repeat_count specifie */
    if (!lua_isnoneornil(L, 3))
        timer_set_repeat(tud, (int32_t)lua_tointeger(L, 3));

    return 1;
}

static int l_timer_del(lua_State *L) {
    timer_ud_t *tud = test_timer(L, 1);
    if (tud) timer_delete(tud);
    return 0;
}

static int l_timer_reset(lua_State *L) {
    timer_ud_t *tud = test_timer(L, 1);
    if (tud && tud->base.timer) lv_timer_reset(tud->base.timer);
    return 0;
}

static int l_timer_set_repeat_count(lua_State *L) {
    timer_ud_t *tud = test_timer(L, 1);
    int32_t repeat = (int32_t)luaL_checkinteger(L, 2);
    if (tud) timer_set_repeat(tud, repeat);
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

/* Duree de vie : chaque copie LVGL lancee par lv.anim.start garde une ref
   registry du userdata anim dans son user_data (var LVGL = aud). Cette ref
   n'est liberee que dans anim_deleted_cb (fin, lv_anim_del, relance).
   Le var Lua et la callback exec sont des user values du userdata : hors
   copie LVGL en cours, rien dans le registre ne les ancre (une closure qui
   capture l'anim est collectable). Les callbacks LVGL ne recoivent que aud :
   la table faible registry[LV_ANIM_UD_KEY][aud] redonne le userdata (ancre
   par la ref de la copie en cours tant qu'elle tourne). */
#define LV_ANIM_UD_KEY "lv.anim_ud"

/* Pousse le userdata de aud (1) ou rien (0). N'alloue pas. */
static int anim_push_ud(lua_State *L, lua_lv_anim_ud_t *aud) {
    if (lua_getfield(L, LUA_REGISTRYINDEX, LV_ANIM_UD_KEY) != LUA_TTABLE) {
        lua_pop(L, 1);
        return 0;
    }
    lua_rawgetp(L, -1, aud);
    lua_remove(L, -2);
    if (lua_touserdata(L, -1) != aud) { lua_pop(L, 1); return 0; }
    return 1;
}

static void anim_deleted_cb(lv_anim_t *a) {
    lua_lv_anim_ud_t *aud = (lua_lv_anim_ud_t *)a->var;
    int ref = (int)(intptr_t)a->user_data;
    a->user_data = (void *)(intptr_t)LUA_NOREF;
    if (aud && ref != LUA_NOREF) luaL_unref(aud->L, LUA_REGISTRYINDEX, ref);
}

/* Si le "var" Lua de l'anim est un LvObj, pointeur vers son handle, sinon NULL.
   N'alloue pas (appelable depuis un callback LVGL). */
static lv_obj_t **anim_var_obj(lua_lv_anim_ud_t *aud) {
    if (!aud->has_var) return NULL;
    lua_State *L = aud->L;
    if (!anim_push_ud(L, aud)) return NULL;
    lua_getiuservalue(L, -1, LUA_LV_ANIM_UV_VAR);
    /* Le var reste ancre par le userdata anim (lui-meme ancre par la copie) */
    lv_obj_t **ud = (lv_obj_t **)luaL_testudata(L, -1, LV_MT_OBJ);
    lua_pop(L, 2);
    return ud;
}

/* Arrete une copie LVGL sans la supprimer : elle peut etre celle dont
   l'exec_cb est en cours (anim_timer la relit au retour). LVGL la retire
   lui-meme via anim_ready_handler (deleted_cb => unref), tout de suite
   pour la copie courante, au tick suivant pour les autres. */
static void anim_neutralize(lv_anim_t *a) {
    a->exec_cb = NULL;
    a->ready_cb = NULL;
    a->repeat_cnt = 0;
    a->playback_time = 0;
    a->act_time = (int32_t)a->time;
}

/* Neutralise les copies Lua dont la var LVGL est aud (sans toucher a la liste) */
static void anim_stop_aud(lua_lv_anim_ud_t *aud) {
    lv_anim_t *a = _lv_ll_get_head(&LV_GC_ROOT(_lv_anim_ll));
    for (; a; a = _lv_ll_get_next(&LV_GC_ROOT(_lv_anim_ll), a))
        if (a->deleted_cb == anim_deleted_cb && a->var == aud) anim_neutralize(a);
}

/* LV_EVENT_DELETE sur un objet servant de var : la var LVGL est aud et non
   l'objet, donc lv_obj_del n'arrete pas l'anim. On neutralise ici les anims
   Lua dont le var est cet objet (ou un handle deja invalide, *ud == NULL). */
static void anim_obj_delete_cb(lv_event_t *e) {
    lv_obj_t *obj = lv_event_get_target(e);
    lv_anim_t *a = _lv_ll_get_head(&LV_GC_ROOT(_lv_anim_ll));
    for (; a; a = _lv_ll_get_next(&LV_GC_ROOT(_lv_anim_ll), a)) {
        if (a->deleted_cb != anim_deleted_cb || !a->var || !a->exec_cb) continue;
        lv_obj_t **ud = anim_var_obj((lua_lv_anim_ud_t *)a->var);
        if (ud && (*ud == obj || *ud == NULL)) anim_neutralize(a);
    }
}

/* Marqueur (user_data) de anim_obj_delete_cb : teste sa presence avec
   lv_obj_get_event_user_data */
static char g_anim_obj_mark;

void lua_lv_anim_unhook_obj(lv_obj_t *obj) {
    while (lv_obj_remove_event_cb(obj, anim_obj_delete_cb)) {}
}

/* Apres lua_close : anims Lua restantes (lv.anim.start dans un __gc
   pendant lua_close). var (aud) et user_data (ref) designent l'etat
   ferme : couper les callbacks avant de retirer l'anim. */
static void anim_after_close(void) {
    for (;;) {
        lv_anim_t *a = _lv_ll_get_head(&LV_GC_ROOT(_lv_anim_ll));
        for (; a; a = _lv_ll_get_next(&LV_GC_ROOT(_lv_anim_ll), a))
            if (a->deleted_cb == anim_deleted_cb) break;
        if (!a) return;
        a->deleted_cb = NULL;
        a->exec_cb = NULL;
        a->ready_cb = NULL;
        a->start_cb = NULL;
        lv_anim_del(a->var, NULL);
    }
}

void lua_lv_event_after_close(void) {
    timer_after_close();
    anim_after_close();
}

static void anim_exec_cb_wrapper(void *var, int32_t value) {
    lua_lv_anim_ud_t *aud = (lua_lv_anim_ud_t *)var;
    if (!aud || !aud->has_exec) return;
    lua_State *L = aud->L;
    int top = lua_gettop(L);

    if (!anim_push_ud(L, aud)) return;
    int ud_idx = lua_gettop(L);   /* ancre le userdata pendant l'appel */
    lua_getiuservalue(L, ud_idx, LUA_LV_ANIM_UV_EXEC);

    /* Pousser le "var" (l'objet animé) */
    if (aud->has_var) {
        lua_getiuservalue(L, ud_idx, LUA_LV_ANIM_UV_VAR);
        /* Garde : objet deja supprime (handle invalide) */
        lv_obj_t **ud = (lv_obj_t **)luaL_testudata(L, -1, LV_MT_OBJ);
        if (ud && *ud == NULL) { lua_settop(L, top); return; }
    } else {
        lua_pushnil(L);
    }

    lua_pushinteger(L, value);

    /* lv.anim.start (early_apply) appelle exec_cb tout de suite : une
       callback qui relance son anim s'imbriquerait sans fin */
    if (!lua_lv_cb_enter("ANIM")) { lua_settop(L, top); return; }
    int rc = lua_pcall(L, 2, 0, 0);
    lua_lv_cb_leave();
    if (rc != LUA_OK) {
        const char *err = lua_tostring(L, -1);
        LV_LOG_ERROR("Lua anim exec callback error: %s", err ? err : "?");
    }
    lua_settop(L, top);
}

static int l_anim_new(lua_State *L) {
    lua_lv_anim_ud_t *aud = (lua_lv_anim_ud_t *)lua_newuserdatauv(L, sizeof(lua_lv_anim_ud_t),
                                                                  LUA_LV_ANIM_NUV);
    lv_anim_init(&aud->anim);
    aud->L = main_thread(L);   /* pas une coroutine qui peut mourir */
    aud->has_var = 0;
    aud->has_exec = 0;

    /* La variable d'animation pointe vers le userdata lui-même
       (utilisé par le wrapper exec_cb pour retrouver les refs Lua) */
    lv_anim_set_var(&aud->anim, aud);
    lv_anim_set_deleted_cb(&aud->anim, anim_deleted_cb);
    aud->anim.user_data = (void *)(intptr_t)LUA_NOREF;

    luaL_setmetatable(L, LV_MT_ANIM);

    /* Index faible aud -> userdata (cree au premier appel) */
    if (lua_getfield(L, LUA_REGISTRYINDEX, LV_ANIM_UD_KEY) != LUA_TTABLE) {
        lua_pop(L, 1);
        lua_newtable(L);
        lua_newtable(L);
        lua_pushstring(L, "v");
        lua_setfield(L, -2, "__mode");
        lua_setmetatable(L, -2);
        lua_pushvalue(L, -1);
        lua_setfield(L, LUA_REGISTRYINDEX, LV_ANIM_UD_KEY);
    }
    lua_pushvalue(L, -2);
    lua_rawsetp(L, -2, aud);
    lua_pop(L, 1);
    return 1;
}

static lua_lv_anim_ud_t *check_anim(lua_State *L, int idx) {
    return (lua_lv_anim_ud_t *)luaL_checkudata(L, idx, LV_MT_ANIM);
}

/* __gc : aucune copie LVGL ne tourne (sinon anim_ref la garderait), sauf a
   lua_close ; on les supprime par securite. var/exec sont des user values
   (rien a liberer dans le registre). */
static int l_anim_gc(lua_State *L) {
    lua_lv_anim_ud_t *aud = (lua_lv_anim_ud_t *)luaL_testudata(L, 1, LV_MT_ANIM);
    if (!aud) return 0;
    aud->has_exec = 0;
    aud->has_var = 0;
    lv_anim_del(aud, NULL);
    return 0;
}

static int l_anim_set_var(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    lua_settop(L, 2);
    /* Var = objet LVGL : arreter l'anim quand l'objet est supprime.
       Un seul callback par objet : l'ajouter seulement s'il manque, sans
       jamais le retirer (pendant LV_EVENT_DELETE, LVGL parcourt la liste
       par index : retirer une entree ferait sauter la suivante, dont
       obj_final_cb, et le handle resterait valide sur l'objet libere). */
    lv_obj_t **obj = (lv_obj_t **)luaL_testudata(L, 2, LV_MT_OBJ);
    if (obj && *obj && !lv_obj_get_event_user_data(*obj, anim_obj_delete_cb)) {
        lua_lv_mem_check(L, 0, 0);   /* lv_obj_add_event_cb : LV_ASSERT_MALLOC */
        if (!lv_obj_add_event_cb(*obj, anim_obj_delete_cb, LV_EVENT_DELETE,
                                 &g_anim_obj_mark))
            return luaL_error(L, "lv.anim.set_var: tas LVGL epuise");
    }
    /* Stocker le "var" (user value, pas de ref registry) */
    lua_pushvalue(L, 2);
    lua_setiuservalue(L, 1, LUA_LV_ANIM_UV_VAR);
    aud->has_var = 1;
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

    lua_pushvalue(L, 2);
    lua_setiuservalue(L, 1, LUA_LV_ANIM_UV_EXEC);
    aud->has_exec = 1;

    lv_anim_set_exec_cb(&aud->anim, anim_exec_cb_wrapper);
    return 0;
}

/* Les lv.anim.path_* exposes (register) : seuls pointeurs de fonction
   acceptes par set_path_cb (une police ou un cbd serait execute) */
static const lv_anim_path_cb_t g_anim_paths[] = {
    lv_anim_path_ease_in_out, lv_anim_path_linear, lv_anim_path_ease_out,
    lv_anim_path_ease_in, lv_anim_path_overshoot, lv_anim_path_bounce, NULL
};

static int l_anim_set_path_cb(lua_State *L) {
    lua_lv_anim_ud_t *aud = check_anim(L, 1);
    if (lua_isnoneornil(L, 2)) return 0;
    void *p = lua_islightuserdata(L, 2) ? lua_touserdata(L, 2) : NULL;
    for (int i = 0; g_anim_paths[i]; i++) {
        if (p == (void *)g_anim_paths[i]) {
            lv_anim_set_path_cb(&aud->anim, g_anim_paths[i]);
            return 0;
        }
    }
    return luaL_argerror(L, 2, "lv.anim.path_* attendu");
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
    lua_lv_mem_check(L, sizeof(lv_anim_t), 0);   /* lv_anim_start : LV_ASSERT_MALLOC */

    /* Garder le userdata anim vivant tant que la copie LVGL existe : la ref
       est portee par la copie (user_data) et liberee dans anim_deleted_cb.
       Une relance supprime l'ancienne copie, donc libere l'ancienne ref. */
    lua_pushvalue(L, 1);
    int anim_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    aud->anim.user_data = (void *)(intptr_t)anim_ref;
    lv_anim_set_deleted_cb(&aud->anim, anim_deleted_cb);

    /* Relance : neutraliser les copies en cours au lieu de laisser
       lv_anim_start les supprimer (lv_anim_del(var, exec_cb)). Appele depuis
       l'exec_cb, cela libererait la copie qu'anim_timer relit au retour
       (use-after-free puis double free). Neutralisees (exec_cb = NULL),
       elles ne sont plus visees par lv_anim_del et LVGL les retire via
       anim_ready_handler (deleted_cb => unref), comme lv.anim_var.del. */
    anim_stop_aud(aud);

    lv_anim_t *running = lv_anim_start(&aud->anim);
    aud->anim.user_data = (void *)(intptr_t)LUA_NOREF;
    if (!running) luaL_unref(L, LUA_REGISTRYINDEX, anim_ref);
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
        /* Neutraliser plutot que lv_anim_del : appelable depuis l'exec_cb */
        if (aud) anim_stop_aud(aud);
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
    /* Metatables creees par create_metatables (lua_lv.c) : ajouter __gc.
       __eq (l_ptr_eq) retire : le 1er champ est L, commun a tous les
       handles, toutes les anims/timers seraient egaux ; identite par defaut. */
    luaL_getmetatable(L, LV_MT_TIMER);
    lua_pushcfunction(L, l_timer_gc);
    lua_setfield(L, -2, "__gc");
    lua_pushnil(L);
    lua_setfield(L, -2, "__eq");
    lua_pop(L, 1);
    luaL_getmetatable(L, LV_MT_ANIM);
    lua_pushcfunction(L, l_anim_gc);
    lua_setfield(L, -2, "__gc");
    lua_pushnil(L);
    lua_setfield(L, -2, "__eq");
    lua_pop(L, 1);

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
    /* lv.mem (emulateur uniquement) : enregistre par lua_lv.c */
}
