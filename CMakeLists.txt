cmake_minimum_required(VERSION 3.16)
project(flam-player C)

set(CMAKE_C_STANDARD 11)

# --- Résolution écran Flam ---
set(FLAM_SCREEN_W 320)
set(FLAM_SCREEN_H 240)
set(FLAM_SCALE 3)  # Facteur d'upscale pour la fenêtre desktop

# --- SDL2 (compilé depuis les sources via submodule libs/SDL2) ---
set(SDL2_DIR "${CMAKE_SOURCE_DIR}/libs/SDL2")
set(SDL_SHARED ON CACHE BOOL "" FORCE)
set(SDL_STATIC OFF CACHE BOOL "" FORCE)
set(SDL_TEST OFF CACHE BOOL "" FORCE)
set(SDL2_DISABLE_INSTALL ON CACHE BOOL "" FORCE)
add_subdirectory(libs/SDL2)
# Fournit les cibles SDL2::SDL2 (partagée) et SDL2::SDL2main + les includes

# --- Lua 5.4 (compilé depuis les sources) ---
# Le mirror officiel lua/lua place les sources à la racine du dépôt
file(GLOB LUA_SOURCES "libs/lua/*.c")
# Retirer lua.c, luac.c et onelua.c (exécutables CLI / amalgame, pas la lib)
list(FILTER LUA_SOURCES EXCLUDE REGEX "(lua|luac|onelua)\\.c$")
add_library(lua54 STATIC ${LUA_SOURCES})
target_include_directories(lua54 PUBLIC libs/lua)
# Éviter les warnings C4996 (MSVC deprecated functions)
target_compile_definitions(lua54 PRIVATE _CRT_SECURE_NO_WARNINGS)

# --- LVGL ---
# lv_conf.h est dans libs/ à côté du dossier lvgl/
add_definitions(-DLV_CONF_INCLUDE_SIMPLE)
set(LV_CONF_PATH "${CMAKE_SOURCE_DIR}/libs/lv_conf.h" CACHE STRING "" FORCE)

# Collecter toutes les sources LVGL
file(GLOB_RECURSE LVGL_SOURCES
    "libs/lvgl/src/*.c"
)
add_library(lvgl STATIC ${LVGL_SOURCES})
target_include_directories(lvgl PUBLIC
    libs
    libs/lvgl
    ${SDL2_DIR}/include   # Pour SDL.h référencé dans lv_conf.h (LV_TICK_CUSTOM)
)
target_compile_definitions(lvgl PRIVATE _CRT_SECURE_NO_WARNINGS)

# --- Application ---
set(APP_SOURCES
    src/main.c
    src/platform/sdl_driver.c
    src/bindings/lua_lv.c
    src/bindings/lua_lv_obj.c
    src/bindings/lua_lv_style.c
    src/bindings/lua_lv_event.c
    src/formats/lif_decoder.c
    src/formats/mp3map_parser.c
    src/formats/pk_reader.c
    src/platform/sdl_audio.c
    src/firmware/fw_globals.c
    src/fonts/nunito_bold_12.c
    src/fonts/nunito_bold_16.c
    src/fonts/nunito_bold_20.c
    src/fonts/nunito_extrabold_12.c
    src/fonts/nunito_extrabold_14.c
    src/fonts/nunito_extrabold_16.c
    src/fonts/nunito_extrabold_20.c
)

add_executable(flam-player ${APP_SOURCES})

target_include_directories(flam-player PRIVATE
    src
    libs
    libs/lvgl
    libs/lua
    libs/minimp3
    ${SDL2_DIR}/include
)

target_link_libraries(flam-player PRIVATE
    SDL2::SDL2main
    SDL2::SDL2
    lvgl
    lua54
    ole32
    shell32
)

target_compile_definitions(flam-player PRIVATE
    _CRT_SECURE_NO_WARNINGS
    FLAM_SCREEN_W=${FLAM_SCREEN_W}
    FLAM_SCREEN_H=${FLAM_SCREEN_H}
    FLAM_SCALE=${FLAM_SCALE}
)

# Copier SDL2.dll à côté de l'exécutable
add_custom_command(TARGET flam-player POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "$<TARGET_FILE:SDL2::SDL2>"
        "$<TARGET_FILE_DIR:flam-player>"
)

# --- Tests ---
option(BUILD_TESTS "Build test executable" OFF)

if(BUILD_TESTS)
    set(TEST_SOURCES
        tests/test_main.c
        tests/test_headless_driver.c
        tests/test_audio_stub.c
        src/bindings/lua_lv.c
        src/bindings/lua_lv_obj.c
        src/bindings/lua_lv_style.c
        src/bindings/lua_lv_event.c
        src/formats/lif_decoder.c
        src/formats/mp3map_parser.c
        src/firmware/fw_globals.c
        src/fonts/nunito_bold_12.c
        src/fonts/nunito_bold_16.c
        src/fonts/nunito_bold_20.c
        src/fonts/nunito_extrabold_12.c
        src/fonts/nunito_extrabold_14.c
        src/fonts/nunito_extrabold_16.c
        src/fonts/nunito_extrabold_20.c
    )

    add_executable(flam-test ${TEST_SOURCES})

    target_include_directories(flam-test PRIVATE
        src
        tests
        libs
        libs/lvgl
        libs/lua/src
        libs/minimp3
        ${SDL2_DIR}/include
    )

    target_link_libraries(flam-test PRIVATE
        SDL2::SDL2
        lvgl
        lua54
    )

    target_compile_definitions(flam-test PRIVATE
        _CRT_SECURE_NO_WARNINGS
        FLAM_SCREEN_W=${FLAM_SCREEN_W}
        FLAM_SCREEN_H=${FLAM_SCREEN_H}
        FLAM_SCALE=1
        FLAM_TEST_MODE
    )

    add_custom_command(TARGET flam-test POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "$<TARGET_FILE:SDL2::SDL2>"
            "$<TARGET_FILE_DIR:flam-test>"
    )
endif()
