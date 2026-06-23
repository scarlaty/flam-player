#ifndef SDL_DRIVER_H
#define SDL_DRIVER_H

#include "lvgl/lvgl.h"
#include "SDL.h"

/* Initialise SDL2, crée la fenêtre et enregistre les drivers LVGL. */
int sdl_driver_init(void);

/* Libère les ressources SDL. */
void sdl_driver_quit(void);

/* Copie le framebuffer LVGL vers la fenêtre SDL et gère les événements.
   Retourne 0 normalement, 1 si l'utilisateur ferme la fenêtre. */
int sdl_driver_poll(void);

/* Change le titre de la fenêtre SDL. */
void sdl_driver_set_title(const char *title);

#endif /* SDL_DRIVER_H */
