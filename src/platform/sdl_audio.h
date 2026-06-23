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
 * Arrete et libere toute lecture audio en cours.
 */
void sdl_audio_stop_all(void);

#endif /* SDL_AUDIO_H */
