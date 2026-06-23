/**
 * test_audio_stub.c — Stub audio module for testing
 *
 * Registers the same "audio" Lua table as sdl_audio.c
 * but all functions are no-ops with argument validation.
 */

#include "test_audio_stub.h"
#include "lauxlib.h"

static int l_stub_load(lua_State *L) {
    luaL_checkinteger(L, 1);     /* track_id */
    luaL_checkstring(L, 2);      /* path */
    /* optional callback at 3 */
    return 0;
}

static int l_stub_play(lua_State *L) {
    (void)L;
    return 0;
}

static int l_stub_stop(lua_State *L) {
    (void)L;
    return 0;
}

static int l_stub_pause(lua_State *L) {
    (void)L;
    return 0;
}

static int l_stub_seek(lua_State *L) {
    luaL_checknumber(L, 1);
    return 0;
}

static int l_stub_duration(lua_State *L) {
    lua_pushnumber(L, 0.0);
    return 1;
}

static int l_stub_get_status(lua_State *L) {
    lua_pushstring(L, "stop");
    return 1;
}

static const luaL_Reg audio_funcs[] = {
    {"load",       l_stub_load},
    {"play",       l_stub_play},
    {"stop",       l_stub_stop},
    {"pause",      l_stub_pause},
    {"seek",       l_stub_seek},
    {"duration",   l_stub_duration},
    {"get_status", l_stub_get_status},
    {NULL, NULL}
};

void test_audio_register(lua_State *L) {
    lua_newtable(L);
    luaL_setfuncs(L, audio_funcs, 0);
    lua_setglobal(L, "audio");
}
