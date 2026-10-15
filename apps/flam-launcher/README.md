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
    features/<domaine>/    Écrans et logique par fonctionnalité (welcome, demo en dev)
    components/            Composants partagés sans logique métier (design system)
    theme/                 Jetons CSS (clair/sombre) et choix du thème
    i18n/                  Textes de l'interface (français)
  docs/DESIGN.md           Design system : principes, palette, composants
  src-tauri/
    src/
      lib.rs               Initialisation (traces, builder Tauri)
      app.rs               Commandes générales (`app_info`)
      error.rs             `AppError` (thiserror), sérialisée en texte vers le front
```

### Modules Rust

Présents : `app` (commandes générales), `error`. Prévus (un module par responsabilité, créés avec leur issue) :

| Module     | Rôle                                                             |
| ---------- | ---------------------------------------------------------------- |
| `sources`  | Registre des sources d'histoires (catalogues, dossiers)          |
| `catalog`  | Lecture des catalogues JSON au format Telmi Store                |
| `folder`   | Source dossier local (`.zip`, `.plain`, `.plain.pk`)             |
| `install`  | Pipeline téléchargement, conversion (`telmi2flam`), installation |
| `player`   | Lancement et suivi de `flam-player`                              |
| `settings` | Lecture/écriture de la configuration                             |
| `compat`   | Statut de compatibilité d'une histoire                           |

### Commandes Tauri

Chaque module Rust porte ses propres commandes (`#[tauri::command]` dans le module concerné,
enregistrées dans `generate_handler!` de `lib.rs`) : pas de fichier `commands.rs` fourre-tout.

Les commandes sont asynchrones (`tokio`), renvoient `AppResult<T>` et sont enregistrées dans
`generate_handler!` (`lib.rs`). Chacune a un wrapper typé dans `src/api/`.

| Commande   | Retour              | État     |
| ---------- | ------------------- | -------- |
| `app_info` | `{ name, version }` | présente |

Les commandes des modules prévus seront ajoutées avec leurs issues.

### Erreurs

`AppError` (`src-tauri/src/error.rs`, `thiserror`) est sérialisée en objet stable
`{ "kind": "<code_stable>", "message": "<texte lisible en français>" }` (ex. `kind: "io"`).
Le miroir front est `src/api/error.ts` (type `AppError`, garde `isAppError`) ; le helper
`invokeCommand<T>(cmd, args?)` appelle `invoke` et normalise toute erreur en `AppError`
(`kind: "unknown"` si la forme est inattendue). Convention : chaque module ajoute ses variantes
à `AppError` avec un `kind` stable ; le front teste le `kind`, jamais le `message`.

### Événements

Convention : événements émis par le backend, nommés `domaine://action` (ex. `install://progress`),
avec une charge utile sérialisable typée côté `src/api/`. Aucun événement pour l'instant ;
la progression d'installation est le premier cas prévu.

### Données

- Configuration : `%APPDATA%/flam-launcher` (sources, paramètres).
- Cache (téléchargements, histoires converties) : dossier configurable, par défaut sous les
  données locales de l'application. Jamais dans le dépôt.

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
  les builds suivants les réutilisent. Cibles de bundle : `nsis` et `msi`.
- Contrôles : `npm run lint` (sans avertissement), `npm run format:check`, `npm run typecheck`,
  `npm test`, puis dans `src-tauri/` `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

## Sécurité

- CSP stricte définie dans `src-tauri/tauri.conf.json` : scripts, objets, frames et formulaires
  verrouillés. `style-src` garde `'unsafe-inline'` : Tauri et React injectent des styles en ligne
  (attributs `style`), qu'une CSP à nonce ne couvre pas ; aucun script en ligne n'est autorisé.
- La capability `core:default` (`src-tauri/capabilities/default.json`) est volontairement large
  pour le squelette : elle sera restreinte permission par permission au fil des issues.
