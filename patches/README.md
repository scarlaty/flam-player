# Patches pour les sous-modules

Correctifs destines aux forks des sous-modules. Ils ne sont **pas** appliques
automatiquement : le sous-module `libs/lvgl` reste sur le commit enregistre.

## lvgl-null-alloc.patch

Cible : fork `scarlaty/lvgl`, branche `release/v8.3` (sous-module `libs/lvgl`).

**Probleme.** Dans LVGL 8.3, `lv_obj_class_create_obj` (`src/core/lv_obj_class.c`)
agrandit la liste des enfants du parent (ou des ecrans du display) avec
`lv_mem_realloc` sans tester le retour. Quand le tas LVGL est plein (~1700
enfants avec `LV_MEM_SIZE` = 512 Ko), il ecrit dans un pointeur NULL :
crash 0xC0000005. `lv_obj_add_event_cb` (`src/core/lv_event.c`) a le meme
defaut et part sur `LV_ASSERT_MALLOC`, donc `abort()`.

**Correctif.** Le tableau est d'abord agrandi dans un pointeur temporaire ;
en cas d'echec rien n'est modifie, l'objet est libere et la fonction renvoie
NULL (`lv_xxx_create` renvoie alors NULL). `lv_obj_add_event_cb` garde son
ancienne liste et renvoie NULL.

**Etat actuel sans le patch.** Le player est deja protege cote bindings
(`src/bindings/lua_lv.c`, `lua_lv_mem_check*` : erreur Lua "tas LVGL epuise"
avant d'appeler LVGL, marge de 32 Ko). Le patch ferme le probleme a la source,
y compris pour les allocations faites par LVGL hors bindings.

**Appliquer.**

```sh
cd libs/lvgl
git apply --check ../../patches/lvgl-null-alloc.patch
git apply ../../patches/lvgl-null-alloc.patch
git commit -am "fix(core): gere l'echec d'allocation dans create_obj et add_event_cb"
git push origin HEAD:release/v8.3
cd ../..
git add libs/lvgl && git commit -m "chore(lvgl): integre le correctif d'allocation NULL"
```

Verifier ensuite : build + `test_run.bat strict` (dont `fuzz_lvgl` et
`regress_heap_exhaust`).
