/**
 * test_main.c — Test runner for Lua bindings
 *
 * Initializes LVGL headless, then runs each .lua test file
 * passed as a command-line argument in a fresh Lua state.
 *
 * Usage: flam-test [--timeout sec] test1.lua test2.lua ...
 * Exit code: 0 if all pass, 1 if any fail or error (chargement, setup(),
 * aucune assertion). Timeout : 3. Crash : code d'exception Windows.
 *
 * Cible flam-test-audio (FLAM_TEST_REAL_AUDIO) : meme runner, mais avec le
 * vrai moteur audio (src/platform/sdl_audio.c) au lieu du stub. Pilote SDL
 * "dummy" par defaut (SDL_AUDIODRIVER). test_tick() pompe aussi l'audio et
 * sdl_audio_stop_all() est appele avant lua_close comme dans main.c : des
 * fichiers passes a la suite testent la fermeture/reouverture du lua_State
 * (issue #1).
 */

#include "test_headless_driver.h"
#ifdef FLAM_TEST_REAL_AUDIO
#include "platform/sdl_audio.h"
#else
#include "test_audio_stub.h"
#endif
#include "bindings/lua_lv.h"
#include "firmware/fw_globals.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define SDL_MAIN_HANDLED   /* pas de SDL_main : main() classique */
#include <SDL.h>

#ifdef _WIN32
#include <windows.h>
#include <crtdbg.h>
#endif

/* Fichier en cours (pour les messages de crash / timeout) */
static const char *g_cur_file = "?";

/* Watchdog : un test qui boucle (ex. timer supprime deux fois) doit
   echouer au lieu de bloquer la suite. Delai par fichier, en secondes. */
static int g_timeout_s = 60;

static Uint32 watchdog_cb(Uint32 interval, void *param)
{
    (void)param;
    fprintf(stderr, "  [TIMEOUT] %s : pas termine apres %d s\n",
            g_cur_file, g_timeout_s);
    fflush(stdout);
    fflush(stderr);
    _exit(3);
    return interval;
}

#ifdef _WIN32
/* Crash (acces invalide...) : afficher le fichier fautif, pas de popup */
static LONG WINAPI crash_filter(EXCEPTION_POINTERS *ep)
{
    fprintf(stderr, "  [CRASH] %s : exception 0x%08lX\n", g_cur_file,
            (unsigned long)ep->ExceptionRecord->ExceptionCode);
    fflush(stdout);
    fflush(stderr);
    return EXCEPTION_EXECUTE_HANDLER;
}

static void no_crash_popups(void)
{
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    SetUnhandledExceptionFilter(crash_filter);
    _set_abort_behavior(0, _WRITE_ABORT_MSG | _CALL_REPORTFAULT);
    _CrtSetReportMode(_CRT_ASSERT, _CRTDBG_MODE_FILE);
    _CrtSetReportFile(_CRT_ASSERT, _CRTDBG_FILE_STDERR);
    _CrtSetReportMode(_CRT_ERROR, _CRTDBG_MODE_FILE);
    _CrtSetReportFile(_CRT_ERROR, _CRTDBG_FILE_STDERR);
}
#endif

static int msgh_traceback(lua_State *L)
{
    const char *msg = lua_tostring(L, 1);
    luaL_traceback(L, L, msg ? msg : "(erreur non textuelle)", 1);
    return 1;
}

/* Execute la fonction au sommet de pile en mode protege avec traceback */
static int pcall_tb(lua_State *L)
{
    int base = lua_gettop(L);
    lua_pushcfunction(L, msgh_traceback);
    lua_insert(L, base);
    int err = lua_pcall(L, 0, 0, base);
    lua_remove(L, base);
    return err;
}

/* Lua function: test_tick(n) — call lv_timer_handler n times with 5ms delay */
static int l_test_tick(lua_State *L) {
    int n = (int)luaL_optinteger(L, 1, 1);
    for (int i = 0; i < n; i++) {
        SDL_Delay(5);
        test_driver_tick();
#ifdef FLAM_TEST_REAL_AUDIO
        sdl_audio_pump(L);
#endif
    }
    return 0;
}

/* Lua function: test_ms() — horloge murale en ms (SDL_GetTicks) */
static int l_test_ms(lua_State *L) {
    lua_pushinteger(L, (lua_Integer)SDL_GetTicks());
    return 1;
}

static int run_test_file(const char *path)
{
    lua_State *L = luaL_newstate();
    if (!L) {
        fprintf(stderr, "  Failed to create Lua state\n");
        return 1;
    }
    luaL_openlibs(L);

    /* Register lv.* bindings */
    luaopen_lv(L);

    /* Create window (content area below 28px header, like main.c) */
    lv_obj_t *content_window = lv_obj_create(lv_scr_act());
    lv_obj_remove_style_all(content_window);
    lv_obj_set_pos(content_window, 0, 28);
    lv_obj_set_size(content_window, FLAM_SCREEN_W, FLAM_SCREEN_H - 28);
    lv_obj_set_style_bg_color(content_window, lv_color_black(), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(content_window, LV_OPA_COVER, LV_PART_MAIN);

    lua_lv_push_obj(L, content_window);
    lua_setglobal(L, "window");

    /* Create focus group (document) */
    lv_group_t *grp = lv_group_create();
    lv_group_set_default(grp);
    lua_lv_push_group(L, grp);
    lua_setglobal(L, "document");

    /* Bind group to input device */
    lv_indev_set_group(test_driver_get_indev(), grp);

#ifdef FLAM_TEST_REAL_AUDIO
    /* Vrai moteur audio (remet a zero l'etat lie a l'ancien lua_State) */
    sdl_audio_register_lua(L);
#else
    /* Audio stub */
    test_audio_register(L);
#endif

    /* Firmware globals (state, progression, context_menu, screen) */
    fw_register_globals(L);

    /* test_tick() helper */
    lua_pushcfunction(L, l_test_tick);
    lua_setglobal(L, "test_tick");
    lua_pushcfunction(L, l_test_ms);
    lua_setglobal(L, "test_ms");

    /* Set package.path to find test_helpers.lua */
    lua_getglobal(L, "package");
    lua_getfield(L, -1, "path");
    const char *cur_path = lua_tostring(L, -1);

    /* Extract directory from the test file path */
    char dir[1024] = "";
    const char *last_sep = strrchr(path, '/');
    if (!last_sep) last_sep = strrchr(path, '\\');
    if (last_sep) {
        size_t len = (size_t)(last_sep - path);
        if (len >= sizeof(dir)) len = sizeof(dir) - 1;
        memcpy(dir, path, len);
        dir[len] = '\0';
    } else {
        strcpy(dir, ".");
    }

    char new_path[2048];
    snprintf(new_path, sizeof(new_path), "%s/?.lua;%s",
             dir, cur_path ? cur_path : "");
    lua_pop(L, 1); /* pop old path */
    lua_pushstring(L, new_path);
    lua_setfield(L, -2, "path");
    lua_pop(L, 1); /* pop package */

    /* Run the test file (-1 = erreur : chargement/execution/setup) */
    int status = 0;
    int err = luaL_loadfile(L, path);
    if (err == LUA_OK) err = pcall_tb(L);
    if (err == LUA_OK) {
        /* Comme le firmware : appeler setup() si le script en definit un */
        lua_getglobal(L, "setup");
        if (lua_isfunction(L, -1)) err = pcall_tb(L);
        else lua_pop(L, 1);
    }
    if (err != LUA_OK) {
        fprintf(stderr, "  [ERROR] %s\n", lua_tostring(L, -1));
        lua_pop(L, 1);
        status = -1;
    }

    /* Read pass/fail counts */
    int pass = 0, fail = 0;
    lua_getglobal(L, "TEST_PASS");
    if (lua_isinteger(L, -1)) pass = (int)lua_tointeger(L, -1);
    lua_pop(L, 1);

    lua_getglobal(L, "TEST_FAIL");
    if (lua_isinteger(L, -1)) fail = (int)lua_tointeger(L, -1);
    lua_pop(L, 1);

    printf("  Result: %d passed, %d failed\n\n", pass, fail);
    if (status == 0 && pass + fail == 0) {
        /* Un fichier sans aucune assertion ne doit pas passer pour vert */
        fprintf(stderr, "  [ERROR] %s : aucune assertion executee\n", path);
        status = -1;
    }

    /* Nettoyage dans le meme ordre que main.c : timers/anims Lua, puis
       arbre LVGL (callbacks DELETE encore valides), puis lua_close.
       Les timers internes LVGL (refresh, indev) sont conserves. */
    lua_lv_cleanup_timers();
    lv_anim_del_all();

    lv_indev_set_group(test_driver_get_indev(), NULL);
    lv_group_set_default(NULL);
    lv_group_remove_all_objs(grp);
    lv_group_del(grp);
    lv_obj_clean(lv_scr_act());

#ifdef FLAM_TEST_REAL_AUDIO
    /* Comme main.c (retour bibliotheque) : oublier le callback de cet etat */
    sdl_audio_stop_all();
#endif
    lua_close(L);

    return status < 0 ? status : fail;
}

int main(int argc, char *argv[])
{
    int first = 1;
    if (argc > 2 && strcmp(argv[1], "--timeout") == 0) {
        g_timeout_s = atoi(argv[2]);
        first = 3;
    }
    if (argc <= first) {
        fprintf(stderr, "Usage: %s [--timeout sec] test1.lua [test2.lua ...]\n", argv[0]);
        return 1;
    }

#ifdef _WIN32
    no_crash_popups();
#endif

    if (test_driver_init() != 0) {
        fprintf(stderr, "Failed to init headless driver\n");
        return 1;
    }

#ifdef FLAM_TEST_REAL_AUDIO
    /* Pas de carte son requise : pilote dummy sauf choix explicite */
    if (!SDL_getenv("SDL_AUDIODRIVER"))
        SDL_setenv("SDL_AUDIODRIVER", "dummy", 1);
    if (SDL_InitSubSystem(SDL_INIT_AUDIO) != 0 || sdl_audio_init() != 0) {
        fprintf(stderr, "Failed to init audio: %s\n", SDL_GetError());
        test_driver_quit();
        return 1;
    }
    printf("Audio driver: %s\n", SDL_GetCurrentAudioDriver());
#endif

    int total_pass = 0, total_fail = 0, total_errors = 0;

    for (int i = first; i < argc; i++) {
        printf("=== %s ===\n", argv[i]);
        fflush(stdout);
        g_cur_file = argv[i];
        SDL_TimerID wd = 0;
        if (g_timeout_s > 0)
            wd = SDL_AddTimer((Uint32)g_timeout_s * 1000u, watchdog_cb, NULL);
        int fail = run_test_file(argv[i]);
        if (wd) SDL_RemoveTimer(wd);
        fflush(stdout);
        if (fail < 0) {
            total_errors++;
        } else {
            total_fail += fail;
        }
    }

    printf("========================================\n");
    printf("Files: %d | Failures: %d | Errors: %d\n",
           argc - first, total_fail, total_errors);
    printf("Result: %s\n", (total_fail + total_errors) == 0 ? "PASS" : "FAIL");

#ifdef FLAM_TEST_REAL_AUDIO
    sdl_audio_quit();
#endif
    test_driver_quit();

    return (total_fail + total_errors) > 0 ? 1 : 0;
}
