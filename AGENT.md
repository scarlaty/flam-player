# AGENT.md — Flam Player

Emulateur du dispositif FLAM (lecteur d'histoires interactives pour enfants, type Lunii).
Reverse-engineering du firmware : LVGL 8.3 + Lua 5.4.8 + SDL2 sur desktop Windows.

## Compilation

**Prerequis** : Visual Studio 2022 (ou +) avec les outils C++ x64
(`Microsoft.VisualStudio.Component.VC.Tools.x86.x64`), CMake et Ninja (ceux de VS, du PATH,
ou `pip install cmake ninja`). Submodules initialises : `git submodule update --init --recursive`.

**Build (script, recommande)** :
```bat
do_build.bat            :: configure build\ si besoin + compile flam-player.exe
do_build.bat tests      :: reconfigure avec -DBUILD_TESTS=ON + compile flam-test.exe
```
`do_build.bat` localise VS via `vswhere`, appelle `vcvarsall.bat x64`, cherche `cmake`/`ninja`
(PATH, puis VS, puis Python/pip) et utilise des chemins relatifs a son emplacement.
`manual_build.bat` n'est plus qu'un alias de `do_build.bat` (memes arguments).

**Build manuel (cmd, ou depuis bash via `cmd /c`)** :
```bat
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat" x64
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTS=ON
cmake --build build
```
Adapter le chemin de `vcvarsall.bat` a l'edition installee (Community, BuildTools...).
Si `ninja` n'est pas dans le PATH, ajouter `-DCMAKE_MAKE_PROGRAM=<chemin\ninja.exe>`.
`vcvarsall.bat` est obligatoire pour que MSVC trouve `stddef.h` et les headers Windows SDK.
Sans cet appel, la compilation echoue avec `fatal error C1083: stddef.h: No such file or directory`.
Un dossier de build autre que `build\` est possible (`-B <dossier>`) ; voir `FLAM_BUILD_DIR`
pour les tests.

**Generateur** : Ninja (pas MSBuild). **Compilateur** : `cl.exe` (MSVC x64).
Les sources de `libs/` sont collectees par `file(GLOB ...)` (re-configurer cmake si elles
changent) ; celles de `src/` sont listees explicitement dans `CMakeLists.txt`.

## Tests

```bat
do_build.bat tests      :: construit build\flam-test.exe
test_run.bat            :: lance la suite puis les tests isoles
test_run.bat strict     :: idem, mais un echec isole rend le code de sortie non nul
```
- Dossier de build : `build\` par defaut, ou `%FLAM_BUILD_DIR%` s'il est defini
  (ex. `set FLAM_BUILD_DIR=C:\tmp\mon-build` puis `test_run.bat`).
- Suite : tous les `tests\lua\test_*.lua` (hors `test_helpers.lua`) dans un seul processus
  `flam-test.exe --timeout 60`. Elle doit rester verte ; son code de sortie est celui du script.
- Tests isoles : `regress_*.lua`, `poc_*.lua`, `fuzz_*.lua`, un processus par fichier
  (`--timeout 30`), pour qu'un crash ou un blocage ne masque pas les autres. Recapitulatif
  en fin de sortie ; codes : 1 = assertion/erreur Lua, 3 = timeout (watchdog), autre = crash.
  Hors mode `strict`, leurs echecs n'affectent pas le code de sortie.
- Usage direct : `flam-test.exe [--timeout sec] test1.lua [test2.lua ...]` (60 s par defaut).

## Execution

```bash
# Lancer une histoire .plain (dossier)
./build/flam-player.exe "stories/mon-histoire.plain"

# Lancer une archive .plain.pk (ZIP store)
./build/flam-player.exe "stories/mon-histoire.plain.pk"

# Lancer un script Lua directement
./build/flam-player.exe "test.lua"

# Browser d'histoires (scan le dossier courant)
./build/flam-player.exe

# Scanner un dossier specifique
./build/flam-player.exe --scan-dir "C:\chemin\vers\histoires"

# Lancer des tests unitaires (voir section Tests)
./build/flam-test.exe --timeout 60 tests/lua/test_obj.lua tests/lua/test_style.lua ...
```

**Timeout en CI/test** : utiliser `timeout 8 ./build/flam-player.exe ...` car le player ouvre une fenetre SDL et attend indefiniment.

## Architecture

```
src/
  main.c                  — Point d'entree, story browser, chargement .plain/.pk
  bindings/
    lua_lv.h              — Core des bindings Lua↔LVGL, metatables, push/check helpers
    lua_lv.c              — Registration des modules lv.*, gestion images/timers
    lua_lv_obj.c           — Bindings lv.obj.*, lv.label.*, lv.btn.*, lv.img.*, etc.
    lua_lv_style.c         — Bindings lv.style.*
    lua_lv_event.c         — Bindings lv.obj.add_event_cb, lv.group.*
  formats/
    lif_decoder.c/h        — Decodeur d'images .lif (format proprietaire FLAM)
    pk_reader.c/h          — Lecteur d'archives .plain.pk (ZIP store, pas de compression)
    mp3map_parser.c/h      — Parser de .mp3map (mapping audio)
  platform/
    sdl_driver.c/h         — Driver SDL2 pour LVGL (affichage + input encodeur)
    sdl_audio.c/h          — Audio SDL2 + minimp3, API Lua audio.*
  firmware/
    fw_globals.c/h         — Emulation des globales firmware (state, save, context_menu)
  fonts/
    nunito_*.c             — Polices LVGL compilees (Nunito Bold/ExtraBold 12-20px)
libs/
  lvgl/                   — LVGL 8.3 (submodule, fork scarlaty/lvgl, voir Dependances)
  lua/                    — Lua 5.4.8 (submodule lua/lua, sources a la racine, pas de src/)
  SDL2/                   — SDL2 (submodule, compile depuis les sources, SDL2.dll copiee au build)
  minimp3/                — Decodeur MP3 header-only (submodule)
  lv_conf.h               — Configuration LVGL (320x240, 32-bit, tas 512 Ko)
tests/
  test_main.c             — Harnais de test headless (watchdog --timeout)
  lua/test_*.lua           — Tests unitaires Lua (obj, style, label, btn, event, etc.)
  lua/regress_*.lua        — Tests de non-regression (un processus par fichier)
  lua/fuzz_lvgl.lua        — Fuzzer LVGL (buffer overflow, UAF, integer overflow, heap)
  lua/poc_*.lua            — PoCs de securite (use-after-free, memory leak, struct dump)
stories/
  *.plain/                — Histoires (dossier avec info.plain + main.lua + img/ + sounds/)
  *.plain.pk              — Archives d'histoires (ZIP store)
```

## Format d'histoire .plain

Un dossier `nom.plain/` contenant :
- `info.plain` — premiere ligne = titre de l'histoire
- `main.lua` — script Lua principal (point d'entree)
- `script/` — modules Lua additionnels (chargeables via `require`)
- `img/` — images .lif (format proprietaire) et `thumbnail.lif`
- `sounds/` — fichiers audio .mp3

Le format `.plain.pk` est un ZIP store (compression=0) contenant les memes fichiers.
Le player extrait automatiquement le `.pk` dans un dossier `.plain` adjacent.

### Creer un .plain.pk depuis un dossier .plain

Utiliser Python avec `zipfile.ZIP_STORED` (compression=0). **Ne pas utiliser** `Compress-Archive` de PowerShell
car il compresse les fichiers meme avec `-CompressionLevel Optimal`, ce qui produit un ZIP invalide pour le player.

```bash
python -c "
import zipfile, pathlib, sys
src = pathlib.Path(sys.argv[1])          # ex: stories/mon-histoire.plain
dst = str(src) + '.pk'                   # => stories/mon-histoire.plain.pk
with zipfile.ZipFile(dst, 'w', compression=zipfile.ZIP_STORED) as zf:
    for f in sorted(src.rglob('*')):
        if f.is_file():
            zf.write(f, f.relative_to(src))
print(f'Created {dst}')
" "stories/mon-histoire.plain"
```

**Points importants** :
- Les chemins dans le ZIP sont relatifs a la racine du dossier `.plain` (pas de prefixe de dossier parent)
- Compression = `ZIP_STORED` (0) obligatoire — le reader `pk_reader.c` lit les fichiers directement sans decompression
- Verifier avec `python -m zipfile -l archive.plain.pk` que `CompressedLength == Length` pour chaque fichier

## API Lua disponible

Le script Lua a acces a :
- `window` — conteneur LVGL principal (320x240, sous la header bar de 28px)
- `document` — focus group principal (navigation encodeur)
- `lv.*` — bindings LVGL complets (obj, label, btn, img, slider, arc, style, color, anim, timer, group)
- `audio.*` — lecture audio (play, stop, pause, resume, set_volume, on_end)
- `state.*` — persistence (get, set, sauvegarde automatique dans saves/)
- `context_menu.*` — menu contextuel (comme le firmware)
- Bibliotheques standard Lua completes (`io`, `os`, `debug`, `package`) — PAS de sandbox dans l'emulateur

**Sur le vrai dispositif FLAM** : la sandbox Lua ne laisse que `lv.*`, pas de `io`/`os`/`debug`/`package`.

## Ecran FLAM

- Resolution : 320x240 pixels
- Facteur d'upscale desktop : x3 (fenetre 960x720)
- Header bar : 28px en haut (titre de l'histoire)
- Zone de contenu (`window`) : 320x212 pixels
- Couleur : 32-bit ARGB
- Heap LVGL : 512 Ko (`LV_MEM_SIZE (512U * 1024U)` dans `libs/lv_conf.h`, allocateur interne
  `LV_MEM_CUSTOM 0`)

**Attention compatibilite** : la taille du tas LVGL du vrai dispositif n'a pas ete mesuree, et
l'etat Lua du simulateur n'a pas de plafond memoire. Le simulateur peut donc accepter une
histoire qui manquerait de memoire sur le device.

## Dependances (submodules)

| Submodule | Source | Version |
|-----------|--------|---------|
| `libs/lua` | `lua/lua` | tag `v5.4.8` (commit `6e22fedb`), branche suivie `v5.4` dans `.gitmodules` |
| `libs/lvgl` | fork `scarlaty/lvgl` | `release/v8.3` amont + 1 commit propre (voir ci-dessous) |
| `libs/SDL2` | `libsdl-org/SDL` | branche `SDL2` |
| `libs/minimp3` | `lieff/minimp3` | `master` |

Le fork `scarlaty/lvgl` ne contient qu'un commit au-dessus de l'amont :
`0c8d7dc6f fix(bar): guard against division by zero when range == 0`. Il modifie :
- `src/widgets/lv_bar.c` (`draw_indic`) : indicateur non dessine quand `min == max` (evite la division par zero) ;
- `src/misc/lv_mem.c/.h` : helper `_lv_assert_crash()` (flush stdout/stderr puis `abort()`),
  aujourd'hui inutilise ; traces `[MEM FAIL]` sur stderr quand `lv_mem_buf_get` echoue ;
- `src/core/lv_refr.c` : ajout d'un `#include <stdio.h>` (sans effet fonctionnel).

Le `LV_ASSERT_HANDLER` de `libs/lv_conf.h` ne vient pas du fork : il appelle
`flam_assert_crash(__FILE__, __LINE__)` (`src/firmware/fw_globals.c`), qui logue sur stderr
fichier:ligne, la traceback Lua et l'etat du tas LVGL, puis fait `abort()` (au lieu de la
boucle infinie par defaut de LVGL).

Verifier la version de Lua : `git -C libs/lua describe --tags` doit afficher `v5.4.8`.

## Vulnerabilites connues (etude de securite)

### Use-After-Free (CWE-416) — corrige
Historique : `lv_obj_del()` liberait la memoire C sans invalider le userdata Lua.
Desormais (`src/bindings/lua_lv_obj.c`, en-tete "Duree de vie des objets") le userdata est mis
a `NULL` a la suppression (`LV_EVENT_DELETE`, enfants compris) et `lua_lv_check_obj()`
(`src/bindings/lua_lv.h`) leve l'erreur Lua `bad argument #n (lv object deleted)`.
`lv.obj.del` / `lv.obj.clean` sur un objet deja supprime sont des no-op.
`tests/lua/poc_use_after_free.lua` verifie ce comportement.

### Integer Overflow dans lv_txt.c
`libs/lvgl/src/misc/lv_txt.c` (`lv_txt_get_size`) : dans la version LVGL utilisee, le calcul
de hauteur de texte est protege par un test d'overflow (`LV_LOG_WARN` puis retour).

## Notes techniques

- L'allocateur LVGL (`lv_mem`) est LIFO best-fit : un bloc libere est reutilise par la prochaine allocation de meme taille
- `lv_label_t` etend `lv_obj_t` avec un champ `char *text` — c'est un pointeur vers un buffer separe
- (Historique, avant le correctif UAF) pour le heap spray : creer des labels (pas des obj generiques) pour que le bloc libere soit reutilise par un autre label
- Les styles (`lv.style.new()`) ne sont pas des `lv_obj_t` — ils ont leur propre metatable et taille d'allocation
- L'encodeur (molette FLAM) est emule via les fleches clavier haut/bas + Enter

## Fichiers batch utiles

| Fichier | Usage |
|---------|-------|
| `do_build.bat` | Configure (si besoin) et compile le player dans `build\` |
| `do_build.bat tests` | Compile les tests (`-DBUILD_TESTS=ON`, cible `flam-test`) |
| `manual_build.bat` | Alias de `do_build.bat` (memes arguments) |
| `run.bat [dossier]` | Lance `build\flam-player.exe` (`--scan-dir` si un dossier est donne), logs `build\stdout.log` / `build\stderr.log` |
| `test_run.bat [strict]` | Suite `test_*.lua` + tests isoles `regress_`/`poc_`/`fuzz_` (voir Tests) |
