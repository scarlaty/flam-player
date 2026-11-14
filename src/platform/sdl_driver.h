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

/* Repere d'authenticite des traces "[EMBED] ..." (#649). Le launcher passe un
   jeton aleatoire par lancement dans la variable d'environnement
   FLAM_EMBED_TOKEN ; l'hote ne reconnait que les lignes qui le portent
   ("[EMBED#<jeton>] repli : ...") et un script Lua, qui ne lit ni l'environnement
   ni cette trace, ne peut donc pas les imiter. sdl_driver_embed_token_init lit
   puis retire la variable de l'environnement (a appeler au debut de main ;
   idempotent) ; sans jeton valide (player lance a la main) le repere reste
   "[EMBED]". */
void sdl_driver_embed_token_init(void);

/* Ecrit sur stderr une ligne "[EMBED] <message>" (ou "[EMBED#<jeton>] <message>").
   fmt ne doit pas contenir le saut de ligne final : il est ajoute ici. */
void sdl_driver_embed_log(const char *fmt, ...);

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
