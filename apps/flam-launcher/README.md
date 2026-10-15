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
      commands.rs          Commandes Tauri
      error.rs             `AppError` (thiserror), sérialisée en texte vers le front
```

### Modules Rust

Présents : `commands`, `error`. Prévus (un module par responsabilité, créés avec leur issue) :

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

Les commandes sont asynchrones (`tokio`), renvoient `AppResult<T>` et sont enregistrées dans
`generate_handler!` (`lib.rs`). Chacune a un wrapper typé dans `src/api/`.

| Commande   | Retour              | État     |
| ---------- | ------------------- | -------- |
| `app_info` | `{ name, version }` | présente |

Les commandes des modules prévus seront ajoutées avec leurs issues.

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
- Un dossier par fonctionnalité dans `src/features/`, composants partagés dans `src/components/`.
- Tests à côté du code (`*.test.ts(x)`), API Tauri simulée avec `@tauri-apps/api/mocks`.
- TypeScript strict, formatage Prettier, ESLint sans avertissement.
