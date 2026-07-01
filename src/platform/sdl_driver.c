/**
 * sdl_driver.c — Driver SDL2 pour LVGL
 *
 * Crée une fenêtre SDL2 et connecte :
 *   - un display driver (LVGL rend dans un buffer, on copie vers une texture SDL)
 *   - un input driver (clavier SDL → touches LVGL pour la navigation encodeur)
 *
 * Mapping clavier :
 *   Flèche gauche  → LV_KEY_LEFT   (bouton gauche Flam)
 *   Flèche droite  → LV_KEY_RIGHT  (bouton droite Flam)
 *   Entrée/Espace  → LV_KEY_ENTER  (bouton centre / clic)
 *   Échap          → LV_KEY_ESC    (retour)
 *   M              → context menu  (bouton latéral)
 */

#include "sdl_driver.h"
#include "firmware/fw_globals.h"

#include <stdio.h>
#include <string.h>

/* État global SDL */
static SDL_Window   *g_window   = NULL;
static SDL_Renderer *g_renderer = NULL;
static SDL_Texture  *g_texture  = NULL;

/* Framebuffer LVGL (un seul buffer plein écran) */
static lv_color_t g_fb[FLAM_SCREEN_W * FLAM_SCREEN_H];

/* Display & input driver LVGL */
static lv_disp_draw_buf_t g_draw_buf;
static lv_disp_drv_t      g_disp_drv;
static lv_indev_drv_t     g_indev_drv;
static lv_disp_t         *g_disp    = NULL;
lv_indev_t               *g_indev   = NULL;  /* non-static : accede depuis main.c */

/* Touche LVGL en attente (file d'une seule touche) */
static uint32_t g_last_key   = 0;
static lv_indev_state_t g_key_state = LV_INDEV_STATE_RELEASED;

/* Screenshot auto */
static int g_screenshot_counter = 0;
static Uint32 g_start_ticks = 0;
static int g_auto_screenshot_done = 0;
#define SCREENSHOT_PATH "C:/temp/flam-player/screenshot.bmp"

static void dump_obj_tree(lv_obj_t *obj, int depth) {
    for (int i = 0; i < depth; i++) fprintf(stderr, "  ");
    lv_coord_t x = lv_obj_get_x(obj);
    lv_coord_t y = lv_obj_get_y(obj);
    lv_coord_t w = lv_obj_get_width(obj);
    lv_coord_t h = lv_obj_get_height(obj);
    uint32_t cnt = lv_obj_get_child_cnt(obj);
    fprintf(stderr, "obj@%p x=%d y=%d w=%d h=%d children=%u\n",
            (void*)obj, x, y, w, h, cnt);
    for (uint32_t i = 0; i < cnt && i < 20; i++) {
        dump_obj_tree(lv_obj_get_child(obj, (int32_t)i), depth + 1);
    }
}

static void save_screenshot(void) {
    /* Dump de l'arbre d'objets */
    lv_obj_t *scr = lv_scr_act();
    fprintf(stderr, "\n=== LVGL Object Tree ===\n");
    dump_obj_tree(scr, 0);
    fprintf(stderr, "========================\n\n");

    SDL_Surface *surf = SDL_CreateRGBSurfaceFrom(
        g_fb, FLAM_SCREEN_W, FLAM_SCREEN_H,
        32, FLAM_SCREEN_W * 4,
        0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000);
    if (surf) {
        SDL_SaveBMP(surf, SCREENSHOT_PATH);
        SDL_FreeSurface(surf);
        fprintf(stderr, "[SCREENSHOT] Saved to %s\n", SCREENSHOT_PATH);
    }
}

/* ------------------------------------------------------------------ */
/* Callbacks LVGL                                                      */
/* ------------------------------------------------------------------ */

/**
 * Flush callback : LVGL nous donne une zone mise à jour du framebuffer.
 * On copie les pixels dans la texture SDL.
 */
static void disp_flush_cb(lv_disp_drv_t *drv, const lv_area_t *area,
                           lv_color_t *color_p)
{
    (void)drv;

    int32_t w = area->x2 - area->x1 + 1;
    int32_t h = area->y2 - area->y1 + 1;

    SDL_Rect rect = {area->x1, area->y1, w, h};
    SDL_UpdateTexture(g_texture, &rect, color_p, w * sizeof(lv_color_t));

    lv_disp_flush_ready(drv);
}

/**
 * Input read callback : LVGL interroge l'état du clavier.
 */
static void indev_read_cb(lv_indev_drv_t *drv, lv_indev_data_t *data)
{
    (void)drv;
    data->key   = g_last_key;
    data->state = g_key_state;
}

/* ------------------------------------------------------------------ */
/* API publique                                                        */
/* ------------------------------------------------------------------ */

int sdl_driver_init(void)
{
    /* SDL */
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_EVENTS) != 0) {
        SDL_Log("SDL_Init failed: %s", SDL_GetError());
        return -1;
    }

    g_window = SDL_CreateWindow(
        "Flam Player",
        SDL_WINDOWPOS_CENTERED, SDL_WINDOWPOS_CENTERED,
        FLAM_SCREEN_W * FLAM_SCALE, FLAM_SCREEN_H * FLAM_SCALE,
        SDL_WINDOW_SHOWN | SDL_WINDOW_RESIZABLE
    );
    if (!g_window) {
        SDL_Log("SDL_CreateWindow failed: %s", SDL_GetError());
        return -1;
    }

    g_renderer = SDL_CreateRenderer(g_window, -1,
        SDL_RENDERER_ACCELERATED | SDL_RENDERER_PRESENTVSYNC);
    if (!g_renderer) {
        SDL_Log("SDL_CreateRenderer failed: %s", SDL_GetError());
        return -1;
    }

    /* Texture : résolution native 320x240, upscalée par le renderer */
    g_texture = SDL_CreateTexture(g_renderer,
        SDL_PIXELFORMAT_ARGB8888,
        SDL_TEXTUREACCESS_STREAMING,
        FLAM_SCREEN_W, FLAM_SCREEN_H);
    if (!g_texture) {
        SDL_Log("SDL_CreateTexture failed: %s", SDL_GetError());
        return -1;
    }

    /* Nearest-neighbor scaling pour garder le rendu pixel-perfect */
    SDL_SetHint(SDL_HINT_RENDER_SCALE_QUALITY, "0");

    /* LVGL : init */
    lv_init();

    /* Draw buffer plein écran (un seul buffer, pas de double-buffering) */
    lv_disp_draw_buf_init(&g_draw_buf, g_fb, NULL,
                          FLAM_SCREEN_W * FLAM_SCREEN_H);

    /* Display driver */
    lv_disp_drv_init(&g_disp_drv);
    g_disp_drv.hor_res  = FLAM_SCREEN_W;
    g_disp_drv.ver_res  = FLAM_SCREEN_H;
    g_disp_drv.draw_buf = &g_draw_buf;
    g_disp_drv.flush_cb = disp_flush_cb;
    g_disp = lv_disp_drv_register(&g_disp_drv);

    /* Input driver (encodeur — LEFT/RIGHT navigates in non-edit mode,
       sends KEY_LEFT/KEY_RIGHT in edit mode, matching Flam behavior) */
    lv_indev_drv_init(&g_indev_drv);
    g_indev_drv.type    = LV_INDEV_TYPE_ENCODER;
    g_indev_drv.read_cb = indev_read_cb;
    g_indev = lv_indev_drv_register(&g_indev_drv);

    /* Le focus group sera cree par main.c et associe via g_indev */

    return 0;
}

void sdl_driver_set_title(const char *title)
{
    if (g_window) SDL_SetWindowTitle(g_window, title);
}

void sdl_driver_quit(void)
{
    if (g_texture)  SDL_DestroyTexture(g_texture);
    if (g_renderer) SDL_DestroyRenderer(g_renderer);
    if (g_window)   SDL_DestroyWindow(g_window);
    SDL_Quit();
}

int sdl_driver_poll(void)
{
    SDL_Event e;
    while (SDL_PollEvent(&e)) {
        switch (e.type) {
        case SDL_QUIT:
            return 1;

        case SDL_WINDOWEVENT:
            if (e.window.event == SDL_WINDOWEVENT_CLOSE)
                return 1;
            break;

        case SDL_KEYDOWN:
            g_key_state = LV_INDEV_STATE_PRESSED;
            switch (e.key.keysym.sym) {
            case SDLK_LEFT:   g_last_key = LV_KEY_LEFT;  break;
            case SDLK_RIGHT:  g_last_key = LV_KEY_RIGHT; break;
            case SDLK_RETURN:
            case SDLK_SPACE:  g_last_key = LV_KEY_ENTER; break;
            case SDLK_ESCAPE: fw_trigger_back(); g_key_state = LV_INDEV_STATE_RELEASED; break;
            case SDLK_m:      fw_trigger_context_menu(); g_key_state = LV_INDEV_STATE_RELEASED; break;
            case SDLK_s:      save_screenshot(); g_key_state = LV_INDEV_STATE_RELEASED; break;
            default:          g_key_state = LV_INDEV_STATE_RELEASED; break;
            }
            break;

        case SDL_KEYUP:
            g_key_state = LV_INDEV_STATE_RELEASED;
            break;
        }
    }

    /* Laisser LVGL traiter les timers et le rendu */
    lv_timer_handler();

    /* Screenshot automatique apres 12 secondes */
    if (!g_auto_screenshot_done) {
        if (!g_start_ticks) g_start_ticks = SDL_GetTicks();
        if (SDL_GetTicks() - g_start_ticks > 12000) {
            save_screenshot();
            g_auto_screenshot_done = 1;
        }
    }

    /* Copier la texture vers la fenêtre en gardant le ratio 4:3 */
    SDL_RenderClear(g_renderer);
    {
        int win_w, win_h;
        SDL_GetRendererOutputSize(g_renderer, &win_w, &win_h);
        float scale_x = (float)win_w / FLAM_SCREEN_W;
        float scale_y = (float)win_h / FLAM_SCREEN_H;
        float scale = scale_x < scale_y ? scale_x : scale_y;
        int dst_w = (int)(FLAM_SCREEN_W * scale);
        int dst_h = (int)(FLAM_SCREEN_H * scale);
        SDL_Rect dst = { (win_w - dst_w) / 2, (win_h - dst_h) / 2, dst_w, dst_h };
        SDL_RenderCopy(g_renderer, g_texture, NULL, &dst);
    }
    SDL_RenderPresent(g_renderer);

    return 0;
}
