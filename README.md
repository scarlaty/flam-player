# Flam Player

Emulateur desktop du firmware Flam (Lunii v3) capable d'executer les histoires `.plain` extraites,
et convertisseur d'histoires TELMI vers FLAM (`tools/telmi2flam`).

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
- Mode `--strict` proche du device et watchdog contre les boucles Lua infinies

## Contenu du depot

| Dossier | Role |
|---------|------|
| `src/` | Emulateur (voir Architecture) |
| `tools/telmi2flam/` | Convertisseur d'histoires TELMI (`.zip`) vers FLAM (`.plain.pk`) — voir [son README](tools/telmi2flam/README.md) |
| `games/casse-briques/` | Jeu casse-briques en contenu Lua FLAM (avec verrou parental) |
| `tests/` | Tests : suite Lua, moteur telmi2flam, audio reel, non-regression, bout en bout |
| `patches/` | Correctifs pour le fork LVGL (non appliques automatiquement) — voir [patches/README.md](patches/README.md) |

## Architecture

- [**LVGL 8.3**](https://github.com/lvgl/lvgl) — moteur de rendu UI (identique au firmware reel)
- [**Lua 5.4.8**](https://www.lua.org/) — runtime des scripts d'histoires
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
  test_main.c               Runner de tests headless (flam-test, flam-test-audio)
  test_headless_driver.c/h  Display driver sans fenetre
  test_audio_stub.c/h       Stub audio (flam-test ; flam-test-audio utilise le vrai sdl_audio.c)
  lua/
    test_helpers.lua         Framework de test (test/expect_eq/expect_true/expect_error)
    test_*.lua               Tests unitaires et d'integration (suite principale)
    regress_*.lua            Tests de non-regression (lances isolement)
    poc_*.lua, fuzz_*.lua    PoCs de securite et fuzzer (lances isolement)
  engine/test_*.lua         Moteur et modules runtime de telmi2flam (avec mocks)
  audio/                    Audio reel (pilote SDL dummy) : seek, pause, reouverture du lua_State
  e2e/e2e_telmi.py          Bout en bout : paquet TELMI -> telmi2flam -> flam-player pilote
```

## Dependances externes

Toutes les dependances sont gerees comme des **submodules git** (voir `.gitmodules`),
compilees depuis les sources lors du build (aucun binaire a placer manuellement).

| Bibliotheque | Version | Submodule | Lien |
|-------------|---------|-----------|------|
| SDL2 | branche `SDL2` (2.x) | `libs/SDL2` | [libsdl-org/SDL](https://github.com/libsdl-org/SDL) |
| Lua | 5.4.8 (tag `v5.4.8`, branche `v5.4`) | `libs/lua` | [lua/lua](https://github.com/lua/lua) |
| LVGL | 8.3 (`release/v8.3` + 1 commit) | `libs/lvgl` | fork [scarlaty/lvgl](https://github.com/scarlaty/lvgl) de [lvgl/lvgl](https://github.com/lvgl/lvgl) |
| minimp3 | `master` | `libs/minimp3` | [lieff/minimp3](https://github.com/lieff/minimp3) |

`libs/lv_conf.h` (config LVGL personnalisee, tas LVGL de 512 Ko) est versionne directement
dans le depot. Le fork LVGL ajoute un seul commit a `release/v8.3` : garde contre la division
par zero de `lv_bar` quand `min == max`, et traces `[MEM FAIL]` (details dans `AGENT.md`).
`LV_ASSERT_HANDLER` appelle `flam_assert_crash` (`src/firmware/fw_globals.c`), qui logue
fichier:ligne, la traceback Lua et le tas LVGL, puis fait `abort()` au lieu de boucler.

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
do_build.bat tests      :: compile flam-test.exe, flam-test-audio.exe et flam-player.exe
```

`do_build.bat` localise Visual Studio (via `vswhere`), charge l'environnement compilateur
x64 (`vcvarsall.bat`), trouve `cmake` et `ninja`, puis lance le build. Aucune configuration
manuelle ; les chemins sont resolus automatiquement.

`manual_build.bat` est un simple alias de `do_build.bat` (memes arguments).

### Windows — manuel

Charger d'abord l'environnement MSVC x64 (`vcvarsall.bat`, ou un *x64 Native Tools Command
Prompt*), sinon `cl.exe` ne trouve pas `stddef.h` :

```bat
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat" x64
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build build
```

Ajouter `-DBUILD_TESTS=ON` pour construire aussi `flam-test.exe` et `flam-test-audio.exe`, et
`-DCMAKE_MAKE_PROGRAM=<chemin\ninja.exe>` si `ninja` n'est pas dans le PATH.

### Tests

```bat
do_build.bat tests      :: compile flam-test.exe, flam-test-audio.exe, flam-player.exe
test_run.bat            :: lance les 6 etapes ci-dessous
test_run.bat strict     :: idem, et un echec d'un test isole compte aussi
```

`test_run.bat` enchaine ces etapes puis affiche un recapitulatif (`RESULTAT : OK` si tout passe) :

| Etape | Contenu |
|-------|---------|
| 1. Suite | `tests\lua\test_*.lua`, un seul processus (flam-test) |
| 2. Moteur/runtime | `tests\engine\test_*.lua` : moteur et modules runtime de telmi2flam |
| 3. Audio reel | `tests\audio\` avec flam-test-audio (vrai `sdl_audio.c`, pilote SDL dummy) |
| 4. Tests isoles | `regress_*.lua`, `poc_*.lua`, `fuzz_*.lua`, un processus chacun (timeout 30 s ; 1 = erreur Lua, 3 = timeout, autre = crash) |
| 5. Convertisseur | `tools\telmi2flam\tests` (python unittest) ; saute si python ou Pillow absent |
| 6. Bout en bout | `tests\e2e\e2e_telmi.py` ; saute si python ou flam-player.exe absent |

- Les echecs de l'etape 4 ne comptent qu'en mode `strict` ; en `strict`, un
  `flam-test-audio.exe` absent est aussi un echec.
- `FLAM_BUILD_DIR` : autre dossier de build que `build\`. `FLAM_PYTHON` : interpreteur python.
  `FLAM_LUA` : Lua 5.4 autonome (verification de `nodes.lua` par les tests du convertisseur).
- La capture d'ecran automatique du player est desactivee pendant les tests.

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

# Charger une histoire directement (dossier .plain ou archive .plain.pk)
flam-player.exe <chemin/vers/histoire.plain>
```

Options :

| Option | Effet |
|--------|-------|
| `--scan-dir <dossier>` | Dossier scanne par le navigateur |
| `--strict` | Plus proche du device : `require()` limite a `script/`, image `.lif` absente = erreur |
| `--watchdog <ms>` | Interrompt un script Lua qui ne rend pas la main (defaut 10000, `0` = desactive) |
| `--screenshot <chemin>` | Fichier BMP des captures (prioritaire sur `FLAM_SCREENSHOT`) |
| `--img-dir`, `--sounds-dir`, `--save-dir` | Dossiers images, sons et sauvegardes (mode script `.lua` direct) |

Le navigateur detecte les dossiers `.plain` et les archives `.plain.pk`, affiche les vignettes et titres, et offre un bouton "Choisir un dossier..." pour changer le repertoire de recherche. Les archives `.plain.pk` sont extraites automatiquement au premier lancement.

### Controles clavier

| Touche | Action |
|--------|--------|
| Fleches gauche/droite | Navigation |
| Entree / Espace | Valider |
| Echap | Retour / Revenir au navigateur |
| M | Menu contextuel |
| P | Pause / reprise de l'audio en cours |
| S | Screenshot (sauvegarde `screenshot.bmp`) |

Captures d'ecran : fichier BMP 320x240 fidele a l'ecran. Chemin par defaut code en dur
`C:/temp/flam-player/screenshot.bmp` ; `--screenshot <chemin>` (prioritaire) ou la
variable `FLAM_SCREENSHOT=<chemin>` le changent. Une capture automatique
est aussi prise 12 s apres le demarrage : `FLAM_SCREENSHOT_AUTO_MS=<ms>` change
ce delai, `FLAM_SCREENSHOT_AUTO_MS=0` la desactive.

## Format .plain / .plain.pk

Les histoires `.plain` sont des dossiers (ou archives ZIP `.plain.pk`) contenant :
- `main.lua` — point d'entree Lua
- `script/` — modules Lua (UI, navigation, logique)
- `img/` — images au format LIF (dont `thumbnail.lif` pour la vignette)
- `sounds/` — audio MP3 + fichiers `.mp3map` (index de seek)
- `info.plain` — metadonnees (titre, auteur, description, age)
- `uuid.bin` — identifiant unique (16 octets)

Les fichiers `.plain.pk` sont des archives ZIP (store, sans compression) generees par des outils de
sauvegarde Lunii ou par `tools/telmi2flam`. L'extraction refuse les chemins dangereux (`..`,
chemins absolus) et les tailles incoherentes.

## Convertir une histoire TELMI

```bat
python tools\telmi2flam\telmi2flam.py "Mon histoire.zip" -o "Mon_histoire.plain.pk"
python tools\telmi2flam\validate.py "Mon_histoire.plain.pk"
```

Prerequis : python 3 et Pillow. Seul le format TELMI (`nodes.json`) est pris en charge, pas le
format STUdio (`story.json`). Options (`--plain`, `--selector`, `--keep-size`, `--allow-missing`...)
et details : [tools/telmi2flam/README.md](tools/telmi2flam/README.md).

## Licence

Ce projet est a usage personnel et educatif.
