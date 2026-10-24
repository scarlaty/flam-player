# flam-player, convertisseur et jeux

[![telmi2flam](https://github.com/scarlaty/flam-player/actions/workflows/telmi2flam.yml/badge.svg)](https://github.com/scarlaty/flam-player/actions/workflows/telmi2flam.yml)
[![telmi2flam-rs](https://github.com/scarlaty/flam-player/actions/workflows/telmi2flam-rs.yml/badge.svg)](https://github.com/scarlaty/flam-player/actions/workflows/telmi2flam-rs.yml)

Cette page regroupe la documentation technique des briques sur lesquelles repose
[Flam Launcher](../README.md) : l'émulateur `flam-player`, le convertisseur `telmi2flam`, les jeux
homebrew et les dépendances C. Elle reprend l'ancien README racine du dépôt.

Pour utiliser Flam Launcher sans rien compiler, voir l'[installation](../README.md#installation).

## Sommaire

- [Émulateur flam-player](#émulateur-flam-player)
- [Contenu du dépôt](#contenu-du-dépôt)
- [Architecture](#architecture)
- [Dépendances externes](#dépendances-externes)
- [Compilation](#compilation)
- [Utilisation](#utilisation)
- [Format .plain / .plain.pk](#format-plain--plainpk)
- [Convertir une histoire TELMI](#convertir-une-histoire-telmi)
- [Jeux et contenus homebrew](#jeux-et-contenus-homebrew)
- [Licence](#licence)

## Émulateur flam-player

Émulateur de bureau du firmware Flam (Lunii v3), capable d'exécuter les histoires `.plain`
extraites. Le convertisseur TELMI vers FLAM (`crates/telmi2flam`) a sa
[propre section](#convertir-une-histoire-telmi).

### Fonctionnalités

- Navigateur d'histoires avec vignettes et sélection de dossier (dialogue natif Windows)
- Prise en charge des archives `.plain.pk` (extraction automatique)
- Barre de titre avec le nom de l'histoire en cours
- Fenêtre redimensionnable avec ratio 4:3 préservé
- Sauvegarde et restauration de la progression entre les sessions
- Retour au navigateur d'histoires avec Échap (sans quitter l'application)
- Menu contextuel (touche M)
- Audio MP3 avec seek et callbacks
- Navigation encodeur fidèle au firmware réel (LEFT/RIGHT/ENTER)
- Mode `--strict` proche de l'appareil et watchdog contre les boucles Lua infinies

### Installer sans compiler (Windows)

L'installateur de Flam Launcher (`Flam-Launcher_<version>_x64-setup.exe` dans une pré-version
publiée, `Flam Launcher_<version>_x64-setup.exe` comme artefact de la CI) installe, **pour
l'utilisateur courant et sans droits d'administrateur**, l'application, `flam-player.exe` et le
convertisseur TELMI vers FLAM (binaire Rust autonome : ni Python ni Pillow à installer). Il n'est pas signé : SmartScreen affiche un
avertissement (« Informations complémentaires » puis « Exécuter quand même »). Utilisation,
sources, dépannage et construction de l'installateur :
[apps/flam-launcher/README.md](../apps/flam-launcher/README.md).

## Contenu du dépôt

| Dossier | Rôle |
|---------|------|
| `src/` | Émulateur (voir [Architecture](#architecture)) |
| `crates/telmi2flam/` | Convertisseur d'histoires TELMI (`.zip`) vers FLAM (`.plain.pk`) livré avec le launcher : binaire Rust, version 2.0.0 (workspace Cargo à la racine) |
| `tools/telmi2flam/` | Version Python d'origine du convertisseur, gardée comme référence de parité, avec ses outils (`validate.py`, `explore.py`, campagne de parité) — voir [son README](../tools/telmi2flam/README.md) |
| `apps/flam-launcher/` | Surcouche de bureau (Tauri 2 : Rust + React/TypeScript, licence GPL v3) pour parcourir, convertir et lancer des histoires — voir [son README](../apps/flam-launcher/README.md) ; installateur Windows (NSIS, non signé) avec player et convertisseur embarqués |
| `games/` | Jeux en contenu Lua FLAM (voir [Jeux et contenus homebrew](#jeux-et-contenus-homebrew)) |
| `skills/flam-homebrew/` | Skill IA pour fabriquer un homebrew `.plain.pk` |
| `tests/` | Tests : suite Lua, moteur telmi2flam, audio réel, non-régression, bout en bout |
| `patches/` | Correctifs pour le fork LVGL (non appliqués automatiquement) — voir [patches/README.md](../patches/README.md) |
| `docs/` | Formats de la Flam ([flam-device-format.md](flam-device-format.md), [flam-carrier.md](flam-carrier.md)), [security/](security/) (audit et inventaire d'un appareil), captures du README ([assets/](assets/)) |

## Architecture

- [**LVGL 8.3**](https://github.com/lvgl/lvgl) — moteur de rendu UI (identique au firmware réel)
- [**Lua 5.4.8**](https://www.lua.org/) — runtime des scripts d'histoires
- [**SDL2**](https://github.com/libsdl-org/SDL) — fenêtre, événements clavier, sortie audio
- [**minimp3**](https://github.com/lieff/minimp3) — décodage MP3

```
src/
  main.c                    Point d'entrée, navigateur, chargement d'histoire
  bindings/
    lua_lv.c/h              Bindings Lua <-> LVGL (table `lv`)
    lua_lv_obj.c            Objets : obj, btn, label, img, slider, arc
    lua_lv_style.c          Styles LVGL
    lua_lv_event.c          Événements, timers et animations
  firmware/
    fw_globals.c/h          Objets firmware (state, progression, context_menu, screen)
  formats/
    lif_decoder.c/h         Décodeur d'images LIF -> LVGL img_dsc
    mp3map_parser.c/h       Parser mp3map pour le seek audio
    pk_reader.c/h           Lecteur d'archives .plain.pk (ZIP store)
  platform/
    sdl_driver.c/h          Driver SDL2 pour LVGL (affichage + entrée encodeur)
    sdl_audio.c/h           Lecture audio MP3 via SDL2
  fonts/
    nunito_*.c              Polices Nunito (Bold/ExtraBold, 12-20 px)
tests/
  test_main.c               Runner de tests headless (flam-test, flam-test-audio)
  test_headless_driver.c/h  Driver d'affichage sans fenêtre
  test_audio_stub.c/h       Stub audio (flam-test ; flam-test-audio utilise le vrai sdl_audio.c)
  lua/
    test_helpers.lua         Framework de test (test/expect_eq/expect_true/expect_error)
    test_*.lua               Tests unitaires et d'intégration (suite principale)
    regress_*.lua            Tests de non-régression (lancés isolément)
    poc_*.lua, fuzz_*.lua    PoC de sécurité et fuzzer (lancés isolément)
  engine/test_*.lua         Moteur et modules runtime de telmi2flam (avec mocks)
  audio/                    Audio réel (pilote SDL dummy) : seek, pause, réouverture du lua_State
  e2e/e2e_telmi.py          Bout en bout : paquet TELMI -> telmi2flam -> flam-player piloté
```

## Dépendances externes

Toutes les dépendances sont gérées comme des **submodules git** (voir `.gitmodules`),
compilées depuis les sources lors du build (aucun binaire à placer manuellement).

| Bibliothèque | Version | Submodule | Lien |
|-------------|---------|-----------|------|
| SDL2 | branche `SDL2` (2.x) | `libs/SDL2` | [libsdl-org/SDL](https://github.com/libsdl-org/SDL) |
| Lua | 5.4.8 (tag `v5.4.8`, branche `v5.4`) | `libs/lua` | [lua/lua](https://github.com/lua/lua) |
| LVGL | 8.3 (`release/v8.3` + 1 commit) | `libs/lvgl` | fork [scarlaty/lvgl](https://github.com/scarlaty/lvgl) de [lvgl/lvgl](https://github.com/lvgl/lvgl) |
| minimp3 | `master` | `libs/minimp3` | [lieff/minimp3](https://github.com/lieff/minimp3) |

`libs/lv_conf.h` (configuration LVGL personnalisée, tas LVGL de 512 Ko) est versionné
directement dans le dépôt. Le fork LVGL ajoute un seul commit à `release/v8.3` : garde contre la
division par zéro de `lv_bar` quand `min == max`, et traces `[MEM FAIL]` (détails dans
[`AGENT.md`](../AGENT.md)). `LV_ASSERT_HANDLER` appelle `flam_assert_crash`
(`src/firmware/fw_globals.c`), qui journalise fichier:ligne, la traceback Lua et le tas LVGL,
puis fait `abort()` au lieu de boucler.

## Compilation

**Prérequis** : Visual Studio (2022 ou plus récent) avec le composant *Desktop development
with C++* (`Microsoft.VisualStudio.Component.VC.Tools.x86.x64`), ainsi que **CMake** et
**Ninja** (fournis par Visual Studio ou via `pip install cmake ninja`).

```bash
# Récupérer les submodules (au clone ou après coup)
git clone --recurse-submodules https://github.com/scarlaty/flam-player.git
# ou, sur un clone existant :
git submodule update --init --recursive
```

### Windows — script automatique (recommandé)

```bat
do_build.bat            :: configure (si besoin) + compile flam-player.exe
do_build.bat tests      :: compile flam-test.exe, flam-test-audio.exe et flam-player.exe
```

`do_build.bat` localise Visual Studio (via `vswhere`), charge l'environnement compilateur
x64 (`vcvarsall.bat`), trouve `cmake` et `ninja`, puis lance le build. Aucune configuration
manuelle : les chemins sont résolus automatiquement.

`manual_build.bat` est un simple alias de `do_build.bat` (mêmes arguments).

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
test_run.bat            :: lance les 6 étapes ci-dessous
test_run.bat strict     :: idem, et un échec d'un test isolé compte aussi
```

`test_run.bat` enchaîne ces étapes puis affiche un récapitulatif (`RESULTAT : OK` si tout passe) :

| Étape | Contenu |
|-------|---------|
| 1. Suite | `tests\lua\test_*.lua`, un seul processus (flam-test) |
| 2. Moteur/runtime | `tests\engine\test_*.lua` : moteur et modules runtime de telmi2flam |
| 3. Audio réel | `tests\audio\` avec flam-test-audio (vrai `sdl_audio.c`, pilote SDL dummy) |
| 4. Tests isolés | `regress_*.lua`, `poc_*.lua`, `fuzz_*.lua`, un processus chacun (timeout 30 s ; 1 = erreur Lua, 3 = timeout, autre = crash) |
| 5. Convertisseur | `tools\telmi2flam\tests` (python unittest, binaire `telmi2flam` requis) ; sauté si python absent |
| 6. Bout en bout | `tests\e2e\e2e_telmi.py` ; sauté si python ou flam-player.exe absent |

- Les échecs de l'étape 4 ne comptent qu'en mode `strict` ; en `strict`, un
  `flam-test-audio.exe` absent est aussi un échec.
- `FLAM_BUILD_DIR` : autre dossier de build que `build\`. `FLAM_PYTHON` : interpréteur python.
  `FLAM_LUA` : Lua 5.4 autonome (vérification de `nodes.lua` par les tests du convertisseur).
- La capture d'écran automatique du player est désactivée pendant les tests.

## Utilisation

Via le script (chemins relatifs, journaux dans `build\stdout.log` / `stderr.log`) :

```bat
run.bat                      :: scanne le dossier build (ex : Enquete.plain)
run.bat "D:\mes\histoires"   :: scanne un dossier spécifique
```

Ou directement :

```bash
# Navigateur d'histoires (scanne le dossier de l'exécutable)
flam-player.exe

# Scanne un dossier spécifique
flam-player.exe --scan-dir <chemin/vers/dossier>

# Charge une histoire directement (dossier .plain ou archive .plain.pk)
flam-player.exe <chemin/vers/histoire.plain>
```

Options :

| Option | Effet |
|--------|-------|
| `--scan-dir <dossier>` | Dossier scanné par le navigateur |
| `--strict` | Plus proche de l'appareil : `require()` limité à `script/`, image `.lif` absente = erreur |
| `--watchdog <ms>` | Interrompt un script Lua qui ne rend pas la main (défaut 10000, `0` = désactivé) |
| `--screenshot <chemin>` | Fichier BMP des captures (prioritaire sur `FLAM_SCREENSHOT`) |
| `--img-dir`, `--sounds-dir`, `--save-dir` | Dossiers images, sons et sauvegardes (mode script `.lua` direct) |

Le navigateur détecte les dossiers `.plain` et les archives `.plain.pk`, affiche les vignettes et
les titres, et offre un bouton « Choisir un dossier... » pour changer le répertoire de recherche.
Les archives `.plain.pk` sont extraites automatiquement au premier lancement.

### Contrôles clavier

| Touche | Action |
|--------|--------|
| Flèches gauche/droite | Navigation |
| Entrée / Espace | Valider |
| Échap | Retour / revenir au navigateur |
| M | Menu contextuel |
| P | Pause / reprise de l'audio en cours |
| S | Capture d'écran (enregistre `screenshot.bmp`) |

Captures d'écran : fichier BMP 320×240 fidèle à l'écran. Chemin par défaut codé en dur :
`C:/temp/flam-player/screenshot.bmp` ; `--screenshot <chemin>` (prioritaire) ou la variable
`FLAM_SCREENSHOT=<chemin>` le changent. Une capture automatique est aussi prise 12 s après le
démarrage : `FLAM_SCREENSHOT_AUTO_MS=<ms>` change ce délai, `FLAM_SCREENSHOT_AUTO_MS=0` la
désactive.

## Format .plain / .plain.pk

Les histoires `.plain` sont des dossiers (ou archives ZIP `.plain.pk`) contenant :

- `main.lua` — point d'entrée Lua
- `script/` — modules Lua (UI, navigation, logique)
- `img/` — images au format LIF (dont `thumbnail.lif` pour la vignette)
- `sounds/` — audio MP3 + fichiers `.mp3map` (index de seek)
- `info.plain` — métadonnées (titre, auteur, description, âge)
- `uuid.bin` — identifiant unique (16 octets)

Les fichiers `.plain.pk` sont des archives ZIP (store, sans compression) générées par des outils
de sauvegarde Lunii ou par `tools/telmi2flam`. L'extraction refuse les chemins dangereux (`..`,
chemins absolus) et les tailles incohérentes.

Format de la Flam elle-même (volume USB, index, chiffrement) :
[flam-device-format.md](flam-device-format.md) et [flam-carrier.md](flam-carrier.md).

## Convertir une histoire TELMI

Flam Launcher embarque le convertisseur : la conversion s'y fait à l'installation d'une
histoire, sans ligne de commande et sans rien installer d'autre.

Le convertisseur est le binaire Rust `telmi2flam` (`crates/telmi2flam`, version 2.0.0). Pour
l'utiliser seul, depuis la racine du dépôt (Rust stable requis) :

```bat
cargo run --release -p telmi2flam -- "Mon histoire.zip" -o "Mon_histoire.plain.pk"
```

Seul le format TELMI (`nodes.json`) est pris en charge, pas le format STUdio (`story.json`).
Il lit les ZIP sans compression ou en deflate et les images PNG, y compris atypiques ; les
images GIF ou JPEG et les ZIP compressés en bzip2, LZMA ou Zstandard ne sont pas pris en
charge : l'archive est refusée avec un message explicite. Les options sont celles de l'ancienne version Python
(`--plain`, `--selector`, `--keep-size`, `--allow-missing`...) : voir
[tools/telmi2flam/README.md](../tools/telmi2flam/README.md).

Python 3 (sans Pillow) ne reste nécessaire que pour les outils de développement de
`tools/telmi2flam/` : `validate.py`, `compat.py`, `explore.py`, parité du binaire avec les
références figées de l'ancienne version Python (supprimée, #140) et leurs tests.

## Jeux et contenus homebrew

Des contenus Lua FLAM, packagés comme des histoires (`.plain.pk`) :

| Jeu | Description |
|-----|-------------|
| [Casse-briques](../games/casse-briques/README.md) | Casse-briques adapté d'un jeu LÖVE2D, avec verrou parental |
| [Attrape-étoiles](../games/attrape-etoiles/README.md) | Petit jeu molette, fabriqué avec le skill `flam-homebrew` |
| [Capi et la rivière](../games/capi-riviere/README.md) | Course-esquive à la molette, fabriquée avec le skill `flam-homebrew` |

Le skill IA [`skills/flam-homebrew`](../skills/flam-homebrew/SKILL.md) guide la fabrication
d'un homebrew `.plain.pk` (Lua + LVGL, images LIF, audio MP3/mp3map) ; référence de l'API Lua :
[lua-api.md](../skills/flam-homebrew/references/lua-api.md).

## Licence

Ce projet (émulateur flam-player, convertisseur telmi2flam, flam-launcher, jeux et skills) est
distribué sous licence GNU GPL v3 ou ultérieure (`GPL-3.0-or-later`), voir
[`LICENSE`](../LICENSE). Les composants tiers gardent chacun leur licence.

L'installateur Windows de flam-launcher redistribue flam-player, le convertisseur telmi2flam
(binaire Rust, sans Python ni Pillow) et son runtime Lua (GPL v3, comme le reste du dépôt) ; les
licences de chaque composant tiers (SDL2, Lua, LVGL, minimp3, FFmpeg, code dérivé de Pillow,
redistribuables Microsoft) sont dans [`apps/flam-launcher/LICENSES.md`](../apps/flam-launcher/LICENSES.md).
