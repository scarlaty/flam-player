#ifndef SDL_DRIVER_H
#define SDL_DRIVER_H

#include <stdint.h>

#include "lvgl/lvgl.h"
#include "SDL.h"

/* Initialise SDL2, crée la fenêtre et enregistre les drivers LVGL. */
int sdl_driver_init(void);

/* Incrustation (Windows) : a appeler avant sdl_driver_init. La fenetre SDL
   devient une fenetre enfant du HWND donne, remplit sa zone cliente et en
   suit la taille ; la disparition du parent fait renvoyer 1 a
   sdl_driver_poll. Echec : trace "[EMBED] repli : ..." et fenetre separee.
   0 : fenetre separee (defaut). Hors Windows : ignore (avec trace). */
void sdl_driver_set_parent(uintptr_t hwnd);

/* Libère les ressources SDL. */
void sdl_driver_quit(void);

/* Copie le framebuffer LVGL vers la fenêtre SDL et gère les événements.
   Retourne 0 normalement, 1 si l'utilisateur ferme la fenêtre (ou, en mode
   incruste, si la fenetre hote a disparu). */
int sdl_driver_poll(void);

/* Change le titre de la fenêtre SDL. */
void sdl_driver_set_title(const char *title);

/* Chemin du fichier BMP des captures (touche S et capture auto).
   Prioritaire sur FLAM_SCREENSHOT ; NULL ou "" : retour au defaut.
   La chaine doit rester valide (argv). */
void sdl_driver_set_screenshot_path(const char *path);

#endif /* SDL_DRIVER_H */
