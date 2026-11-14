/**
 * path_guard.h - garde des chemins que les scripts Lua donnent aux chargeurs de
 * l'hote (images lv.img_src.load, sons audio.load, modules require).
 *
 * L'appareil ne laisse pas un script sortir du dossier de l'histoire : un
 * chemin relatif au dossier de l'histoire est accepte, rien d'autre. Sont
 * refuses : chemin absolu ("/" ou "\" en tete), lecteur ou flux de donnees
 * alternatif (":"), composant ".." (separateurs "/" et "\"), nom de
 * peripherique Windows (CON, NUL, COM1... : fopen y bloquerait ou lirait la
 * console), composant termine par un point ou une espace (Windows les ignore).
 * Un chemin vide n'est pas refuse ici (les chargeurs le traitent comme absent).
 * Fonctions statiques en ligne : aucun fichier source a declarer.
 */
#ifndef FLAM_PATH_GUARD_H
#define FLAM_PATH_GUARD_H

#include <ctype.h>
#include <string.h>

/* 1 si le composant [c, c+n) est un nom de peripherique Windows reserve
   (avec ou sans extension : "NUL.txt" designe aussi le peripherique). */
static inline int path_guard_is_device_name(const char *c, size_t n)
{
    size_t base = 0;
    while (base < n && c[base] != '.') base++;
    while (base > 0 && c[base - 1] == ' ') base--;
    char up[5];
    if (base == 3 || base == 4) {
        for (size_t i = 0; i < base; i++) up[i] = (char)toupper((unsigned char)c[i]);
        up[base] = '\0';
        if (base == 3 && (!strcmp(up, "CON") || !strcmp(up, "PRN") ||
                          !strcmp(up, "AUX") || !strcmp(up, "NUL")))
            return 1;
        if (base == 4 && (!strncmp(up, "COM", 3) || !strncmp(up, "LPT", 3)) &&
            up[3] >= '1' && up[3] <= '9')
            return 1;
    }
    return 0;
}

/* 1 si `path` reste dans le dossier de base (voir l'en-tete), 0 sinon. */
static inline int path_guard_is_confined(const char *path)
{
    if (!path) return 0;
    if (path[0] == '/' || path[0] == '\\') return 0;
    if (strchr(path, ':')) return 0;
    const char *p = path;
    while (1) {
        const char *e = p;
        while (*e && *e != '/' && *e != '\\') e++;
        size_t n = (size_t)(e - p);
        if (n == 2 && p[0] == '.' && p[1] == '.') return 0;
        if (n > 0 && (p[n - 1] == '.' || p[n - 1] == ' ') && !(n == 1 && p[0] == '.'))
            return 0;
        if (path_guard_is_device_name(p, n)) return 0;
        if (!*e) break;
        p = e + 1;
    }
    return 1;
}

#endif /* FLAM_PATH_GUARD_H */
