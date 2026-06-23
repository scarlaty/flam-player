#ifndef FW_GLOBALS_H
#define FW_GLOBALS_H

#include "lua.h"

/**
 * Enregistre tous les objets firmware dans l'etat Lua :
 *   - state          (table persistante)
 *   - progression    (save/load vers fichiers)
 *   - context_menu   (set_entries + overlay LVGL)
 *   - back_callback  (fonction globale, initialisee a goto_library)
 *   - goto_library   (fonction C : quitter l'histoire)
 *   - screen         (stubs : set_state, wake_up, set_brightness)
 *   - progress       (variable numerique globale, 0-100)
 *
 * Doit etre appele apres luaopen_lv() et apres injection de window/document.
 */
void fw_register_globals(lua_State *L);

/**
 * Configure le repertoire de sauvegarde pour state/progression.
 * Typiquement : {story_dir}/../saves/{uuid}/
 */
void fw_set_save_dir(const char *dir);

/**
 * Pompe context_menu : gere la touche M (appeler dans la boucle principale).
 * Retourne 1 si goto_library a ete demande (= quitter).
 */
int fw_pump(lua_State *L);

/**
 * Signale un appui sur la touche M (menu contextuel).
 */
void fw_trigger_context_menu(void);

/**
 * Signale un appui sur la touche ESC (back).
 */
void fw_trigger_back(void);

/**
 * Sauvegarde l'etat courant (state) sur disque.
 */
void fw_save_state(lua_State *L);

/**
 * Recharge l'etat (state) depuis le save_dir courant.
 * A appeler apres fw_set_save_dir() quand le Lua est deja initialise.
 */
void fw_reload_state(lua_State *L);

/**
 * Reinitialise l'etat interne du firmware (flags, refs, overlay).
 * A appeler avant de recreer l'etat Lua (retour au browser).
 */
void fw_reset(void);

#endif /* FW_GLOBALS_H */
