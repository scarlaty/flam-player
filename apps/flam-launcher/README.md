# flam-launcher

Surcouche de bureau (Windows d'abord) pour parcourir des sources d'histoires, les convertir
avec `telmi2flam` et les lancer dans `flam-player`. Application Tauri 2 : backend Rust,
interface React + TypeScript (strict). Licence **GPL v3** ou ulterieure (voir [LICENSE](../../LICENSE)).

Suivi : issue « Suivi : surcouche flam-launcher ».

Ce document décrit l'installation, l'utilisation, le dépannage puis l'architecture visée. Les éléments marqués _prévu_ n'existent pas encore
et arrivent avec les issues du jalon M3.

## Installation (Windows)

1. Télécharger l'installateur `Flam Launcher_<version>_x64-setup.exe` (artefact de la CI ou
   version publiée).
2. Le lancer : l'installation se fait **pour l'utilisateur courant**, sans droits
   d'administrateur (dossier par défaut `%LOCALAPPDATA%\Flam Launcher`).
3. **SmartScreen** : l'installateur n'est **pas signé**. Windows affiche « Windows a protégé
   votre ordinateur » : cliquer sur « Informations complémentaires » puis « Exécuter quand
   même ». Ce message disparaîtra si l'éditeur est un jour signé.
4. Lancer « Flam Launcher » depuis le menu Démarrer. WebView2 est requis (présent sur Windows
   11 et sur Windows 10 à jour).

L'installation contient tout : `flam-launcher.exe`, `flam-player.exe` et `SDL2.dll` (le
player), le convertisseur `telmi2flam/` avec son Python embarqué (aucun Python à installer) et
le runtime Lua du projet, ainsi que `LICENSES.md` et `licenses/`. Désinstaller : « Applications
installées » de Windows, ou `uninstall.exe` du dossier d'installation. Les données vivent hors
de ce dossier et survivent à une mise à jour ou une désinstallation : sources, paramètres et
sauvegardes de progression sous `%APPDATA%`, cache (catalogues, téléchargements et histoires
installées de « Mes histoires ») sous `%LOCALAPPDATA%` (« Paramètres » → « Cache » indique son
emplacement).

**Désinstallation et données** : la désinstallation retire le dossier d'installation seulement.
Les données (`%APPDATA%lam-launcher` : sources, paramètres, sauvegardes ; et
`%LOCALAPPDATA%lam-launcher` : cache et histoires installées) sont **conservées**, y compris si
l'on coche « supprimer les données de l'application » du désinstallateur, qui ne les connaît pas :
pour tout effacer, supprimer ces deux dossiers à la main.

## Ajouter des sources

Écran **Sources** (« Ajouter une source », puis « Ajouter un catalogue » ou « Ajouter un dossier ») :

- **Catalogue** : l'adresse (`https://…`) d'un catalogue au format Telmi Store (un JSON
  `{ "data": [ { uuid, title, download, … } ] }`). Les archives `.zip` Telmi qu'il liste sont
  téléchargées puis converties à l'installation. Le catalogue est gardé en cache : il reste
  consultable hors ligne.
- **Dossier local** : un dossier contenant des archives Telmi `.zip` (converties à
  l'installation) ou des histoires déjà converties `.plain` / `.plain.pk` (copiées telles
  quelles). Le dossier n'est jamais modifié.

Dans la bibliothèque d'une source, « Installer » télécharge et convertit (progression et
annulation possibles), puis « Jouer » lance l'histoire dans `flam-player` ; la fermeture du
player ramène à la surcouche. Les histoires installées sont sous « Mes histoires ».

## Dépannage

| Symptôme                               | Piste                                                                                                                                                                                                                           |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows « a protégé votre ordinateur » | Installateur non signé : « Informations complémentaires » → « Exécuter quand même »                                                                                                                                             |
| La fenêtre reste blanche au démarrage  | Installer ou réparer WebView2 (Microsoft Edge WebView2 Runtime)                                                                                                                                                                 |
| « flam-player est introuvable »        | Réinstaller ; sinon indiquer `flam-player.exe` dans « Paramètres ». Un antivirus peut avoir mis le fichier en quarantaine                                                                                                       |
| « Python introuvable »                 | Ne devrait pas arriver avec l'installateur (Python embarqué). Dans une installation partielle ou un build de développement, indiquer `python.exe` dans « Paramètres ». Avec le convertisseur embarqué, ce chemin est **ignoré** |
| Une conversion échoue                  | Le message cite le code de `telmi2flam` ; l'histoire peut être incompatible (badge sur la fiche)                                                                                                                                |
| Catalogue « hors ligne »               | Réseau ou serveur indisponible : le cache est affiché. « Actualiser » réessaie                                                                                                                                                  |
| Disque plein                           | « Paramètres » → « Cache » → « Vider le cache » : supprime aussi les histoires installées (à réinstaller depuis leur source) ; la progression sauvegardée et les sources sont conservées                                        |
| Paramètres ou sources réinitialisés    | Un fichier de configuration corrompu a été sauvegardé en `*.bak` et remplacé ; un bandeau l'indique                                                                                                                             |

## Développement

### Prérequis

Rust stable (MSVC), Visual Studio 2022 avec les outils C++ x64, WebView2, Node 24+ et npm.

Le launcher est membre d'un **workspace Cargo** dont le `Cargo.toml` est à la racine du dépôt (avec
`crates/telmi2flam`) : `Cargo.lock` et `target/` sont à la racine, pas dans `src-tauri/`. Les commandes
`cargo` se lancent toujours depuis `src-tauri/` (ou avec `-p flam-launcher`) ; le profil `release` est
défini dans le `Cargo.toml` racine.

## Commandes

Depuis `apps/flam-launcher/` :

| Commande                             | Effet                                                                                |
| ------------------------------------ | ------------------------------------------------------------------------------------ |
| `npm install`                        | Installe les dépendances front et la CLI Tauri                                       |
| `npm run tauri dev`                  | Lance l'application en développement                                                 |
| `npm run dev` / `npm run build`      | Front seul (Vite) / build du front                                                   |
| `npm run lint` / `npm run typecheck` | ESLint / TypeScript strict                                                           |
| `npm test`                           | Vitest + Testing Library (API Tauri simulée)                                         |
| `npm run tauri build`                | Exécutable seul avec installeurs MSI et NSIS (sans player ni convertisseur)          |
| `npm run build:notices`              | Génère `build/THIRD-PARTY-NOTICES.txt` (licences Rust et npm, installé)              |
| `npm run build:converter`            | Construit le convertisseur embarqué (réseau)                                         |
| `npm run build:player`               | Compile `flam-player.exe` (Release) et rassemble `SDL2.dll` et les licences          |
| `npm run build:installer`            | Enchaîne notices, converter, ffmpeg, player puis produit l'installateur NSIS complet |
| `npm run test:e2e`                   | Test de bout en bout (voir « Test de bout en bout »)                                 |

Dans `src-tauri/` : `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

## Convertisseur embarqué

L'application installée ne suppose aucun Python sur la machine : le convertisseur est livré avec
un Python embarquable officiel (3.14.8, signé PSF) et Pillow 12.3.0, sous `<exe>/telmi2flam/`
(`python/python.exe`, `telmi2flam.py`, `lif.py`, `mp3map.py`, `engine/`, `runtime/`,
`LICENSES.md`, `BUILD.txt`). `convert.rs` et l'annulation Ctrl+Break sont inchangés : ils lancent
`python -E -s -B -u -X utf8 telmi2flam.py`. Environ 20 Mo sur disque, 9 Mo compressés dans
l'installeur.

1. **Construire l'artefact** (réseau, PowerShell seul, sans Python sur la machine de build) :
   `npm run build:converter`. Le script télécharge Python et Pillow à des versions épinglées,
   **vérifie leur SHA-256** (publié par python.org / PyPI), allège le paquet, écrit
   `build/telmi2flam/` (ignoré par Git) puis le contrôle sans Python dans le `PATH`.
2. **L'embarquer** : `src-tauri/tauri.bundle.conf.json` déclare la ressource
   `../build/telmi2flam/` vers `telmi2flam/` (voir « Installateur Windows » plus bas pour le
   reste). Il est passé explicitement, car l'ajouter à `tauri.conf.json` ferait échouer tout
   build sans les artefacts :

   ```
   npm run build:converter
   npm run tauri build -- --config src-tauri/tauri.bundle.conf.json                 # installateur NSIS
   npm run tauri build -- --no-bundle --config src-tauri/tauri.bundle.conf.json     # exe seul
   ```

   **ffmpeg** (conversion audio des podcasts, #209) : `npm run build:ffmpeg` prépare
   `build/ffmpeg/` (ffmpeg et ffprobe LGPL minimaux, statiques, 6 Mo, avec licences et
   `BUILD.txt`). Le binaire est construit depuis des sources épinglées (`tools/ffmpeg/`) : sans
   argument le script le construit avec **Docker** ; sinon `-ArtifactDir <dossier>` reprend la
   sortie de `tools/ffmpeg/build.sh` (la CI la produit dans le job `ffmpeg`).

   Le convertisseur est toujours lancé avec `-B` (aucun `__pycache__` écrit dans le dossier
   d'installation : le désinstallateur ne les connaîtrait pas).

   Même avec `--no-bundle`, Tauri copie `telmi2flam/` à côté de l'exe dans
   `target/release/` à la racine du dépôt (workspace Cargo ; vérifié : copie identique à l'artefact). Sans `--config`, le
   build reste valable mais sans convertisseur (le launcher cherche alors Python dans le `PATH`).

3. **Vérifier en réel** (Python absent du `PATH`) : deux tests Rust ignorés par défaut
   (`install::convert::tests::embarque`) pilotent le convertisseur embarqué, conversion puis
   annulation :

   ```powershell
   # depuis src-tauri/ ; `cargo test --lib --no-run` affiche le binaire de test
   $env:FLAM_TEST_CONVERTER_DIR = "<...>\target\release\telmi2flam"
   $env:FLAM_TEST_ARCHIVE = "<histoire de test>.zip"   # synthétique, hors dépôt
   $env:PATH = "C:\Windows\System32"
   & <binaire-de-test>.exe --ignored --nocapture embarque
   ```

   L'archive de test se génère avec `make_story.py` (issue #46) ; ne jamais committer de
   contenu sous licence. Pillow embarqué n'a pas FreeType : avec le Python embarqué, retirer
   `font_size=48` de ce script.

**Python configuré ignoré** : quand le convertisseur embarqué est présent, son Python est
toujours utilisé ; le chemin de Python des paramètres ne sert que de repli (build de
développement, installation partielle). L'écran Paramètres l'indique.

**Licences redistribuées** : `telmi2flam/LICENSES.md` (produit par `build-converter.ps1`) liste
Python (PSF-2.0, libffi, libtommath…), `python.cat`, `vcruntime140*.dll` (redistribuables
Microsoft), Pillow (renvoi vers `licenses/LICENSE` de la roue, qui regroupe les bibliothèques
intégrées), le runtime et les scripts du dépôt (GPL v3). `LICENSES.md` à la racine de
l'installation couvre le reste (flam-launcher et flam-player GPL v3, SDL2 zlib, Lua MIT, LVGL MIT,
minimp3 CC0, polices Nunito et Montserrat OFL) ; les textes sont copiés des sous-modules dans
`licenses/` par `build-player.ps1`.

## Installateur Windows

Installateur **NSIS**, par utilisateur (`installMode: currentUser`, sans droits
d'administrateur), **non signé** (SmartScreen avertit). Prérequis de build : Rust, Node 24+,
Visual Studio **2022** avec les outils C++ x64 (le build Rust doit se faire depuis
« x64 Native Tools » ou après `vcvars64.bat`), CMake et Ninja (`pip install cmake ninja` si
absents), sous-modules initialisés (`git submodule update --init --recursive`), réseau (Python,
Pillow, NSIS).

**Mise à jour** : avant la copie, le hook NSIS `src-tauri/windows/installer-hooks.nsh`
(`NSIS_HOOK_PREINSTALL`) lance le désinstallateur de la version précédente (`/S /UPDATE`, sur
place). Il ne supprime que les fichiers que cette version avait installés : un fichier retiré
entre deux versions disparaît (#180), un fichier étranger du dossier d'installation est
conservé, aucun `RMDir /r` aveugle. Raccourcis et données utilisateur sont conservés. Si le
désinstallateur échoue, la mise à jour continue par-dessus (comportement précédent).

Limites connues de la mise à jour (#186) :

- **Application ouverte** : en mode silencieux (`/S`), l'installateur Tauri ferme
  `flam-launcher.exe` de lui-même (`CheckIfAppIsRunning`) ; la CI le vérifie (N-1 lancé, mise à
  jour silencieuse bornée à 180 s, N-1 arrêté, arborescence identique à une installation propre).
  En mode interactif, l'installateur demande de fermer l'application.
- **Échec de N après le nettoyage** : le désinstallateur de N-1 passe avant la copie de N. L'archive
  de N est vérifiée par le CRC de NSIS au démarrage de l'installateur (avant toute action, donc avant
  le hook ; l'option `/NCRC` n'est pas utilisée), ce qui écarte un installateur tronqué ou corrompu, y
  compris en mode silencieux `/S` (code de sortie 2, rien n'est exécuté ; la CI le vérifie). En
  revanche, un échec en cours de copie (disque plein, fichier verrouillé) laisse une installation
  incomplète : relancer l'installateur de N la répare, y compris sans entrée « Programmes et
  fonctionnalités ». Il n'y a pas de sauvegarde de N-1.
- **N-1 trop ancienne** : la conservation des raccourcis et du démarrage automatique repose sur le
  drapeau `/UPDATE` du désinstallateur de N-1, présent dans les installateurs générés par Tauri 2
  (le launcher utilise Tauri 2.12). Un installateur N-1 d'une génération qui l'ignore (hors Tauri 2) perd
  raccourcis et démarrage automatique : les recréer en relançant l'application ou l'installateur de N.
  **Version minimale de N-1 : 0.1.0**, le premier installateur du launcher (Tauri 2, `/UPDATE` géré) :
  toute version publiée est compatible. Le nettoyage des fichiers retirés ne dépend que du hook de N.

```
npm run build:installer      # = build:notices + build:converter + build:ffmpeg + build:player + tauri build --config …
```

**Runtime Visual C++** : `flam-player.exe` importe `VCRUNTIME140.dll`, absente d'un Windows
propre. Choix : livrer `vcruntime140.dll` (fichier redistribuable de Visual Studio, copié depuis
son dossier `VC\Redist` par `build-player.ps1`) à côté du player plutôt que passer en runtime
statique, qui changerait les options de compilation de tout le player, de SDL2, de Lua et de LVGL
sans changer le résultat pour l'utilisateur. Le launcher (Rust) et `SDL2.dll` n'importent que
des DLL Windows (UCRT, Windows 10 et plus). `scripts/check-imports.ps1 <dossier>` (étape de la CI,
sur l'installation) liste les imports des trois binaires avec `dumpbin /dependents` et échoue si
une DLL du runtime VC++ est importée sans être à côté du binaire, ou si une autre DLL est
introuvable.

soit, en détail :

1. `npm run build:converter` : `build/telmi2flam/` (voir plus haut) ;
2. `npm run build:player` : compile `flam-player.exe` en Release (`<dépôt>/build/release`,
   `do_build.bat` et son Debug ne sont pas touchés) et dépose `flam-player.exe`, `SDL2.dll` et
   `licenses/` dans `build/player/` ;
3. `npm run tauri build -- --config src-tauri/tauri.bundle.conf.json` : produit
   `target/release/bundle/nsis/Flam Launcher_<version>_x64-setup.exe` (environ 11 Mo)
   et dépose les mêmes ressources à côté de `flam-launcher.exe`.

`tauri.bundle.conf.json` (fusionné avec `tauri.conf.json`) restreint les cibles à `nsis` et
déclare les ressources : `telmi2flam/` (Python, Pillow, scripts, `runtime/`), `flam-player.exe` et
`SDL2.dll` **à côté de l'exécutable** (c'est là que `settings::detect_player` les cherche),
`vcruntime140.dll`, `licenses/` et `LICENSES.md`. Le launcher lance le player avec pour dossier courant celui de
`flam-player.exe` : `SDL2.dll` est ainsi trouvée. Installation sans interaction :
`"Flam Launcher_<version>_x64-setup.exe" /S /D=<dossier>` (`/D=` en dernier, sans guillemets) ;
désinstallation : `<dossier>\uninstall.exe /S _?=<dossier>`.

### Test de bout en bout

`src-tauri/tests/e2e.rs` pilote les **services réels** du backend, sans WebView :
`CatalogService` lit un catalogue servi par un serveur HTTP local factice, `InstallService`
télécharge l'archive et la convertit avec le vrai `telmi2flam.py` (Python embarqué), `Library`
la liste, `PlayerService` lance le vrai `flam-player.exe` (`SDL_VIDEODRIVER=dummy`,
`SDL_AUDIODRIVER=dummy`, `SDL_RENDER_DRIVER=software`), puis le player est fermé et
l'événement `exited` est reçu. Le player ne se termine pas seul : sa fin est obtenue par
`taskkill /F` (garde RAII qui le ferme aussi si le test échoue). Une sortie volontaire n'est pas
simple à obtenir : avec `SDL_VIDEODRIVER=dummy` il n'y a aucune fenêtre à qui envoyer `WM_CLOSE`
(`taskkill` sans `/F`), et le player n'a ni commande de sortie ni entrée console ; en ajouter une
toucherait le code du player pour les besoins d'un test (suivi laissé ouvert). Le test vérifie
qu'il charge l'histoire et reste actif avant cela. L'archive est synthétique (images unies, MP3 muets), générée par
le test : aucun contenu sous licence. Choix : tester le backend Rust plutôt que piloter
l'interface par WebDriver, qui exigerait `tauri-driver` et `msedgedriver` appariés à la version
de WebView2 (fragile, lent) alors que la logique à risque est dans le backend ; l'interface a
ses tests Vitest et la liaison front/backend repose sur des types miroirs.

```
npm run build:installer
npm run test:e2e             # sur target/release (racine du dépôt) ; ou FLAM_E2E_INSTALL_DIR=<installation>
```

Sans `FLAM_E2E_REQUIRE=1` et sans `flam-player.exe`, `cargo test` **saute** ce test : il passe au
vert sans rien vérifier, et le message n'apparaît qu'avec `-- --nocapture` ; `npm run test:e2e` et
la CI posent `FLAM_E2E_REQUIRE=1` (le test échoue alors si un prérequis manque) ;
variables : `FLAM_E2E_INSTALL_DIR`, ou `FLAM_E2E_PLAYER` + `FLAM_E2E_CONVERTER_DIR`. La CI
(`.github/workflows/launcher.yml`) compile le player, construit l'installateur, l'installe en
silence, y exécute ce test puis le désinstalle en vérifiant qu'il ne reste aucun fichier.

### À vérifier sur une session Windows propre

Non couvert par la CI ni par le poste de développement (où Python, Visual Studio et WebView2
sont déjà présents) : à cocher par le mainteneur avant publication.

- [ ] L'installateur s'exécute sans droits d'administrateur ; SmartScreen propose
      « Exécuter quand même ».
- [ ] Aucun Python ni Visual Studio sur la machine : une histoire Telmi s'installe
      (convertisseur embarqué) et se joue dans `flam-player`.
- [ ] Windows 10 et Windows 11 ; WebView2 absent : comportement et message.
- [ ] Raccourci du menu Démarrer, icône, entrée dans « Applications installées ».
- [ ] Fenêtre du player (carte graphique réelle, audio réel) : image et son.
- [ ] Mise à jour par-dessus une version installée ; désinstallation (données utilisateur
      conservées, dossier d'installation vide).
- [ ] Antivirus : pas de quarantaine de `flam-player.exe` ni de `python.exe`.

## Architecture

```
apps/flam-launcher/
  src/                     Front React/TypeScript
    api/                   Couche typée au-dessus de `invoke` / événements Tauri
    router.ts              Routage par hash (`#/`, `#/sources`, `#/settings`, `#/library[/<id>]`, `#/demo` en dev)
    features/<domaine>/    Écrans et logique par fonctionnalité (app, welcome, sources, settings,
                           library, compat, stories, recovery, demo en dev)
    components/            Composants partagés sans logique métier (design system)
    theme/                 Jetons CSS (clair/sombre) et choix du thème
    i18n/                  Textes de l'interface (français)
  docs/DESIGN.md           Design system : principes, palette, composants
  src-tauri/
    src/
      lib.rs               Initialisation (traces, builder Tauri)
      app.rs               Commandes générales (`app_info`)
      error.rs             `AppError` (thiserror), sérialisée en `{ kind, message }`
      storage.rs           Persistance JSON : écriture atomique, `*.bak`, migration de schéma
      settings.rs          `settings.json`, détection du player et de Python, commandes `settings_*`
      cache.rs             Taille du cache et vidage (sous-dossiers de l'application seulement)
      sources.rs           `sources.json`, source par défaut et implicite, commandes `sources_*`
      story.rs             `StoryEntry` / `StoryKind` : type commun des histoires de toutes les sources
      catalog.rs           Source catalogue : téléchargement, lecture tolérante, cache, vignettes
      folder.rs            Source dossier local (`.zip` Telmi, `.plain`, `.plain.pk`), commandes `folder_*`
      thumbnail.rs         Vignettes : PNG, LIF (décodeur de `crates/telmi2flam`), réduction, cache `thumbs/`, URL `data:`
      watch.rs             Surveillance optionnelle d'un dossier local (`notify`), `folder_watch`
      install/             Pipeline d'installation : file, téléchargement, conversion (`mod.rs`,
                           `download.rs`, `convert.rs`, `pack.rs`), commandes `install_*`
      library.rs           Bibliothèque `<cache>/library/` et `library.json`, commandes `library_*`
      library_thumb.rs     Vignettes des histoires installées (`library/thumbs/`), `library_thumbnail`
      compat.rs            Statut de compatibilité (`status.json` de la campagne + conversions locales), commande `compat_statuses`
      device/              Flam branchée en USB : `detect.rs` (détection, `.mdf`), `crypto.rs` (AES-128-CBC,
                           clés d'appareil, fichier `key`),
                           `prepare.rs` (dossier `str/<uuid>/` préparé hors appareil depuis un `.plain.pk`)
```

### Modules Rust

Présents : `app` (commandes générales), `error`, `storage`, `settings`, `cache`, `sources`, `story`, `catalog`, `folder`, `lif`, `thumbnail`, `install`, `library`, `library_thumb`, `compat`, `player`, `device` (Flam branchée en USB : détection #225, `device::detect` ; index #226, `device::index` ; chiffrement et préparation #228, `device::crypto`, `device::prepare` ; état et histoires en lecture seule pour l'écran « Ma Flam » #227, `device::status`).

### Commandes Tauri

Chaque module Rust porte ses propres commandes (`#[tauri::command]` dans le module concerné,
enregistrées dans `generate_handler!` de `lib.rs`) : pas de fichier `commands.rs` fourre-tout.

Les commandes sont asynchrones (`tokio`), renvoient `AppResult<T>` et sont enregistrées dans
`generate_handler!` (`lib.rs`). Chacune a un wrapper typé dans `src/api/`.

| Commande               | Arguments                  | Retour                                         | Module           |
| ---------------------- | -------------------------- | ---------------------------------------------- | ---------------- |
| `app_info`             |                            | `{ name, version }`                            | `app`            |
| `settings_get`         |                            | `SettingsView`                                 | `settings`       |
| `settings_set`         | `settings: Settings`       | `SettingsView`                                 | `settings`       |
| `sources_list`         |                            | `SourcesView`                                  | `sources`        |
| `sources_add`          | `input: SourceInput`       | `Source`                                       | `sources`        |
| `sources_update`       | `id`, `input: SourceInput` | `Source`                                       | `sources`        |
| `sources_remove`       | `id`                       | rien                                           | `sources`        |
| `sources_reorder`      | `ids: string[]`            | `Source[]` (liste complète)                    | `sources`        |
| `config_ack_recovery`  | `file?: string`            | rien                                           | `storage`        |
| `cache_usage`          |                            | `{ path, bytes }`                              | `cache`          |
| `cache_clear`          |                            | `{ freed_bytes }`                              | `cache`          |
| `catalog_refresh`      | `source_id`                | `CatalogView`                                  | `catalog`        |
| `catalog_entries`      | `source_id`                | `CatalogView`                                  | `catalog`        |
| `catalog_thumbnail`    | `source_id`, `thumb_url`   | URL `data:` (string)                           | `catalog`        |
| `folder_scan`          | `source_id`, `recursive?`  | `FolderScan`                                   | `folder`         |
| `folder_thumbnail`     | `source_id`, `entry_id`    | URL `data:` (chaîne)                           | `folder`         |
| `folder_watch`         | `source_id`, `enabled`     | rien                                           | `watch`          |
| `install_start`        | `source_id`, `entry_id`    | `InstallTask`                                  | `install`        |
| `install_cancel`       | `task_id`                  | rien                                           | `install`        |
| `install_list`         |                            | `InstallTask[]`                                | `install`        |
| `library_list`         |                            | `LibraryEntry[]`                               | `library`        |
| `library_remove`       | `uuid`, `purge_saves?`     | rien                                           | `library`        |
| `library_thumbnail`    | `uuid`                     | URL `data:` (`not_found` sans vignette)        | `library_thumb`  |
| `player_launch`        | `uuid`                     | `PlayerSession`                                | `player`         |
| `player_launch_folder` | `source_id`, `entry_id`    | `PlayerSession`                                | `player`         |
| `player_status`        |                            | `PlayerSession \| null`                        | `player`         |
| `player_stop`          |                            | `bool` (faux : aucun player)                   | `player`         |
| `player_embed_layout`  | `rect \| null` (px CSS)    | rien                                           | `player`         |
| `player_embed_focus`   |                            | rien                                           | `player`         |
| `device_status`        |                            | `DeviceStatus`                                 | `device::status` |
| `device_list_stories`  |                            | `DeviceStories \| null` (`null` : aucune Flam) | `device::status` |

`device_status` et `device_list_stories` sont **en lecture seule** sur la Flam : ni reprise
d'index (`recover`), ni création de dossier. La seule écriture est la copie personnelle de
`.mdf` (`backup_mdf`) dans le dossier de données du launcher, faite par `device_status`
(idempotente). Le titre d'une histoire est chiffré dans `str/<uuid>/info` : il n'est connu que
si l'histoire figure dans la bibliothèque du launcher, sinon l'écran affiche l'UUID court.
Le numéro de série et le firmware ne sont pas affichés : `parse_mdf` ne lit encore aucun champ
(format de `.mdf` non spécifié, #224). L'écran « Ma Flam » (`#/device`, `src/features/device/`)
scrute `device_status` toutes les 2 s tant qu'il est ouvert et laisse des emplacements
(`toolbar`, `storyActions`) pour l'envoi (#230) et le retrait (#231).

`sources_list` renvoie `SourcesView { sources, recovery }` ; `settings_get` / `settings_set`
renvoient `SettingsView { settings, resolved, issues, recovery }` (voir « Données »).
`settings_set` **remplace l'objet complet** (pas de fusion partielle).

Wrappers front : `src/api/settings.ts` (`getSettings`, `setSettings`), `src/api/sources.ts`
(`listSources`, `addSource`, `updateSource`, `removeSource`, `reorderSources`) et
`src/api/recovery.ts` (`ackRecovery`) et `src/api/catalog.ts` (`refreshCatalog`,
`getCatalogEntries`, `getCatalogThumbnail`, types `StoryEntry`, `CatalogView`). La logique
`src/api/cache.ts` (`getCacheUsage`, `clearCache`) et `src/api/dialog.ts` (`pickFolder`,
`pickExecutable` : sélecteurs natifs du plugin `tauri-plugin-dialog`), `src/api/install.ts`
(`startInstall`, `cancelInstall`, `listInstalls`, `onInstallProgress`, `onInstallDone`,
`onInstallError`) et `src/api/library.ts` (`listLibrary`, `removeFromLibrary`, `hasUpdate`). La logique
(`SettingsStore`, `SourcesStore`, détection) ne dépend pas de Tauri et reçoit son dossier de
configuration en paramètre : elle est testée sur des dossiers temporaires ; les commandes
ne sont que de fines enveloppes qui prennent un verrou (`ConfigState`) autour de chaque opération.

### Source dossier local (`folder_scan`, `folder_thumbnail`)

`folder_scan(source_id, recursive?)` analyse une source de type `folder` (autre type :
`validation` ; dossier absent : `folder_unavailable`) et renvoie
`FolderScan { entries: StoryEntry[], warnings: { path, message }[] }`. Le parcours est
non récursif par défaut (récursif : 8 niveaux, 5000 histoires au plus, liens symboliques et
jonctions ignorés) ; un fichier invalide est **ignoré et signalé** dans `warnings`, jamais une
erreur. Le verrou de configuration n'est pas tenu pendant l'analyse (thread dédié).

| Entrée            | `kind`       | Lu                                                                                                                                     |
| ----------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| `*.zip`           | `telmi_zip`  | `metadata.json` (UTF-8, BOM toléré) du dossier racine (`find_root` de `telmi2flam`), vignette `title.png` ou `cover.png` selon `image` |
| dossier `*.plain` | `flam_plain` | 1re ligne d'`info.plain` (titre), `uuid.bin` (16 octets, uuid en hexadécimal), `img/thumbnail.lif`                                     |
| `*.plain.pk`      | `flam_pk`    | idem, dans l'archive (zip « stored »)                                                                                                  |

`StoryEntry` (`src-tauri/src/story.rs`, partagé avec la source catalogue ; miroir
`src/api/story.ts`) : `source_id, uuid, version, title, description, age, category, author,
voice, license, thumb_url, download_url, size, updated_at, kind`. Pour un dossier,
`download_url` est le **chemin local absolu** de l'archive ou du dossier `.plain`, `size` est
absente pour un `.plain`, `updated_at` est la date de modification (ISO 8601 UTC) ; les champs
inconnus valent `null`.

**Doublons** : un `.plain` et un `.plain.pk` de même `uuid` donnent **deux entrées** (`kind`
différent). Chaque entrée porte un `entry_id` (stable d'un scan à l'autre, unique
dans la source) à utiliser comme clé d'affichage et pour `folder_thumbnail`. Format
`"<source_id>:<hash>"`, comme le catalogue (`"<source_id>:<uuid>"`) ; le hash est un FNV-1a 64 bits du
chemin relatif au dossier de la source (séparateurs `/`).

**Identifiants non fiables** : l'`uuid` d'un zip Telmi (et `version` si c'est une chaîne) doit
respecter `^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$`, sans `..` ni nom réservé Windows (`CON`, `PRN`,
`AUX`, `NUL`, `COM1-9`, `LPT1-9`) ; sinon l'histoire est ignorée avec un avertissement.

**Archives non fiables** : rien n'est extrait sur disque ; `metadata.json` et `info.plain` sont
limités à 1 Mio, les vignettes à 8 Mio (lecture bornée même si la taille déclarée est fausse ; PNG
décodé : 8192 px et 64 Mio au plus) ; les entrées aux noms dangereux (`..`, absolu, lecteur,
`__MACOSX`, `._*`) sont écartées ; les `\` des noms de zip sont lus comme des `/` ; 100 000 entrées
au plus par archive.

**Surveillance du dossier (`folder_watch`, événement `folder://changed`)** : `folder_watch(source_id,
enabled)` démarre ou arrête la surveillance (crate `notify`, non récursive comme le scan par défaut)
du dossier d'une source ; dossier absent : `folder_unavailable`. Une rafale de changements (ajout,
suppression, renommage, modification) produit **un seul** événement `folder://changed`
`{ source_id }`, 400 ms après le dernier changement (3 s au plus après le premier, même si le
dossier ne se calme jamais). L'événement ne porte aucune histoire : le front relance `folder_scan`
(`onFolderChanged` / `watchFolder` dans `src/api/folder.ts`). Les accès en lecture et les
changements dans le dossier de cache sont ignorés. La surveillance est un confort : si elle ne
démarre pas, la liste reste utilisable avec le bouton _Rafraîchir_. L'écran d'une source dossier
l'active à l'ouverture et l'arrête en la quittant.

**Vignettes à la demande** : `folder_scan` ne renvoie aucune image (5000 entrées feraient des
centaines de Mio d'IPC). `thumb_url` est une **référence à résoudre par `folder_thumbnail`** (égale à
`entry_id`) ou `null` sans vignette : ce n'est pas une URL affichable. `folder_thumbnail(source_id, entry_id)` renvoie
l'URL `data:image/png;base64,…` d'une seule histoire (compatible CSP `img-src data:`), uniquement
pour une entrée du **dernier scan** de cette source (sinon `not_found`, jamais un chemin libre).
L'image (PNG, ou LIF via `telmi2flam::lif`) est réduite à 160 px et gardée dans `<cache>/thumbs/<clé>.png`
(clé = chemin, taille et date de l'archive, plus taille et date de `img/thumbnail.lif` pour un
`.plain`) ; le cache est consulté **avant** d'ouvrir l'archive.

**Cache de métadonnées** : en mémoire, par archive (chemin, taille, date de modification), pour ne
pas rouvrir chaque zip à chaque scan ; les dossiers `.plain` sont relus à chaque scan.

`thumbs/` est enregistré dans `APP_CACHE_SUBDIRS` (`cache.rs`) : « Vider le cache » le nettoie.

**Purge de `thumbs/`** : au démarrage, en arrière-plan et sans jamais bloquer ni échouer le lancement, `thumbnail::purge_thumbs` supprime les vignettes de plus de 30 jours (clé périmée : archive modifiée ou disparue), les restes d'écriture `*.tmp-<pid>` de plus d'une heure, puis les plus anciennes vignettes tant que le total dépasse 128 Mio. Seuls les fichiers réguliers nommés exactement comme ceux de `ThumbCache` (`<16 chiffres hexadécimaux minuscules>.png` ou `.tmp-<pid>`) sont touchés : `thumbs/` lien ou jonction refusé (jamais parcouru), liens, sous-dossiers et fichiers étrangers ignorés. Une suppression refusée (fichier ouvert, droits) est journalisée et la purge continue ; le fichier est réessayé au démarrage suivant.

### Erreurs

`AppError` (`src-tauri/src/error.rs`, `thiserror`) est sérialisée en objet stable
`{ "kind": "<code_stable>", "message": "<texte lisible en français>" }` (ex. `kind: "io"`).
Le miroir front est `src/api/error.ts` (type `AppError`, garde `isAppError`) ; le helper
`invokeCommand<T>(cmd, args?)` appelle `invoke` et normalise toute erreur en `AppError`
(`kind: "unknown"` si la forme est inattendue). Convention : chaque module ajoute ses variantes
à `AppError` avec un `kind` stable ; le front teste le `kind`, jamais le `message`.

| `kind`                  | Sens                                                                                                |
| ----------------------- | --------------------------------------------------------------------------------------------------- |
| `io`                    | Erreur d'entrée/sortie                                                                              |
| `validation`            | Valeur refusée (URL, chemin, extension, nom, ordre) ou opération interdite                          |
| `not_found`             | Élément introuvable (identifiant de source inconnu)                                                 |
| `config_version`        | Fichier de configuration écrit par une version plus récente (laissé intact)                         |
| `config_dir`            | Dossier de configuration introuvable                                                                |
| `serialization`         | Échec de sérialisation JSON                                                                         |
| `network`               | Réseau indisponible, délai dépassé, réponse HTTP en échec, réponse trop grosse                      |
| `catalog_invalid`       | Catalogue (ou vignette) reçu inexploitable : JSON invalide, pas de liste `data`, image non reconnue |
| `folder_unavailable`    | Dossier d'une source locale absent ou inaccessible (disque débranché…)                              |
| `cancelled`             | Tâche d'installation annulée                                                                        |
| `download_invalid`      | Téléchargement incomplet, trop gros (4 Gio), ou qui n'est pas l'archive attendue                    |
| `converter_unavailable` | Python ou `telmi2flam.py` introuvable                                                               |
| `conversion`            | Échec de `telmi2flam` ; le code du convertisseur (`missing_asset`…) est dans `TaskError.code`       |
| `player_unavailable`    | `flam-player.exe` introuvable (ni configuré ni détecté) ou impossible à lancer                      |
| `player_busy`           | Un player tourne déjà (un seul à la fois)                                                           |
| `story_file_missing`    | Le fichier de l'histoire à lancer a disparu                                                         |
| `unknown`               | Côté front uniquement : erreur de forme inattendue                                                  |

Un fichier de configuration **corrompu** n'est pas une erreur : il est renommé en
`<fichier>.<AAAAMMJJ-HHMMSS>.bak`, un avertissement est tracé et les valeurs par défaut sont utilisées.
Cette restauration est mémorisée dans `recovery.json` et renvoyée dans le champ `recovery`
(`{ file, backup }`) de `settings_get` / `sources_list` **jusqu'à** `config_ack_recovery`
(même après un redémarrage) : l'interface avertit l'utilisateur puis acquitte.

### Source catalogue (`catalog`)

Un catalogue est un JSON au format Telmi Store : `{ banner, data: [ { uuid, version?, title,
description, age, category, author, voice, license, thumbs: { small, medium }, download,
updated_at, … } ] }`. Il est normalisé en `StoryEntry` (module partagé `story.rs` / `src/api/story.ts`,
aussi utilisé par la source dossier local) :
`source_id, entry_id, uuid, version?, title, description?, age?, category?, author?, voice?,
license?, thumb_url?, download_url, size?, updated_at?, kind (telmi_zip | flam_plain | flam_pk)`.

- **Identifiants** : `entry_id` est unique et stable dans une source ; c'est la clé de grille de
  l'interface et la clé de cache des vignettes côté front. Catalogue : `"<source_id>:<uuid>"`.
  `story::validate_story_id` (`^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$`, sans `..`, sans point final,
  hors noms réservés Windows `CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`) valide tout
  identifiant destiné à devenir un nom de fichier ; une entrée dont l'`uuid` ne passe pas est
  ignorée avec un avertissement.
- **`thumb_url`** n'est **pas** une URL affichable : la CSP bloque les images distantes. C'est une
  référence que le front résout avec `getCatalogThumbnail` (voir « Vignettes »).
- **`download_url`** est une URL absolue http **ou https**, telle que publiée par le catalogue.
  Le pipeline d'installation (#15) accepte le http clair : l'archive est de toute façon traitée
  comme non fiable (lectures bornées, conversion par `telmi2flam`), et les redirections de https
  vers http restent refusées.
- **`size`** est toujours absente pour le Telmi Store (le format ne la fournit pas) : #15 doit
  s'appuyer sur le `Content-Length` du téléchargement.
- **Lecture tolérante** : champs inconnus ignorés ; champ absent ou de mauvais type = valeur par
  défaut (`null` pour les champs optionnels, y compris les textes vides ; titre vide si absent) ;
  entrée sans `uuid` valide ou sans lien `download` http(s) valide ignorée avec un avertissement
  (`warnings`, 50 maximum puis résumé) ; `uuid` en double ignoré (premier conservé, détection en
  temps constant). Au-delà de 10 000 entrées, les suivantes sont ignorées (avertissement). URL
  relatives résolues par rapport à l'URL du catalogue ; vignette `small` (grille) sinon `medium`.
  Une racine qui est directement une liste, ou un BOM UTF-8, sont acceptés. Sans liste
  d'histoires : erreur `catalog_invalid`. La lecture se fait hors du fil asynchrone
  (`spawn_blocking`).
- **Réseau** : `reqwest` avec rustls (pas d'OpenSSL), délai de connexion 10 s, délai total 30 s,
  corps limité à 16 Mio (contrôlé sur `Content-Length` puis sur le flux reçu),
  `User-Agent` `flam-launcher/<version> (+https://github.com/scarlaty/flam-player)`.
  Redirections : 5 sauts au plus, jamais de https vers http. Requête conditionnelle
  `If-None-Match` / `If-Modified-Since` quand un cache de confiance existe.
- **Risque SSRF** : le launcher fait des GET « à l'aveugle » vers des adresses publiées par le
  catalogue (catalogue, vignettes, plus tard téléchargements). Un catalogue hostile peut donc
  faire joindre une adresse locale ou privée ; la réponse n'est jamais exposée telle quelle
  (JSON analysé, images reconnues par leurs premiers octets), mais la requête part. Le front ne
  peut demander que des vignettes présentes dans le catalogue en cache. N'ajouter que des
  catalogues de confiance.
- **Cache hors ligne** sous `<cache>/catalogs/<source_id>/` : `catalog.json` (dernier catalogue
  valide, tel que reçu), `meta.json` (`url`, `etag`, `last_modified`, `sha256`, `fetched_at`),
  `thumbs/<sha256 de l'URL>.img`. L'identifiant de source doit être un nom de dossier sûr
  (`[A-Za-z0-9_-]`, 64 max).
  - Un cache dont l'URL diffère de celle de la source est ignoré.
  - `meta.json` contient le SHA-256 de `catalog.json` : s'il ne correspond pas au chargement
    (écriture interrompue, fichier modifié), le catalogue reste lisible mais l'ETag est ignoré
    (téléchargement complet).
  - Un seul `catalog_refresh` à la fois par source (verrou par source) ; les suivants attendent.
  - Un catalogue **valide mais sans aucune entrée exploitable** (`data: []`) ne remplace pas un
    cache non vide : le cache est conservé et `status.error` (`catalog_invalid`) l'indique.
- **`catalog_refresh`** renvoie `CatalogView { source_id, entries, warnings, status }` avec
  `status { offline, updated_at, error }` :

  | Réponse du serveur                        | Résultat                                                                                |
  | ----------------------------------------- | --------------------------------------------------------------------------------------- |
  | 200 valide                                | nouveau cache, `offline: false`                                                         |
  | 304                                       | cache conservé, `updated_at` = maintenant                                               |
  | échec réseau, délai, redirection, 4xx/5xx | cache servi, `offline: true`, `error.kind = "network"` (ou erreur `network` sans cache) |
  | 200 mais catalogue invalide               | cache servi, `offline: false`, `error.kind = "catalog_invalid"` (ou erreur sans cache)  |
  | 200 valide mais vide, cache non vide      | cache conservé, `offline: false`, `error.kind = "catalog_invalid"`                      |

  `status.updated_at` (secondes Unix) alimente l'indicateur « hors ligne / dernière mise à jour ».

- **`catalog_entries`** lit uniquement le cache (aucun réseau) : à appeler d'abord pour afficher
  immédiatement, puis `catalog_refresh`.
- **Vignettes** : `catalog_thumbnail(source_id, thumb_url)` télécharge la vignette à la première
  demande (2 Mio max, PNG / JPEG / GIF / WebP reconnus à leurs premiers octets, SVG refusé, 6
  téléchargements simultanés au plus) puis la lit sur le disque, et la renvoie en URL `data:`
  utilisable dans `<img src>`. `thumb_url` doit appartenir à une entrée du catalogue en cache :
  l'ensemble des `thumb_url` connues est gardé en mémoire par source (remplacé à chaque
  rafraîchissement), le catalogue n'est donc pas relu à chaque vignette. Le protocole d'asset de
  Tauri n'est **pas** activé : le dossier de cache est configurable, un scope d'asset statique
  serait faux ou trop large, et la CSP (`img-src … data:`) reste inchangée.
- **Nettoyage du cache** : `sources_remove` d'une source catalogue, et `sources_update` qui
  change son URL, suppriment `<cache>/catalogs/<source_id>/` et l'état en mémoire de la source
  (`CatalogService::purge_source`). Seul ce sous-dossier est supprimé : identifiant validé
  (`[A-Za-z0-9_-]`, 64 caractères au plus), jamais le dossier de cache ni `catalogs/`, aucun
  lien suivi. Un échec de purge est journalisé (la source est déjà modifiée ; le cache
  orphelin n'est jamais relu).
- **Revalidation des vignettes** : chaque `thumbs/<hash>.img` a un suivi `<hash>.meta.json`
  (`etag`, `last_modified`, `fetched_at`). Au-delà de 7 jours, la vignette est redemandée avec
  `If-None-Match` / `If-Modified-Since` (304 : copie conservée) ; en cas d'échec, la copie locale
  reste servie ; après un échec réseau (connexion, délai, 5xx) la source n'est pas recontactée pendant 10 minutes (copies locales servies sans réseau). Une vignette sans suivi (cache antérieur) est redemandée une fois. Une URL
  de vignette modifiée a un autre nom de fichier, donc est téléchargée à nouveau. À chaque
  rafraîchissement réussi (ni hors ligne ni en erreur), les vignettes qui ne correspondent plus
  à aucune entrée sont supprimées. L'écriture d'une vignette prend le portillon du cache en
  lecture : un vidage du cache (`cache_clear`) l'attend.
- **Limite connue** : une vignette téléchargée pendant qu'un rafraîchissement remplace le
  catalogue peut rester jusqu'au rafraîchissement suivant.

### Événements

Convention : événements émis par le backend, nommés `domaine://action` (ex. `install://progress`),
avec une charge utile sérialisable typée côté `src/api/`.

| Événement             | Charge                                                                                                                                                          |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `install://progress`  | `{ task_id, state, percent, received, total, phase }` (au plus toutes les 200 ms par tâche, plus chaque changement d'état)                                      |
| `install://done`      | `{ task_id, entry: LibraryEntry, warnings }`                                                                                                                    |
| `install://error`     | `{ task_id, state: "failed" \| "cancelled", error: TaskError }`                                                                                                 |
| `player://started`    | `PlayerSession { origin, uuid, title, pid, started_at }`                                                                                                        |
| `player://exited`     | `{ session, code, success, duration_ms, stderr, saves_error }` (`code` nul : processus tué ; `saves_error` : progression non conservée telle quelle, sinon nul) |
| `player://updated`    | `PlayerSession` (le player s'est replié en fenêtre séparée : `embedded` faux)                                                                                   |
| `player://focus-host` | aucune (F6 dans le player incrusté : le clavier revient au launcher)                                                                                            |

### Lancement du player (`player`)

`player_launch(uuid)` lance une histoire de la bibliothèque, `player_launch_folder(source_id,
entry_id)` une histoire `.plain` / `.plain.pk` du dernier scan d'une source dossier (sans
installation ; un zip Telmi est refusé). **Le front ne fournit jamais de chemin** : le fichier est
relu de `library.json` ou du scan, le player vient de `settings::resolve` (configuré valide, sinon
détecté) et est revérifié juste avant l'exécution. Commande :
`flam-player.exe --stdin-control [--parent-hwnd <n>] "<fichier>"` sans shell, le chemin en **un
seul argument** (dernier), dossier courant = dossier du player.

- **Incrustation (#201, Windows)** : le player s'affiche dans l'écran **Lecture** du launcher.
  Avant le lancement, `embed::TauriEmbed` crée dans la fenêtre `main` une fenêtre enfant native
  (`STATIC`, `WS_CHILD | WS_CLIPSIBLINGS`, cachée) dont le HWND est passé par `--parent-hwnd` ; le
  player remplit sa zone cliente et suit sa taille. Toutes les opérations Win32 ont lieu sur le
  thread principal (`run_on_main_thread`) ; la fenêtre d'accueil est détruite à la fin du player.
  `PlayerSession.embedded` dit si le lancement est incrusté.
- **Placement** : l'écran Lecture (`src/features/player/PlayerScreen.tsx`) mesure sa zone
  (`getBoundingClientRect`, `ResizeObserver`, redimensionnement de la fenêtre) et appelle
  `player_embed_layout(rect)` (px CSS) ; le backend convertit en px physiques avec le DPI de la
  fenêtre (`GetDpiForWindow`, `embed::to_physical`) puis `SetWindowPos`. `null` cache la fenêtre
  d'accueil (démontage de l'écran). Sans effet si aucun player incrusté ne tourne.
- **Écran modal** : tant qu'un player incrusté tourne, l'écran Lecture remplace toute
  l'application (pas de navigation : la fenêtre native resterait au-dessus des autres écrans, des
  dialogues et des notifications). La fenêtre n'est jamais réduite pour un player incrusté
  (le paramètre « Réduire la fenêtre pendant la lecture » ne vaut que pour la fenêtre séparée).
- **Repli** : hors Windows, si la fenêtre d'accueil ne peut pas être créée, ou avec
  `FLAM_LAUNCHER_NO_EMBED=1`, le player est lancé sans `--parent-hwnd`, dans sa propre fenêtre,
  comme avant. Si c'est le player qui ne parvient pas à s'incruster, il trace `[EMBED] repli : …`
  sur stderr et ouvre sa fenêtre séparée : le lecteur de stderr repère cette trace, la fenêtre
  d'accueil est détruite, la session passe à `embedded = false` et `player://updated` (charge :
  `PlayerSession`) l'annonce ; l'interface quitte alors l'écran Lecture (affichage sans
  incrustation, comme une fenêtre séparée). Si la création de la fenêtre d'accueil ne répond pas
  en 3 s, elle est abandonnée (et détruite si elle a eu lieu entre-temps).
- **Clavier** : à l'ouverture de l'écran Lecture, le focus va au titre ; « Clavier au lecteur »
  (`player_embed_focus`) donne le clavier à la fenêtre du player (sinon un clic dans la zone) ;
  à la fin, le focus revient au contenu principal. Les erreurs d'arrêt s'affichent en alerte
  dans l'en-tête, hors de la zone (une notification pourrait passer sous la fenêtre native).
  `player_stop` qui renvoie faux (aucun player : fin manquée) ferme l'écran ; sans fin de
  partie 10 s après « Arrêter », le bouton redevient disponible avec une alerte.
  **Retour du clavier (F6)** : Tab reste dans la fenêtre native du player et Échap est le
  retour du player ; la touche **F6**, active dans le player incrusté seulement, rend le
  clavier au launcher. Le player donne le focus à la fenêtre racine du launcher et écrit la
  trace `[EMBED] focus-host` sur stderr ; le lecteur de stderr la relaie par l'événement
  `player://focus-host` (sans charge), le backend donne alors le focus système à la WebView et
  l'écran Lecture place le focus sur « Clavier au lecteur ». Pas de piège clavier : F6 ressort,
  « Clavier au lecteur » rentre. L'aide sous la zone le rappelle. Un ancien `flam-player.exe`
  sans F6 oblige à cliquer hors de la zone. Les placements `player_embed_layout` sont envoyés
  l'un après l'autre (le dernier rectangle l'emporte).
- **Arrêt (`player_stop`, bouton « Arrêter »)** : le player est lancé avec `--stdin-control` et
  son entrée standard est un tube gardé par le service pendant la session. `player_stop` y écrit
  `quit` : le player sauvegarde et sort avec le code 0 (`player://exited`, `success`). S'il tourne
  encore 5 s plus tard (même session), il est arrêté de force (`TerminateProcess`, code 1) ; le
  pid ne peut pas avoir été réutilisé, le processus étant détenu (non attendu) par le thread
  d'attente, et la vérification se fait sous le verrou que ce thread prend à la fin. Si le
  launcher s'arrête, le tube se ferme : le player (fin de stdin) s'arrête proprement lui aussi.
  Un ancien `flam-player.exe` qui ne connaît pas ces options les prend pour des chemins et
  retient le dernier argument, l'histoire : il reste lançable (sans arrêt par stdin).

- **Un seul player à la fois** : un second lancement est refusé (`player_busy`) ; le créneau est
  libéré avant `player://exited`, on peut relancer dès réception. `player_status` restitue l'état
  après un rechargement de l'interface.
- **Fin** : `player://exited` porte le code, la durée et les 20 dernières lignes de stderr
  (8 Kio) ; `success = false` (code non nul ou processus tué) : afficher `stderr`, c'est la
  raison d'un échec au démarrage (« Erreur de chargement… »).
- **Copie de travail** : le player extrait un `.plain.pk` et écrit ses sauvegardes (`saves/<nom>`)
  à côté de l'histoire (le code C n'offre aucune option pour le changer en mode histoire). Il
  n'est donc **jamais lancé sur l'original** (dossier de l'utilisateur ni `library/`) : l'histoire
  est copiée dans `<cache>/play/<empreinte de la clé>/<nom>` (`Workspace::stage`, sous le portillon
  du cache en lecture), vidée avant chaque lancement, puis lancée. Clé : `library:<uuid>` ou
  `folder:<source>:<entrée>`. La source doit être, sans suivre de lien, un fichier ordinaire
  (`.plain.pk`) ou un dossier (`.plain`) conforme au `kind` de l'entrée.
  Le refus d'une **jonction** est testé en CI ; celui d'un **lien symbolique** ne l'est pas (test
  `#[ignore]` : sa création demande le mode développeur ou l'élévation) : à lancer à la main avec
  `cargo test check_source_refuse_les_liens_symboliques -- --ignored`.
- **Sauvegardes hors cache** : la progression vit dans `<config>/saves/<empreinte>/` (données
  de l'application, **jamais effacées par « Vider le cache »**). Avant le lancement elle est
  recopiée vers `<copie>/saves/` ; à la fin du player, succès ou échec (avant la libération du
  créneau), `<copie>/saves/` est rapatrié (copie vers un dossier `.tmp` voisin puis échange par
  renommage, l'ancienne version étant gardée en `.old` jusqu'au succès et reprise au lancement
  suivant si l'application s'arrête entre les deux). Liens ignorés, au plus 5000 fichiers et
  64 Mio, 16 niveaux ; au-delà, voir « Limites des sauvegardes ». Le nom du sous-dossier
  suit le fichier lancé : une nouvelle version d'une histoire de la bibliothèque
  (`<uuid>-<version>.plain.pk`) repart donc d'une progression vierge.
- **Limites des sauvegardes** : au-delà des limites (5000 fichiers, 64 Mio, 16 niveaux), la
  progression n'est plus un motif de blocage. Le `saves/` trop gros est mis de côté en
  `<config>/saves/<empreinte>.rejected` (**une seule génération** : un nouveau rejet remplace
  le précédent, remplacé seulement une fois le nouveau contenu en sécurité dans
  `<empreinte>.rejected.new` ; si le renommage échoue (autre volume), copie sans aucune limite
  (profondeur comprise ; parcours de copie itératif, sans récursion) puis suppression de la
  source seulement après réussite : jamais de suppression sans copie ; la copie se fait dans
  `<empreinte>.rejected.new.part` (partiel si l'on s'arrête en route), puis un renommage atomique
  en `.rejected.new`, toujours complet, avant la suppression de la source ; le repli par copie **ignore** les liens de la source, qui
  disparaissent avec elle, sans toucher à leur cible). Si le remplacement de `.rejected` échoue
  **après** ce déplacement, ce n'est **pas une erreur** : la progression est intacte dans
  `.rejected.new`, la note le dit à l'utilisateur (`saves_error`) et l'état est réparé au
  lancement ou rapatriement suivant (`.rejected.new` promu en `.rejected`, `.rejected.new.part` partiel
  supprimé). La version précédente est
  conservée, un avertissement est tracé et le message est renvoyé dans
  `player://exited.saves_error` (`string | null` côté TS) : l'interface doit l'afficher. Même
  traitement d'une sauvegarde conservée devenue trop grosse au moment de la restaurer (le
  lancement a lieu sans progression, l'information est portée par la fin de cette partie).
  Au-delà de 16 niveaux, c'est une erreur explicite (« arborescence trop profonde »), plus
  jamais une copie tronquée en silence. Les autres échecs (disque plein, fichier verrouillé)
  restent des erreurs : au rapatriement de fin de partie ils sont aussi dans `saves_error`, au
  lancement ils refusent le lancement sans rien supprimer. Quand le lancement échoue **après**
  la préparation de la copie de travail (`story_file_missing`, `player_unavailable` au spawn),
  les mises de côté déjà faites n'iront à aucun `player://exited` : leur texte est ajouté au
  message de l'erreur de la commande (« … Information sur la progression : … », `kind`
  inchangé) pour que l'interface l'affiche (`AppError::with_note`).
- **Compromis sur un plantage du player** : le rapatriement a lieu aussi quand le player
  échoue ; un `saves/` partiel (écriture interrompue) remplace donc la version précédente, qui
  n'est pas gardée au-delà de l'échange (`.old` supprimé après succès). Choix assumé : sans lui,
  une partie jouée longtemps avant un plantage serait perdue. Si cela pose problème, garder
  `.old` jusqu'au rapatriement suivant est une évolution simple.
- **Progression supprimée à la main** : effacer `<config>/saves/<empreinte>` à la main ne remet
  pas la progression à zéro tant que `<cache>/play/<empreinte>/saves` existe : elle est
  rapatriée depuis la copie de travail au lancement suivant (et ainsi « revient »). Pour une
  vraie remise à zéro, supprimer aussi la copie de travail (« Vider le cache » le fait) ou
  désinstaller avec `purge_saves`. Si l'interface ajoute une remise à zéro, elle devra purger
  les deux.
- **Désinstallation et mise à jour** : `library_remove(uuid, purge_saves?)` supprime la copie de
  travail `<cache>/play/<empreinte>/` ; la progression (`<config>/saves/<empreinte>/` et ses
  `.tmp`, `.old`, `.rejected`, `.rejected.new`, `.rejected.new.part`) n'est supprimée que
  si `purge_saves` est vrai (défaut : faux,
  une progression d'enfant ne se perd jamais par défaut ; réinstaller l'histoire la retrouve).
  Refusé avec `player_busy` si le player lit cette histoire. Tout se passe sous le créneau du player (un lancement ne peut pas s'intercaler ; un lancement
  qui attendait relit l'index sous le créneau et reçoit `not_found`) : verrou de la
  bibliothèque (attente de **2 s au plus** : si une autre instance le tient, erreur `io`
  « bibliothèque utilisée par une autre instance… réessayer » sans rien purger, pour ne pas
  retarder `player_launch` ni la sortie du player), vérification de l'installation (`not_found`
  avant toute suppression), puis purge, puis retrait, le tout sous ce verrou. La purge est
  faite **avant** le retrait de la bibliothèque : un échec laisse
  l'histoire installée (désinstallation à retenter). Les `.rejected` ne sont libérés qu'avec
  `purge_saves` (une génération par histoire au plus). Aucun composant n'affiche encore
  `saves_error` ni n'appelle `purge_saves` (suite : #18). Mettre à jour une histoire
  (nouvelle version du même `uuid`) ne supprime rien : la progression de l'ancienne version
  reste dans `saves/<empreinte>/<uuid>-<ancienne version>.plain.pk/…` (le player relit
  `saves/<nom du fichier lancé>`, voir ci-dessus : la nouvelle version repart de zéro) et n'est
  libérée qu'à la désinstallation avec `purge_saves`. Sans UI pour l'instant (suite : #18).
- **Statut sans blocage** : `player_status` (commande `async`) utilise `try_lock` : le créneau
  est tenu pendant la préparation d'un lancement, un vidage du cache (`while_idle`) et une
  désinstallation (`while_not_playing`, possible pendant qu'un player lit une autre histoire), et
  ne doit pas bloquer un worker tokio. Créneau tenu : réponse `null` **même si un player
  tourne** (pas une preuve d'absence) ; fiez-vous à `player://started` / `player://exited`. La
  règle du player unique reste garantie par `launch` (`player_busy`).
- **Partie non rapatriée** : si le launcher a été fermé ou tué pendant la partie, ou si le
  rapatriement a échoué, `<copie>/saves/` existe encore au lancement suivant. `stage` le
  rapatrie (même fonction, idempotente) **avant** de vider la copie de travail ; en cas
  d'échec, le lancement est refusé (erreur explicite) et rien n'est supprimé.
- **Robustesse** : le créneau est tenu par une garde libérée même en cas de panique ou d'échec de
  thread ; la fin du player ne dépend pas de la fermeture de stderr (attente bornée à 500 ms).
- Tests : l'exemple `src-tauri/examples/fake_player.rs` (scénario lu dans le fichier d'histoire)
  sert de faux player ; `cargo test` le construit. Wrappers : `src/api/player.ts`
  (`launchPlayer`, `launchFolderStory`, `getPlayerStatus`, `onPlayerStarted`, `onPlayerExited`).
- **Instance unique du launcher** : plugin officiel `tauri-plugin-single-instance` (2.4.5,
  MIT ou Apache-2.0, enregistré en premier dans `lib.rs`). Un second lancement se termine
  aussitôt et ramène la fenêtre de la première instance au premier plan (restaurée, focus).
  Deux launchers ne se disputent donc ni le cache, ni `<config>/saves/`, ni la file
  d'installation.

### Pipeline d'installation (`install`, `library`)

`install_start(source_id, entry_id)` crée une tâche pour une entrée **retrouvée par le backend**
(catalogue : cache, sans réseau ; dossier : dernier `folder_scan`, sinon `not_found`) : le front ne
transmet jamais d'URL ni de chemin. Une tâche active pour le même `uuid` est renvoyée telle quelle.
`InstallTask { id, entry, state, percent, error, created_at }`, `state` parmi `queued`,
`downloading`, `converting`, `installing`, `done`, `failed`, `cancelled` ;
`TaskError { kind, message, code }` (`code` : code `telmi2flam` pour `kind = "conversion"`).

1. **Téléchargement** (lien http(s) seulement) vers `<cache>/archives/<uuid>-<version>.<zip|plain.pk>`
   via `<…>.part` : flux, progression, taille vérifiée sur `Content-Length` / `Content-Range`
   (4 Gio au plus), archive zip vérifiée en entier (répertoire central lisible : sans
   `Content-Length`, une coupure ne passe pas pour un succès), reprise par `Range` + `If-Range` (ETag fort ou
   `Last-Modified`, mémorisé dans la file) ; un `416` ou une réponse `200` repart de zéro. Délai
   de connexion 15 s, 60 s au plus sans données, pas de délai total. Annulation : `.part` supprimé.
2. **Conversion** (zip Telmi) : `python telmi2flam.py <archive> -o <cache>/library/.staging/<tâche>.plain.pk
--progress-json`, processus sans console (`CREATE_NO_WINDOW` + `CREATE_NEW_PROCESS_GROUP`),
   interpréteur durci (`-E -s -B -u -X utf8`, lancé depuis `library/.staging/` ; `-I` retirerait
   le dossier du script de `sys.path`, d'où `telmi2flam` importe ses modules). stdout est lu ligne par ligne (ligne non JSON ou événement inconnu
   ignoré) ; stderr, décodé en tolérant les erreurs, ne sert qu'au message de repli (fin du
   flux, lu par morceaux dans un tampon borné à 4 Kio) si aucun événement `error` n'est reçu
   (`code` = `exit_<n>`). Le convertisseur est lancé **suspendu** (`CREATE_SUSPENDED`), rattaché
   à un Job Object Windows (`KILL_ON_JOB_CLOSE`), puis son thread principal est repris : aucun
   sous-processus ne peut échapper au job, et l'arrêt forcé les termine tous. Si le job ne peut pas
   être créé (avertissement tracé), la conversion continue sans lui : l'arrêt forcé se limite alors
   au processus direct. La création du job et la reprise (appels Win32 bloquants) s'exécutent
   hors du runtime (`spawn_blocking`).
   **Annulation** : `CTRL_BREAK_EVENT` (le convertisseur nettoie et sort avec 130), envoyé par un
   court assistant Python isolé (`-I`) sans fenêtre qui s'attache à la console cachée du convertisseur (une
   application graphique n'a pas de console, et `GenerateConsoleCtrlEvent` ne vise que la
   sienne) ; arrêt forcé après 5 s de grâce ; puis suppression de `<sortie>.<pid>.part`.
   Un `.plain.pk` est vérifié (`info.plain`, `uuid.bin`, `main.lua`) et copié ; un dossier `.plain`
   est empaqueté en zip « stored » : racine, fichiers requis ou entrées qui sont des liens
   symboliques ou jonctions sont **refusés** (fichiers ouverts sans suivre les liens), taille
   cumulée bornée à 4 Gio. Les fichiers **OneDrive « à la demande »** (non téléchargés) sont des
   points de réanalyse : ils sont refusés de même, sans téléchargement silencieux ; copier le
   dossier hors du dossier synchronisé, puis réanalyser (« toujours conserver sur cet appareil »
   n'est pas garanti de lever le refus).
3. **Installation** : `uuid` et `version` **revalidés** (`story::validate_story_id`), renommage
   vers `<cache>/library/<uuid>-<version>.plain.pk` (version inconnue : `0`), chemin vérifié sous
   `library/`, index `library.json` mis à jour, archive téléchargée supprimée. Ordre sûr : un marqueur d'intention `.staging/<fichier>.pending` est écrit, l'ancien fichier de
   même nom passe en `.staging/*.bak` (**unique par fichier** : une installation est refusée tant
   qu'un `.bak` ou un marqueur du même fichier subsiste, un `.bak` n'est donc jamais écrasé ; il
   n'est jamais jugé à sa taille, seul le marqueur fait foi), le nouveau est renommé, l'index est écrit, puis le
   marqueur est effacé (échec : retour à l'état précédent). Au chargement, seules les
   publications **qui ont un marqueur** sont défaites ; un `.plain.pk` non indexé sans marqueur
   n'est jamais supprimé. Le marqueur n'est effacé que si la restauration a réussi ; sinon
   marqueur et `.bak` sont conservés (avertissement) et la réconciliation suivante réessaie (publication terminée : le `.bak` est supprimé avant le marqueur, qui reste si la suppression échoue ; marqueur illisible avec un `.bak` : rien n'est supprimé) ; tant qu'ils subsistent, une nouvelle installation du même fichier est refusée (erreur de validation), sans rien toucher.
   Le marqueur identifie la publication par `installed_at` **et la taille attendue** du fichier
   publié (deux publications du même fichier dans la même seconde restent distinguées), et liste
   les **anciennes versions du même `uuid`** : leur suppression se fait avant celle du marqueur, qui
   reste (avertissement) si l'une échoue, donc rien ne fuit après un arrêt. Un index **corrompu**
   (mis de côté) ne défait ni ne nettoie rien, et tant qu'un `library.json.*.bak` existe, un index
   absent est traité de même. Le marqueur porte l'entrée d'index de la publication : avant de
   réécrire l'index, `install` **réintègre** les publications terminées (fichier présent à la
   taille attendue), de la plus ancienne à la plus récente (la plus récente d'un même `uuid`
   l'emporte, les fichiers évincés sont supprimés avec les anciennes versions). Un marqueur
   **non jugeable** (ancien format, illisible, fichier absent ou de taille différente) qui
   concerne le fichier ou l'`uuid` installé fait refuser l'installation, sans rien toucher :
   « installation de `<fichier>` refusée : le marqueur `.staging/<x>.pending` ne peut pas être
   jugé (index perdu, sauvegardé en `library.json.*.bak`). Vérifier `library/<x>`, puis
   supprimer ce marqueur (et `.staging/<x>.bak` s'il existe) ou restaurer l'index depuis la
   sauvegarde, et réessayer ». Ce refus est **précontrôlé avant le téléchargement** (au début de la
   tâche) quand l'index est perdu. Les marqueurs non jugeables des **autres** histoires ne
   bloquent pas l'installation, mais ils sont **neutralisés** au moment où l'index est réécrit :
   `.staging/<x>.pending` devient `.staging/<x>.pending.unjudged` (avertissement tracé). Sans
   cela, la réconciliation suivante les jugerait inachevés et supprimerait leur fichier (ou
   restaurerait son `.bak`). La réconciliation ne lit pas les `*.pending.unjudged` et ne touche
   jamais à leur fichier ni à leur `.bak` : à traiter à la main (vérifier `library/<x>`, puis
   supprimer le `.pending.unjudged` **et** `.staging/<x>.bak` s'il existe : tant que ce `.bak`
   subsiste, réinstaller le même fichier est refusé « restauration ou nettoyage précédent non
   terminé »).
   **Fuite ponctuelle** : un marqueur écrit par une version antérieure de l'application ne
   liste pas les anciennes versions du même `uuid` (`old_files`) ; s'il est terminé après la
   mise à jour de l'application, ces anciens `.plain.pk` restent dans `library/`, non indexés et
   sans marqueur, donc jamais supprimés (choix sûr) : à supprimer à la main, une seule fois.
   Aucun `.plain.pk` réintégré n'est perdu. L'attente du verrou de la bibliothèque est unique
   (30 s) : au-delà, erreur « bibliothèque utilisée par une autre instance… réessayer » et le
   fichier préparé est supprimé.
   Publication et réconciliation tiennent un verrou inter-processus
   (`library/.lock`, aussi entre threads, sans autre verrou devant lui) ; `library_list` ne fait
   que l'essayer, donc ne se fige jamais derrière une installation ou une autre instance ;
   `install` et `remove` l'attendent au plus 30 s (2 s pour `library_remove`, voir plus haut)
   (erreur `io` explicite « bibliothèque occupée… réessayer plus tard » au-delà) ; si une autre instance le tient, ou si le verrou ne peut pas être ouvert,
   la liste lit simplement l'index sans réconcilier (l'installation et la suppression, elles,
   échouent). Les avertissements répétés à chaque `list` (verrou indisponible, restauration ou
   nettoyage bloqué) ne sont émis qu'une fois par site d'appel et par processus, puis en `debug`.
4. **Vignette** (#80, `library_thumb`) : préparée avant la publication, sans réseau — vignette
   déjà téléchargée dans le cache du catalogue d'origine (`catalogs/<id>/thumbs/`), sinon
   `img/thumbnail.lif` du `.plain.pk` produit, réencodée en PNG ≤ 192 px ; image reconnue à sa
   signature (PNG, JPEG, GIF, WebP), **256 Kio au plus**. Écrite sous le verrou, après les refus
   et avant l'index, dans `library/thumbs/<uuid>-<version>.<installed_at>.<ext>` (nom unique par
   publication : la vignette d'une installation précédente n'est jamais écrasée), référencée par
   `LibraryEntry.thumb` (absent des index antérieurs : `null`). Une vignette ne fait **jamais**
   échouer l'installation : préparation ou écriture en échec → `thumb: null`. Après chaque
   écriture réussie de l'index (installation avec un index qui faisait foi, désinstallation),
   les fichiers de `thumbs/` que l'index ne référence pas sont supprimés (publication défaite,
   ancienne version, histoire désinstallée, temporaires) ; index perdu, ou mis de côté
   (`library.json.*.bak` présent, restaurable à la main) : rien n'est supprimé (#97), et un
   avertissement unique par processus le signale ; `library/` illisible : rien non plus (#108).
   « Vider le cache » supprime `library/`, donc histoires et vignettes ensemble ; vider les seules
   vignettes de catalogue ne touche pas celles de la bibliothèque. Le front (« Mes histoires » et la
   fiche) demande `library_thumbnail(uuid)`, puis la source d'origine (installation antérieure),
   puis affiche l'illustration par défaut.

- **Concurrence** : `max_downloads` téléchargements simultanés (`settings.json`, 1 à 4, 2 par
  défaut, relu à chaque tâche), **une** conversion à la fois.
- **Cache** : chaque tâche tient le portillon du cache (`CatalogService::cache_gate`) en lecture :
  « Vider le cache » attend la fin des tâches en cours et bloque les nouvelles.
- **File persistée** : `<config>/install_queue.json` (`schema_version`, tâches non terminées avec
  leur validateur HTTP). Au démarrage, le fichier est relu comme une donnée non fiable : une tâche
  dont l'identifiant n'est pas un UUID canonique est ignorée, et l'entrée est **ré-résolue depuis
  sa source** (`source_id`, `entry_id` : cache du catalogue, ou analyse récursive du dossier)
  au lieu d'être reprise du fichier ; introuvable, la tâche est abandonnée (journal). Les
  téléchargements reprennent si le lien et la version n'ont pas changé. Tout chemin dérivé
  (`.staging/<tâche>.plain.pk`, `archives/<uuid>-<version>.<ext>`) est vérifié directement sous
  son dossier. Une tâche annulée (y compris en attente du créneau de conversion) supprime son
  archive téléchargée.
  Les tâches terminées ne sont gardées qu'en mémoire (100 au plus). Le lien et le validateur
  d'une archive sont mémorisés dans `archives/<archive>.meta.json` (4 Kio au plus, validateur revalidé) : une nouvelle demande après
  une coupure reprend le `.part`, et une archive complète n'est réutilisée que si son lien et
  sa taille correspondent (sinon elle est retéléchargée). L'écriture de `install_queue.json` se
  fait hors des verrous, sur un thread bloquant, avec des écritures ordonnées.
- **Convertisseur** : `<exe>/telmi2flam/telmi2flam.py` (application installée ; construit par `npm run build:converter`, voir « Convertisseur embarqué »),
  sinon en développement `tools/telmi2flam/telmi2flam.py` d'un dossier ancêtre. Python : `resolved.python_path`.
- **Bibliothèque** : `library.json` = `{ schema_version, stories: LibraryEntry[] }` avec
  `LibraryEntry { uuid, version, title, file, size, installed_at, converter, source: StoryEntry }`.
  `converter` est la version de `telmi2flam` lue dans le champ `version` de l'événement `start`
  (#60), avec en repli l'empreinte `sha256:<12 hex>` de `telmi2flam.py`. **Une installation par `uuid`** : la dernière installée remplace la précédente (autre
  version, ou `.plain` / `.plain.pk` de même `uuid` d'un dossier source). Un index corrompu est
  sauvegardé en `*.bak` et repart vide. `library_list` ignore les entrées dont le fichier a
  disparu.
- **Mises à jour** : `hasUpdate(installed, entry)` (front) et `LibraryEntry::is_outdated_by`
  (Rust) : même `uuid`, versions connues et différentes.
- **Suivis de revue** (stderr borné, Job Object, reprise après erreur réseau, réconciliation de
  l'index…) : #62.
- **Test réel** (hors CI) : `FLAM_TELMI_ZIP=<archive.zip> cargo test installation_reelle -- --ignored --nocapture`.

### Données

- Configuration : `%APPDATA%/flam-launcher` (`settings.json`, `sources.json`), détaillée ci-dessous.
- Cache (catalogues, vignettes, téléchargements, histoires converties) : dossier configurable, par défaut sous les
  données locales de l'application. Jamais dans le dépôt.

#### `settings.json`

```json
{
  "schema_version": 1,
  "player_path": null,
  "python_path": null,
  "cache_dir": null,
  "default_catalog_url": "https://gist.githubusercontent.com/DantSu/49ed776755f3a01c995e78e3fd1cb79f/raw/telmi-interactive.json",
  "theme": "system",
  "max_downloads": 2
}
```

`max_downloads` (1 à 4, 2 par défaut) borne les téléchargements simultanés du pipeline
d'installation ; absent d'un fichier ancien, il prend sa valeur par défaut (`#[serde(default)]`).

`theme` (`system`, `light` ou `dark`) est le thème de l'interface. Le champ est lu avec
`#[serde(default)]` : un fichier écrit avant son ajout prend `system`, sans migration de schéma
(`schema_version` reste 1) ; une valeur inconnue retombe aussi sur `system` au lieu de faire
rejeter le fichier. Au démarrage, l'interface applique le thème enregistré.

Un chemin à `null` signifie « automatique » ; `settings_get` renvoie aussi les valeurs effectives
(`resolved`) :

- `player_path` : `flam-player.exe` à côté de l'exécutable de l'appli, sinon (build de
  développement) `build/flam-player.exe` dans un dossier ancêtre (racine du dépôt) ;
- `python_path` : d'abord le Python embarqué `<exe>/telmi2flam/python/python.exe` (présent seulement si
  `<exe>/telmi2flam/telmi2flam.py` l'est aussi), sinon `python.exe`, `python3.exe` puis `py.exe` (lanceur Windows) trouvés dans le
  `PATH` (aucun processus ni script n'est exécuté ; les entrées relatives du `PATH` sont ignorées) ;
- `cache_dir` : `%LOCALAPPDATA%/flam-launcher/cache`.

**Alias du Microsoft Store** : un fichier vide dans `WindowsApps` est ignoré (savoir s'il désigne
un Python réellement installé exigerait de le lancer). Si c'est tout ce qui est trouvé,
`issues` contient `{ field: "python_path", kind: "store_alias" }` : l'interface invite à choisir
le chemin de `python.exe`.

`settings_set` normalise (espaces, chaîne vide = automatique) puis valide :

- `player_path` et `python_path` : chemin **absolu**, **local** (UNC `\\serveur\…`, `\\?\UNC\…`
  et lecteurs réseau refusés), extension `.exe` (insensible à la casse), fichier existant ;
- `cache_dir` : chemin absolu qui n'est pas un fichier, ni racine de disque, ni dossier
  utilisateur, ni dossier Windows. L'application ne supprime **jamais** que les sous-dossiers
  qu'elle a elle-même créés dans le cache, jamais le dossier configuré lui-même (voir
  « Cache » ci-dessous) ;
- `default_catalog_url` : URL http(s) avec hôte, 2048 caractères max, sans identifiants
  (`user:pass@`).

Le fichier existant est relu avant d'écrire : version de schéma future refusée sans toucher au
fichier, fichier corrompu sauvegardé en `*.bak`.

**Revalidation à la lecture** : à chaque `settings_get`, une valeur configurée devenue invalide
ou disparue est écartée au profit de la détection automatique dans `resolved`, et signalée dans
`issues` (`[{ field, kind, message }]`, `kind` = `invalid_path`, `not_found` ou `store_alias`).
La valeur enregistrée dans `settings` n'est pas modifiée. `resolved.*` n'est **pas une
garantie** : le fichier peut disparaître ensuite, le lancement (#16) revérifie juste avant
d'exécuter. Accesseurs Rust pour les issues suivantes : `SourcesStore::get(id)` et
`SettingsStore::resolved_cache_dir` (dossier créé si absent).

#### Cache

`cache_usage` renvoie le dossier de cache effectif et les octets occupés ; `cache_clear`
supprime et renvoie les octets libérés. Le dossier de cache pouvant être choisi par
l'utilisateur, ces deux commandes ne comptent et ne suppriment **que** les sous-dossiers créés
par l'application, listés dans `APP_CACHE_SUBDIRS` (`cache.rs`) : `archives`, `catalogs`, `compat-local`,
`library`, `play`, `thumbs`. Le reste du dossier et le dossier lui-même ne sont jamais touchés ; les
liens symboliques et jonctions ne sont ni suivis ni supprimés. `play/` ne contient que des copies de travail jetables ; les sauvegardes du player sont dans `<config>/saves/`, hors cache (voir « Lancement du player »). **Tout module qui crée un nouveau
sous-dossier de cache doit l'ajouter à cette liste.** L'interface demande une confirmation
(boîte `alertdialog`, focus sur « Annuler ») qui rappelle les dossiers concernés et la taille
libérée. `cache_clear` est refusé avec `player_busy` tant qu'un player tourne (sa copie de
travail est dans `play/`) ; le créneau du player est tenu pendant le vidage
(`PlayerService::while_idle`), aucun lancement ne peut démarrer entre-temps.

#### `sources.json`

```json
{
  "schema_version": 1,
  "sources": [
    {
      "type": "catalog",
      "id": "telmi-store",
      "name": "Telmi Store",
      "url": "https://…/telmi-interactive.json"
    },
    { "type": "folder", "id": "3f2c…", "name": "Mon dossier", "path": "C:\\Histoires" }
  ]
}
```

- Liste ordonnée ; les identifiants sont des UUID v4 (« Telmi Store » : `telmi-store`).
- « Telmi Store » est créée avec le fichier ; une fois supprimée, elle ne revient pas.
- « Mes histoires » (`type: "library"`, id `library`) est **implicite** : jamais écrite dans le
  fichier, toujours renvoyée en premier par `sources_list`, ni modifiable, ni supprimable, ni
  réordonnable.
- Un fichier édité à la main avec des identifiants en double est dédoublonné au chargement
  (premier conservé, avertissement tracé).
- Validation : nom non vide sans caractère de contrôle (100 caractères max), URL http(s) avec
  hôte, 2048 caractères max, sans identifiants, dossier absolu existant,
  type immuable à la mise à jour ; `sources_reorder` exige une permutation exacte des identifiants
  enregistrés.

#### Écriture, corruption, migration

- Écritures atomiques : fichier temporaire voisin (nom unique : pid + compteur) puis renommage,
  avec quelques nouvelles tentatives si l'accès est refusé (antivirus).
- `schema_version` absent = version 0. Les migrations s'appliquent version par version
  (`MigrationStep` dans `storage.rs`) ; un fichier de version **supérieure** à celle de
  l'application est refusé (`config_version`) sans être modifié.

### Conventions front

- Tout appel au backend passe par `src/api/` (fonctions typées, miroir des structures Rust) ;
  les composants n'appellent jamais `invoke` directement.
- Un dossier par fonctionnalité dans `src/features/`, composants partagés dans `src/components/`
  (voir [docs/DESIGN.md](docs/DESIGN.md) ; démonstration en dev sur `http://localhost:1420/#/demo`).
- Aucun texte en dur dans les composants : tout passe par `src/i18n/`.
- Accessibilité vérifiée dans les tests avec axe-core (`src/test/axe.ts`).
- Tests à côté du code (`*.test.ts(x)`), API Tauri simulée avec `@tauri-apps/api/mocks`.
- TypeScript strict, formatage Prettier, ESLint sans avertissement.

## Outillage

- TypeScript est épinglé à 6.0.3 : `typescript-eslint` n'accepte pas encore TypeScript >= 6.1.
  À relever quand le plugin le permettra.
- Le premier `npm run tauri build` télécharge WiX (MSI) et NSIS (installeur) depuis GitHub ;
  les builds suivants les réutilisent. Cibles de bundle : `nsis` et `msi` (`tauri.conf.json`),
  `nsis` seul avec `tauri.bundle.conf.json` (installateur livré).
- Contrôles : `npm run lint` (sans avertissement), `npm run format:check`, `npm run typecheck`,
  `npm test`, puis dans `src-tauri/` `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

## Sécurité

- CSP stricte définie dans `src-tauri/tauri.conf.json` : scripts, objets, frames et formulaires
  verrouillés. `style-src` garde `'unsafe-inline'` : Tauri et React injectent des styles en ligne
  (attributs `style`), qu'une CSP à nonce ne couvre pas ; aucun script en ligne n'est autorisé.
- La capability `core:default` (`src-tauri/capabilities/default.json`) est volontairement large
  pour le squelette : elle sera restreinte permission par permission au fil des issues. Y
  s'ajoutent `core:window:allow-minimize`, `allow-unminimize` et `allow-set-focus` (réduire la
  fenêtre pendant la lecture, option _Réduire la fenêtre pendant la lecture_), appelées depuis
  `src/api/window.ts` par l'API de fenêtre de Tauri, sans code Rust.
- Plugin `tauri-plugin-dialog` (sélecteurs natifs de dossier et de fichier `.exe`) avec la seule
  permission **`dialog:allow-open`** : ni `dialog:default` (messages, confirmations, sauvegarde)
  ni accès au système de fichiers depuis le front. Le chemin choisi n'est qu'une proposition :
  il est revalidé par le backend (`sources_add`, `settings_set`).

## Écrans

Routage par hash (`src/router.ts`), sans bibliothèque : `#/` accueil, `#/sources`,
`#/settings`, `#/library` (première source) et `#/library/<id de source>`. Captures dans
`docs/screenshots/` :

- **Bibliothèque** (`src/features/library/`) : la barre latérale liste « Mes histoires » puis
  les sources dans l'ordre de `sources.json`, avec nombre d'histoires et indicateur hors ligne
  / erreur. La vue d'une source affiche une grille de `StoryCard` (vignettes chargées à la
  demande quand la carte approche de l'écran, 60 cartes puis « Afficher plus »), une
  recherche (titre, auteur, catégorie, description ; sans accents ni casse), les filtres âge
  (histoires recommandées dès cet âge ou avant ; sans âge connu : toujours visibles) et
  catégorie, le tri (titre, plus récentes, popularité : `download_count` décroissant, entrées sans
  valeur à la fin, départage par titre ; l'option n'apparaît que si au moins une entrée de la
  source publie `download_count`, donc pas pour un dossier ni « Mes histoires ») et « Actualiser ». Un catalogue est lu dans le cache, puis téléchargé s'il ne l'a
  jamais été (`catalog_entries` puis `catalog_refresh`) ; un dossier est réanalysé
  (`folder_scan`, éléments ignorés détaillés dans un bandeau). Chargement, erreur (bouton
  Réessayer), hors ligne, liste vide et « aucun résultat » ont chacun leur état. « Mes histoires »
  liste les histoires installées (`library_list`) ; chaque carte garde les métadonnées de sa
  source d'origine.
  État affiché par carte : _à télécharger_ (catalogue), _à convertir_ (`.zip` Telmi local),
  _à installer_ (`.plain`, `.plain.pk`), puis _installée_, _mise à jour disponible_,
  _en cours_, _incompatible_, _non testée_, fournis par les points d'accroche ci-dessous.
  ![Bibliothèque, thème clair](docs/screenshots/bibliotheque-clair.png)
  ![Bibliothèque, thème sombre](docs/screenshots/bibliotheque-sombre.png)
  ![Dossier local avec élément ignoré](docs/screenshots/bibliotheque-dossier-clair.png)
  ![Catalogue hors ligne](docs/screenshots/bibliotheque-hors-ligne-clair.png)

  **Points d'accroche** (`StoryHooks`, props de `LibraryScreen` / `SourceView`) :
  `renderDetail` (fiche ouverte au clic sur un titre ; remplace `onOpenStory` /
  `selectedEntryId`), `reloadKey` (relit « Mes histoires »), `resolveState(entry)` (installée, mise à jour, incompatible…), `progressFor(entry)`
  (barre de progression, état « En cours »), `renderActions(entry)` (Télécharger, Jouer),
  `compatFor(entry)` (badge de compatibilité), `onOpenStory(entry)` (fiche détaillée), `selectedEntryId`. Les données de l'écran

  **Compatibilité** (`compat.rs`, `src/features/compat/`) : commande `compat_statuses` (sans argument, aucun
  chemin ne vient du front) qui renvoie `{ entries, issue }`. Sources fusionnées :
  le `status.json` de la campagne `tools/telmi2flam` (lu en lecture seule ; `schema_version` doit valoir 1,
  sinon `issue = unsupported_version` et tout est « non testée » ; fichier absent : aucun problème ;
  corrompu : `unreadable` ; plus de 8 Mio : `too_large`) et `<cache>/compat-local/launcher_results.json` (dossier vidé avec le cache ; `<cache>/compat/` ne l'est jamais), où chaque
  conversion faite par la surcouche enregistre son résultat réel (succès, avertissements, ou échec avec le code
  du convertisseur). Chemin du `status.json` : variable `FLAM_COMPAT_STATUS_FILE` (absolu), sinon en
  développement `<dépôt>/.cache/telmi/compat/status.json`, sinon `<cache>/compat/status.json`.
  Niveaux : `OK` compatible ; `OK_AVERTISSEMENTS` avertissements ; chaque `ECHEC_*` connu (`TELECHARGEMENT`, `CONVERSION`, `VALIDATION`, `EXECUTION`)
  incompatible ; autre statut (y compris un `ECHEC_*` inconnu), fiche absente ou illisible : non testée. Une conversion locale remplace une
  fiche périmée (`stale`), sinon le pire niveau l'emporte. Chaque carte affiche un badge supplémentaire ;
  la raison (`summary`, sinon une phrase par statut) est visible pour les avertissements et incompatibilités.
  `IncompatibleInstallDialog` (avertissement non bloquant avant installation d'une histoire incompatible)
  est affiché par `StoriesProvider` ; `compatBadge` / `describeCompat` alimentent les cartes et la fiche.
  (`loadSourceData`) et l'état des sources (`LibraryProvider`) sont testés avec l'API simulée.

- **Fiche, installation et lecture** (`src/features/stories/`) : un clic sur le titre d'une carte
  ouvre la **fiche** (`StoryDetail`, dialogue) : grande vignette, titre, description, âge,
  catégorie, auteur, voix, licence, taille, source, état, badge de compatibilité avec sa raison et
  actions selon l'état : _Télécharger et jouer_ / _Télécharger_ (catalogue ; _Installer…_ pour un
  dossier), _Jouer_, _Mettre à jour_ (le catalogue propose une autre version que celle
  installée), _Supprimer_ (avec confirmation), _Annuler_ (tâche active) ; un `.plain` /
  `.plain.pk` d'un dossier peut aussi être joué sans installation (_Jouer depuis le dossier_,
  `player_launch_folder`). Les cartes portent une action rapide (_Jouer_, _Télécharger_) et la barre
  de progression de leur tâche. Un seul modal à la fois : la fiche s'efface le temps d'une
  confirmation (suppression, installation d'une histoire incompatible).
  `StoriesProvider` (sous `CompatProvider`) tient les tâches (`install_list` + événements
  `install://progress|done|error`), la bibliothèque installée, le player en cours
  (`player_status`, `player://started|exited`) et les actions. « Télécharger et jouer » lance
  l'histoire (`player_launch`) à la fin de l'installation ; l'annulation ou le retrait de la tâche
  retire ce lancement, un échec le garde pour _Réessayer_. Un seul player à la fois : _Jouer_ est
  désactivé tant qu'il tourne. À la fin de la lecture, une notification donne l'issue (dernières
  lignes de stderr si le player s'est arrêté en erreur) et, si `saves_error` n'est pas vide, une
  seconde notification (erreur, à fermer à la main) dit que la progression n'a pas pu être
  conservée telle quelle. Quand la lecture a été demandée **depuis la fiche** (_Jouer_,
  _Jouer depuis le dossier_, _Télécharger et jouer_), l'application revient à la fin sur cette
  fiche (`#/library/<source>`, fiche rouverte) même si l'utilisateur a changé d'écran ou de
  source entre-temps ; une lecture lancée depuis la carte ne rouvre rien. Le retour est abandonné (sans erreur) si une fiche ou un dialogue est déjà ouvert à ce moment, si la source est en erreur ou supprimée, ou si l'histoire n'y est plus ; un lancement refusé n'efface que sa propre demande. Si l'option
  _Réduire la fenêtre pendant la lecture_ (Paramètres > Lecture, désactivée par défaut) est
  cochée, la fenêtre est réduite à `player://started` et rétablie, avec le focus, à
  `player://exited` ; c'est une préférence du poste, gardée dans le stockage local de la
  fenêtre (`localStorage`, pas dans `settings.json`), et un échec de réduction n'empêche jamais
  la lecture. À la fermeture de la fiche (bouton, Échap, après une confirmation) le focus revient
  à la carte d'origine (ou, pour une fiche rouverte automatiquement, à la carte de l'histoire).
  _Supprimer_ demande une confirmation avec la case _Supprimer aussi la progression_
  (décochée à chaque ouverture) : décochée, `library_remove(uuid)` conserve les sauvegardes de
  l'enfant ; cochée, `library_remove(uuid, true)` les efface aussi.
  Le **panneau des tâches** (`StoriesTaskPanel`, zone `aside`) montre téléchargement (octets,
  %), conversion par phase (images, audio, scripts, archive), installation, et pour les échecs
  le message du backend avec _Réessayer_ (relance `install_start` avec la même entrée ; si
  l'entrée n'est plus dans le cache du catalogue ou le dernier scan, `not_found` devient un
  message qui invite à rafraîchir la source, et la tâche en échec reste affichée) ; une
  installation réussie se retire d'elle-même du panneau après 8 s (`DONE_TASK_TTL_MS`), les
  échecs et annulations restent jusqu'à _Retirer_ ou _Réessayer_. L'installation recharge le rapport de compatibilité
  (la conversion y enregistre son résultat). Tests : `stories.test.tsx` (installer → progression →
  jouer, annulation, erreur de conversion, avertissement d'incompatibilité, mise à jour,
  suppression, player occupé).
- **Sources** : ajout d'un catalogue (nom + URL, validation en direct, erreurs du backend
  affichées), d'un dossier (sélecteur natif), renommage, réordonnancement (glisser-déposer ou
  boutons Monter/Descendre au clavier, annoncé aux lecteurs d'écran) et suppression avec
  confirmation. « Mes histoires » reste en tête, intouchable.
  ![Sources, thème clair](docs/screenshots/sources-clair.png)
  ![Sources, thème sombre](docs/screenshots/sources-sombre.png)
- **Paramètres** : chemins du player, de Python et du cache (sélecteur `.exe`, valeurs utilisées,
  problèmes signalés), taille du cache et vidage, thème Système/Clair/Sombre, lecture (réduire
  la fenêtre pendant la lecture), à propos.
  ![Paramètres, thème clair](docs/screenshots/settings-clair.png)
  ![Paramètres, thème sombre](docs/screenshots/settings-sombre.png)
- **Restauration** : si un fichier de configuration corrompu a été remplacé, un bandeau
  d'avertissement (nom de la sauvegarde `.bak`) reste affiché sur tous les écrans jusqu'à
  « J'ai compris » (`config_ack_recovery`).

Le réglage des téléchargements simultanés (`max_downloads`) existe dans `settings.json` mais
n'a pas encore de champ dans l'écran Paramètres.
