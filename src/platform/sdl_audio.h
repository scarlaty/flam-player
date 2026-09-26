#ifndef SDL_AUDIO_H
#define SDL_AUDIO_H

#include "lua.h"

/**
 * Initialise le sous-systeme audio SDL2.
 * Retourne 0 si OK, -1 si erreur.
 */
int sdl_audio_init(void);

/**
 * Libere les ressources audio.
 */
void sdl_audio_quit(void);

/**
 * Pompe audio : decode des frames MP3 et les envoie a SDL.
 * Appeler dans la boucle principale (~60 fois/seconde).
 * Gere aussi les callbacks Lua de feedback audio.
 */
void sdl_audio_pump(lua_State *L);

/**
 * Enregistre la table globale `audio` dans l'etat Lua.
 */
void sdl_audio_register_lua(lua_State *L);

/**
 * Configure le chemin de base pour les fichiers audio.
 */
void sdl_audio_set_base_path(const char *path);

/**
 * Arrete et libere toute lecture audio en cours, et remet a zero l'etat
 * lie a Lua (callback oublie sans luaL_unref, "stop" differe annule).
 * A appeler avant lua_close ; sdl_audio_register_lua l'appelle aussi
 * pour chaque nouvel etat Lua.
 */
void sdl_audio_stop_all(void);

/**
 * Bascule lecture <-> pause du son courant (touche P de l'emulateur).
 * Emet ensuite le callback Lua "pause" (puis chaque seconde en pause) ou
 * "play" comme sur device, depuis sdl_audio_pump. Sans effet si aucun
 * son n'est en lecture ou en pause.
 */
void sdl_audio_toggle_pause(void);

#endif /* SDL_AUDIO_H */
