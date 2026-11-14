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

L'installateur de Flam Launcher (`Flam-Launcher_<version>_x64-setup.exe` dans une version
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
| `--watchdog <ms>` | Interrompt un script Lua qui ne rend pas la main, même enveloppé dans `pcall` ou réparti sur des coroutines ; réarmé à chaque tour de la boucle principale, donc les callbacks suivant un callback interrompu dans le même tour échouent aussi (défaut 10000, `0` = désactivé) |
| `--lua-mem <Mo>` | Plafond mémoire de l'état Lua (défaut 256, `0` = aucun, sinon 8 à 16384 ; une valeur invalide est refusée, code de sortie 2) ; au-delà, Lua lève `not enough memory`, rattrapable seulement dans le script (l'hôte dispose d'une petite réserve hors script) |
| `--unsafe-libs` | **Développement seulement** : rouvre `io`, `os`, `dofile`, `loadfile`, `package.loadlib` (code natif), `string.dump` et le bytecode aux scripts, ce que l'appareil n'expose pas ; avertissement sur stderr ; jamais par défaut, et le launcher ne le passe pas. `debug` reste retiré. Ne jamais lancer une histoire inconnue avec cette option |
| `--screenshot <chemin>` | Fichier BMP des captures (prioritaire sur `FLAM_SCREENSHOT`) |
| `--img-dir`, `--sounds-dir`, `--save-dir` | Dossiers images, sons et sauvegardes (mode script `.lua` direct) |

#### Bac à sable Lua (#575)

Décision : le player n'offre pas aux scripts plus que l'appareil, pour qu'une histoire malveillante
lancée dans le player n'agisse pas sur le poste (fichiers, processus, code natif). Le blocage vaut
**dans tous les modes**, avec ou sans `--strict` : le launcher lance le player sans `--strict`
(`--stdin-control`, `--parent-hwnd`, chemin de l'histoire) ; aucun script du dépôt n'a besoin de ce qui
est retiré. Seule exception, `--unsafe-libs` (développement, avertissement sur stderr).

Référence (ce que l'appareil expose) et état du player. « Non documenté » : le dépôt ne dit rien, rien
n'est deviné ; à essayer sur un appareil réel.

| Élément | Appareil (source) | Player par défaut | Player `--unsafe-libs` |
|---------|-------------------|-------------------|------------------------|
| `io` | `nil` (`docs/security/security-audit-report.md` §2, §4.2 ; `skills/flam-homebrew/references/device-pitfalls.md` piège 12) | `nil` (dans `_G` et `package.loaded`) | complet |
| `os` (dont `os.time`, `os.clock`, `os.date`) | `nil` (rapport §2, §4.2 : aucune de ses fonctions n'est citée ; piège 12). Le rapport §3.3 relève une globale `current_time` à l'appareil, sans la décrire | `nil` : un script qui lit l'heure par `os.time()` échoue comme sur l'appareil (aucun script du dépôt n'en a besoin) | complet |
| `debug` | `nil` (rapport §2) | `nil` (toujours, y compris `--unsafe-libs` : `debug.sethook` retirerait le watchdog) | `nil` |
| `package` | rapport §2 : `nil` ; mais le runtime éprouvé sur appareil écrit `package.loaded[nom] = nil` et lit `package.loaded` (`tools/telmi2flam/runtime/script/global.lua`, `lua-api.md` §3) : contradiction, `package.loaded` existe en pratique | table réduite à `loaded`, `preload`, `searchers` (+ `config`) ; **sans** `loadlib`, `cpath`, `path`, `searchpath` | complète |
| `require` | relatif à `script/` (rapport §2) ; l'appareil accepte `require("../main")` (rapport §3.2, vulnérabilité confirmée, recommandation §8 : la filtrer) | `package.preload` puis chercheur de l'hôte (texte seulement, `script/` et, hors `--strict`, la racine de l'histoire) ; `..`, chemin absolu, `:` et noms de périphériques refusés : plus strict que l'appareil, volontairement | + chercheurs de fichier et de bibliothèque C de Lua |
| `package.loadlib`, chercheurs C | non documenté (le rapport dit `package` = nil) | absents : aucun code natif chargeable | présents (charge une DLL sous Windows) |
| `load` | non documenté | présent, **texte seulement** : le mode est forcé à `"t"` même si le script passe `"b"` ou `"bt"` (un morceau binaire donne `nil, "attempt to load a binary chunk (mode is 't')"`) ; l'environnement (4e argument) passe tel quel | binaire accepté |
| `loadstring` | n'existe pas en Lua 5.4 | absent | absent |
| `loadfile`, `dofile` | `nil` (rapport §4.2 : « No dofile, no loadfile ») | `nil` | présents, lisent n'importe quel fichier, y compris du binaire |
| `string.dump` | non documenté | `nil` (sans objet une fois `load` limité au texte) | présent |
| chargeurs de l'hôte (`main.lua`, `require`, `load_lua_table` des sauvegardes) | n.a. | texte seulement (déjà le cas) | idem |
| `lv.img_src.load`, `audio.load` | audio : `audio.load(1, "../../etc/library/list")` échoue sans fuite (rapport §4.3) ; images : non documenté | **mode histoire** (dossier `.plain` ou `.plain.pk`) : chemin absolu, `..`, lecteur `X:`, antislash de tête, noms de périphériques Windows (`CON`, `NUL`, `COM1`...) et composants terminés par un point ou une espace refusés comme un fichier absent (`nil` ou erreur en `--strict` pour une image ; `-1` pour un son). **Script `.lua` lancé directement** : chemins pris tels quels (les tests passent des chemins absolus). Le mode se déduit de la cible, sans tenir compte de la casse (ASCII) : fichier se terminant par `.plain.pk` (`Evil.Plain.Pk` aussi, comme le launcher) ou dossier contenant `main.lua` = histoire (confinée) ; fichier se terminant par `.lua` = script direct ; toute autre cible est refusée (message sur stderr, code de sortie 2) | idem |
| `coroutine`, `string`, `table`, `math`, `utf8`, `pcall`/`xpcall`, `select`, `rawget`/`rawset`/`rawequal`/`rawlen`, `next`, `pairs`, `tostring`... | `ipairs`, `tostring`, `pairs`, `string`, `math`, `table`, `math.random` cités (`lua-api.md` §1, §12) ; le reste non documenté | présents, inchangés | idem |
| `getmetatable`, `setmetatable` | `setmetatable` utilisé par le runtime (`button-theme.lua`) ; `getmetatable` non documenté | présents, inchangés | idem |
| `collectgarbage` | `"collect"` et `"count"` utilisés par le runtime ; les autres options non documentées | toutes les options présentes ; `"stop"` ne contourne pas `--lua-mem` | idem |
| `print`, `warn`, `error`, `assert`, `select`, `_G`, `_VERSION` | `print` : sortie série (rapport §2) ; le reste non documenté | présents | idem |

Les erreurs sont celles de Lua devant un `nil` : `attempt to index a nil value (global 'io')`,
`attempt to call a nil value (global 'dofile')`, `attempt to call a nil value (field 'loadlib')` ; aucun
message inventé. Les noms sont retirés de `_G` ET de `package.loaded` : `require("io")`,
`package.loaded.io`, `_G.io`, `load("return io")()`, une coroutine ou la métatable des chaînes
(`getmetatable("").__index.dump`) ne retrouvent rien. Tests : `tests/e2e/e2e_sandbox.py` (un script par
voie d'évasion, rien créé ni modifié hors du dossier de l'histoire, player vivant puis arrêt propre).

**Plus permissif que l'appareil, volontairement ou faute de savoir** : `load` (texte), `string`,
`collectgarbage` (toutes options), `getmetatable`, `coroutine` et le reste de la bibliothèque de base,
dont l'appareil n'a pas documenté le contenu exact ; `package.loaded`, `preload` et `searchers` (le
runtime en dépend) ; `lua_lv`/`audio`/`progression` exposent des fonctions de l'émulateur qui ne sont
pas toutes vérifiées sur l'appareil. À essayer sur un appareil réel, une histoire qui affiche
`type(load)`, `type(getmetatable)`, `type(package)`, `type(package.loaded)`, `type(string.dump)`,
`type(collectgarbage)`, `type(coroutine)`, `type(rawget)` et le résultat de `collectgarbage("stop")` :
aligner ensuite le player sur la réponse.

**Ce que le bac à sable ne couvre PAS** : les finaliseurs `__gc` (Lua coupe les hooks, une boucle y est
infinie ; le comportement de l'appareil est inconnu) ; le temps passé dans une fonction C (motif de
`string.find` catastrophique) ; les défauts de mémoire de l'interpréteur (Lua 5.4.8) et des bindings
`lv.*`/`audio.*` (les audits de l'appareil en ont trouvé, dont l'utilisation après libération) ; les
liens symboliques et jonctions placés dans le dossier d'une histoire (via le launcher, la copie de travail ignore liens et jonctions : le trou ne concerne que l'appel direct en ligne de commande sur un dossier ; l'extraction des `.plain.pk` les
refuse, un dossier `.plain` d'auteur est joué tel quel) ; les scripts lancés directement (`flam-player
script.lua`) pour les chemins d'images et de sons. `debug` mis à part, `--unsafe-libs` n'a aucune
protection : c'est le comportement d'avant #575.

Limites du watchdog, non corrigées (corriger demanderait de modifier Lua lui-même, sous-module) :
Lua coupe les hooks pendant l'exécution d'un finaliseur, donc une boucle dans un `__gc` n'est jamais
interrompue ; une fonction C longue (motif de `string.find`/`match`/`gsub` à retour arrière
catastrophique) n'est pas interrompue non plus. Le plafond `--lua-mem` borne seulement la mémoire
(`string.rep` ou `table.concat` géants), pas le temps. Si l'appareil refuse `__gc` sur les tables des
histoires, le player pourrait le refuser de même (non vérifié sur l'appareil).

Sans périphérique audio (session sans son, machine virtuelle) ou si l'envoi du son échoue, la lecture
est cadencée sur l'horloge murale : sans cela, l'horloge audio, donc les callbacks `play`/`stop` et
les enchaînements, courait environ dix fois trop vite. `FLAM_AUDIO_NO_DEVICE=1` simule ce cas et
`FLAM_AUDIO_QUEUE_FAIL=1` fait échouer l'envoi du son : ce sont des aides de test lues par le
binaire, seule la valeur exacte `1` compte (`0` ou toute autre valeur est sans effet).

Réserve mémoire : au-delà du plafond `--lua-mem`, l'hôte (le player, hors exécution d'un script)
peut encore allouer jusqu'à 4 Mo pour ses propres appels, afin qu'un script ayant rempli la mémoire
ne fasse pas paniquer l'état Lua. Le test vérifie que la réserve est refusée au script ; l'autre sens
(la réserve effectivement accordée à l'hôte) n'est pas testé. Le plancher de `--lua-mem` est de 8 Mo :
un parcours complet d'une histoire convertie y tient (`tests/e2e/e2e_telmi.py`, cas B8).

Arguments non ASCII (Windows) : SDL2main donne `argv` en UTF-8, le player le reconvertit en page de code
ANSI dans un tableau local (le `argv` de SDL2main n'est jamais modifié). Un nom représentable dans la
page ANSI (`Café.lua`, `Histoire été.plain.pk`) est repris tel quel. Un nom contenant un caractère absent
de la page ANSI (`Histoire✓.plain.pk`) est remplacé par son nom court 8.3 (`HISTOI~1.PK`) : l'archive
est reconnue comme `.plain.pk`, extraite dans `HISTOI~1.plain` (nom court sans `.pk`, plus `.plain` : `HISTOI~1`
seul serait un nom court, résolu par Windows vers un dossier voisin comme `Histoires`, qui serait écrasé ; un
dossier existant dont le nom réel diffère est refusé) et ses sauvegardes vivent sous `saves/HISTOI~1.plain`,
pas sous le nom d'origine. Cela exige que les noms courts soient activés sur
le volume ; sinon la cible est introuvable. Via le launcher (copie de travail vidée avant chaque lancement,
un seul fichier à côté de `saves/`), le nom court est stable d'un lancement à l'autre, donc la progression
aussi. Le launcher ne renomme volontairement pas la copie : la progression est rangée sous un nom dérivé
du nom de la cible, un renommage ferait perdre celle des histoires déjà jouées.
Test : `tests/e2e/e2e_argv_unicode.py`.

Le navigateur détecte les dossiers `.plain` et les archives `.plain.pk`, affiche les vignettes et
les titres, et offre un bouton « Choisir un dossier... » pour changer le répertoire de recherche.
Les archives `.plain.pk` sont extraites automatiquement au premier lancement, dans le dossier
`.plain` voisin. Le player y pose des témoins : `.extracting` pendant l'extraction, remplacé par
`.extracted` (taille et date du `.pk`, puis la liste des fichiers extraits) quand elle est complète.
L'extraction n'est complète que si l'archive va jusqu'à son répertoire central : une signature
inconnue, une archive tronquée ou une entrée à data descriptor la laissent incomplète (trace
`[PK]` sur stderr, `.extracting` conservé, donc nouvelle tentative au lancement suivant) lorsque le
dossier n'avait pas encore d'extraction du player. Si le dossier porte déjà une extraction (témoin
présent), une passe à blanc du `.pk` précède toute purge : une archive incomplète, à data descriptor
ou dont `main.lua` n'est pas stocké (ou de taille 0) est refusée d'emblée (trace « mise à jour
refusée »), rien n'est purgé et l'ancienne version est jouée. Un `.extracted` qui n'a pas pu être
écrit laisse aussi l'extraction à refaire. Le player ré-extrait si `main.lua` manque, si
`.extracting` reste (extraction interrompue) ou si `.extracted` ne correspond plus au `.pk`.
Avant de ré-extraire un dossier qui porte un témoin, il supprime les fichiers de l'ancienne
extraction (liste du témoin) pour qu'un fichier disparu du `.pk` ne survive pas ; les fichiers
ajoutés à la main restent, rien n'est supprimé à travers un lien ou une jonction, et un dossier
extrait par une version plus ancienne (témoin sans liste) n'est pas purgé.
Un dossier `.plain` avec `main.lua` et sans aucun témoin est un dossier d'auteur : il est joué tel
quel, jamais écrasé ni purgé par le `.pk` voisin (trace sur stderr, y compris quand le navigateur
masque le `.pk`). Cas d'une extraction interrompue par une version plus ancienne, qui laissait un
dossier partiel avec `main.lua` et sans témoin : il passe pour un dossier d'auteur ; supprimer le
dossier pour forcer la ré-extraction. Pour adopter un dossier extrait comme dossier d'auteur,
supprimer ses deux témoins (`.extracted` et `.extracting`). Si l'extraction est impossible (`.pk`
supprimé, illisible, sans `main.lua` ou remplacé par un fichier qui n'est pas un ZIP), rien n'est
lu ni supprimé dans le dossier : il est joué tel quel (trace « mise à jour refusée ou extraction
impossible » ou, pour un `.pk` absent ou illisible, « .pk absent ou illisible » ; « à jour, non
extrait » est réservé au cas où le `.pk` est lisible et correspond). Un `.pk` ne peut ni écrire ni forger les témoins (noms `.extracted` /
`.extracting`, et leur alias court `EXTRAC~N`, refusés pour tous les composants, sans tenir compte de
la casse, des points et espaces finaux). Sont aussi refusés les composants `.` (comme `..`), ceux qui
finissent par un point ou une espace, les caractères `: < > " | ? *` et de contrôle, et les noms de
périphériques Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`9`, `LPT1`-`9`, exposants compris, avec ou
sans extension). Refuser `con.txt` ou `nul.txt` est un choix conservateur : ce sont des fichiers
ordinaires sous Windows 11, mais des périphériques sous les versions antérieures. Limites connues : un `.pk` produit par le `tar.exe` de Windows (« data descriptor » sur chaque entrée)
n'a jamais été extractible et reste refusé (trace « illisible, incomplet ou sans main.lua ») ;
un `.pk` verrouillé par un autre programme (ouverture impossible) mais à jour est traité comme
absent ou illisible (trace « .pk absent ou illisible ») ; les fichiers « à la demande »
d'un dossier OneDrive (attribut de point d'analyse) ne sont pas supprimés lors d'une purge (fuite
sans danger) ; un nom court 8.3 haché (`EX577D~1`, quand `EXTRAC~1` à `~4` sont pris par d'autres
fichiers) ne peut pas écraser `.extracting` (verrou d'écriture), mais l'extraction reste incomplète et
recommence à chaque lancement. Le navigateur affiche au plus 32 histoires ; les suivantes sont
comptées (« N histoires non affichées », à l'écran et sur stderr).

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
Il lit les ZIP sans compression, deflate, bzip2, LZMA ou Zstandard, et les images PNG (y
compris atypiques), GIF et JPEG ; une image d'un autre format (BMP...) ou un JPEG arithmétique
ou sans perte n'est pas pris en charge : l'archive est refusée avec un message explicite. Les options sont celles de l'ancienne version Python
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
