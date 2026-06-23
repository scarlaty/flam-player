/**
 * test_main.c — Test runner for Lua bindings
 *
 * Initializes LVGL headless, then runs each .lua test file
 * passed as a command-line argument in a fresh Lua state.
 *
 * Usage: flam-test test1.lua test2.lua ...
 * Exit code: 0 if all pass, 1 if any fail.
 */

#include "test_headless_driver.h"
#include "test_audio_stub.h"
#include "bindings/lua_lv.h"
#include "firmware/fw_globals.h"

#include <stdio.h>
#include <string.h>

/* Lua function: test_tick(n) — call lv_timer_handler n times with 5ms delay */
static int l_test_tick(lua_State *L) {
    int n = (int)luaL_optinteger(L, 1, 1);
    for (int i = 0; i < n; i++) {
        SDL_Delay(5);
        test_driver_tick();
    }
    return 0;
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

    /* Audio stub */
    test_audio_register(L);

    /* Firmware globals (state, progression, context_menu, screen) */
    fw_register_globals(L);

    /* test_tick() helper */
    lua_pushcfunction(L, l_test_tick);
    lua_setglobal(L, "test_tick");

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

    /* Run the test file */
    int err = luaL_dofile(L, path);
    if (err) {
        fprintf(stderr, "  [ERROR] %s\n", lua_tostring(L, -1));
        lua_close(L);
        lv_obj_clean(lv_scr_act());
        return 1;
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

    /* Delete all user-created LVGL timers before closing Lua state,
       otherwise timer callbacks will try to use the freed lua_State. */
    {
        lv_timer_t *t = lv_timer_get_next(NULL);
        while (t) {
            lv_timer_t *next = lv_timer_get_next(t);
            /* Only delete timers with user_data (ours), keep LVGL internal ones */
            if (t->user_data != NULL) {
                lv_timer_del(t);
            }
            t = next;
        }
    }

    /* Delete all animations */
    lv_anim_del_all();

    lua_close(L);

    /* Clean LVGL widget tree for next test */
    lv_obj_clean(lv_scr_act());

    return fail;
}

int main(int argc, char *argv[])
{
    if (argc < 2) {
        fprintf(stderr, "Usage: %s test1.lua [test2.lua ...]\n", argv[0]);
        return 1;
    }

    if (test_driver_init() != 0) {
        fprintf(stderr, "Failed to init headless driver\n");
        return 1;
    }

    int total_pass = 0, total_fail = 0, total_errors = 0;

    for (int i = 1; i < argc; i++) {
        printf("=== %s ===\n", argv[i]);
        int fail = run_test_file(argv[i]);
        if (fail < 0) {
            total_errors++;
        } else {
            total_fail += fail;
        }
    }

    printf("========================================\n");
    printf("Files: %d | Failures: %d | Errors: %d\n",
           argc - 1, total_fail, total_errors);
    printf("Result: %s\n", (total_fail + total_errors) == 0 ? "PASS" : "FAIL");

    test_driver_quit();

    return (total_fail + total_errors) > 0 ? 1 : 0;
}
