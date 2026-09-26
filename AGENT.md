# AGENT.md — Flam Player

Emulateur du dispositif FLAM (lecteur d'histoires interactives pour enfants, type Lunii).
Reverse-engineering du firmware : LVGL 8.3 + Lua 5.4.8 + SDL2 sur desktop Windows.
Le depot contient aussi `tools/telmi2flam/` (convertisseur d'histoires TELMI `.zip` -> FLAM
`.plain.pk`, avec son moteur Lua embarque ; doc : `tools/telmi2flam/README.md`),
`games/casse-briques/` (jeu en contenu Lua FLAM) et `patches/` (correctifs pour le fork LVGL).

## Compilation

**Prerequis** : Visual Studio 2022 (ou +) avec les outils C++ x64
(`Microsoft.VisualStudio.Component.VC.Tools.x86.x64`), CMake et Ninja (ceux de VS, du PATH,
ou `pip install cmake ninja`). Submodules initialises : `git submodule update --init --recursive`.

**Build (script, recommande)** :
```bat
do_build.bat            :: configure build\ si besoin + compile flam-player.exe
do_build.bat tests      :: reconfigure avec -DBUILD_TESTS=ON + compile flam-test.exe,
                        :: flam-test-audio.exe et flam-player.exe (requis par test_run.bat)
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
Pile des executables : 8 Mo (`/STACK:8388608` dans `CMakeLists.txt`) ; avec la pile par defaut
de 1 Mo, une recursion Lua -> C (pcall imbriques) debordait en Debug (0xC00000FD).
Les sources de `libs/` sont collectees par `file(GLOB ...)` (re-configurer cmake si elles
changent) ; celles de `src/` sont listees explicitement dans `CMakeLists.txt`.

## Tests

```bat
do_build.bat tests      :: construit flam-test.exe, flam-test-audio.exe, flam-player.exe
test_run.bat            :: lance les 6 etapes (voir ci-dessous)
test_run.bat strict     :: idem, mais un echec isole rend le code de sortie non nul
```
Etapes, recapitulatif en fin de sortie (`RESULTAT : OK` si tout passe) :
1. **Suite** : tous les `tests\lua\test_*.lua` (hors `test_helpers.lua`) dans un seul
   processus `flam-test.exe --timeout 60`.
2. **Moteur/runtime** : `tests\engine\test_*.lua` (moteur `story.lua`/`main.lua` et modules
   runtime de telmi2flam, avec mocks), flam-test.
3. **Audio reel** : `tests\audio\test_*.lua` puis `reopen_1`/`reopen_2` dans le meme
   processus (fermeture/reouverture du `lua_State`), flam-test-audio = vrai `sdl_audio.c`
   avec le pilote SDL `dummy`. En `strict`, un flam-test-audio.exe absent est un echec.
4. **Tests isoles** : `regress_*.lua`, `poc_*.lua`, `fuzz_*.lua`, un processus par fichier
   (`--timeout 30`), pour qu'un crash ou un blocage ne masque pas les autres ; codes :
   1 = assertion/erreur Lua, 3 = timeout (watchdog), autre = crash. Hors `strict`, leurs
   echecs n'affectent pas le code de sortie.
5. **Convertisseur** : `tools\telmi2flam\tests` (python unittest) ; saute sans python/Pillow.
6. **Bout en bout** : `tests\e2e\e2e_telmi.py` (paquet TELMI -> telmi2flam -> flam-player
   pilote, pilotes SDL dummy) ; saute sans python ou flam-player.exe.

Variables : `FLAM_BUILD_DIR` (dossier de build, defaut `build\`), `FLAM_PYTHON` (python),
`FLAM_LUA` (Lua 5.4 autonome pour verifier les `nodes.lua` generes). `test_run.bat` fixe
`FLAM_SCREENSHOT_AUTO_MS=0` (pas de capture auto pendant les tests).
Usage direct : `flam-test.exe [--timeout sec] test1.lua [test2.lua ...]` (60 s par defaut).
Un fichier sans aucune assertion, ou dont le chargement/`setup()` echoue, compte en erreur.

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

Options du player :
- `--strict` : plus proche du device (`require()` limite a `script/`, `lv.img_src.load` sur
  un `.lif` absent = erreur Lua au lieu de `nil`).
- `--watchdog <ms>` : interrompt un script Lua qui ne rend pas la main (defaut 10000, `0` = off).
- `--screenshot <chemin>` : fichier BMP des captures (sinon `FLAM_SCREENSHOT`, sinon
  `C:/temp/flam-player/screenshot.bmp` code en dur). Capture auto 12 s apres le demarrage :
  `FLAM_SCREENSHOT_AUTO_MS=<ms>`, `0` = desactivee.
- `--img-dir`, `--sounds-dir`, `--save-dir` : dossiers en mode script `.lua` direct.

Touches : gauche/droite = molette, Entree/Espace = OK, Echap = retour (`back_callback`),
M = menu contextuel, P = pause/reprise audio, S = capture d'ecran.

**Timeout en CI/test** : utiliser `timeout 8 ./build/flam-player.exe ...` car le player ouvre une fenetre SDL et attend indefiniment.
Sans fenetre : `SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy` (voir `tests/e2e/e2e_telmi.py`).

## Architecture

```
src/
  main.c                  — Point d'entree, story browser, chargement .plain/.pk
  bindings/
    lua_lv.h              — Core des bindings Lua↔LVGL, metatables, push/check helpers
    lua_lv.c              — Registration des modules lv.*, garde "tas LVGL epuise",
                            lv.mem.monitor (API emulateur uniquement, absente du device)
    lua_lv_obj.c           — Bindings lv.obj.*, lv.label.*, lv.btn.*, lv.img.*, etc. ;
                            duree de vie des objets (userdata invalide sur LV_EVENT_DELETE,
                            styles/images/callbacks ancres a leur objet)
    lua_lv_style.c         — Bindings lv.style.*
    lua_lv_event.c         — Evenements, timers et animations, lv.group.*
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
  lua/poc_*.lua            — PoCs de securite (verifient un comportement sur)
  engine/test_*.lua        — Moteur et modules runtime de telmi2flam (mocks)
  audio/                   — Audio reel (flam-test-audio) : seek, pause, reouverture
  e2e/e2e_telmi.py         — Bout en bout TELMI -> telmi2flam -> flam-player
tools/telmi2flam/
  telmi2flam.py            — Convertisseur TELMI -> .plain.pk (python 3 + Pillow)
  validate.py              — Verification d'un .plain.pk ou d'un dossier .plain
  engine/                  — Moteur d'histoire Lua embarque (story.lua, main.lua)
  runtime/                 — Modules runtime Lunii embarques (carrousel, audio-player...)
  tests/                   — Tests unittest du convertisseur
games/casse-briques/        — Jeu casse-briques (contenu Lua FLAM)
patches/                    — Correctifs pour le fork LVGL (voir patches/README.md)
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
Le player extrait automatiquement le `.pk` dans un dossier `.plain` adjacent, et le re-extrait
si le `.pk` a change (taille ou date, temoin `<dossier>/.extracted`). L'extraction refuse les chemins dangereux (`..`, absolus, `C:`,
`\` ou `/` en tete) et les tailles incoherentes.

Pour une histoire TELMI, ne pas assembler a la main : `python tools/telmi2flam/telmi2flam.py
histoire.zip -o histoire.plain.pk` puis `python tools/telmi2flam/validate.py histoire.plain.pk`.
Les images de scene sont generees en 320x212 (zone sous le bandeau), ajustees sans deformation.

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
- `window` — conteneur LVGL principal (320x212, sous la header bar de 28px)
- `document` — focus group principal (navigation encodeur)
- `lv.*` — bindings LVGL (obj, label, btn, img, slider, arc, style, color, anim, timer, group) ;
  un appel sur un objet supprime leve une erreur Lua ; `lv.event.send(obj, lv.EVENT_DELETE)`
  est refuse (utiliser `lv.obj.del`)
- `audio.*` — `load`, `play`, `stop`, `pause`, `seek`, `duration`, `get_status` ; callback de
  `load` appele avec `play` / `pause` (chaque seconde en pause) / `stop` ; `load` en echec
  renvoie -1 et emet un seul `stop` differe
- `state` — table globale persistante (sauvegardee dans `saves/`, ecriture atomique)
- `progression.save(cle, t)` / `progression.load(cle)` — cle limitee a `[A-Za-z0-9_-]`, 64 car.
- `context_menu.set_entries({...})` — menu contextuel (comme le firmware)
- `goto_library()`, `back_callback` (defaut = `goto_library`), `screen.*` (stubs), `progress`
- Bibliotheques standard Lua completes (`io`, `os`, `debug`, `package`) — PAS de sandbox dans
  l'emulateur ; le bytecode Lua est refuse (chargement en mode texte uniquement)

**Sur le vrai dispositif FLAM** : la sandbox Lua ne laisse que `lv.*`, pas de `io`/`os`/`debug`/`package`.

## Ecran FLAM

- Resolution : 320x240 pixels
- Facteur d'upscale desktop : x3 (fenetre 960x720 au demarrage, redimensionnable, ratio 4:3)
- Header bar : 28px en haut (titre de l'histoire)
- Zone de contenu (`window`) : 320x212 pixels
- Couleur : 32-bit ARGB
- Heap LVGL : 512 Ko (`LV_MEM_SIZE (512U * 1024U)` dans `libs/lv_conf.h`, allocateur interne
  `LV_MEM_CUSTOM 0`)

**Attention compatibilite** : la taille du tas LVGL du vrai dispositif n'a pas ete mesuree, et
l'etat Lua du simulateur n'a pas de plafond memoire. Le simulateur peut donc accepter une
histoire qui manquerait de memoire sur le device.
Tas LVGL plein : les constructeurs et les `set_*` qui allouent levent l'erreur Lua
"tas LVGL epuise" (garde des bindings, marge 32 Ko) au lieu de planter dans LVGL.
Polices : les fontes compilees ne couvrent que U+0020-007F et U+00A0-00FF (telmi2flam
translittere les autres caracteres a la conversion).

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

`patches/lvgl-null-alloc.patch` (a appliquer sur le fork, procedure dans `patches/README.md`,
**non applique**) corrige a la source les `lv_mem_realloc` non testes de
`lv_obj_class_create_obj` et `lv_obj_add_event_cb`, qui plantent quand le tas est plein.

## Vulnerabilites connues (etude de securite)

### Use-After-Free (CWE-416) — corrige
Historique : `lv_obj_del()` liberait la memoire C sans invalider le userdata Lua.
Desormais (`src/bindings/lua_lv_obj.c`, en-tete "Duree de vie des objets") le userdata est mis
a `NULL` a la suppression (`LV_EVENT_DELETE`, enfants compris) et `lua_lv_check_obj()`
(`src/bindings/lua_lv.h`) leve l'erreur Lua `bad argument #n (lv object deleted)`.
`lv.obj.del` / `lv.obj.clean` sur un objet deja supprime sont des no-op.
`tests/lua/poc_use_after_free.lua` verifie ce comportement.

### Autres correctifs (tests `tests/lua/regress_*.lua`)
- Styles, images et callbacks ancres a leur objet : plus de liberation par le GC Lua pendant
  qu'ils sont affiches. Timers one-shot et animations surs (double free, UAF, fuite de refs).
- Zip-slip et debordement de tas a l'extraction des `.plain.pk` (`src/formats/pk_reader.c`).
- Cle de progression filtree (pas de traversee de chemin dans `saves/`).
- Etat audio remis a zero avant `lua_close` (la relance d'une histoire corrompait le registre).

### Integer Overflow dans lv_txt.c
`libs/lvgl/src/misc/lv_txt.c` (`lv_txt_get_size`) : dans la version LVGL utilisee, le calcul
de hauteur de texte est protege par un test d'overflow (`LV_LOG_WARN` puis retour).

## Notes techniques

- L'allocateur LVGL (`lv_mem`) est LIFO best-fit : un bloc libere est reutilise par la prochaine allocation de meme taille
- `lv_label_t` etend `lv_obj_t` avec un champ `char *text` — c'est un pointeur vers un buffer separe
- (Historique, avant le correctif UAF) pour le heap spray : creer des labels (pas des obj generiques) pour que le bloc libere soit reutilise par un autre label
- Les styles (`lv.style.new()`) ne sont pas des `lv_obj_t` — ils ont leur propre metatable et taille d'allocation
- L'encodeur (molette FLAM) est emule via les fleches clavier gauche/droite + Entree/Espace

## Fichiers batch utiles

| Fichier | Usage |
|---------|-------|
| `do_build.bat` | Configure (si besoin) et compile le player dans `build\` |
| `do_build.bat tests` | Compile les tests (`-DBUILD_TESTS=ON`, cibles `flam-test`, `flam-test-audio`, `flam-player`) |
| `manual_build.bat` | Alias de `do_build.bat` (memes arguments) |
| `run.bat [dossier]` | Lance `build\flam-player.exe` (`--scan-dir` si un dossier est donne), logs `build\stdout.log` / `build\stderr.log` |
| `test_run.bat [strict]` | Les 6 etapes de test : suite, moteur, audio reel, isoles, convertisseur, bout en bout (voir Tests) |
