/**
 * lua_lv_obj.c — Bindings pour lv.obj, lv.btn, lv.label, lv.img,
 *                lv.slider, lv.img_src
 */

#include "lua_lv.h"
#include "formats/lif_decoder.h"
#include <stdlib.h>
#include <string.h>

/* ================================================================== */
/* lv.obj                                                              */
/* ================================================================== */

static int l_obj_new(lua_State *L) {
    lv_obj_t *parent = lua_lv_opt_obj(L, 1);
    lua_lv_push_obj(L, lv_obj_create(parent));
    return 1;
}

static int l_obj_del(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    if (obj) lv_obj_del(obj);
    return 0;
}

static int l_obj_clean(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    if (obj) lv_obj_clean(obj);
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
    return 0;
}

static int l_obj_remove_style(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lv_style_t *style = lua_lv_check_style(L, 2);
    lv_style_selector_t sel = (lv_style_selector_t)luaL_optinteger(L, 3, 0);
    lv_obj_remove_style(obj, style, sel);
    return 0;
}

static int l_obj_remove_style_all(lua_State *L) {
    lv_obj_remove_style_all(lua_lv_check_obj(L, 1));
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
    const lv_font_t *font = (const lv_font_t *)lua_touserdata(L, 2);
    lv_style_selector_t sel = (lv_style_selector_t)luaL_optinteger(L, 3, 0);
    if (font) lv_obj_set_style_text_font(obj, font, sel);
    return 0;
}

/* --- Event callbacks --- */

static void event_cb_wrapper(lv_event_t *e) {
    lua_lv_cb_data_t *cbd = (lua_lv_cb_data_t *)lv_event_get_user_data(e);
    if (!cbd) return;
    lua_State *L = cbd->L;

    lua_rawgeti(L, LUA_REGISTRYINDEX, cbd->func_ref);

    /* Pousser l'événement comme light userdata (valide pendant le callback) */
    lua_pushlightuserdata(L, (void *)e);

    if (lua_pcall(L, 1, 0, 0) != LUA_OK) {
        const char *err = lua_tostring(L, -1);
        fprintf(stderr, "[EVENT] callback error (code=%d): %s\n",
                lv_event_get_code(e), err ? err : "?");
        lua_pop(L, 1);
    }
}

static int l_obj_add_event_cb(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    luaL_checktype(L, 2, LUA_TFUNCTION);
    lv_event_code_t code = (lv_event_code_t)luaL_checkinteger(L, 3);

    lua_lv_cb_data_t *cbd = (lua_lv_cb_data_t *)malloc(sizeof(lua_lv_cb_data_t));
    cbd->L = L;

    /* Stocker la fonction dans le registry */
    lua_pushvalue(L, 2);
    cbd->func_ref = luaL_ref(L, LUA_REGISTRYINDEX);

    lv_obj_add_event_cb(obj, event_cb_wrapper, code, cbd);

    /* Return cbd as light userdata — used by remove_event_cb */
    lua_pushlightuserdata(L, cbd);
    return 1;
}

static int l_obj_remove_event_cb(lua_State *L) {
    lv_obj_t *obj = lua_lv_check_obj(L, 1);
    lua_lv_cb_data_t *cbd = (lua_lv_cb_data_t *)lua_touserdata(L, 2);
    if (!obj || !cbd) return 0;

    /* Remove the LVGL event using the user_data pointer to identify it */
    bool removed = lv_obj_remove_event_cb_with_user_data(obj, event_cb_wrapper, cbd);
    if (removed) {
        luaL_unref(L, LUA_REGISTRYINDEX, cbd->func_ref);
        free(cbd);
    }
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
    lua_lv_push_obj(L, lv_btn_create(lua_lv_opt_obj(L, 1)));
    return 1;
}

static const luaL_Reg btn_funcs[] = {
    {"new", l_btn_new},
    {NULL, NULL}
};

/* ================================================================== */
/* lv.arc                                                              */
/* ================================================================== */

static int l_arc_new(lua_State *L) {
    lua_lv_push_obj(L, lv_arc_create(lua_lv_opt_obj(L, 1)));
    return 1;
}

static int l_arc_set_angles(lua_State *L) {
    lv_arc_set_angles(lua_lv_check_obj(L, 1),
                      (uint16_t)luaL_checkinteger(L, 2),
                      (uint16_t)luaL_checkinteger(L, 3));
    return 0;
}

static int l_arc_set_rotation(lua_State *L) {
    lv_arc_set_rotation(lua_lv_check_obj(L, 1),
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
    lua_lv_push_obj(L, lv_label_create(lua_lv_opt_obj(L, 1)));
    return 1;
}

static int l_label_set_text(lua_State *L) {
    lv_label_set_text(lua_lv_check_obj(L, 1), luaL_checkstring(L, 2));
    return 0;
}

static int l_label_set_long_mode(lua_State *L) {
    lv_label_set_long_mode(lua_lv_check_obj(L, 1),
                           (lv_label_long_mode_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_label_get_text(lua_State *L) {
    lua_pushstring(L, lv_label_get_text(lua_lv_check_obj(L, 1)));
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
    lua_lv_push_obj(L, lv_img_create(lua_lv_opt_obj(L, 1)));
    return 1;
}

static int l_img_set_src(lua_State *L) {
    lv_obj_t *img = lua_lv_check_obj(L, 1);
    /* La source peut être un LvImgDsc userdata ou nil */
    if (lua_isuserdata(L, 2)) {
        lv_img_dsc_t **dsc = (lv_img_dsc_t **)luaL_checkudata(L, 2, LV_MT_IMGDSC);
        if (*dsc) lv_img_set_src(img, *dsc);
    }
    return 0;
}

static int l_img_set_zoom(lua_State *L) {
    lv_img_set_zoom(lua_lv_check_obj(L, 1), (uint16_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_img_set_angle(lua_State *L) {
    lv_img_set_angle(lua_lv_check_obj(L, 1), (int16_t)luaL_checkinteger(L, 2));
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
    lua_lv_push_obj(L, lv_slider_create(lua_lv_opt_obj(L, 1)));
    return 1;
}

static int l_slider_set_range(lua_State *L) {
    lv_slider_set_range(lua_lv_check_obj(L, 1),
                        (int32_t)luaL_checkinteger(L, 2),
                        (int32_t)luaL_checkinteger(L, 3));
    return 0;
}

static int l_slider_set_value(lua_State *L) {
    lv_slider_set_value(lua_lv_check_obj(L, 1),
                        (int32_t)luaL_checkinteger(L, 2),
                        (lv_anim_enable_t)luaL_optinteger(L, 3, LV_ANIM_OFF));
    return 0;
}

static int l_slider_get_value(lua_State *L) {
    lua_pushinteger(L, lv_slider_get_value(lua_lv_check_obj(L, 1)));
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

static void set_subtable(lua_State *L, int lv_idx, const char *name,
                         const luaL_Reg *funcs) {
    lua_newtable(L);
    luaL_setfuncs(L, funcs, 0);
    lua_setfield(L, lv_idx, name);
}

void lua_lv_register_obj(lua_State *L, int lv_idx) {
    set_subtable(L, lv_idx, "obj",     obj_funcs);
    set_subtable(L, lv_idx, "btn",     btn_funcs);
    set_subtable(L, lv_idx, "arc",     arc_funcs);
    set_subtable(L, lv_idx, "label",   label_funcs);
    set_subtable(L, lv_idx, "img",     img_funcs);
    set_subtable(L, lv_idx, "slider",  slider_funcs);
    set_subtable(L, lv_idx, "img_src", img_src_funcs);
}
