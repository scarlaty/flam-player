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

/* Comparaison sans casse (ASCII) du composant [a, a+alen) avec `b` (minuscules) */
static inline int path_guard_ieq(const char *a, size_t alen, const char *b)
{
    if (strlen(b) != alen) return 0;
    for (size_t i = 0; i < alen; i++) {
        if (tolower((unsigned char)a[i]) != (unsigned char)b[i]) return 0;
    }
    return 1;
}

/* 1 si le composant [comp, comp+len) est un nom de peripherique Windows reserve :
   CON, PRN, AUX, NUL, CONIN$, CONOUT$, COM1-9, LPT1-9 (aussi avec un chiffre en
   exposant 1, 2, 3 : octet Latin-1 0xB9, 0xB2, 0xB3, ou sa forme UTF-8 0xC2 puis cet
   octet), avec ou sans extension ("NUL.txt" designe aussi le peripherique).
   Liste unique, partagee avec pk_reader.c (pk_name_is_safe) : fopen recoit les
   octets du nom tels quels (page de code ANSI), les deux formes sont couvertes. */
static inline int path_guard_is_device_name(const char *comp, size_t len)
{
    static const char *const fixed[] = { "con", "prn", "aux", "nul", "conin$", "conout$" };
    size_t base = 0;
    while (base < len && comp[base] != '.') base++;   /* le nom sans extension */
    while (base > 0 && comp[base - 1] == ' ') base--;
    for (size_t i = 0; i < sizeof(fixed) / sizeof(fixed[0]); i++) {
        if (path_guard_ieq(comp, base, fixed[i])) return 1;
    }
    if (base < 4) return 0;
    int c3 = tolower((unsigned char)comp[0]) == 'c' && tolower((unsigned char)comp[1]) == 'o' &&
             tolower((unsigned char)comp[2]) == 'm';
    int l3 = tolower((unsigned char)comp[0]) == 'l' && tolower((unsigned char)comp[1]) == 'p' &&
             tolower((unsigned char)comp[2]) == 't';
    if (c3 || l3) {
        unsigned char d = (unsigned char)comp[3];
        if (base == 4 && ((d >= '1' && d <= '9') || d == 0xB9 || d == 0xB2 || d == 0xB3))
            return 1;
        if (base == 5 && d == 0xC2) {
            unsigned char e = (unsigned char)comp[4];
            if (e == 0xB9 || e == 0xB2 || e == 0xB3) return 1;
        }
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
