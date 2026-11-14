/**
 * sdl_audio.c — Moteur audio : SDL2 + minimp3 + bindings Lua
 *
 * Architecture :
 *   - minimp3 decode le MP3 frame par frame en PCM S16 (frequence et
 *     nombre de canaux du fichier)
 *   - un SDL_AudioStream convertit ce PCM au format du device
 *     (AUDIO_FREQ Hz, stereo S16)
 *   - SDL_QueueAudio envoie le PCM a la carte son (pas de callback thread)
 *   - sdl_audio_pump() est appele dans la boucle principale pour decoder
 *     et alimenter SDL, et pour emettre les callbacks Lua
 */

#include "sdl_audio.h"
#include "formats/mp3map_parser.h"
#include "formats/path_guard.h"

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
#define FRAME_BYTES      (AUDIO_CHANNELS * (int)sizeof(int16_t))
#define MP3MAP_RATE      88200.0 /* unites de position du .mp3map par seconde */

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

    /* Conversion vers le format du device (frequence / canaux du MP3
       -> AUDIO_FREQ stereo). Recree si le format des frames change. */
    SDL_AudioStream *stream;
    int           stream_hz;
    int           stream_ch;

    /* Seek table */
    mp3map_t      mp3map;
    int           has_mp3map;

    /* Etat */
    audio_state_e state;
    float         duration_s;

    /* Fin du decodage atteinte : on attend que la file SDL soit vide
       avant de passer en STOP et d'emettre "stop" (etat DRAINING). */
    int           eof_draining;

    /* Position en frames de sortie (AUDIO_FREQ Hz, pour calculer le temps) */
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

    /* Pause : comme sur device, "pause" est emis au passage en pause puis
       chaque seconde tant que la pause dure (emission depuis
       sdl_audio_pump, meme raison que le stop differe). */
    int           pending_pause_cb;
    Uint32        last_pause_cb_ms;

    /* SDL device */
    SDL_AudioDeviceID dev_id;

    /* File virtuelle, quand il n'y a pas de peripherique (dev_id == 0 : session
       sans son, VM) ou que SDL_QueueAudio echoue : SDL ne consomme rien, et
       sans elle l'horloge audio (frames queuees - frames en attente) courrait
       a la vitesse du decodage, ~10 fois trop vite (#575). Elle se vide au
       rythme de l'horloge murale tant que la lecture est en cours. */
    int           queue_failed;      /* SDL_QueueAudio a echoue pour cette piste */
    uint32_t      vq_bytes;          /* octets en attente dans la file virtuelle */
    Uint32        vq_last_ms;        /* SDL_GetTicks de la derniere mise a jour */
    double        vq_credit_frames;  /* fraction de frame deja ecoulee, non consommee */

} audio_ctx_t;

static audio_ctx_t g_audio = {0};
static char g_sounds_base_path[512] = "";

static void audio_reset_state(void);

/* ================================================================== */
/* File de sortie : SDL, ou virtuelle cadencee sur l'horloge murale     */
/* ================================================================== */

static int g_test_queue_fail = 0;  /* FLAM_AUDIO_QUEUE_FAIL=1 */

static int env_is_one(const char *name) {
    const char *v = SDL_getenv(name);
    return v && strcmp(v, "1") == 0;
}

static int audio_queue_is_virtual(void) {
    return g_audio.dev_id == 0 || g_audio.queue_failed;
}

/* Fait s'ecouler la file virtuelle selon le temps reel (seulement en lecture) */
static void audio_vq_sync(void) {
    Uint32 now = SDL_GetTicks();
    if (g_audio.state == ASTATE_PLAY && g_audio.vq_bytes > 0) {
        g_audio.vq_credit_frames +=
            (double)(Uint32)(now - g_audio.vq_last_ms) * AUDIO_FREQ / 1000.0;
        uint64_t frames = (uint64_t)g_audio.vq_credit_frames;
        uint64_t bytes = frames * (uint64_t)FRAME_BYTES;
        if (bytes >= g_audio.vq_bytes) {
            g_audio.vq_bytes = 0;
            g_audio.vq_credit_frames = 0.0;
        } else {
            g_audio.vq_bytes -= (uint32_t)bytes;
            g_audio.vq_credit_frames -= (double)frames;
        }
    } else {
        g_audio.vq_credit_frames = 0.0;
    }
    g_audio.vq_last_ms = now;
}

/* Octets en attente de lecture */
static uint32_t audio_queued_bytes(void) {
    if (!audio_queue_is_virtual()) return SDL_GetQueuedAudioSize(g_audio.dev_id);
    audio_vq_sync();
    return g_audio.vq_bytes;
}

static void audio_queue_pcm(const void *buf, uint32_t bytes) {
    if (!audio_queue_is_virtual()) {
        if (!g_test_queue_fail && SDL_QueueAudio(g_audio.dev_id, buf, bytes) == 0) return;
        /* Echec : on bascule sur la file virtuelle pour le reste de la piste,
           en gardant ce que SDL avait deja en attente */
        fprintf(stderr, "audio: SDL_QueueAudio a echoue (%s), lecture cadencee "
                        "sur l'horloge murale\n",
                g_test_queue_fail ? "FLAM_AUDIO_QUEUE_FAIL" : SDL_GetError());
        g_audio.queue_failed = 1;
        g_audio.vq_bytes = SDL_GetQueuedAudioSize(g_audio.dev_id);
        g_audio.vq_credit_frames = 0.0;
        g_audio.vq_last_ms = SDL_GetTicks();
    }
    audio_vq_sync();
    g_audio.vq_bytes += bytes;
}

static void audio_clear_queue(void) {
    if (g_audio.dev_id) SDL_ClearQueuedAudio(g_audio.dev_id);
    g_audio.vq_bytes = 0;
    g_audio.vq_credit_frames = 0.0;
    g_audio.vq_last_ms = SDL_GetTicks();
}

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

    /* allowed_changes = 0 : SDL garantit le format demande (il convertit
       lui-meme si le materiel differe). La sortie est donc toujours
       AUDIO_FREQ / stereo / S16, cible du SDL_AudioStream. */
    g_audio.state = ASTATE_STOP;
    g_audio.callback_ref = LUA_NOREF;
    g_audio.last_cb_time = -1.0f;
    g_audio.queue_failed = 0;
    audio_clear_queue();

    /* Variables de test (#575), valeur exacte "1" seulement :
       FLAM_AUDIO_NO_DEVICE simule l'absence de peripherique ;
       FLAM_AUDIO_QUEUE_FAIL fait echouer SDL_QueueAudio */
    g_test_queue_fail = env_is_one("FLAM_AUDIO_QUEUE_FAIL");
    if (env_is_one("FLAM_AUDIO_NO_DEVICE")) {
        fprintf(stderr, "audio: desactive par FLAM_AUDIO_NO_DEVICE, lecture cadencee "
                        "sur l'horloge murale (sans son)\n");
        g_audio.dev_id = 0;
        return 0;
    }
    g_audio.dev_id = SDL_OpenAudioDevice(NULL, 0, &want, &have, 0);
    if (g_audio.dev_id == 0) {
        /* Le player continue : l'audio est alors cadence sur l'horloge murale
           (file virtuelle), sans son */
        SDL_Log("SDL_OpenAudioDevice failed: %s", SDL_GetError());
        return -1;
    }

    /* Demarrer le device (il jouera quand on queue du PCM) */
    if (g_audio.dev_id) SDL_PauseAudioDevice(g_audio.dev_id, 0);

    return 0;
}

void sdl_audio_quit(void) {
    audio_reset_state();
    if (g_audio.dev_id) {
        SDL_CloseAudioDevice(g_audio.dev_id);
        g_audio.dev_id = 0;
    }
}

/* Confinement au dossier de l'histoire (main.c, mode histoire) ; desactive par
   defaut (tests et scripts lances directement : chemins absolus). */
static int g_confine_paths = 0;

void sdl_audio_set_confine_paths(int on) { g_confine_paths = on ? 1 : 0; }

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

static void audio_stream_free(void) {
    if (g_audio.stream) {
        SDL_FreeAudioStream(g_audio.stream);
        g_audio.stream = NULL;
    }
    g_audio.stream_hz = 0;
    g_audio.stream_ch = 0;
}

/* Transfere le PCM converti disponible vers la file SDL */
static void audio_stream_to_queue(void) {
    if (!g_audio.stream) return;
    int16_t buf[4096];
    while (SDL_AudioStreamAvailable(g_audio.stream) > 0) {
        int got = SDL_AudioStreamGet(g_audio.stream, buf, (int)sizeof(buf));
        if (got <= 0) break;
        audio_queue_pcm(buf, (uint32_t)got);
        g_audio.samples_queued += (uint64_t)(got / FRAME_BYTES);
    }
}

/* (Re)cree le convertisseur si le format source change. 0 si OK. */
static int audio_stream_setup(int hz, int ch) {
    if (g_audio.stream && g_audio.stream_hz == hz && g_audio.stream_ch == ch)
        return 0;
    if (g_audio.stream) {
        /* Changement de format en cours de fichier : vider l'ancien */
        SDL_AudioStreamFlush(g_audio.stream);
        audio_stream_to_queue();
        audio_stream_free();
    }
    g_audio.stream = SDL_NewAudioStream(AUDIO_S16SYS, (Uint8)ch, hz,
                                        AUDIO_S16SYS, AUDIO_CHANNELS, AUDIO_FREQ);
    if (!g_audio.stream) {
        fprintf(stderr, "audio: SDL_NewAudioStream(%d Hz, %d ch) failed: %s\n",
                hz, ch, SDL_GetError());
        return -1;
    }
    g_audio.stream_hz = hz;
    g_audio.stream_ch = ch;
    return 0;
}

/* Libere la piste courante et remet l'etat a zero, sans toucher au
   registre Lua (callback_ref n'est pas modifie). */
static void audio_reset_state(void) {
    audio_clear_queue();
    g_audio.queue_failed = 0;
    audio_stream_free();

    if (g_audio.mp3_data) { free(g_audio.mp3_data); g_audio.mp3_data = NULL; }
    g_audio.mp3_size = 0;
    g_audio.mp3_pos = 0;

    if (g_audio.mp3map.entries) { free(g_audio.mp3map.entries); g_audio.mp3map.entries = NULL; }
    memset(&g_audio.mp3map, 0, sizeof(g_audio.mp3map));
    g_audio.has_mp3map = 0;

    g_audio.state = ASTATE_STOP;
    g_audio.eof_draining = 0;
    g_audio.samples_played = 0;
    g_audio.samples_queued = 0;
    g_audio.duration_s = 0.0f;
    g_audio.last_cb_time = -1.0f;
    g_audio.pending_stop_cb = 0;  /* annuler tout "stop" differe non emis */
    g_audio.pending_stop_time = 0.0f;
    g_audio.pending_pause_cb = 0;
}

static void audio_unload(lua_State *L) {
    if (g_audio.callback_ref != LUA_NOREF && L) {
        luaL_unref(L, LUA_REGISTRYINDEX, g_audio.callback_ref);
    }
    g_audio.callback_ref = LUA_NOREF;
    audio_reset_state();
}

static float audio_current_time(void) {
    /* Temps = frames de sortie jouees / AUDIO_FREQ (le PCM queue est
       deja converti a la frequence du device) */
    uint32_t queued_bytes = audio_queued_bytes();
    uint32_t queued_samples = queued_bytes / (uint32_t)FRAME_BYTES;
    uint64_t played = g_audio.samples_queued > queued_samples
                    ? g_audio.samples_queued - queued_samples : 0;
    return (float)played / (float)AUDIO_FREQ;
}

static void audio_emit(lua_State *L, const char *status, float t) {
    if (g_audio.callback_ref == LUA_NOREF || !L) return;
    lua_rawgeti(L, LUA_REGISTRYINDEX, g_audio.callback_ref);
    lua_pushstring(L, status);
    lua_pushnumber(L, t);
    if (lua_pcall(L, 2, 0, 0) != LUA_OK) {
        const char *err = lua_tostring(L, -1);
        fprintf(stderr, "Audio %s callback error: %s\n", status, err ? err : "?");
        lua_pop(L, 1);
    }
}

/* ================================================================== */
/* Pump : decoder et queuer du PCM                                     */
/* ================================================================== */

void sdl_audio_pump(lua_State *L) {
    /* Emettre le callback "stop" differe (audio.stop() appele depuis Lua,
       ou audio.load() en echec). Fait ici, dans la boucle principale, pour
       que le module puisse etre detruit/recharge sans danger (hors
       dispatch d'evenement LVGL). */
    if (g_audio.pending_stop_cb) {
        g_audio.pending_stop_cb = 0;
        audio_emit(L, "stop", g_audio.pending_stop_time);
    }

    /* "pause" au passage en pause puis chaque seconde (le runtime
       global.lua / audio-player s'en sert pour l'overlay pause et le
       mini-player). Annule si la lecture a repris avant ce tick
       (sequence pause -> seek -> play du runtime). */
    if (g_audio.state == ASTATE_PAUSE) {
        Uint32 now = SDL_GetTicks();
        if (g_audio.pending_pause_cb ||
            (Uint32)(now - g_audio.last_pause_cb_ms) >= 1000u) {
            g_audio.pending_pause_cb = 0;
            g_audio.last_pause_cb_ms = now;
            audio_emit(L, "pause", audio_current_time());
        }
        return;
    }
    g_audio.pending_pause_cb = 0;

    if (g_audio.state != ASTATE_PLAY) return;
    if (!g_audio.mp3_data) return;

    uint32_t queued = audio_queued_bytes();

    if (g_audio.eof_draining) {
        /* Tout est decode : "stop" seulement quand SDL a tout consomme */
        if (queued == 0) {
            g_audio.eof_draining = 0;
            g_audio.state = ASTATE_STOP;
            audio_emit(L, "stop", audio_current_time());
            return;
        }
    } else if (queued < QUEUE_LOW_MARK) {
        /* Decoder si le buffer SDL est bas */
        for (int i = 0; i < PUMP_FRAMES; i++) {
            if (g_audio.mp3_pos >= g_audio.mp3_size) {
                /* Fin du fichier : vider le convertisseur puis attendre
                   la fin de la lecture (DRAINING) */
                if (g_audio.stream) {
                    SDL_AudioStreamFlush(g_audio.stream);
                    audio_stream_to_queue();
                }
                g_audio.eof_draining = 1;
                break;
            }

            mp3dec_frame_info_t info;
            int16_t pcm[MINIMP3_MAX_SAMPLES_PER_FRAME];
            memset(&info, 0, sizeof(info));
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

            if (samples == 0 && info.hz > 0 && info.channels > 0) {
                /* Trame valide mais non decodable (reservoir de bits absent
                   juste apres un seek) : silence de la duree de la trame,
                   pour que l'horloge reste alignee sur le fichier */
                samples = info.layer == 1 ? 384
                        : (info.layer == 3 && info.hz < 32000) ? 576 : 1152;
                memset(pcm, 0, (size_t)samples * (size_t)info.channels * sizeof(int16_t));
            }

            if (samples > 0 && info.hz > 0 && info.channels > 0 &&
                audio_stream_setup(info.hz, info.channels) == 0) {
                int bytes = samples * info.channels * (int)sizeof(int16_t);
                SDL_AudioStreamPut(g_audio.stream, pcm, bytes);
                audio_stream_to_queue();
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
            audio_emit(L, "play", t);
        }
    }
}

/* ================================================================== */
/* Bindings Lua : table `audio`                                        */
/* ================================================================== */

/* audio.load(track_id, path, callback)
   Retourne 0 si OK, -1 si echec. En cas d'echec, le callback recoit un
   "stop" differe au prochain tick (contrat partage avec global.lua). */
static int l_audio_load(lua_State *L) {
    /* track_id est ignore (le firmware n'en a qu'un seul) */
    (void)luaL_checkinteger(L, 1);
    const char *path = luaL_checkstring(L, 2);

    /* Decharger l'audio precedent */
    audio_unload(L);

    /* Stocker le callback (argument 3, optionnel) avant l'ouverture :
       il doit recevoir "stop" meme si le chargement echoue */
    if (lua_isfunction(L, 3)) {
        lua_pushvalue(L, 3);
        g_audio.callback_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    }

    /* Chemin hors de l'histoire (confinement actif) : echec comme un fichier
       absent, sans toucher au disque */
    if (g_confine_paths && !path_guard_is_confined(path)) {
        fprintf(stderr, "audio.load: chemin hors de l'histoire refuse\n");
        goto fail;
    }

    /* Construire le chemin complet */
    char full_path[1024];
    /* path[1] n'est lu que si path n'est pas vide */
    if (g_sounds_base_path[0] && path[0] != '/' &&
        (path[0] == '\0' || path[1] != ':')) {
        snprintf(full_path, sizeof(full_path), "%s/%s", g_sounds_base_path, path);
    } else {
        strncpy(full_path, path, sizeof(full_path) - 1);
        full_path[sizeof(full_path) - 1] = '\0';
    }

    /* Charger le MP3 en memoire */
    FILE *f = fopen(full_path, "rb");
    if (!f) {
        fprintf(stderr, "audio.load: cannot open '%s'\n", full_path);
        goto fail;
    }
    long fsize = -1;
    if (fseek(f, 0, SEEK_END) == 0) fsize = ftell(f);
    if (fsize <= 0 || fseek(f, 0, SEEK_SET) != 0) {
        fprintf(stderr, "audio.load: empty or unreadable '%s'\n", full_path);
        fclose(f);
        goto fail;
    }
    g_audio.mp3_data = (uint8_t *)malloc((size_t)fsize);
    if (!g_audio.mp3_data) {
        fprintf(stderr, "audio.load: out of memory (%ld bytes) for '%s'\n", fsize, full_path);
        fclose(f);
        goto fail;
    }
    if (fread(g_audio.mp3_data, 1, (size_t)fsize, f) != (size_t)fsize) {
        fprintf(stderr, "audio.load: read error on '%s'\n", full_path);
        fclose(f);
        goto fail;
    }
    fclose(f);
    g_audio.mp3_size = (size_t)fsize;

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

    g_audio.state = ASTATE_STOP;
    g_audio.samples_played = 0;
    g_audio.samples_queued = 0;
    g_audio.last_cb_time = -1.0f;

    lua_pushinteger(L, 0);  /* succes : retourner 0 (verifie par global.lua) */
    return 1;

fail:
    if (g_audio.mp3_data) { free(g_audio.mp3_data); g_audio.mp3_data = NULL; }
    g_audio.mp3_size = 0;
    g_audio.mp3_pos = 0;
    g_audio.state = ASTATE_STOP;
    /* "stop" differe : la scene peut se terminer au lieu de rester figee */
    g_audio.pending_stop_cb = 1;
    g_audio.pending_stop_time = 0.0f;
    lua_pushinteger(L, -1);
    return 1;
}

static void audio_do_play(void) {
    if (!g_audio.mp3_data) return;
    /* Reprise apres pause : "play" des le prochain tick (retire l'overlay
       pause du runtime sans attendre la seconde suivante) */
    if (g_audio.state == ASTATE_PAUSE) g_audio.last_cb_time = -1.0f;
    if (audio_queue_is_virtual()) audio_vq_sync();  /* l'ecoulement reprend ici */
    g_audio.state = ASTATE_PLAY;
    g_audio.pending_pause_cb = 0;
    if (g_audio.dev_id) SDL_PauseAudioDevice(g_audio.dev_id, 0);
}

static void audio_do_pause(void) {
    if (g_audio.state != ASTATE_PLAY) return;
    if (audio_queue_is_virtual()) audio_vq_sync();  /* solde l'ecoule avant la pause */
    g_audio.state = ASTATE_PAUSE;
    g_audio.pending_pause_cb = 1;
    if (g_audio.dev_id) SDL_PauseAudioDevice(g_audio.dev_id, 1);
}

/* audio.play() */
static int l_audio_play(lua_State *L) {
    (void)L;
    audio_do_play();
    return 0;
}

void sdl_audio_toggle_pause(void) {
    if (g_audio.state == ASTATE_PLAY) audio_do_pause();
    else if (g_audio.state == ASTATE_PAUSE) audio_do_play();
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
    g_audio.eof_draining = 0;
    g_audio.pending_pause_cb = 0;
    audio_clear_queue();
    if (g_audio.stream) SDL_AudioStreamClear(g_audio.stream);
    /* Rembobiner */
    if (g_audio.mp3_data) {
        mp3dec_init(&g_audio.decoder);
        g_audio.mp3_pos = 0;
        g_audio.samples_queued = 0;
    }
    return 0;
}

void sdl_audio_stop_all(void) {
    /* Appele avant lua_close : le callback appartient a l'etat Lua qui va
       etre ferme. On oublie la reference sans luaL_unref (sinon, apres
       relance, l'unref viserait un slot du nouveau registre). */
    g_audio.callback_ref = LUA_NOREF;
    audio_reset_state();
}

/* audio.pause() */
static int l_audio_pause(lua_State *L) {
    (void)L;
    audio_do_pause();
    return 0;
}

/* audio.seek(seconds) : garde l'etat courant (lecture ou pause) */
static int l_audio_seek(lua_State *L) {
    float seconds = (float)luaL_checknumber(L, 1);
    if (!g_audio.mp3_data) return 0;

    /* Borner [0, duree] (NaN compris) : un negatif converti en uint64
       est indefini */
    if (!(seconds > 0.0f)) seconds = 0.0f;
    if (g_audio.duration_s > 0.0f && seconds > g_audio.duration_s)
        seconds = g_audio.duration_s;

    /* Position retenue (frames de sortie) : l'horloge rapportee au Lua
       repart de la position reellement atteinte, pas de la cible */
    uint64_t start_samples = (uint64_t)((double)seconds * AUDIO_FREQ);

    if (g_audio.has_mp3map && g_audio.mp3map.num_entries > 0) {
        /* Point de depart de la table (entree <= cible, ou debut des
           donnees id3_offset pour une cible avant entries[0]), puis
           avance trame par trame jusqu'a la frontiere de trame la plus
           proche de la cible (la table n'a qu'une entree par ~1 s) */
        uint32_t byte_off, unit_pos;
        if (mp3map_seek_floor(&g_audio.mp3map, seconds, &byte_off, &unit_pos) != 0 ||
            byte_off >= g_audio.mp3_size) {
            return 0;   /* table incoherente avec le fichier : seek ignore */
        }
        double target = (double)seconds * MP3MAP_RATE;
        double units = (double)unit_pos;
        size_t pos = byte_off;
        mp3dec_t probe;
        mp3dec_init(&probe);
        while (pos < g_audio.mp3_size) {
            mp3dec_frame_info_t info;
            /* pcm = NULL : lecture de l'en-tete seulement (pas de decodage) */
            int n = mp3dec_decode_frame(&probe, g_audio.mp3_data + pos,
                                        (int)(g_audio.mp3_size - pos), NULL, &info);
            if (n <= 0 || info.frame_bytes <= 0 || info.hz <= 0) break;
            double fu = (double)n * MP3MAP_RATE / (double)info.hz;
            if (units + fu / 2.0 > target) break;
            pos += (size_t)info.frame_bytes;
            units += fu;
        }
        g_audio.mp3_pos = pos;
        start_samples = (uint64_t)(units * AUDIO_FREQ / MP3MAP_RATE + 0.5);
    } else if (g_audio.duration_s > 0.0f) {
        /* Sans mp3map : estimation lineaire */
        float frac = seconds / g_audio.duration_s;
        g_audio.mp3_pos = (size_t)((float)g_audio.mp3_size * frac);
    } else {
        /* Ni table ni duree : position inconnue, seek ignore */
        return 0;
    }

    audio_clear_queue();
    if (g_audio.stream) SDL_AudioStreamClear(g_audio.stream);
    mp3dec_init(&g_audio.decoder);
    g_audio.samples_queued = start_samples;
    g_audio.last_cb_time = (float)start_samples / (float)AUDIO_FREQ;
    g_audio.eof_draining = 0;

    /* Pas de changement d'etat : un seek pendant la pause reste en pause
       (audio.play() reprend a la nouvelle position) */
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

#ifdef FLAM_TEST_REAL_AUDIO
static int l_test_toggle_pause(lua_State *L) {
    (void)L;
    sdl_audio_toggle_pause();
    return 0;
}
#endif

void sdl_audio_register_lua(lua_State *L) {
    /* Nouvel etat Lua : toute reference d'un etat precedent est caduque
       (filet de securite si sdl_audio_stop_all n'a pas ete appele avant
       lua_close) */
    sdl_audio_stop_all();

    lua_newtable(L);
    luaL_setfuncs(L, audio_funcs, 0);
    lua_setglobal(L, "audio");

#ifdef FLAM_TEST_REAL_AUDIO
    /* flam-test-audio : expose la touche P (sdl_audio_toggle_pause) aux
       tests tests/audio, hors table `audio` (absente du device) */
    lua_register(L, "test_audio_toggle_pause", l_test_toggle_pause);
#endif
}
