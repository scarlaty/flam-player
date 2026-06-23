/**
 * sdl_audio.c — Moteur audio : SDL2 + minimp3 + bindings Lua
 *
 * Architecture :
 *   - minimp3 decode le MP3 frame par frame en PCM S16 stereo
 *   - SDL_QueueAudio envoie le PCM a la carte son (pas de callback thread)
 *   - sdl_audio_pump() est appele dans la boucle principale pour decoder
 *     et alimenter SDL, et pour emettre les callbacks Lua
 */

#include "sdl_audio.h"
#include "formats/mp3map_parser.h"

#include "SDL.h"
#include "lua.h"
#include "lauxlib.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* minimp3 — implementation dans ce fichier uniquement */
#define MINIMP3_IMPLEMENTATION
#include "minimp3.h"

/* ================================================================== */
/* Constantes                                                          */
/* ================================================================== */

#define AUDIO_FREQ       44100
#define AUDIO_CHANNELS   2
#define AUDIO_SAMPLES    2048   /* taille du buffer SDL */
#define PUMP_FRAMES      8     /* frames MP3 a decoder par appel pump */
#define QUEUE_LOW_MARK   8192  /* octets : seuil pour decoder plus */

/* ================================================================== */
/* Etat audio global                                                   */
/* ================================================================== */

typedef enum {
    ASTATE_STOP = 0,
    ASTATE_PLAY,
    ASTATE_PAUSE
} audio_state_e;

typedef struct {
    /* Donnees MP3 en memoire */
    uint8_t      *mp3_data;
    size_t        mp3_size;
    size_t        mp3_pos;       /* position courante dans mp3_data */

    /* Decodeur minimp3 */
    mp3dec_t      decoder;
    int           sample_rate;
    int           channels;

    /* Seek table */
    mp3map_t      mp3map;
    int           has_mp3map;

    /* Etat */
    audio_state_e state;
    float         duration_s;

    /* Position en samples (pour calculer le temps) */
    uint64_t      samples_played;
    uint64_t      samples_queued;

    /* Callback Lua */
    int           callback_ref;  /* LUA_NOREF si pas de callback */
    float         last_cb_time;  /* dernier temps de callback */

    /* Stop differe : audio.stop() peut etre appele depuis un event LVGL
       (ex : clic encodeur sur title-card). Le callback "stop" est emis
       depuis sdl_audio_pump (boucle principale) pour ne pas detruire le
       module courant pendant le dispatch d'evenement. */
    int           pending_stop_cb;
    float         pending_stop_time;

    /* SDL device */
    SDL_AudioDeviceID dev_id;

} audio_ctx_t;

static audio_ctx_t g_audio = {0};
static char g_sounds_base_path[512] = "";

/* ================================================================== */
/* Init / Quit                                                         */
/* ================================================================== */

int sdl_audio_init(void) {
    SDL_AudioSpec want, have;
    memset(&want, 0, sizeof(want));
    want.freq     = AUDIO_FREQ;
    want.format   = AUDIO_S16SYS;
    want.channels = AUDIO_CHANNELS;
    want.samples  = AUDIO_SAMPLES;
    want.callback = NULL;  /* mode queue */

    g_audio.dev_id = SDL_OpenAudioDevice(NULL, 0, &want, &have, 0);
    if (g_audio.dev_id == 0) {
        SDL_Log("SDL_OpenAudioDevice failed: %s", SDL_GetError());
        return -1;
    }

    g_audio.state = ASTATE_STOP;
    g_audio.callback_ref = LUA_NOREF;
    g_audio.last_cb_time = -1.0f;

    /* Demarrer le device (il jouera quand on queue du PCM) */
    SDL_PauseAudioDevice(g_audio.dev_id, 0);

    return 0;
}

void sdl_audio_quit(void) {
    if (g_audio.dev_id) {
        SDL_CloseAudioDevice(g_audio.dev_id);
        g_audio.dev_id = 0;
    }
    if (g_audio.mp3_data) { free(g_audio.mp3_data); g_audio.mp3_data = NULL; }
    if (g_audio.mp3map.entries) { free(g_audio.mp3map.entries); g_audio.mp3map.entries = NULL; }
}

void sdl_audio_set_base_path(const char *path) {
    if (path) {
        strncpy(g_sounds_base_path, path, sizeof(g_sounds_base_path) - 1);
        g_sounds_base_path[sizeof(g_sounds_base_path) - 1] = '\0';
    } else {
        g_sounds_base_path[0] = '\0';
    }
}

/* ================================================================== */
/* Fonctions internes                                                  */
/* ================================================================== */

static void audio_unload(lua_State *L) {
    SDL_ClearQueuedAudio(g_audio.dev_id);

    if (g_audio.mp3_data) { free(g_audio.mp3_data); g_audio.mp3_data = NULL; }
    g_audio.mp3_size = 0;
    g_audio.mp3_pos = 0;

    if (g_audio.mp3map.entries) { free(g_audio.mp3map.entries); g_audio.mp3map.entries = NULL; }
    g_audio.has_mp3map = 0;

    if (g_audio.callback_ref != LUA_NOREF && L) {
        luaL_unref(L, LUA_REGISTRYINDEX, g_audio.callback_ref);
        g_audio.callback_ref = LUA_NOREF;
    }

    g_audio.state = ASTATE_STOP;
    g_audio.samples_played = 0;
    g_audio.samples_queued = 0;
    g_audio.duration_s = 0.0f;
    g_audio.last_cb_time = -1.0f;
    g_audio.pending_stop_cb = 0;  /* annuler tout "stop" differe non emis */
}

static float audio_current_time(void) {
    /* Temps = samples joues / sample_rate */
    uint32_t queued_bytes = SDL_GetQueuedAudioSize(g_audio.dev_id);
    uint32_t queued_samples = queued_bytes / (AUDIO_CHANNELS * sizeof(int16_t));
    uint64_t played = g_audio.samples_queued > queued_samples
                    ? g_audio.samples_queued - queued_samples : 0;
    return (float)played / (float)AUDIO_FREQ;
}

/* ================================================================== */
/* Pump : decoder et queuer du PCM                                     */
/* ================================================================== */

void sdl_audio_pump(lua_State *L) {
    /* Emettre le callback "stop" differe (audio.stop() appele depuis Lua).
       Fait ici, dans la boucle principale, pour que le module puisse etre
       detruit/recharge sans danger (hors dispatch d'evenement LVGL). */
    if (g_audio.pending_stop_cb) {
        g_audio.pending_stop_cb = 0;
        if (g_audio.callback_ref != LUA_NOREF && L) {
            lua_rawgeti(L, LUA_REGISTRYINDEX, g_audio.callback_ref);
            lua_pushstring(L, "stop");
            lua_pushnumber(L, g_audio.pending_stop_time);
            if (lua_pcall(L, 2, 0, 0) != LUA_OK) {
                const char *err = lua_tostring(L, -1);
                fprintf(stderr, "Audio stop callback error: %s\n", err ? err : "?");
                lua_pop(L, 1);
            }
        }
    }

    if (g_audio.state != ASTATE_PLAY) return;
    if (!g_audio.mp3_data) return;

    /* Decoder si le buffer SDL est bas */
    uint32_t queued = SDL_GetQueuedAudioSize(g_audio.dev_id);
    if (queued < QUEUE_LOW_MARK) {
        for (int i = 0; i < PUMP_FRAMES; i++) {
            if (g_audio.mp3_pos >= g_audio.mp3_size) {
                /* Fin du fichier */
                g_audio.state = ASTATE_STOP;

                /* Emettre le callback "stop" */
                if (g_audio.callback_ref != LUA_NOREF && L) {
                    lua_rawgeti(L, LUA_REGISTRYINDEX, g_audio.callback_ref);
                    lua_pushstring(L, "stop");
                    lua_pushnumber(L, audio_current_time());
                    if (lua_pcall(L, 2, 0, 0) != LUA_OK) {
                        const char *err = lua_tostring(L, -1);
                        fprintf(stderr, "Audio stop callback error: %s\n", err ? err : "?");
                        lua_pop(L, 1);
                    }
                }
                break;
            }

            mp3dec_frame_info_t info;
            int16_t pcm[MINIMP3_MAX_SAMPLES_PER_FRAME];
            int samples = mp3dec_decode_frame(&g_audio.decoder,
                g_audio.mp3_data + g_audio.mp3_pos,
                (int)(g_audio.mp3_size - g_audio.mp3_pos),
                pcm, &info);

            if (info.frame_bytes > 0) {
                g_audio.mp3_pos += (size_t)info.frame_bytes;
            } else {
                /* Pas de frame valide : avancer d'un octet */
                g_audio.mp3_pos++;
                continue;
            }

            if (samples > 0) {
                int bytes = samples * info.channels * (int)sizeof(int16_t);

                /* Si mono, dupliquer en stereo */
                if (info.channels == 1) {
                    int16_t stereo[MINIMP3_MAX_SAMPLES_PER_FRAME * 2];
                    for (int s = 0; s < samples; s++) {
                        stereo[s*2]   = pcm[s];
                        stereo[s*2+1] = pcm[s];
                    }
                    SDL_QueueAudio(g_audio.dev_id, stereo, (Uint32)(samples * 2 * sizeof(int16_t)));
                    g_audio.samples_queued += (uint64_t)samples;
                } else {
                    SDL_QueueAudio(g_audio.dev_id, pcm, (Uint32)bytes);
                    g_audio.samples_queued += (uint64_t)samples;
                }
            }
        }
    }

    /* Callback Lua : chaque seconde */
    if (g_audio.callback_ref != LUA_NOREF && L) {
        float t = audio_current_time();
        float last_sec = (float)(int)g_audio.last_cb_time;
        float cur_sec  = (float)(int)t;
        if (cur_sec > last_sec || g_audio.last_cb_time < 0.0f) {
            g_audio.last_cb_time = t;
            lua_rawgeti(L, LUA_REGISTRYINDEX, g_audio.callback_ref);
            lua_pushstring(L, "play");
            lua_pushnumber(L, t);
            if (lua_pcall(L, 2, 0, 0) != LUA_OK) {
                const char *err = lua_tostring(L, -1);
                fprintf(stderr, "Audio play callback error: %s\n", err ? err : "?");
                lua_pop(L, 1);
            }
        }
    }
}

/* ================================================================== */
/* Bindings Lua : table `audio`                                        */
/* ================================================================== */

/* audio.load(track_id, path, callback) */
static int l_audio_load(lua_State *L) {
    /* track_id est ignore (le firmware n'en a qu'un seul) */
    (void)luaL_checkinteger(L, 1);
    const char *path = luaL_checkstring(L, 2);

    /* Decharger l'audio precedent */
    audio_unload(L);

    /* Construire le chemin complet */
    char full_path[1024];
    if (g_sounds_base_path[0] && path[0] != '/' && path[1] != ':') {
        snprintf(full_path, sizeof(full_path), "%s/%s", g_sounds_base_path, path);
    } else {
        strncpy(full_path, path, sizeof(full_path) - 1);
        full_path[sizeof(full_path) - 1] = '\0';
    }

    /* Charger le MP3 en memoire */
    FILE *f = fopen(full_path, "rb");
    if (!f) {
        fprintf(stderr, "audio.load: cannot open '%s'\n", full_path);
        return 0;
    }
    fseek(f, 0, SEEK_END);
    g_audio.mp3_size = (size_t)ftell(f);
    fseek(f, 0, SEEK_SET);
    g_audio.mp3_data = (uint8_t *)malloc(g_audio.mp3_size);
    fread(g_audio.mp3_data, 1, g_audio.mp3_size, f);
    fclose(f);

    /* Initialiser le decodeur */
    mp3dec_init(&g_audio.decoder);
    g_audio.mp3_pos = 0;

    /* Decoder la premiere frame pour obtenir le sample rate */
    mp3dec_frame_info_t info;
    int16_t tmp[MINIMP3_MAX_SAMPLES_PER_FRAME];
    int samples = mp3dec_decode_frame(&g_audio.decoder,
        g_audio.mp3_data, (int)g_audio.mp3_size, tmp, &info);
    g_audio.sample_rate = info.hz ? info.hz : AUDIO_FREQ;
    g_audio.channels = info.channels ? info.channels : 2;
    /* Rembobiner */
    mp3dec_init(&g_audio.decoder);
    g_audio.mp3_pos = 0;
    (void)samples;

    /* Charger le mp3map si present */
    char map_path[1024];
    snprintf(map_path, sizeof(map_path), "%smap", full_path);
    if (mp3map_parse(map_path, &g_audio.mp3map) == 0) {
        g_audio.has_mp3map = 1;
        g_audio.duration_s = g_audio.mp3map.duration_s;
    } else {
        g_audio.has_mp3map = 0;
        /* Estimation grossiere : taille fichier / bitrate moyen */
        g_audio.duration_s = 0.0f;
    }

    /* Stocker le callback (argument 3, optionnel) */
    if (lua_isfunction(L, 3)) {
        lua_pushvalue(L, 3);
        g_audio.callback_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    }

    g_audio.state = ASTATE_STOP;
    g_audio.samples_played = 0;
    g_audio.samples_queued = 0;
    g_audio.last_cb_time = -1.0f;

    lua_pushinteger(L, 0);  /* succes : retourner 0 (verifie par global.lua) */
    return 1;
}

/* audio.play() */
static int l_audio_play(lua_State *L) {
    (void)L;
    if (g_audio.mp3_data) {
        g_audio.state = ASTATE_PLAY;
        SDL_PauseAudioDevice(g_audio.dev_id, 0);
    }
    return 0;
}

/* audio.stop() */
static int l_audio_stop(lua_State *L) {
    (void)L;
    /* Si l'audio etait actif, programmer l'emission du callback "stop"
       (contrat firmware : audio.stop() notifie). L'emission est differee
       a sdl_audio_pump pour ne pas reentrer dans Lua pendant un event. */
    if (g_audio.state != ASTATE_STOP) {
        g_audio.pending_stop_cb = 1;
        g_audio.pending_stop_time = audio_current_time();
    }
    g_audio.state = ASTATE_STOP;
    SDL_ClearQueuedAudio(g_audio.dev_id);
    /* Rembobiner */
    if (g_audio.mp3_data) {
        mp3dec_init(&g_audio.decoder);
        g_audio.mp3_pos = 0;
        g_audio.samples_queued = 0;
    }
    return 0;
}

void sdl_audio_stop_all(void) {
    g_audio.state = ASTATE_STOP;
    if (g_audio.dev_id) SDL_ClearQueuedAudio(g_audio.dev_id);
    if (g_audio.mp3_data) {
        free(g_audio.mp3_data);
        g_audio.mp3_data = NULL;
        g_audio.mp3_size = 0;
        g_audio.mp3_pos = 0;
    }
}

/* audio.pause() */
static int l_audio_pause(lua_State *L) {
    (void)L;
    if (g_audio.state == ASTATE_PLAY) {
        g_audio.state = ASTATE_PAUSE;
        SDL_PauseAudioDevice(g_audio.dev_id, 1);
    }
    return 0;
}

/* audio.seek(seconds) */
static int l_audio_seek(lua_State *L) {
    float seconds = (float)luaL_checknumber(L, 1);
    if (!g_audio.mp3_data) return 0;

    SDL_ClearQueuedAudio(g_audio.dev_id);

    if (g_audio.has_mp3map) {
        uint32_t byte_off = mp3map_seek(&g_audio.mp3map, seconds);
        if (byte_off < g_audio.mp3_size) {
            g_audio.mp3_pos = byte_off;
        }
    } else {
        /* Sans mp3map : estimation lineaire */
        if (g_audio.duration_s > 0.0f) {
            float frac = seconds / g_audio.duration_s;
            if (frac < 0.0f) frac = 0.0f;
            if (frac > 1.0f) frac = 1.0f;
            g_audio.mp3_pos = (size_t)((float)g_audio.mp3_size * frac);
        }
    }

    mp3dec_init(&g_audio.decoder);
    g_audio.samples_queued = (uint64_t)(seconds * AUDIO_FREQ);
    g_audio.last_cb_time = seconds;

    if (g_audio.state == ASTATE_PAUSE) {
        SDL_PauseAudioDevice(g_audio.dev_id, 0);
        g_audio.state = ASTATE_PLAY;
    }

    return 0;
}

/* audio.duration() */
static int l_audio_duration(lua_State *L) {
    lua_pushnumber(L, (lua_Number)g_audio.duration_s);
    return 1;
}

/* audio.get_status() */
static int l_audio_get_status(lua_State *L) {
    switch (g_audio.state) {
    case ASTATE_PLAY:  lua_pushstring(L, "play");  break;
    case ASTATE_PAUSE: lua_pushstring(L, "pause"); break;
    default:           lua_pushstring(L, "stop");  break;
    }
    return 1;
}

static const luaL_Reg audio_funcs[] = {
    {"load",       l_audio_load},
    {"play",       l_audio_play},
    {"stop",       l_audio_stop},
    {"pause",      l_audio_pause},
    {"seek",       l_audio_seek},
    {"duration",   l_audio_duration},
    {"get_status", l_audio_get_status},
    {NULL, NULL}
};

void sdl_audio_register_lua(lua_State *L) {
    lua_newtable(L);
    luaL_setfuncs(L, audio_funcs, 0);
    lua_setglobal(L, "audio");
}
