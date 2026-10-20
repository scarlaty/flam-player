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
 *   P              → pause / reprise du son courant (sdl_audio_toggle_pause)
 *   S              → capture d'ecran BMP 320x240
 *
 * Capture d'ecran : chemin par defaut SCREENSHOT_PATH, remplace par
 * FLAM_SCREENSHOT=<chemin> ou l'option --screenshot <chemin> (prioritaire).
 * Une capture automatique est faite 12 s apres le premier poll (outil de
 * debug historique) ; FLAM_SCREENSHOT_AUTO_MS=<ms> change ce delai et
 * FLAM_SCREENSHOT_AUTO_MS=0 la desactive.
 *
 * Incrustation (Windows, option --parent-hwnd <hwnd>) : la fenetre SDL devient
 * une fenetre enfant (WS_CHILD) du HWND donne par l'application hote, remplit
 * sa zone cliente et en suit la taille a chaque poll ; la disparition du
 * parent arrete le player (comme une fermeture). Le focus clavier est pris a
 * l'incrustation et a chaque clic. En cas d'echec : trace "[EMBED] repli : ..."
 * et fenetre separee habituelle. Hors Windows, l'option est ignoree.
 */

#include "sdl_driver.h"
#include "sdl_audio.h"
#include "firmware/fw_globals.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include "SDL_syswm.h"
#include <windows.h>
#endif

/* État global SDL */
static SDL_Window   *g_window   = NULL;
static SDL_Renderer *g_renderer = NULL;
static SDL_Texture  *g_texture  = NULL;

/* Incrustation : HWND parent demande (0 : fenetre separee) */
static uintptr_t g_parent_req = 0;
#ifdef _WIN32
static HWND g_embed_parent = NULL;   /* parent effectif (NULL : non incruste) */
static HWND g_embed_child  = NULL;   /* HWND de la fenetre SDL incrustee */
static int  g_embed_w = -1, g_embed_h = -1;
#endif

/* Framebuffer LVGL (un seul buffer plein écran) */
static lv_color_t g_fb[FLAM_SCREEN_W * FLAM_SCREEN_H];

/* Framebuffer d'ombre : image complete de l'ecran, mise a jour a chaque
   flush. g_fb ne l'est pas : sans full_refresh, LVGL y rend chaque zone
   invalidee de facon compacte depuis le debut du buffer (largeur de la
   zone), donc g_fb ne ressemble a l'ecran qu'apres un rendu plein ecran. */
static lv_color_t g_shadow_fb[FLAM_SCREEN_W * FLAM_SCREEN_H];

/* Display & input driver LVGL */
static lv_disp_draw_buf_t g_draw_buf;
static lv_disp_drv_t      g_disp_drv;
static lv_indev_drv_t     g_indev_drv;
static lv_disp_t         *g_disp    = NULL;
lv_indev_t               *g_indev   = NULL;  /* non-static : accede depuis main.c */

/* File circulaire des evenements clavier (touche, etat) : un appui et un
   relachement recus dans le meme poll ne sont plus fusionnes. LVGL en
   consomme un par lecture (continue_reading tant qu'il en reste). */
typedef struct {
    uint32_t         key;
    lv_indev_state_t state;
} key_evt_t;

#define KEY_QUEUE_LEN 32
static key_evt_t g_key_queue[KEY_QUEUE_LEN];
static int       g_key_head  = 0;   /* prochain a lire */
static int       g_key_count = 0;

/* Dernier etat transmis a LVGL (renvoye tant que la file est vide) */
static uint32_t g_last_key   = 0;
static lv_indev_state_t g_key_state = LV_INDEV_STATE_RELEASED;

static void key_queue_push(uint32_t key, lv_indev_state_t state)
{
    if (g_key_count >= KEY_QUEUE_LEN) {
        /* File pleine : ignorer (ne devrait pas arriver a 5 ms par poll) */
        return;
    }
    int tail = (g_key_head + g_key_count) % KEY_QUEUE_LEN;
    g_key_queue[tail].key   = key;
    g_key_queue[tail].state = state;
    g_key_count++;
}

/* Touche SDL -> touche LVGL (0 si non transmise a LVGL) */
static uint32_t sdl_to_lv_key(SDL_Keycode sym)
{
    switch (sym) {
    case SDLK_LEFT:   return LV_KEY_LEFT;
    case SDLK_RIGHT:  return LV_KEY_RIGHT;
    case SDLK_RETURN:
    case SDLK_SPACE:  return LV_KEY_ENTER;
    default:          return 0;
    }
}

/* Screenshot auto */
static Uint32 g_start_ticks = 0;
static int g_auto_screenshot_done = 0;
#define SCREENSHOT_PATH "C:/temp/flam-player/screenshot.bmp"
#define SCREENSHOT_AUTO_MS_DEFAULT 12000

/* Chemin fixe par --screenshot (NULL : FLAM_SCREENSHOT, sinon defaut) */
static const char *g_screenshot_path = NULL;

void sdl_driver_set_screenshot_path(const char *path)
{
    g_screenshot_path = (path && *path) ? path : NULL;
}

static const char *screenshot_path(void)
{
    const char *p = g_screenshot_path;
    if (!p) p = getenv("FLAM_SCREENSHOT");
    return (p && *p) ? p : SCREENSHOT_PATH;
}

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

    /* Image de l'ecran = framebuffer d'ombre (ARGB8888, LV_COLOR_DEPTH 32) */
    const char *path = screenshot_path();
    SDL_Surface *surf = SDL_CreateRGBSurfaceFrom(
        g_shadow_fb, FLAM_SCREEN_W, FLAM_SCREEN_H,
        32, FLAM_SCREEN_W * (int)sizeof(lv_color_t),
        0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000);
    if (surf) {
        if (SDL_SaveBMP(surf, path) == 0)
            fprintf(stderr, "[SCREENSHOT] Saved to %s\n", path);
        else
            fprintf(stderr, "[SCREENSHOT] Echec %s : %s\n", path, SDL_GetError());
        SDL_FreeSurface(surf);
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

    /* Copie dans le framebuffer d'ombre (zone bornee a l'ecran) */
    for (int32_t y = 0; y < h; y++) {
        int32_t sy = area->y1 + y;
        if (sy < 0 || sy >= FLAM_SCREEN_H) continue;
        int32_t x0 = area->x1, x1 = area->x2, skip = 0;
        if (x0 < 0) { skip = -x0; x0 = 0; }
        if (x1 >= FLAM_SCREEN_W) x1 = FLAM_SCREEN_W - 1;
        if (x1 < x0) continue;
        memcpy(&g_shadow_fb[sy * FLAM_SCREEN_W + x0],
               &color_p[y * w + skip],
               (size_t)(x1 - x0 + 1) * sizeof(lv_color_t));
    }

    lv_disp_flush_ready(drv);
}

/**
 * Input read callback : LVGL interroge l'état du clavier.
 */
static void indev_read_cb(lv_indev_drv_t *drv, lv_indev_data_t *data)
{
    (void)drv;
    if (g_key_count > 0) {
        g_last_key  = g_key_queue[g_key_head].key;
        g_key_state = g_key_queue[g_key_head].state;
        g_key_head  = (g_key_head + 1) % KEY_QUEUE_LEN;
        g_key_count--;
    }
    data->key   = g_last_key;
    data->state = g_key_state;
    data->continue_reading = (g_key_count > 0);
}

/* ------------------------------------------------------------------ */
/* Incrustation dans une fenetre hote (Windows)                        */
/* ------------------------------------------------------------------ */

void sdl_driver_set_parent(uintptr_t hwnd)
{
    g_parent_req = hwnd;
}

#ifdef _WIN32
/* Clic dans la fenetre incrustee : lui donner le clavier. Les files d'entree
   du parent et de l'enfant sont liees par Windows : SetFocus est permis. */
static void SDLCALL embed_msg_hook(void *userdata, void *hWnd,
                                   unsigned int message, Uint64 wParam,
                                   Sint64 lParam)
{
    (void)userdata; (void)wParam; (void)lParam;
    if (!g_embed_child || (HWND)hWnd != g_embed_child) return;
    if (message == WM_LBUTTONDOWN || message == WM_RBUTTONDOWN ||
        message == WM_MBUTTONDOWN) {
        if (GetFocus() != g_embed_child) SetFocus(g_embed_child);
    }
}

/* Procedure de fenetre de SDL, sous-classee par embed_wndproc. */
static WNDPROC g_sdl_wndproc = NULL;

/* SDL calcule la zone cliente d'une fenetre sans bordure non
   redimensionnable a partir de sa taille de creation (WM_NCCALCSIZE) et fige
   sa taille (WM_GETMINMAXINFO) : la fenetre enfant ne suivrait pas le parent.
   Ces deux messages vont donc au traitement par defaut de Windows (sans
   bordure ni legende : zone cliente = fenetre entiere, aucune limite). */
static LRESULT CALLBACK embed_wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    if (msg == WM_NCCALCSIZE || msg == WM_GETMINMAXINFO)
        return DefWindowProcW(hwnd, msg, wp, lp);
    return CallWindowProcW(g_sdl_wndproc, hwnd, msg, wp, lp);
}

/* Ajuste la fenetre enfant a la zone cliente du parent.
   Retourne 1 si le parent a disparu, 0 sinon. */
static int embed_follow_parent(void)
{
    if (!g_embed_child) return 0;
    if (!IsWindow(g_embed_parent)) return 1;
    RECT rc;
    if (!GetClientRect(g_embed_parent, &rc)) return 0;
    int w = rc.right - rc.left, h = rc.bottom - rc.top;
    if (w < 1) w = 1;
    if (h < 1) h = 1;
    /* Comparer a la taille reelle de l'enfant (pas seulement a la derniere
       demande) : SDL peut lui rendre sa taille d'origine au premier affichage */
    RECT cur;
    int cw = -1, ch = -1;
    if (GetClientRect(g_embed_child, &cur)) {
        cw = cur.right - cur.left;
        ch = cur.bottom - cur.top;
    }
    if (w != g_embed_w || h != g_embed_h || w != cw || h != ch) {
        SetWindowPos(g_embed_child, NULL, 0, 0, w, h,
                     SWP_NOZORDER | SWP_NOACTIVATE);
        g_embed_w = w;
        g_embed_h = h;
    }
    return 0;
}

/* Cree la fenetre SDL et l'incruste dans le parent demande.
   NULL en cas d'echec (trace "[EMBED] repli : ...") : l'appelant ouvre alors
   la fenetre separee habituelle. */
static SDL_Window *embed_create_window(void)
{
    HWND parent = (HWND)g_parent_req;
    if (!IsWindow(parent)) {
        fprintf(stderr, "[EMBED] repli : HWND parent invalide (%llu)\n",
                (unsigned long long)g_parent_req);
        return NULL;
    }
    SDL_Window *w = SDL_CreateWindow("Flam Player", 0, 0,
        FLAM_SCREEN_W, FLAM_SCREEN_H,
        SDL_WINDOW_HIDDEN | SDL_WINDOW_BORDERLESS);
    if (!w) {
        fprintf(stderr, "[EMBED] repli : SDL_CreateWindow : %s\n", SDL_GetError());
        return NULL;
    }
    SDL_SysWMinfo info;
    SDL_VERSION(&info.version);
    if (!SDL_GetWindowWMInfo(w, &info) || info.subsystem != SDL_SYSWM_WINDOWS ||
        !info.info.win.window) {
        const char *drv = SDL_GetCurrentVideoDriver();
        fprintf(stderr, "[EMBED] repli : pas de HWND (pilote video %s)\n",
                drv ? drv : "?");
        SDL_DestroyWindow(w);
        return NULL;
    }
    HWND child = info.info.win.window;

    /* Sous-classement avant le changement de style (voir embed_wndproc) */
    g_sdl_wndproc = (WNDPROC)GetWindowLongPtrW(child, GWLP_WNDPROC);
    SetWindowLongPtrW(child, GWLP_WNDPROC, (LONG_PTR)embed_wndproc);

    LONG_PTR style = GetWindowLongPtrW(child, GWL_STYLE);
    style &= ~(LONG_PTR)(WS_POPUP | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU |
                         WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
    style |= WS_CHILD | WS_CLIPSIBLINGS;
    SetWindowLongPtrW(child, GWL_STYLE, style);
    LONG_PTR ex = GetWindowLongPtrW(child, GWL_EXSTYLE);
    ex &= ~(LONG_PTR)WS_EX_APPWINDOW;
    SetWindowLongPtrW(child, GWL_EXSTYLE, ex);

    if (!SetParent(child, parent)) {
        fprintf(stderr, "[EMBED] repli : SetParent refuse (erreur %lu)\n",
                (unsigned long)GetLastError());
        SetWindowLongPtrW(child, GWLP_WNDPROC, (LONG_PTR)g_sdl_wndproc);
        SDL_DestroyWindow(w);
        return NULL;
    }
    g_embed_parent = parent;
    g_embed_child  = child;
    SDL_SetWindowsMessageHook(embed_msg_hook, NULL);
    embed_follow_parent();
    SetWindowPos(child, HWND_TOP, 0, 0, 0, 0,
                 SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED | SWP_NOACTIVATE);
    SDL_ShowWindow(w);
    SetFocus(child);
    fprintf(stderr, "[EMBED] ok : fenetre enfant de %llu (%dx%d)\n",
            (unsigned long long)g_parent_req, g_embed_w, g_embed_h);
    return w;
}

/* F6 en mode incruste : rend le clavier a la fenetre hote (piege clavier : Tab ne
   sort pas d'une fenetre native). Le focus va a la fenetre racine de l'hote ; la trace
   "[EMBED] focus-host" previent l'application hote, qui place le focus sur son propre
   controle (la WebView2 du launcher ne le reprend pas toujours d'elle-meme). */
static void embed_release_focus(void)
{
    if (!g_embed_child || !IsWindow(g_embed_parent)) return;
    HWND root = GetAncestor(g_embed_parent, GA_ROOT);
    SetFocus(root ? root : g_embed_parent);
    fprintf(stderr, "[EMBED] focus-host\n");
    fflush(stderr);
}
#endif /* _WIN32 */

/* ------------------------------------------------------------------ */
/* API publique                                                        */
/* ------------------------------------------------------------------ */

int sdl_driver_init(void)
{
    /* Incrustation : meme sensibilite DPI que l'hote (Tauri : par moniteur v2),
       sinon Windows refuse ou dimensionne mal un enfant d'un autre processus.
       Le mode fenetre separee reste inchange. */
#ifdef _WIN32
    /* Pas de hint si le parent est deja invalide : le repli le plus courant
       garde la fenetre separee habituelle (autres echecs : fenetre separee
       en DPI par moniteur, donc plus petite sur ecran HiDPI) */
    if (g_parent_req && IsWindow((HWND)g_parent_req))
        SDL_SetHint(SDL_HINT_WINDOWS_DPI_AWARENESS, "permonitorv2");
#else
    if (g_parent_req) {
        fprintf(stderr, "[EMBED] repli : --parent-hwnd n'est gere que sous Windows\n");
        g_parent_req = 0;
    }
#endif

    /* SDL */
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_EVENTS) != 0) {
        SDL_Log("SDL_Init failed: %s", SDL_GetError());
        return -1;
    }

    g_window = NULL;
#ifdef _WIN32
    if (g_parent_req) g_window = embed_create_window();
#endif
    if (!g_window) g_window = SDL_CreateWindow(
        "Flam Player",
        SDL_WINDOWPOS_CENTERED, SDL_WINDOWPOS_CENTERED,
        FLAM_SCREEN_W * FLAM_SCALE, FLAM_SCREEN_H * FLAM_SCALE,
        SDL_WINDOW_SHOWN | SDL_WINDOW_RESIZABLE
    );
    if (!g_window) {
        SDL_Log("SDL_CreateWindow failed: %s", SDL_GetError());
        return -1;
    }

#ifdef _WIN32
    /* Incruste : rendu direct3d11 (chaine d'echange DXGI "flip", qui a son
       propre visuel de composition). La WebView2 du launcher dessine par
       DirectComposition, au-dessus de la surface de redirection ou arrivent
       les rendus direct3d (D3D9, defaut de SDL2), opengl et software : la
       fenetre du player y serait recouverte malgre l'ordre Z (#332).
       Priorite normale : SDL_RENDER_DRIVER (environnement) l'emporte. Si
       direct3d11 echoue, SDL_CreateRenderer reprend d'elle-meme la liste
       par defaut. */
    if (g_embed_child) SDL_SetHint(SDL_HINT_RENDER_DRIVER, "direct3d11");
#endif
    g_renderer = SDL_CreateRenderer(g_window, -1,
        SDL_RENDERER_ACCELERATED | SDL_RENDERER_PRESENTVSYNC);
    if (!g_renderer) {
        SDL_Log("SDL_CreateRenderer failed: %s", SDL_GetError());
        return -1;
    }
#ifdef _WIN32
    if (g_embed_child) {
        SDL_RendererInfo info;
        const char *name = SDL_GetRendererInfo(g_renderer, &info) == 0
                           ? info.name : "?";
        fprintf(stderr, "[EMBED] rendu : %s\n", name);
        const char *forced = SDL_getenv("SDL_RENDER_DRIVER");
        if (strcmp(name, "direct3d11") != 0 && !(forced && *forced))
            fprintf(stderr, "[EMBED] rendu direct3d11 indisponible : l'image "
                    "peut etre masquee par l'hote\n");
    }
#endif

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
#ifdef _WIN32
    if (g_embed_child) SDL_SetWindowsMessageHook(NULL, NULL);
    g_embed_child  = NULL;
    g_embed_parent = NULL;
#endif
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

        case SDL_KEYDOWN: {
            uint32_t k = sdl_to_lv_key(e.key.keysym.sym);
            if (k) {
                /* La repetition auto est geree par LVGL (appui long) */
                if (!e.key.repeat) key_queue_push(k, LV_INDEV_STATE_PRESSED);
                break;
            }
            if (e.key.repeat) break;
            switch (e.key.keysym.sym) {
            case SDLK_ESCAPE: fw_trigger_back(); break;
            case SDLK_m:      fw_trigger_context_menu(); break;
            case SDLK_s:      save_screenshot(); break;
            case SDLK_p:      sdl_audio_toggle_pause(); break;
#ifdef _WIN32
            case SDLK_F6:     embed_release_focus(); break;
#endif
            default:          break;
            }
            break;
        }

        case SDL_KEYUP: {
            uint32_t k = sdl_to_lv_key(e.key.keysym.sym);
            if (k) key_queue_push(k, LV_INDEV_STATE_RELEASED);
            break;
        }
        }
    }

#ifdef _WIN32
    /* Incrustation : suivre la taille du parent ; parent disparu = arret */
    if (embed_follow_parent()) {
        fprintf(stderr, "[EMBED] fenetre hote disparue : arret\n");
        return 1;
    }
#endif

    /* Laisser LVGL traiter les timers et le rendu */
    lv_timer_handler();

    /* Screenshot automatique (outil de debug) apres 12 s par defaut,
       FLAM_SCREENSHOT_AUTO_MS=<ms> pour changer le delai, 0 : desactive */
    if (!g_start_ticks) g_start_ticks = SDL_GetTicks();
    if (!g_auto_screenshot_done) {
        static long auto_ms = -1; /* -1 : non lu */
        if (auto_ms < 0) {
            const char *v = getenv("FLAM_SCREENSHOT_AUTO_MS");
            auto_ms = (v && *v) ? strtol(v, NULL, 10)
                                : SCREENSHOT_AUTO_MS_DEFAULT;
            if (auto_ms <= 0) { auto_ms = 0; g_auto_screenshot_done = 1; }
        }
        if (!g_auto_screenshot_done &&
            SDL_GetTicks() - g_start_ticks > (Uint32)auto_ms) {
            save_screenshot();
            g_auto_screenshot_done = 1;
        }
    }

    /* Tests (pilote SDL dummy, sans clavier) : FLAM_TEST_CTX_MENU_MS=<ms>
       simule un appui sur M, une seule fois, <ms> apres le premier poll */
    {
        static int ctx_test_ms = -2; /* -2 : non lu, -1 : inactif/fait */
        if (ctx_test_ms == -2) {
            const char *v = getenv("FLAM_TEST_CTX_MENU_MS");
            ctx_test_ms = (v && *v) ? atoi(v) : -1;
            if (ctx_test_ms < 0) ctx_test_ms = -1;
        }
        if (ctx_test_ms >= 0 &&
            SDL_GetTicks() - g_start_ticks >= (Uint32)ctx_test_ms) {
            ctx_test_ms = -1;
            fw_trigger_context_menu();
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
