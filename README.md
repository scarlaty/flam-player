# Flam Player

Emulateur desktop du firmware Flam (Lunii v3) capable d'executer les histoires `.plain` extraites.

## Fonctionnalites

- Navigateur d'histoires avec vignettes et selection de dossier (dialog natif Windows)
- Support des archives `.plain.pk` (extraction automatique)
- Barre de titre avec le nom de l'histoire en cours
- Fenetre redimensionnable avec ratio 4:3 preserve
- Sauvegarde/restauration de la progression entre les sessions
- Retour au navigateur d'histoires avec ESC (sans quitter l'application)
- Menu contextuel (touche M)
- Audio MP3 avec seek et callbacks
- Navigation encodeur fidele au firmware reel (LEFT/RIGHT/ENTER)

## Architecture

- [**LVGL 8.3**](https://github.com/lvgl/lvgl) — moteur de rendu UI (identique au firmware reel)
- [**Lua 5.4**](https://www.lua.org/) — runtime des scripts d'histoires
- [**SDL2**](https://github.com/libsdl-org/SDL) — fenetre, evenements clavier, sortie audio
- [**minimp3**](https://github.com/lieff/minimp3) — decodage MP3

```
src/
  main.c                    Point d'entree, navigateur, chargement d'histoire
  bindings/
    lua_lv.c/h              Bindings Lua <-> LVGL (table `lv`)
    lua_lv_obj.c            Objets: obj, btn, label, img, slider, arc
    lua_lv_style.c          Styles LVGL
    lua_lv_event.c          Evenements, timers et animations
  firmware/
    fw_globals.c/h          Objets firmware (state, progression, context_menu, screen)
  formats/
    lif_decoder.c/h         Decodeur d'images LIF -> LVGL img_dsc
    mp3map_parser.c/h       Parser mp3map pour le seek audio
    pk_reader.c/h           Lecteur d'archives .plain.pk (ZIP store)
  platform/
    sdl_driver.c/h          Driver SDL2 pour LVGL (display + input encodeur)
    sdl_audio.c/h           Lecture audio MP3 via SDL2
  fonts/
    nunito_*.c              Polices Nunito (Bold/ExtraBold, 12-20px)
tests/
  test_main.c               Runner de tests headless
  test_headless_driver.c/h  Display driver sans fenetre
  test_audio_stub.c/h       Stub audio pour les tests
  lua/
    test_helpers.lua         Framework de test (test/expect_eq/expect_true/expect_error)
    test_*.lua               Tests unitaires et d'integration (~280 tests)
```

## Dependances externes

Toutes les dependances sont gerees comme des **submodules git** (voir `.gitmodules`),
compilees depuis les sources lors du build (aucun binaire a placer manuellement).

| Bibliotheque | Version | Submodule | Lien |
|-------------|---------|-----------|------|
| SDL2 | branche `SDL2` (2.x) | `libs/SDL2` | [libsdl-org/SDL](https://github.com/libsdl-org/SDL) |
| Lua | 5.4.8 | `libs/lua` | [lua/lua](https://github.com/lua/lua) |
| LVGL | 8.3 (`release/v8.3`) | `libs/lvgl` | [lvgl/lvgl](https://github.com/lvgl/lvgl) |
| minimp3 | `master` | `libs/minimp3` | [lieff/minimp3](https://github.com/lieff/minimp3) |

`libs/lv_conf.h` (config LVGL personnalisee) est versionne directement dans le depot.

## Compilation

**Prerequis** : Visual Studio (2022 ou +) avec le composant *Desktop development with C++*
(`Microsoft.VisualStudio.Component.VC.Tools.x86.x64`), ainsi que **CMake** et **Ninja**
(fournis par Visual Studio ou via `pip install cmake ninja`).

```bash
# Recuperer les submodules (au clone ou apres coup)
git clone --recurse-submodules https://github.com/scarlaty/flam-player.git
# ou, sur un clone existant :
git submodule update --init --recursive
```

### Windows — script automatique (recommande)

```bat
do_build.bat            :: configure (si besoin) + compile flam-player.exe
do_build.bat tests      :: compile la suite de tests (flam-test.exe)
```

`do_build.bat` localise Visual Studio (via `vswhere`), charge l'environnement compilateur
x64 (`vcvarsall.bat`), trouve `cmake` et `ninja`, puis lance le build. Aucune configuration
manuelle ; les chemins sont resolus automatiquement.

### Windows — manuel (Developer Command Prompt for VS)

```bash
mkdir build && cd build
cmake .. -G Ninja
ninja
```

### Tests

```bat
do_build.bat tests      :: compile flam-test.exe
test_run.bat            :: execute tous les tests tests\lua\*.lua
```

## Utilisation

Via le script (chemins relatifs, logs dans `build\stdout.log` / `stderr.log`) :

```bat
run.bat                      :: scanne le dossier build (ex: Enquete.plain)
run.bat "D:\mes\histoires"   :: scanne un dossier specifique
```

Ou directement :

```bash
# Navigateur d'histoires (scan le dossier de l'executable)
flam-player.exe

# Scan un dossier specifique
flam-player.exe --scan-dir <chemin/vers/dossier>

# Charger une histoire directement
flam-player.exe <chemin/vers/histoire.plain>
```

Le navigateur detecte les dossiers `.plain` et les archives `.plain.pk`, affiche les vignettes et titres, et offre un bouton "Choisir un dossier..." pour changer le repertoire de recherche. Les archives `.plain.pk` sont extraites automatiquement au premier lancement.

### Controles clavier

| Touche | Action |
|--------|--------|
| Fleches gauche/droite | Navigation |
| Entree / Espace | Valider |
| Echap | Retour / Revenir au navigateur |
| M | Menu contextuel |
| S | Screenshot (sauvegarde `screenshot.bmp`) |

## Format .plain / .plain.pk

Les histoires `.plain` sont des dossiers (ou archives ZIP `.plain.pk`) contenant :
- `main.lua` — point d'entree Lua
- `script/` — modules Lua (UI, navigation, logique)
- `img/` — images au format LIF (dont `thumbnail.lif` pour la vignette)
- `sounds/` — audio MP3 + fichiers `.mp3map` (index de seek)
- `info.plain` — metadonnees (titre, auteur, description, age)
- `uuid.bin` — identifiant unique (16 octets)

Les fichiers `.plain.pk` sont des archives ZIP (store, sans compression) generees par des outils de sauvegarde Lunii.

## Licence

Ce projet est a usage personnel et educatif.
