/**
 * control.c — Pilotage par stdin (--stdin-control), voir control.h.
 *
 * Le thread ne touche ni a LVGL ni a Lua : il ne fait que lever un drapeau
 * atomique, lu par la boucle principale a chaque tour (5 ms). Il reste
 * bloque dans fgets jusqu'a la fin du processus (thread detache).
 */

#include "control.h"
#include "SDL.h"

#include <stdio.h>
#include <string.h>

static SDL_atomic_t g_quit;

static int SDLCALL stdin_thread(void *data)
{
    (void)data;
    char line[256];
    while (fgets(line, sizeof(line), stdin)) {
        size_t n = strlen(line);
        while (n > 0 && (line[n - 1] == '\n' || line[n - 1] == '\r' ||
                         line[n - 1] == ' ' || line[n - 1] == '\t'))
            line[--n] = '\0';
        if (strcmp(line, "quit") == 0) {
            fprintf(stderr, "[CONTROL] quit recu sur stdin : arret\n");
            SDL_AtomicSet(&g_quit, 1);
            return 0;
        }
        if (n > 0) fprintf(stderr, "[CONTROL] commande inconnue ignoree : %s\n", line);
    }
    fprintf(stderr, "[CONTROL] fin de stdin : arret\n");
    SDL_AtomicSet(&g_quit, 1);
    return 0;
}

int control_stdin_start(void)
{
    SDL_Thread *t = SDL_CreateThread(stdin_thread, "flam-stdin", NULL);
    if (!t) {
        fprintf(stderr, "[CONTROL] thread stdin impossible : %s\n", SDL_GetError());
        return -1;
    }
    SDL_DetachThread(t);
    return 0;
}

int control_quit_requested(void)
{
    return SDL_AtomicGet(&g_quit) != 0;
}
