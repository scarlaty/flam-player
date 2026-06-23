/**
 * lua_lv_style.c — Bindings pour lv.style.*
 *
 * lv.style.new()                     → crée un userdata style
 * lv.style.reset(style)              → réinitialise
 * lv.style.set_<prop>(style, value)  → setters de propriétés
 */

#include "lua_lv.h"

/* ================================================================== */
/* Création / reset                                                    */
/* ================================================================== */

static int l_style_new(lua_State *L) {
    lv_style_t *s = (lv_style_t *)lua_newuserdata(L, sizeof(lv_style_t));
    lv_style_init(s);
    luaL_setmetatable(L, LV_MT_STYLE);
    return 1;
}

static int l_style_reset(lua_State *L) {
    lv_style_reset(lua_lv_check_style(L, 1));
    return 0;
}

/* ================================================================== */
/* Background                                                          */
/* ================================================================== */

static int l_style_set_bg_color(lua_State *L) {
    lv_style_set_bg_color(lua_lv_check_style(L, 1), lua_lv_check_color(L, 2));
    return 0;
}

static int l_style_set_bg_opa(lua_State *L) {
    lv_style_set_bg_opa(lua_lv_check_style(L, 1), (lv_opa_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Bordure                                                             */
/* ================================================================== */

static int l_style_set_border_width(lua_State *L) {
    lv_style_set_border_width(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_border_color(lua_State *L) {
    lv_style_set_border_color(lua_lv_check_style(L, 1), lua_lv_check_color(L, 2));
    return 0;
}

static int l_style_set_border_opa(lua_State *L) {
    lv_style_set_border_opa(lua_lv_check_style(L, 1), (lv_opa_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Rayon                                                               */
/* ================================================================== */

static int l_style_set_radius(lua_State *L) {
    lv_style_set_radius(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Padding                                                             */
/* ================================================================== */

static int l_style_set_pad_top(lua_State *L) {
    lv_style_set_pad_top(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_pad_bottom(lua_State *L) {
    lv_style_set_pad_bottom(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_pad_left(lua_State *L) {
    lv_style_set_pad_left(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_pad_right(lua_State *L) {
    lv_style_set_pad_right(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Texte                                                               */
/* ================================================================== */

static int l_style_set_text_color(lua_State *L) {
    lv_style_set_text_color(lua_lv_check_style(L, 1), lua_lv_check_color(L, 2));
    return 0;
}

static int l_style_set_text_opa(lua_State *L) {
    lv_style_set_text_opa(lua_lv_check_style(L, 1), (lv_opa_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_text_font(lua_State *L) {
    lv_style_t *s = lua_lv_check_style(L, 1);
    const lv_font_t *font = (const lv_font_t *)lua_touserdata(L, 2);
    if (font) lv_style_set_text_font(s, font);
    return 0;
}

static int l_style_set_text_align(lua_State *L) {
    lv_style_set_text_align(lua_lv_check_style(L, 1),
                            (lv_text_align_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_text_line_space(lua_State *L) {
    lv_style_set_text_line_space(lua_lv_check_style(L, 1),
                                (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Dimensions                                                          */
/* ================================================================== */

static int l_style_set_width(lua_State *L) {
    lv_style_set_width(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_height(lua_State *L) {
    lv_style_set_height(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Padding row/column                                                  */
/* ================================================================== */

static int l_style_set_pad_row(lua_State *L) {
    lv_style_set_pad_row(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_pad_column(lua_State *L) {
    lv_style_set_pad_column(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Image                                                               */
/* ================================================================== */

static int l_style_set_img_opa(lua_State *L) {
    lv_style_set_img_opa(lua_lv_check_style(L, 1), (lv_opa_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Padding convenience                                                 */
/* ================================================================== */

static int l_style_set_pad_all(lua_State *L) {
    lv_style_t *s = lua_lv_check_style(L, 1);
    lv_coord_t v = (lv_coord_t)luaL_checkinteger(L, 2);
    lv_style_set_pad_top(s, v);
    lv_style_set_pad_bottom(s, v);
    lv_style_set_pad_left(s, v);
    lv_style_set_pad_right(s, v);
    return 0;
}

static int l_style_set_pad_hor(lua_State *L) {
    lv_style_t *s = lua_lv_check_style(L, 1);
    lv_coord_t v = (lv_coord_t)luaL_checkinteger(L, 2);
    lv_style_set_pad_left(s, v);
    lv_style_set_pad_right(s, v);
    return 0;
}

static int l_style_set_pad_ver(lua_State *L) {
    lv_style_t *s = lua_lv_check_style(L, 1);
    lv_coord_t v = (lv_coord_t)luaL_checkinteger(L, 2);
    lv_style_set_pad_top(s, v);
    lv_style_set_pad_bottom(s, v);
    return 0;
}

/* ================================================================== */
/* Arc                                                                 */
/* ================================================================== */

static int l_style_set_arc_color(lua_State *L) {
    lv_style_set_arc_color(lua_lv_check_style(L, 1), lua_lv_check_color(L, 2));
    return 0;
}

static int l_style_set_arc_width(lua_State *L) {
    lv_style_set_arc_width(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Background gradient                                                 */
/* ================================================================== */

static int l_style_set_bg_grad_color(lua_State *L) {
    lv_style_set_bg_grad_color(lua_lv_check_style(L, 1), lua_lv_check_color(L, 2));
    return 0;
}

static int l_style_set_bg_grad_dir(lua_State *L) {
    lv_style_set_bg_grad_dir(lua_lv_check_style(L, 1), (lv_grad_dir_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_bg_grad_stop(lua_State *L) {
    lv_style_set_bg_grad_stop(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_bg_main_stop(lua_State *L) {
    lv_style_set_bg_main_stop(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_bg_dither_mode(lua_State *L) {
    lv_style_set_bg_dither_mode(lua_lv_check_style(L, 1), (lv_dither_mode_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Border side                                                         */
/* ================================================================== */

static int l_style_set_border_side(lua_State *L) {
    lv_style_set_border_side(lua_lv_check_style(L, 1), (lv_border_side_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Image recolor                                                       */
/* ================================================================== */

static int l_style_set_img_recolor(lua_State *L) {
    lv_style_set_img_recolor(lua_lv_check_style(L, 1), lua_lv_check_color(L, 2));
    return 0;
}

static int l_style_set_img_recolor_opa(lua_State *L) {
    lv_style_set_img_recolor_opa(lua_lv_check_style(L, 1), (lv_opa_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Translation                                                         */
/* ================================================================== */

static int l_style_set_translate_x(lua_State *L) {
    lv_style_set_translate_x(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

static int l_style_set_translate_y(lua_State *L) {
    lv_style_set_translate_y(lua_lv_check_style(L, 1), (lv_coord_t)luaL_checkinteger(L, 2));
    return 0;
}

/* ================================================================== */
/* Table lv.style                                                      */
/* ================================================================== */

static const luaL_Reg style_funcs[] = {
    {"new",                l_style_new},
    {"reset",              l_style_reset},
    /* Background */
    {"set_bg_color",       l_style_set_bg_color},
    {"set_bg_opa",         l_style_set_bg_opa},
    /* Border */
    {"set_border_width",   l_style_set_border_width},
    {"set_border_color",   l_style_set_border_color},
    {"set_border_opa",     l_style_set_border_opa},
    /* Radius */
    {"set_radius",         l_style_set_radius},
    /* Background gradient */
    {"set_bg_grad_color",  l_style_set_bg_grad_color},
    {"set_bg_grad_dir",    l_style_set_bg_grad_dir},
    {"set_bg_grad_stop",   l_style_set_bg_grad_stop},
    {"set_bg_main_stop",   l_style_set_bg_main_stop},
    {"set_bg_dither_mode", l_style_set_bg_dither_mode},
    /* Border */
    {"set_border_side",    l_style_set_border_side},
    /* Padding */
    {"set_pad_all",        l_style_set_pad_all},
    {"set_pad_hor",        l_style_set_pad_hor},
    {"set_pad_ver",        l_style_set_pad_ver},
    {"set_pad_top",        l_style_set_pad_top},
    {"set_pad_bottom",     l_style_set_pad_bottom},
    {"set_pad_left",       l_style_set_pad_left},
    {"set_pad_right",      l_style_set_pad_right},
    /* Texte */
    {"set_text_color",      l_style_set_text_color},
    {"set_text_opa",        l_style_set_text_opa},
    {"set_text_font",       l_style_set_text_font},
    {"set_text_align",      l_style_set_text_align},
    {"set_text_line_space",  l_style_set_text_line_space},
    /* Dimensions */
    {"set_width",          l_style_set_width},
    {"set_height",         l_style_set_height},
    /* Padding row/column */
    {"set_pad_row",        l_style_set_pad_row},
    {"set_pad_column",     l_style_set_pad_column},
    /* Translation */
    {"set_translate_x",    l_style_set_translate_x},
    {"set_translate_y",    l_style_set_translate_y},
    /* Arc */
    {"set_arc_color",      l_style_set_arc_color},
    {"set_arc_width",      l_style_set_arc_width},
    /* Image */
    {"set_img_opa",        l_style_set_img_opa},
    {"set_img_recolor",    l_style_set_img_recolor},
    {"set_img_recolor_opa", l_style_set_img_recolor_opa},
    {NULL, NULL}
};

/* ================================================================== */
/* Registration                                                        */
/* ================================================================== */

void lua_lv_register_style(lua_State *L, int lv_idx) {
    lua_newtable(L);
    luaL_setfuncs(L, style_funcs, 0);
    lua_setfield(L, lv_idx, "style");
}
