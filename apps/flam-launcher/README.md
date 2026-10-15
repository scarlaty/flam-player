# flam-launcher

Surcouche de bureau (Windows d'abord) pour parcourir des sources d'histoires, les convertir
avec `telmi2flam` et les lancer dans `flam-player`. Application Tauri 2 : backend Rust,
interface React + TypeScript (strict). Licence **MIT** (voir [LICENSE](LICENSE)).

L'ergonomie s'inspire de TelmiSync, **sans reprendre de code ni de style** (TelmiSync est
en GPL-3.0). Suivi : issue « Suivi : surcouche flam-launcher ».

Ce document décrit l'architecture visée. Les éléments marqués _prévu_ n'existent pas encore
et arrivent avec les issues du jalon M3.

## Prérequis

Rust stable (MSVC), Visual Studio 2022 avec les outils C++ x64, WebView2, Node 24+ et npm.

## Commandes

Depuis `apps/flam-launcher/` :

| Commande                             | Effet                                          |
| ------------------------------------ | ---------------------------------------------- |
| `npm install`                        | Installe les dépendances front et la CLI Tauri |
| `npm run tauri dev`                  | Lance l'application en développement           |
| `npm run dev` / `npm run build`      | Front seul (Vite) / build du front             |
| `npm run lint` / `npm run typecheck` | ESLint / TypeScript strict                     |
| `npm test`                           | Vitest + Testing Library (API Tauri simulée)   |
| `npm run tauri build`                | Exécutable et installeurs (MSI, NSIS)          |
| `npm run tauri build -- --no-bundle` | Exécutable seul, sans installeur               |

Dans `src-tauri/` : `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

## Architecture

```
apps/flam-launcher/
  src/                     Front React/TypeScript
    api/                   Couche typée au-dessus de `invoke` / événements Tauri
    features/<domaine>/    Écrans et logique par fonctionnalité (aujourd'hui : welcome)
    components/            Composants partagés sans logique métier (_prévu_)
  src-tauri/
    src/
      lib.rs               Initialisation (traces, builder Tauri)
      app.rs               Commandes générales (`app_info`)
      error.rs             `AppError` (thiserror), sérialisée en `{ kind, message }`
      storage.rs           Persistance JSON : écriture atomique, `*.bak`, migration de schéma
      settings.rs          `settings.json`, détection du player et de Python, commandes `settings_*`
      sources.rs           `sources.json`, source par défaut et implicite, commandes `sources_*`
```

### Modules Rust

Présents : `app` (commandes générales), `error`, `storage`, `settings`, `sources`. Prévus (un module par responsabilité, créés avec leur issue) :

| Module    | Rôle                                                             |
| --------- | ---------------------------------------------------------------- |
| `catalog` | Lecture des catalogues JSON au format Telmi Store                |
| `folder`  | Source dossier local (`.zip`, `.plain`, `.plain.pk`)             |
| `install` | Pipeline téléchargement, conversion (`telmi2flam`), installation |
| `player`  | Lancement et suivi de `flam-player`                              |
| `compat`  | Statut de compatibilité d'une histoire                           |

### Commandes Tauri

Chaque module Rust porte ses propres commandes (`#[tauri::command]` dans le module concerné,
enregistrées dans `generate_handler!` de `lib.rs`) : pas de fichier `commands.rs` fourre-tout.

Les commandes sont asynchrones (`tokio`), renvoient `AppResult<T>` et sont enregistrées dans
`generate_handler!` (`lib.rs`). Chacune a un wrapper typé dans `src/api/`.

| Commande          | Arguments                  | Retour                      | Module     |
| ----------------- | -------------------------- | --------------------------- | ---------- |
| `app_info`        |                            | `{ name, version }`         | `app`      |
| `settings_get`    |                            | `SettingsView`              | `settings` |
| `settings_set`    | `settings: Settings`       | `SettingsView`              | `settings` |
| `sources_list`    |                            | `Source[]`                  | `sources`  |
| `sources_add`     | `input: SourceInput`       | `Source`                    | `sources`  |
| `sources_update`  | `id`, `input: SourceInput` | `Source`                    | `sources`  |
| `sources_remove`  | `id`                       | rien                        | `sources`  |
| `sources_reorder` | `ids: string[]`            | `Source[]` (liste complète) | `sources`  |

Wrappers front : `src/api/settings.ts` (`getSettings`, `setSettings`) et `src/api/sources.ts`
(`listSources`, `addSource`, `updateSource`, `removeSource`, `reorderSources`). La logique
(`SettingsStore`, `SourcesStore`, détection) ne dépend pas de Tauri et reçoit son dossier de
configuration en paramètre : elle est testée sur des dossiers temporaires ; les commandes
ne sont que de fines enveloppes qui prennent un verrou (`ConfigState`) autour de chaque opération.

### Erreurs

`AppError` (`src-tauri/src/error.rs`, `thiserror`) est sérialisée en objet stable
`{ "kind": "<code_stable>", "message": "<texte lisible en français>" }` (ex. `kind: "io"`).
Le miroir front est `src/api/error.ts` (type `AppError`, garde `isAppError`) ; le helper
`invokeCommand<T>(cmd, args?)` appelle `invoke` et normalise toute erreur en `AppError`
(`kind: "unknown"` si la forme est inattendue). Convention : chaque module ajoute ses variantes
à `AppError` avec un `kind` stable ; le front teste le `kind`, jamais le `message`.

| `kind`           | Sens                                                                        |
| ---------------- | --------------------------------------------------------------------------- |
| `io`             | Erreur d'entrée/sortie                                                      |
| `validation`     | Valeur refusée (URL, chemin, nom, ordre) ou opération interdite             |
| `not_found`      | Élément introuvable (identifiant de source inconnu)                         |
| `config_version` | Fichier de configuration écrit par une version plus récente (laissé intact) |
| `config_dir`     | Dossier de configuration introuvable                                        |
| `serialization`  | Échec de sérialisation JSON                                                 |
| `unknown`        | Côté front uniquement : erreur de forme inattendue                          |

Un fichier de configuration **corrompu** n'est pas une erreur : il est renommé en
`<fichier>.<AAAAMMJJ-HHMMSS>.bak`, un avertissement est tracé et les valeurs par défaut sont utilisées.

### Événements

Convention : événements émis par le backend, nommés `domaine://action` (ex. `install://progress`),
avec une charge utile sérialisable typée côté `src/api/`. Aucun événement pour l'instant ;
la progression d'installation est le premier cas prévu.

### Données

- Configuration : `%APPDATA%/flam-launcher` (`settings.json`, `sources.json`), détaillée ci-dessous.
- Cache (téléchargements, histoires converties) : dossier configurable, par défaut sous les
  données locales de l'application. Jamais dans le dépôt.

#### `settings.json`

```json
{
  "schema_version": 1,
  "player_path": null,
  "python_path": null,
  "cache_dir": null,
  "default_catalog_url": "https://gist.githubusercontent.com/DantSu/49ed776755f3a01c995e78e3fd1cb79f/raw/telmi-interactive.json"
}
```

Un chemin à `null` signifie « automatique » ; `settings_get` renvoie aussi les valeurs effectives
(`resolved`) :

- `player_path` : `flam-player.exe` à côté de l'exécutable de l'appli, sinon (build de
  développement) `build/flam-player.exe` dans un dossier ancêtre (racine du dépôt) ;
- `python_path` : `python.exe` trouvé dans le `PATH` (aucun script n'est exécuté ; les alias
  vides du Microsoft Store sont ignorés) ;
- `cache_dir` : `%LOCALAPPDATA%/flam-launcher/cache`.

`settings_set` normalise (espaces, chaîne vide = automatique) puis valide : les chemins du player
et de Python doivent être des fichiers absolus existants, le cache un chemin absolu qui n'est pas
un fichier, l'URL du catalogue une URL http(s).

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
    { "type": "folder", "id": "3f2c…", "name": "Mon dossier", "path": "C:\Histoires" }
  ]
}
```

- Liste ordonnée ; les identifiants sont des UUID v4 (« Telmi Store » : `telmi-store`).
- « Telmi Store » est créée avec le fichier ; une fois supprimée, elle ne revient pas.
- « Mes histoires » (`type: "library"`, id `library`) est **implicite** : jamais écrite dans le
  fichier, toujours renvoyée en premier par `sources_list`, ni modifiable, ni supprimable, ni
  réordonnable.
- Validation : nom non vide (100 caractères max), URL http(s) avec hôte, dossier absolu existant,
  type immuable à la mise à jour ; `sources_reorder` exige une permutation exacte des identifiants
  enregistrés.

#### Écriture, corruption, migration

- Écritures atomiques : fichier temporaire voisin puis renommage.
- `schema_version` absent = version 0. Les migrations s'appliquent version par version
  (`MigrationStep` dans `storage.rs`) ; un fichier de version **supérieure** à celle de
  l'application est refusé (`config_version`) sans être modifié.

### Conventions front

- Tout appel au backend passe par `src/api/` (fonctions typées, miroir des structures Rust) ;
  les composants n'appellent jamais `invoke` directement.
- Un dossier par fonctionnalité dans `src/features/`, composants partagés dans `src/components/`.
- Tests à côté du code (`*.test.ts(x)`), API Tauri simulée avec `@tauri-apps/api/mocks`.
- TypeScript strict, formatage Prettier, ESLint sans avertissement.

## Outillage

- TypeScript est épinglé à 6.0.3 : `typescript-eslint` n'accepte pas encore TypeScript >= 6.1.
  À relever quand le plugin le permettra.
- Le premier `npm run tauri build` télécharge WiX (MSI) et NSIS (installeur) depuis GitHub ;
  les builds suivants les réutilisent. Cibles de bundle : `nsis` et `msi`.
- Contrôles : `npm run lint` (sans avertissement), `npm run format:check`, `npm run typecheck`,
  `npm test`, puis dans `src-tauri/` `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

## Sécurité

- CSP stricte définie dans `src-tauri/tauri.conf.json` : scripts, objets, frames et formulaires
  verrouillés. `style-src` garde `'unsafe-inline'` : Tauri et React injectent des styles en ligne
  (attributs `style`), qu'une CSP à nonce ne couvre pas ; aucun script en ligne n'est autorisé.
- La capability `core:default` (`src-tauri/capabilities/default.json`) est volontairement large
  pour le squelette : elle sera restreinte permission par permission au fil des issues.
