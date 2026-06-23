/**
 * test_headless_driver.c — Headless LVGL display driver for testing
 *
 * Initializes LVGL with an in-memory framebuffer and no-op flush.
 * Uses SDL_Init(SDL_INIT_TIMER) only for SDL_GetTicks() needed by lv_conf.h.
 */

#include "test_headless_driver.h"
#include <SDL.h>
#include <stdio.h>
#include <string.h>

static lv_color_t g_fb[FLAM_SCREEN_W * FLAM_SCREEN_H];
static lv_disp_draw_buf_t g_draw_buf;
static lv_disp_drv_t      g_disp_drv;
static lv_indev_drv_t     g_indev_drv;
static lv_disp_t         *g_disp  = NULL;
static lv_indev_t        *g_indev = NULL;

static void flush_noop(lv_disp_drv_t *drv, const lv_area_t *area,
                       lv_color_t *buf)
{
    (void)area;
    (void)buf;
    lv_disp_flush_ready(drv);
}

static void indev_read_noop(lv_indev_drv_t *drv, lv_indev_data_t *data)
{
    (void)drv;
    data->key   = 0;
    data->state = LV_INDEV_STATE_RELEASED;
}

int test_driver_init(void)
{
    if (SDL_Init(SDL_INIT_TIMER) != 0) {
        fprintf(stderr, "SDL_Init(TIMER) failed: %s\n", SDL_GetError());
        return -1;
    }

    lv_init();

    lv_disp_draw_buf_init(&g_draw_buf, g_fb, NULL,
                          FLAM_SCREEN_W * FLAM_SCREEN_H);

    lv_disp_drv_init(&g_disp_drv);
    g_disp_drv.hor_res  = FLAM_SCREEN_W;
    g_disp_drv.ver_res  = FLAM_SCREEN_H;
    g_disp_drv.draw_buf = &g_draw_buf;
    g_disp_drv.flush_cb = flush_noop;
    g_disp = lv_disp_drv_register(&g_disp_drv);

    lv_indev_drv_init(&g_indev_drv);
    g_indev_drv.type    = LV_INDEV_TYPE_KEYPAD;
    g_indev_drv.read_cb = indev_read_noop;
    g_indev = lv_indev_drv_register(&g_indev_drv);

    /* Black background like the real player */
    lv_obj_set_style_bg_color(lv_scr_act(), lv_color_black(), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(lv_scr_act(), LV_OPA_COVER, LV_PART_MAIN);

    return 0;
}

void test_driver_tick(void)
{
    lv_timer_handler();
}

void test_driver_quit(void)
{
    SDL_Quit();
}

lv_indev_t *test_driver_get_indev(void)
{
    return g_indev;
}
