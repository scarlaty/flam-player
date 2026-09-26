/**
 * lua_lv_obj.c — Bindings pour lv.obj, lv.btn, lv.label, lv.img,
 *                lv.slider, lv.img_src
 */

#include "lua_lv.h"
#include "formats/lif_decoder.h"
#include <stdlib.h>
#include <string.h>

/* ================================================================== */
/* Duree de vie des objets (F03, F02, F06, F37)                        */
/* ================================================================== */
/* Chaque lv_obj_t pousse vers Lua recoit UN enregistrement obj_rec_t
   (userdata plein, metatable LV_MT_OBJREC) range dans
   registry[LV_OBJ_REC_KEY][ptr]. Ses user values ancrent ce que LVGL
   reference sans le posseder :
     REC_UV_STYLES : { [style_ud] = true }  (lv.obj.add_style)
     REC_UV_IMG    : LvImgDsc courant       (lv.img.set_src)
     REC_UV_CBS    : { [cbd] = true }       (lv.obj.add_event_cb)
   Suppression en deux temps :
     1. obj_delete_cb (pose au premier push) ajoute obj_final_cb en fin
        de liste : LVGL relit la liste par index, il l'appelle donc
        APRES les autres handlers DELETE (dont ceux de Lua, qui peuvent
        encore utiliser l'objet).
     2. obj_final_cb : *ud = NULL, purge du cache, cbd liberes, puis
        l'enregistrement part au "cimetiere", vide au prochain tour de
        lv_timer_handler (lv_async_call) : l'objet et ses enfants sont
        alors detruits et plus rien ne lit ses styles/image.
   A lua_close, le __gc de l'enregistrement d'un objet encore vivant
   retire les handlers (plus aucun appel vers l'etat ferme). */

#define LV_MT_OBJREC       "LvObjRec"
#define LV_OBJ_REC_KEY     "lv.obj_rec"
#define LV_OBJ_GRAVE_KEY   "lv.obj_grave"
#define LV_OBJ_GUARD_KEY   "lv.obj_grave_guard"

#define REC_UV_STYLES 1
#define REC_UV_IMG    2
#define REC_UV_CBS    3
#define REC_NUV       3

typedef struct {
    lua_State *L;      /* thread principal */
    lv_obj_t  *obj;    /* NULL une fois detache */
    int        deleting;
    int        pending_del;   /* lv_async_call(deferred_del_cb, rec) en attente */
} obj_rec_t;

static void event_cb_wrapper(lv_event_t *e);
static void obj_delete_cb(lv_event_t *e);
static void obj_final_cb(lv_event_t *e);
static void deferred_del_cb(void *user_data);

/* Contextes ou lv_obj_del/lv_obj_clean ne peuvent pas s'executer tout de
   suite (voir l_obj_del) :
   - g_defocus_depth : callbacks Lua LV_EVENT_DEFOCUSED en cours ;
   - g_del_stack : objets dont la suppression est en cours (handler
     LV_EVENT_DELETE, pose par obj_delete_cb, retire par obj_final_cb ;
     racine d'un lv.obj.del/clean pendant l'appel LVGL). Au-dela de
     DEL_STACK_MAX, g_del_overflow force le report de toute suppression. */
#define DEL_STACK_MAX 64
static lv_obj_t *g_del_stack[DEL_STACK_MAX];
static int       g_del_top = 0;
static int       g_del_overflow = 0;
static int       g_defocus_depth = 0;
static int       g_cb_depth = 0;

static void del_stack_push(lv_obj_t *obj) {
    if (g_del_top < DEL_STACK_MAX) g_del_stack[g_del_top++] = obj;
    else g_del_overflow++;
}

static void del_stack_pop(lv_obj_t *obj) {
    for (int i = g_del_top - 1; i >= 0; i--) {
        if (g_del_stack[i] == obj) {
            for (; i < g_del_top - 1; i++) g_del_stack[i] = g_del_stack[i + 1];
            g_del_top--;
            return;
        }
    }
    if (g_del_overflow > 0) g_del_overflow--;
}

int lua_lv_in_defocus(void) {
    return g_defocus_depth > 0;
}

int lua_lv_cb_enter(const char *what) {
    if (g_cb_depth >= LUA_LV_CB_MAX_DEPTH) {
        fprintf(stderr, "[%s] recursion trop profonde (%d callbacks imbriques) : "
                "callback Lua ignore\n", what, g_cb_depth);
        return 0;
    }
    g_cb_depth++;
    return 1;
}

void lua_lv_cb_leave(void) {
    if (g_cb_depth > 0) g_cb_depth--;
}

/* Etat dont le cimetiere attend un vidage (lv_async_call en cours) */
static lua_State *g_grave_L = NULL;

static lua_State *main_thread(lua_State *L) {
    lua_rawgeti(L, LUA_REGISTRYINDEX, LUA_RIDX_MAINTHREAD);
    lua_State *M = lua_tothread(L, -1);
    lua_pop(L, 1);
    return M ? M : L;
}

/* Pousse registry[key], cree si absent (mode : NULL ou "v") */
static void push_reg_table(lua_State *L, const char *key, const char *mode) {
    if (lua_getfield(L, LUA_REGISTRYINDEX, key) == LUA_TTABLE) return;
    lua_pop(L, 1);
    lua_newtable(L);
    if (mode) {
        lua_newtable(L);
        lua_pushstring(L, mode);
        lua_setfield(L, -2, "__mode");
        lua_setmetatable(L, -2);
    }
    lua_pushvalue(L, -1);
    lua_setfield(L, LUA_REGISTRYINDEX, key);
}

/* Pousse l'enregistrement de obj (ou nil si absent et !create) */
static obj_rec_t *rec_push(lua_State *L, lv_obj_t *obj, int create) {
    push_reg_table(L, LV_OBJ_REC_KEY, NULL);
    lua_rawgetp(L, -1, obj);
    obj_rec_t *rec = (obj_rec_t *)lua_touserdata(L, -1);
    if (!rec && create) {
        lua_pop(L, 1);
        rec = (obj_rec_t *)lua_newuserdatauv(L, sizeof(obj_rec_t), REC_NUV);
        rec->L = main_thread(L);
        rec->obj = obj;
        rec->deleting = 0;
        rec->pending_del = 0;
        luaL_setmetatable(L, LV_MT_OBJREC);
        lua_pushvalue(L, -1);
        lua_rawsetp(L, -3, obj);
        lv_obj_add_event_cb(obj, obj_delete_cb, LV_EVENT_DELETE, rec);
    }
    lua_remove(L, -2);
    return rec;
}

/* Pousse la table user value n de l'enregistrement en rec_idx (creee) */
static void rec_push_uv_table(lua_State *L, int rec_idx, int n) {
    rec_idx = lua_absindex(L, rec_idx);
    if (lua_getiuservalue(L, rec_idx, n) == LUA_TTABLE) return;
    lua_pop(L, 1);
    lua_newtable(L);
    lua_pushvalue(L, -1);
    lua_setiuservalue(L, rec_idx, n);
}

/* Coupe l'enregistrement en rec_idx de son objet : retire les handlers
   LVGL et libere les cbd. unref = 0 pendant lua_close. */
static void rec_detach(lua_State *L, int rec_idx, int unref) {
    rec_idx = lua_absindex(L, rec_idx);
    obj_rec_t *rec = (obj_rec_t *)lua_touserdata(L, rec_idx);
    lv_obj_t *obj = rec->obj;
    if (!obj) return;
    rec->obj = NULL;
    /* Suppression differee en attente : ne plus viser l'objet (supprime)
       ni l'enregistrement (collecte) */
    if (rec->pending_del) {
        rec->pending_del = 0;
        lv_async_call_cancel(deferred_del_cb, rec);
    }
    if (lua_getiuservalue(L, rec_idx, REC_UV_CBS) == LUA_TTABLE) {
        lua_pushnil(L);
        while (lua_next(L, -2)) {
            lua_lv_cb_data_t *cbd = (lua_lv_cb_data_t *)lua_touserdata(L, -2);
            lua_pop(L, 1);
            lv_obj_remove_event_cb_with_user_data(obj, event_cb_wrapper, cbd);
            if (unref) luaL_unref(L, LUA_REGISTRYINDEX, cbd->func_ref);
            free(cbd);
        }
    }
    lua_pop(L, 1);
    lua_pushnil(L);
    lua_setiuservalue(L, rec_idx, REC_UV_CBS);
    lv_obj_remove_event_cb_with_user_data(obj, obj_delete_cb, rec);
    lv_obj_remove_event_cb_with_user_data(obj, obj_final_cb, rec);
}

static void grave_flush_cb(void *user_data) {
    lua_State *L = (lua_State *)user_data;
    if (L != g_grave_L) return;
    g_grave_L = NULL;
    lua_pushnil(L);
    lua_setfield(L, LUA_REGISTRYINDEX, LV_OBJ_GRAVE_KEY);
}

/* __gc de la sentinelle : lua_close, annuler le vidage en attente */
static int grave_guard_gc(lua_State *L) {
    lua_State *M = main_thread(L);
    if (g_grave_L == M) {
        lv_async_call_cancel(grave_flush_cb, M);
        g_grave_L = NULL;
    }
    return 0;
}

/* __gc de l'enregistrement : seulement a lua_close si l'objet vit encore */
static int rec_gc(lua_State *L) {
    rec_detach(L, 1, 0);
    return 0;
}

static void obj_delete_cb(lv_event_t *e) {
    obj_rec_t *rec = (obj_rec_t *)lv_event_get_user_data(e);
    /* target != obj : DELETE d'un enfant remonte par bubbling */
    if (!rec || !rec->obj || rec->obj != lv_event_get_target(e) || rec->deleting) return;
    rec->deleting = 1;
    /* obj_final_cb retire l'objet de g_del_stack (fin des handlers DELETE) */
    if (lv_obj_add_event_cb(rec->obj, obj_final_cb, LV_EVENT_DELETE, rec))
        del_stack_push(rec->obj);
}

static void obj_final_cb(lv_event_t *e) {
    obj_rec_t *rec = (obj_rec_t *)lv_event_get_user_data(e);
    if (!rec || !rec->obj || rec->obj != lv_event_get_target(e)) return;
    lua_State *L = rec->L;
    lv_obj_t *obj = rec->obj;
    int top = lua_gettop(L);
    del_stack_pop(obj);

    /* 1. Invalider le userdata (s'il n'a pas ete collecte) et le cache */
    push_reg_table(L, LV_OBJ_CACHE_KEY, "v");
    lua_rawgetp(L, -1, obj);
    lv_obj_t **ud = (lv_obj_t **)lua_touserdata(L, -1);
    if (ud) *ud = NULL;
    lua_pop(L, 1);
    lua_pushnil(L);
    lua_rawsetp(L, -2, obj);
    lua_pop(L, 1);

    /* 2. Retirer l'enregistrement et liberer les cbd */
    push_reg_table(L, LV_OBJ_REC_KEY, NULL);
    lua_rawgetp(L, -1, obj);
    lua_pushnil(L);
    lua_rawsetp(L, -3, obj);
    if (lua_touserdata(L, -1) == rec) {
        rec_detach(L, -1, 1);
        /* 3. Styles/image gardes jusqu'au prochain lv_timer_handler */
        push_reg_table(L, LV_OBJ_GRAVE_KEY, NULL);
        lua_pushvalue(L, -2);
        lua_rawseti(L, -2, (lua_Integer)lua_rawlen(L, -2) + 1);
        if (g_grave_L != L && lv_async_call(grave_flush_cb, L) == LV_RES_OK)
            g_grave_L = L;
    } else {
        rec->obj = NULL;
    }
    lua_settop(L, top);
}

/* Apres lua_close (lua_lv_after_close) : retire recursivement les handlers
   des bindings. Leur user_data (rec, cbd) vient de l'etat ferme : un objet
   pousse vers Lua par un __gc pendant lua_close garde un enregistrement
   jamais finalise (rec_gc ne s'execute pas), la suppression suivante de
   l'objet lirait la memoire liberee. Les cbd (malloc) sont liberes. */
static void obj_after_close_tree(lv_obj_t *obj) {
    void *cbd;
    while ((cbd = lv_obj_get_event_user_data(obj, event_cb_wrapper)) != NULL) {
        lv_obj_remove_event_cb_with_user_data(obj, event_cb_wrapper, cbd);
        free(cbd);
    }
    while (lv_obj_remove_event_cb(obj, obj_delete_cb)) {}
    while (lv_obj_remove_event_cb(obj, obj_final_cb)) {}
    lua_lv_anim_unhook_obj(obj);
    uint32_t n = lv_obj_get_child_cnt(obj);
    for (uint32_t i = 0; i < n; i++)
        obj_after_close_tree(lv_obj_get_child(obj, (int32_t)i));
}

void lua_lv_obj_after_close(void) {
    for (lv_disp_t *d = lv_disp_get_next(NULL); d; d = lv_disp_get_next(d)) {
        for (uint32_t i = 0; i < d->screen_cnt; i++)
            obj_after_close_tree(d->screens[i]);
        if (d->top_layer) obj_after_close_tree(d->top_layer);
        if (d->sys_layer) obj_after_close_tree(d->sys_layer);
    }
    /* Vidage du cimetiere programme apres le __gc de la sentinelle */
    if (g_grave_L) {
        lv_async_call_cancel(grave_flush_cb, g_grave_L);
        g_grave_L = NULL;
    }
    /* Aucune suppression n'est en cours entre deux etats */
    g_del_top = 0;
    g_del_overflow = 0;
}

void lua_lv_push_obj(lua_State *L, lv_obj_t *obj) {
    if (!obj) { lua_pushnil(L); return; }

    /* Chercher le userdata existant */
    push_reg_table(L, LV_OBJ_CACHE_KEY, "v");
    if (lua_rawgetp(L, -1, obj) != LUA_TNIL) {
        lua_remove(L, -2);  /* enlever cache_table */
        return;
    }
    lua_pop(L, 1);  /* pop nil */

    /* Objet deja passe par LV_EVENT_DELETE : ne pas le ressusciter */
    if (obj->being_deleted) {
        lua_pop(L, 1);
        lua_pushnil(L);
        return;
    }

    /* Créer un nouveau userdata et l'ajouter au cache */
    lv_obj_t **ud = (lv_obj_t **)lua_newuserdatauv(L, sizeof(lv_obj_t *), 0);
    *ud = obj;
    luaL_setmetatable(L, LV_MT_OBJ);
    lua_pushvalue(L, -1);
    lua_rawsetp(L, -3, obj);  /* cache[ptr] = userdata */
    lua_remove(L, -2);        /* enlever cache_table */

    /* Enregistrement de duree de vie (une seule fois par objet) */
    rec_push(L, obj, 1);
    lua_pop(L, 1);
}

/* ================================================================== */
/* lv.obj                                                              */
/* ================================================================== */

/* Constructeur commun (X1a) : garde du tas LVGL avant lv_xxx_create, qui
   planterait sur un lv_mem_realloc NULL (lv_obj_class.c) au lieu d'echouer */
static int obj_new_guarded(lua_State *L, const lv_obj_class_t *cls,
                           lv_obj_t *(*create)(lv_obj_t *)) {
    lv_obj_t *parent = lua_lv_opt_obj(L, 1);
    lua_lv_mem_check_create(L, parent, cls);
    lv_obj_t *obj = create(parent);
    if (!obj) return luaL_error(L, "tas LVGL epuise (creation d'objet)");
    lua_lv_push_obj(L, obj);
    return 1;
}

/* Fonctions non gardees par lua_lv_setfuncs_guarded : constructeurs
   (garde propre), liberations (doivent marcher tas plein), sans allocation */
static const char *const obj_unguarded[] = {
    "new", "del", "clean", "invalidate", "add_event_cb", "remove_event_cb",
    "remove_style", "remove_style_all", "remove_flag", "clear_flag", NULL
};

static int l_obj_new(lua_State *L) {
    return obj_new_guarded(L, &lv_obj_class, lv_obj_create);
}

/* Vrai si obj est deja en cours de suppression : handler LV_EVENT_DELETE
   en cours (rec->deleting, pose par obj_delete_cb avant les handlers Lua ;
   LVGL ne met being_deleted qu'apres DELETE) ou objet/ancetre marque
   being_deleted (suppression recursive des enfants). Sinon lv.obj.del(target)
   dans un handler DELETE relancerait DELETE sans fin (debordement de la
   pile C, 0xC00000FD). */
static int obj_is_deleting(lua_State *L, lv_obj_t *obj) {
    obj_rec_t *rec = rec_push(L, obj, 0);
    int deleting = rec && rec->obj == obj && rec->deleting;
    lua_pop(L, 1);
    for (lv_obj_t *o = obj; o && !deleting; o = lv_obj_get_parent(o))
        if (o->being_deleted) deleting = 1;
    return deleting;
}

/* Vrai si supprimer (ou vider) obj maintenant liberait de la memoire que
   LVGL reutilise au retour :
   - callback DEFOCUSED en cours : lv_group_focus_obj/focus_next_core ont
     mis de cote le noeud de groupe de la nouvelle cible (lv_obj_del le
     libere via lv_group_remove_obj) ;
   - obj est un ancetre d'un objet en cours de suppression : lv_obj_del
     exterieur relit son parent apres obj_del_core (use-after-free), et
     clean relancerait DELETE sans fin (0xC00000FD). */
static int obj_del_must_defer(lv_obj_t *obj) {
    if (g_defocus_depth > 0 || g_del_overflow > 0) return 1;
    for (int i = 0; i < g_del_top; i++)
        for (lv_obj_t *o = g_del_stack[i]; o; o = lv_obj_get_parent(o))
            if (o == obj) return 1;
    return 0;
}

/* Suppression differee (lv_async_call) : annulee par rec_detach si l'objet
   est supprime avant (sinon lv_obj_del_async viserait un objet libere) */
static void deferred_del_cb(void *user_data) {
    obj_rec_t *rec = (obj_rec_t *)user_data;
    rec->pending_del = 0;   /* avant lv_obj_del : rec_detach n'annule plus */
    lv_obj_t *obj = rec->obj;
    if (!obj || rec->deleting || obj->being_deleted) return;
    del_stack_push(obj);
    lv_obj_del(obj);
    del_stack_pop(obj);
}

static void obj_del_deferred(lua_State *L, lv_obj_t *obj) {
    if (obj_is_deleting(L, obj)) return;
    obj_rec_t *rec = rec_push(L, obj, 1);
    if (rec && rec->obj == obj && !rec->pending_del) {
        /* lv_async_call : lv_timer_create (LV_ASSERT_MALLOC) + info */
        lua_lv_mem_check(L, sizeof(lv_timer_t), 2 * sizeof(void *));
        if (lv_async_call(deferred_del_cb, rec) != LV_RES_OK)
            luaL_error(L, "lv.obj.del: tas LVGL epuise (suppression differee)");
        rec->pending_del = 1;
    }
    lua_pop(L, 1);
}

/* Contexte dangereux (obj_del_must_defer) : suppression reportee au
   prochain lv_timer_handler, l'objet reste utilisable jusque-la. */
static int l_obj_del(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj_or_null(L, 1);  /* deja supprime : no-op */
    if (!obj || obj_is_deleting(L, obj)) return 0;
    if (obj_del_must_defer(obj)) {
        obj_del_deferred(L, obj);
        return 0;
    }
    del_stack_push(obj);
    lv_obj_del(obj);
    del_stack_pop(obj);
    return 0;
}

static int l_obj_clean(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj_or_null(L, 1);
    if (!obj || obj_is_deleting(L, obj)) return 0;
    if (obj_del_must_defer(obj)) {
        /* Enfants presents a l'appel seulement : ceux crees ensuite dans le
           meme handler restent (meme resultat que clean immediat) */
        uint32_t n = lv_obj_get_child_cnt(obj);
        for (uint32_t i = 0; i < n; i++) {
            lv_obj_t *child = lv_obj_get_child(obj, (int32_t)i);
            if (child) obj_del_deferred(L, child);
        }
        return 0;
    }
    del_stack_push(obj);
    lv_obj_clean(obj);
    del_stack_pop(obj);
    return 0;
}

static int l_obj_invalidate(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    if (obj) lv_obj_invalidate(obj);
    return 0;
}

/* --- Taille & position --- */

static int l_obj_set_size(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_coord_t w = (lv_coord_t)luaL_checkinteger(L, 2);
    lv_coord_t h = (lv_coord_t)luaL_checkinteger(L, 3);
    lv_obj_set_size(obj, w, h);
    return 0;
}

static int l_obj_set_width(lua_State *L) {
    lv_obj_set_width(lua_lv_check_obj(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_set_height(lua_State *L) {
    lv_obj_set_height(lua_lv_check_obj(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_align(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_align_t align = (lv_align_t)luaL_checkinteger(L, 2);
    lv_coord_t x = (lv_coord_t)luaL_optinteger(L, 3, 0);
    lv_coord_t y = (lv_coord_t)luaL_optinteger(L, 4, 0);
    lv_obj_align(obj, align, x, y);
    return 0;
}

static int l_obj_set_pos(lua_State *L) {
    lv_obj_set_pos(lua_lv_check_obj(L, 1),
                   (lv_coord_t)luaL_checkinteger(L, 2),
                   (lv_coord_t)luaL_checkinteger(L, 3));
    return 0;
}

/* --- Style --- */

static int l_obj_add_style(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_style_t *style = lua_lv_check_style(L, 2);
    lv_style_selector_t sel = (lv_style_selector_t)luaL_optinteger(L, 3, 0);
    lv_obj_add_style(obj, style, sel);
    /* Ancrer le style a l'objet : sinon le GC le libere (lv_style_reset)
       alors que LVGL le lit encore au rendu (F02) */
    rec_push(L, obj, 1);
    rec_push_uv_table(L, -1, REC_UV_STYLES);
    lua_pushvalue(L, 2);
    lua_pushboolean(L, 1);
    lua_rawset(L, -3);
    lua_pop(L, 2);
    return 0;
}

/* Retire l'ancre des styles qui ne sont plus attaches a obj */
static void rec_sync_styles(lua_State *L, lv_obj_t *obj) {
    if (!rec_push(L, obj, 0)) { lua_pop(L, 1); return; }
    if (lua_getiuservalue(L, -1, REC_UV_STYLES) == LUA_TTABLE) {
        lua_pushnil(L);
        while (lua_next(L, -2)) {
            lv_style_t *style = (lv_style_t *)lua_touserdata(L, -2);
            uint32_t i;
            lua_pop(L, 1);
            for (i = 0; i < obj->style_cnt; i++)
                if (obj->styles[i].style == style) break;
            if (i == obj->style_cnt) {
                lua_pushvalue(L, -1);
                lua_pushnil(L);
                lua_rawset(L, -4);  /* autorise pendant lua_next */
            }
        }
    }
    lua_pop(L, 2);
}

static int l_obj_remove_style(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_style_t *style = lua_lv_check_style(L, 2);
    lv_style_selector_t sel = (lv_style_selector_t)luaL_optinteger(L, 3, 0);
    lv_obj_remove_style(obj, style, sel);
    rec_sync_styles(L, obj);
    return 0;
}

static int l_obj_remove_style_all(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_obj_remove_style_all(obj);
    rec_sync_styles(L, obj);
    return 0;
}

/* --- Flags --- */

static int l_obj_add_flag(lua_State *L) {
    lv_obj_add_flag(lua_lv_check_obj(L, 1), (lv_obj_flag_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_remove_flag(lua_State *L) {
    /* Lua dit "remove_flag", LVGL dit "clear_flag" */
    lv_obj_clear_flag(lua_lv_check_obj(L, 1), (lv_obj_flag_t)luaL_checkinteger(L, 2));
    return 0;
}

/* --- Flex --- */

static int l_obj_set_flex_flow(lua_State *L) {
    lv_obj_set_flex_flow(lua_lv_check_obj(L, 1),
                         (lv_flex_flow_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_set_flex_align(lua_State *L) {
    lv_obj_set_flex_align(lua_lv_check_obj(L, 1),
                          (lv_flex_align_t)luaL_checkinteger(L, 2),
                          (lv_flex_align_t)luaL_checkinteger(L, 3),
                          (lv_flex_align_t)luaL_checkinteger(L, 4));
    return 0;
}

/* --- Scroll --- */

static int l_obj_scroll_by(lua_State *L) {
    lv_obj_scroll_by(lua_lv_check_obj(L, 1),
                     (lv_coord_t)luaL_checkinteger(L, 2),
                     (lv_coord_t)luaL_checkinteger(L, 3),
                     (lv_anim_enable_t)luaL_optinteger(L, 4, LV_ANIM_OFF));
    return 0;
}

static int l_obj_scroll_to(lua_State *L) {
    lv_obj_scroll_to(lua_lv_check_obj(L, 1),
                     (lv_coord_t)luaL_checkinteger(L, 2),
                     (lv_coord_t)luaL_checkinteger(L, 3),
                     (lv_anim_enable_t)luaL_optinteger(L, 4, LV_ANIM_OFF));
    return 0;
}

static int l_obj_get_scroll_y(lua_State *L) {
    lua_pushinteger(L, lv_obj_get_scroll_y(lua_lv_check_obj(L, 1)));
    return 1;
}

/* --- Coords --- */

static int l_obj_get_coords(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_area_t *area = (lv_area_t *)luaL_checkudata(L, 2, LV_MT_AREA);
    lv_obj_update_layout(obj);
    lv_obj_get_coords(obj, area);
    return 0;
}

/* --- Getters --- */

static int l_obj_get_width(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_obj_update_layout(obj);
    lua_pushinteger(L, lv_obj_get_width(obj));
    return 1;
}

static int l_obj_get_height(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_obj_update_layout(obj);
    lua_pushinteger(L, lv_obj_get_height(obj));
    return 1;
}

static int l_obj_get_x(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_obj_update_layout(obj);
    lua_pushinteger(L, lv_obj_get_x(obj));
    return 1;
}

static int l_obj_set_x(lua_State *L) {
    lv_obj_set_x(lua_lv_check_obj(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_get_child(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    int32_t idx = (int32_t)luaL_checkinteger(L, 2);
    lv_obj_t *child = lv_obj_get_child(obj, idx);
    lua_lv_push_obj(L, child);
    return 1;
}

static int l_obj_get_child_cnt(lua_State *L) {
    lua_pushinteger(L, lv_obj_get_child_cnt(lua_lv_check_obj(L, 1)));
    return 1;
}

static int l_obj_set_style_pad_bottom(lua_State *L) {
    lv_obj_set_style_pad_bottom(lua_lv_check_obj(L, 1),
                                (lv_coord_t)luaL_checkinteger(L, 2),
                                (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_y(lua_State *L) {
    lv_obj_set_y(lua_lv_check_obj(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_set_align(lua_State *L) {
    lv_obj_set_align(lua_lv_check_obj(L, 1), (lv_align_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_get_state(lua_State *L) {
    lua_pushinteger(L, lv_obj_get_state(lua_lv_check_obj(L, 1)));
    return 1;
}

static int l_obj_set_style_pad_left(lua_State *L) {
    lv_obj_set_style_pad_left(lua_lv_check_obj(L, 1),
                              (lv_coord_t)luaL_checkinteger(L, 2),
                              (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_pad_top(lua_State *L) {
    lv_obj_set_style_pad_top(lua_lv_check_obj(L, 1),
                             (lv_coord_t)luaL_checkinteger(L, 2),
                             (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_radius(lua_State *L) {
    lv_obj_set_style_radius(lua_lv_check_obj(L, 1),
                            (lv_coord_t)luaL_checkinteger(L, 2),
                            (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_text_align(lua_State *L) {
    lv_obj_set_style_text_align(lua_lv_check_obj(L, 1),
                                (lv_text_align_t)luaL_checkinteger(L, 2),
                                (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

/* --- State --- */

static int l_obj_add_state(lua_State *L) {
    lv_obj_add_state(lua_lv_check_obj(L, 1),
                     (lv_state_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_clear_state(lua_State *L) {
    lv_obj_clear_state(lua_lv_check_obj(L, 1),
                       (lv_state_t)luaL_checkinteger(L, 2));
    return 0;
}

/* --- Scroll snap --- */

static int l_obj_set_scroll_snap_y(lua_State *L) {
    lv_obj_set_scroll_snap_y(lua_lv_check_obj(L, 1),
                             (lv_scroll_snap_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_obj_update_snap(lua_State *L) {
    lv_obj_update_snap(lua_lv_check_obj(L, 1),
                       (lv_anim_enable_t)luaL_optinteger(L, 2, LV_ANIM_OFF));
    return 0;
}

static int l_obj_set_style_img_opa(lua_State *L) {
    lv_obj_set_style_img_opa(lua_lv_check_obj(L, 1),
                             (lv_opa_t)luaL_checkinteger(L, 2),
                             (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

/* --- center (alias) --- */

static int l_obj_center(lua_State *L) {
    lv_obj_center(lua_lv_check_obj(L, 1));
    return 0;
}

/* --- Style inline (lv.obj.set_style_*) --- */

static int l_obj_set_style_bg_color(lua_State *L) {
    lv_obj_set_style_bg_color(lua_lv_check_obj(L, 1),
                              lua_lv_check_color(L, 2),
                              (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_bg_opa(lua_State *L) {
    lv_obj_set_style_bg_opa(lua_lv_check_obj(L, 1),
                            (lv_opa_t)luaL_checkinteger(L, 2),
                            (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_pad_row(lua_State *L) {
    lv_obj_set_style_pad_row(lua_lv_check_obj(L, 1),
                             (lv_coord_t)luaL_checkinteger(L, 2),
                             (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_pad_column(lua_State *L) {
    lv_obj_set_style_pad_column(lua_lv_check_obj(L, 1),
                                (lv_coord_t)luaL_checkinteger(L, 2),
                                (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_translate_x(lua_State *L) {
    lv_obj_set_style_translate_x(lua_lv_check_obj(L, 1),
                                 (lv_coord_t)luaL_checkinteger(L, 2),
                                 (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_translate_y(lua_State *L) {
    lv_obj_set_style_translate_y(lua_lv_check_obj(L, 1),
                                 (lv_coord_t)luaL_checkinteger(L, 2),
                                 (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_text_color(lua_State *L) {
    lv_obj_set_style_text_color(lua_lv_check_obj(L, 1),
                                lua_lv_check_color(L, 2),
                                (lv_style_selector_t)luaL_optinteger(L, 3, 0));
    return 0;
}

static int l_obj_set_style_text_font(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    const lv_font_t *font = lua_lv_opt_font(L, 2);   /* nil : ignore */
    lv_style_selector_t sel = (lv_style_selector_t)luaL_optinteger(L, 3, 0);
    if (font) lv_obj_set_style_text_font(obj, font, sel);
    return 0;
}

/* --- Event callbacks --- */

/* Pile des evenements dont le callback Lua est en cours (maillons sur la
   pile C de event_cb_wrapper). Le lv_event_t est une variable locale de
   lv_event_send : garde par le script apres son callback, il designe une
   zone de pile reutilisee ; lv.event.get_* le refusent (lua_lv_event.c). */
typedef struct evt_frame {
    const lv_event_t       *e;
    const struct evt_frame *prev;
} evt_frame_t;

static const evt_frame_t *g_evt_top = NULL;

int lua_lv_event_is_active(const void *e) {
    for (const evt_frame_t *f = g_evt_top; f; f = f->prev)
        if ((const void *)f->e == e) return 1;
    return 0;
}

static void event_cb_wrapper(lv_event_t *e) {
    lua_lv_cb_data_t *cbd = (lua_lv_cb_data_t *)lv_event_get_user_data(e);
    if (!cbd) return;
    /* Retire pendant la suppression de l'objet (l_obj_remove_event_cb) */
    if (cbd->func_ref == LUA_NOREF) return;
    lua_State *L = cbd->L;
    lv_event_code_t code = lv_event_get_code(e);

    /* Handler LV_EVENT_ALL : codes internes (DRAW_*, COVER_CHECK...) emis
       pendant le rendu, jamais transmis a Lua */
    if (!lua_lv_event_code_exposed(code)) return;
    if (!lua_lv_cb_enter("EVENT")) return;
    int defocus = (code == LV_EVENT_DEFOCUSED);

    lua_rawgeti(L, LUA_REGISTRYINDEX, cbd->func_ref);

    /* Pousser l'événement comme light userdata (valide pendant le callback) */
    lua_pushlightuserdata(L, (void *)e);

    /* lua_pcall rattrape toute erreur : le maillon est toujours depile */
    evt_frame_t frame = { e, g_evt_top };
    g_evt_top = &frame;
    g_defocus_depth += defocus;
    int rc = lua_pcall(L, 1, 0, 0);
    g_defocus_depth -= defocus;
    g_evt_top = frame.prev;
    lua_lv_cb_leave();
    if (rc != LUA_OK) {
        const char *err = lua_tostring(L, -1);
        fprintf(stderr, "[EVENT] callback error (code=%d): %s\n",
                lv_event_get_code(e), err ? err : "?");
        lua_pop(L, 1);
    }
}

static int l_obj_add_event_cb(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    luaL_checktype(L, 2, LUA_TFUNCTION);
    lua_Integer icode = luaL_checkinteger(L, 3);
    lv_event_code_t code = (lv_event_code_t)icode;
    /* lv.EVENT_* exposes, ou 0 (LV_EVENT_ALL, filtre dans event_cb_wrapper).
       Les autres codes (DRAW_*, COVER_CHECK, HIT_TEST...) appelleraient Lua
       pendant le rendu, ou lv.obj.del libererait l'objet en cours de dessin. */
    if (icode != LV_EVENT_ALL
        && (icode < 0 || icode >= _LV_EVENT_LAST || !lua_lv_event_code_exposed(code)))
        return luaL_argerror(L, 3, "code d'evenement non supporte");

    /* lv_obj_add_event_cb : realloc de event_dsc (LV_ASSERT_MALLOC) ;
       lv_event_dsc_t est prive a lv_event.c : cb + user_data + filtre */
    size_t ndsc = obj->spec_attr ? obj->spec_attr->event_dsc_cnt : 0;
    lua_lv_mem_check(L, (ndsc + 1) * 3 * sizeof(void *),
                     obj->spec_attr ? 0 : sizeof(_lv_obj_spec_attr_t));

    lua_lv_cb_data_t *cbd = (lua_lv_cb_data_t *)malloc(sizeof(lua_lv_cb_data_t));
    if (!cbd) return luaL_error(L, "add_event_cb: out of memory");
    cbd->L = main_thread(L);  /* pas une coroutine qui peut mourir */

    /* Stocker la fonction dans le registry */
    lua_pushvalue(L, 2);
    cbd->func_ref = luaL_ref(L, LUA_REGISTRYINDEX);

    if (!lv_obj_add_event_cb(obj, event_cb_wrapper, code, cbd)) {
        luaL_unref(L, LUA_REGISTRYINDEX, cbd->func_ref);
        free(cbd);
        return luaL_error(L, "add_event_cb: tas LVGL epuise");
    }

    /* Rattacher le cbd a l'objet : libere sur LV_EVENT_DELETE (F37) */
    rec_push(L, obj, 1);
    rec_push_uv_table(L, -1, REC_UV_CBS);
    lua_pushboolean(L, 1);
    lua_rawsetp(L, -2, cbd);
    lua_pop(L, 2);

    /* Return cbd as light userdata — used by remove_event_cb */
    lua_pushlightuserdata(L, cbd);
    return 1;
}

static int l_obj_remove_event_cb(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj_or_null(L, 1);  /* supprime : deja libere */
    lua_lv_cb_data_t *cbd = (lua_lv_cb_data_t *)lua_touserdata(L, 2);
    if (!obj || !cbd) return 0;

    /* Le cbd doit appartenir a cet objet : sinon (deja retire, autre
       objet, pointeur perime) ne rien liberer (pas de double free) */
    int owned = 0, deleting = 0;
    int top = lua_gettop(L);
    obj_rec_t *rec = rec_push(L, obj, 0);
    if (rec && lua_getiuservalue(L, -1, REC_UV_CBS) == LUA_TTABLE
        && lua_rawgetp(L, -1, cbd) != LUA_TNIL) {
        owned = 1;
        deleting = rec->obj == obj && rec->deleting;
        if (!deleting) {
            lua_pushnil(L);
            lua_rawsetp(L, -3, cbd);
        }
    }
    lua_settop(L, top);
    if (!owned) return 0;

    /* Handlers LV_EVENT_DELETE en cours : LVGL parcourt la liste par index,
       retirer une entree ferait sauter la suivante (obj_final_cb : handle
       jamais invalide, use-after-free). Le cbd reste dans la liste et dans
       REC_UV_CBS, neutralise (event_cb_wrapper l'ignore) ; rec_detach le
       libere dans obj_final_cb. */
    if (deleting) {
        luaL_unref(L, LUA_REGISTRYINDEX, cbd->func_ref);
        cbd->func_ref = LUA_NOREF;
        return 0;
    }

    /* Remove the LVGL event using the user_data pointer to identify it */
    lv_obj_remove_event_cb_with_user_data(obj, event_cb_wrapper, cbd);
    luaL_unref(L, LUA_REGISTRYINDEX, cbd->func_ref);
    free(cbd);
    return 0;
}

/* --- Table lv.obj --- */

static const luaL_Reg obj_funcs[] = {
    {"new",              l_obj_new},
    {"del",              l_obj_del},
    {"clean",            l_obj_clean},
    {"invalidate",       l_obj_invalidate},
    {"set_size",         l_obj_set_size},
    {"set_width",        l_obj_set_width},
    {"set_height",       l_obj_set_height},
    {"set_pos",          l_obj_set_pos},
    {"set_x",            l_obj_set_x},
    {"set_y",            l_obj_set_y},
    {"set_align",        l_obj_set_align},
    {"align",            l_obj_align},
    {"add_style",        l_obj_add_style},
    {"remove_style",     l_obj_remove_style},
    {"remove_style_all", l_obj_remove_style_all},
    {"add_flag",         l_obj_add_flag},
    {"remove_flag",      l_obj_remove_flag},
    {"clear_flag",       l_obj_remove_flag},
    {"set_flex_flow",    l_obj_set_flex_flow},
    {"set_flex_align",   l_obj_set_flex_align},
    {"scroll_by",        l_obj_scroll_by},
    {"scroll_to",        l_obj_scroll_to},
    {"get_scroll_y",     l_obj_get_scroll_y},
    {"get_width",        l_obj_get_width},
    {"get_height",       l_obj_get_height},
    {"get_x",            l_obj_get_x},
    {"get_child",        l_obj_get_child},
    {"get_child_cnt",    l_obj_get_child_cnt},
    {"get_state",        l_obj_get_state},
    {"get_coords",       l_obj_get_coords},
    {"add_state",        l_obj_add_state},
    {"clear_state",      l_obj_clear_state},
    {"set_scroll_snap_y", l_obj_set_scroll_snap_y},
    {"update_snap",      l_obj_update_snap},
    {"add_event_cb",     l_obj_add_event_cb},
    {"remove_event_cb",  l_obj_remove_event_cb},
    /* set_style_* inline */
    {"set_style_bg_color",      l_obj_set_style_bg_color},
    {"set_style_bg_opa",        l_obj_set_style_bg_opa},
    {"set_style_pad_row",       l_obj_set_style_pad_row},
    {"set_style_pad_column",    l_obj_set_style_pad_column},
    {"set_style_translate_x",   l_obj_set_style_translate_x},
    {"set_style_translate_y",   l_obj_set_style_translate_y},
    {"set_style_text_color",    l_obj_set_style_text_color},
    {"set_style_text_font",     l_obj_set_style_text_font},
    {"set_style_img_opa",       l_obj_set_style_img_opa},
    {"set_style_pad_bottom",    l_obj_set_style_pad_bottom},
    {"set_style_pad_left",      l_obj_set_style_pad_left},
    {"set_style_pad_top",       l_obj_set_style_pad_top},
    {"set_style_radius",        l_obj_set_style_radius},
    {"set_style_text_align",    l_obj_set_style_text_align},
    {"center",                  l_obj_center},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.btn                                                              */
/* ================================================================== */

static int l_btn_new(lua_State *L) {
    return obj_new_guarded(L, &lv_btn_class, lv_btn_create);
}

static const luaL_Reg btn_funcs[] = {
    {"new", l_btn_new},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.arc                                                              */
/* ================================================================== */

static int l_arc_new(lua_State *L) {
    return obj_new_guarded(L, &lv_arc_class, lv_arc_create);
}

static int l_arc_set_angles(lua_State *L) {
    lv_arc_set_angles(lua_lv_check_obj_class(L, 1, &lv_arc_class, "arc"),
                      (uint16_t)luaL_checkinteger(L, 2),
                      (uint16_t)luaL_checkinteger(L, 3));
    return 0;
}

static int l_arc_set_rotation(lua_State *L) {
    lv_arc_set_rotation(lua_lv_check_obj_class(L, 1, &lv_arc_class, "arc"),
                        (uint16_t)luaL_checkinteger(L, 2));
    return 0;
}

static const luaL_Reg arc_funcs[] = {
    {"new",          l_arc_new},
    {"set_angles",   l_arc_set_angles},
    {"set_rotation", l_arc_set_rotation},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.label                                                            */
/* ================================================================== */

static int l_label_new(lua_State *L) {
    return obj_new_guarded(L, &lv_label_class, lv_label_create);
}

static int l_label_set_text(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj_class(L, 1, &lv_label_class, "label");
    size_t len;
    const char *txt = luaL_checklstring(L, 2, &len);
    /* lv_label_set_text : realloc du texte (LV_ASSERT_MALLOC) */
    lua_lv_mem_check(L, len + 1, 0);
    lv_label_set_text(obj, txt);
    return 0;
}

static int l_label_set_long_mode(lua_State *L) {
    lv_label_set_long_mode(lua_lv_check_obj_class(L, 1, &lv_label_class, "label"),
                           (lv_label_long_mode_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_label_get_text(lua_State *L) {
    lua_pushstring(L, lv_label_get_text(lua_lv_check_obj_class(L, 1, &lv_label_class, "label")));
    return 1;
}

static const luaL_Reg label_funcs[] = {
    {"new",           l_label_new},
    {"set_text",      l_label_set_text},
    {"get_text",      l_label_get_text},
    {"set_long_mode", l_label_set_long_mode},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.img                                                              */
/* ================================================================== */

static int l_img_new(lua_State *L) {
    return obj_new_guarded(L, &lv_img_class, lv_img_create);
}

static int l_img_set_src(lua_State *L) {
    lv_obj_t *img = lua_lv_check_obj_class(L, 1, &lv_img_class, "img");
    /* La source peut être un LvImgDsc userdata ou nil */
    if (lua_isuserdata(L, 2)) {
        lv_img_dsc_t **dsc = (lv_img_dsc_t **)luaL_checkudata(L, 2, LV_MT_IMGDSC);
        if (*dsc) {
            lv_img_set_src(img, *dsc);
            /* Ancrer la source : le __gc LvImgDsc libere les pixels (F06) */
            rec_push(L, img, 1);
            lua_pushvalue(L, 2);
            lua_setiuservalue(L, -2, REC_UV_IMG);
            lua_pop(L, 1);
        }
    }
    return 0;
}

static int l_img_set_zoom(lua_State *L) {
    lv_img_set_zoom(lua_lv_check_obj_class(L, 1, &lv_img_class, "img"), (uint16_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_img_set_angle(lua_State *L) {
    lv_img_set_angle(lua_lv_check_obj_class(L, 1, &lv_img_class, "img"), (int16_t)luaL_checkinteger(L, 2));
    return 0;
}

static const luaL_Reg img_funcs[] = {
    {"new",       l_img_new},
    {"set_src",   l_img_set_src},
    {"set_zoom",  l_img_set_zoom},
    {"set_angle", l_img_set_angle},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.slider                                                           */
/* ================================================================== */

static int l_slider_new(lua_State *L) {
    return obj_new_guarded(L, &lv_slider_class, lv_slider_create);
}

static int l_slider_set_range(lua_State *L) {
    lv_slider_set_range(lua_lv_check_obj_class(L, 1, &lv_slider_class, "slider"),
                        (int32_t)luaL_checkinteger(L, 2),
                        (int32_t)luaL_checkinteger(L, 3));
    return 0;
}

static int l_slider_set_value(lua_State *L) {
    lv_slider_set_value(lua_lv_check_obj_class(L, 1, &lv_slider_class, "slider"),
                        (int32_t)luaL_checkinteger(L, 2),
                        (lv_anim_enable_t)luaL_optinteger(L, 3, LV_ANIM_OFF));
    return 0;
}

static int l_slider_get_value(lua_State *L) {
    lua_pushinteger(L, lv_slider_get_value(lua_lv_check_obj_class(L, 1, &lv_slider_class, "slider")));
    return 1;
}

static const luaL_Reg slider_funcs[] = {
    {"new",       l_slider_new},
    {"set_range", l_slider_set_range},
    {"set_value", l_slider_set_value},
    {"get_value", l_slider_get_value},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.img_src (stub — le décodeur LIF sera ajouté à l'étape 3)        */
/* ================================================================== */

/* __gc pour liberer les images decodees */
static int l_imgdsc_gc(lua_State *L) {
    lv_img_dsc_t **dsc = (lv_img_dsc_t **)luaL_checkudata(L, 1, LV_MT_IMGDSC);
    if (*dsc) {
        lif_free(*dsc);
        *dsc = NULL;
    }
    return 0;
}

/* Chemin de base pour les images (configure par le chargeur d'histoire) */
static char g_img_base_path[512] = "";

/* Mode strict (F33), positionne par main.c (--strict) */
static int g_strict = 0;

void lua_lv_set_strict(int strict) { g_strict = strict ? 1 : 0; }
int  lua_lv_get_strict(void) { return g_strict; }

void lua_lv_set_img_base_path(const char *path) {
    if (path) {
        strncpy(g_img_base_path, path, sizeof(g_img_base_path) - 1);
        g_img_base_path[sizeof(g_img_base_path) - 1] = '\0';
    } else {
        g_img_base_path[0] = '\0';
    }
}

static int l_img_src_load(lua_State *L) {
    const char *path = luaL_checkstring(L, 1);

    /* Construire le chemin complet */
    char full_path[1024];
    if (g_img_base_path[0] && path[0] != '/' && path[1] != ':') {
        snprintf(full_path, sizeof(full_path), "%s/%s", g_img_base_path, path);
    } else {
        strncpy(full_path, path, sizeof(full_path) - 1);
        full_path[sizeof(full_path) - 1] = '\0';
    }

    lv_img_dsc_t *dsc = lif_decode_file(full_path);

    if (!dsc) {
        /* F33 : en --strict, un fichier absent est une erreur (pas un nil
           silencieux qui masque un pack incomplet) */
        if (g_strict) {
            FILE *f = fopen(full_path, "rb");
            if (!f) return luaL_error(L, "lv.img_src.load: fichier absent: %s", full_path);
            fclose(f);
        }
        lua_pushnil(L);
        return 1;
    }

    /* Creer un userdata qui pointe vers le descripteur */
    lv_img_dsc_t **ud = (lv_img_dsc_t **)lua_newuserdata(L, sizeof(lv_img_dsc_t *));
    *ud = dsc;

    /* Metatable avec __gc */
    if (luaL_newmetatable(L, LV_MT_IMGDSC)) {
        lua_pushcfunction(L, l_imgdsc_gc);
        lua_setfield(L, -2, "__gc");
    }
    lua_setmetatable(L, -2);

    return 1;
}

static int l_img_src_get_width(lua_State *L) {
    if (lua_isuserdata(L, 1)) {
        lv_img_dsc_t **dsc = (lv_img_dsc_t **)luaL_checkudata(L, 1, LV_MT_IMGDSC);
        if (*dsc) { lua_pushinteger(L, (*dsc)->header.w); return 1; }
    }
    lua_pushinteger(L, 0);
    return 1;
}

static int l_img_src_get_height(lua_State *L) {
    if (lua_isuserdata(L, 1)) {
        lv_img_dsc_t **dsc = (lv_img_dsc_t **)luaL_checkudata(L, 1, LV_MT_IMGDSC);
        if (*dsc) { lua_pushinteger(L, (*dsc)->header.h); return 1; }
    }
    lua_pushinteger(L, 0);
    return 1;
}

static const luaL_Reg img_src_funcs[] = {
    {"load",       l_img_src_load},
    {"get_width",  l_img_src_get_width},
    {"get_height", l_img_src_get_height},
    {NULL, NULL}
};

/* ================================================================== */
/* Registration                                                        */
/* ================================================================== */

/* Sous-table dont les fonctions (hors skip et get_*) sont precedees de la
   garde du tas LVGL (X1a) */
static void set_subtable(lua_State *L, int lv_idx, const char *name,
                         const luaL_Reg *funcs, const char *const *skip) {
    lua_newtable(L);
    if (skip) lua_lv_setfuncs_guarded(L, funcs, skip);
    else luaL_setfuncs(L, funcs, 0);
    lua_setfield(L, lv_idx, name);
}

void lua_lv_register_obj(lua_State *L, int lv_idx) {
    /* Enregistrements de duree de vie + sentinelle du cimetiere */
    luaL_newmetatable(L, LV_MT_OBJREC);
    lua_pushcfunction(L, rec_gc);
    lua_setfield(L, -2, "__gc");
    lua_pop(L, 1);
    lua_newuserdatauv(L, 1, 0);
    lua_newtable(L);
    lua_pushcfunction(L, grave_guard_gc);
    lua_setfield(L, -2, "__gc");
    lua_setmetatable(L, -2);
    lua_setfield(L, LUA_REGISTRYINDEX, LV_OBJ_GUARD_KEY);

    static const char *const widget_unguarded[] = { "new", "set_text", NULL };
    set_subtable(L, lv_idx, "obj",     obj_funcs,    obj_unguarded);
    set_subtable(L, lv_idx, "btn",     btn_funcs,    widget_unguarded);
    set_subtable(L, lv_idx, "arc",     arc_funcs,    widget_unguarded);
    set_subtable(L, lv_idx, "label",   label_funcs,  widget_unguarded);
    set_subtable(L, lv_idx, "img",     img_funcs,    widget_unguarded);
    set_subtable(L, lv_idx, "slider",  slider_funcs, widget_unguarded);
    set_subtable(L, lv_idx, "img_src", img_src_funcs, NULL);   /* malloc systeme */
}
