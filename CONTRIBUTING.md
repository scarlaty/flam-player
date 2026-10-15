# Contribuer à flam-player

Plusieurs personnes et agents travaillent en même temps sur ce dépôt. Les règles
ci-dessous évitent que deux contributeurs fassent le même travail.

## 1. Organisation

- **Toute tâche est une issue GitHub.** Le suivi d'ensemble est dans l'issue épique
  épinglée « Suivi : surcouche flam-launcher ».
- **Jalons** = phases : `M0 Organisation`, `M1 Compatibilité`, `M2 Corrections telmi2flam`,
  `M3 Surcouche flam-launcher`.
- **Labels** :
  - `zone:*` : partie du code touchée (`zone:telmi2flam`, `zone:player`,
    `zone:launcher-rust`, `zone:launcher-ui`, `zone:ci`).
  - `modele:sonnet` / `modele:opus` : niveau de complexité (tâche courante / complexe).
  - `en-cours` : l'issue est **réservée** (voir §2).
  - `bloque` : l'issue attend une autre issue (citée dans « Dépend de »).
  - `plus-tard` : hors périmètre actuel.

## 2. Réserver une tâche (obligatoire avant d'écrire du code)

1. Vérifier que l'issue n'a **ni** le label `en-cours`, **ni** le label `bloque`, et que
   toutes les issues de sa section « Dépend de » sont fermées.
2. Vérifier qu'aucune branche `*/<numéro>-*` n'existe déjà : `git ls-remote origin "*/<numéro>-*"`.
3. Réserver :
   - ajouter le label `en-cours` ;
   - commenter : `🔒 Réservé par <nom> — branche <type>/<numéro>-<slug>` ;
   - créer la branche depuis `origin/main` et **la pousser immédiatement** (commit vide
     accepté : `git commit --allow-empty -m "chore: réserve #<numéro>"`).
4. Relire les commentaires de l'issue : si un autre 🔒 **antérieur** existe, abandonner
   la réservation (§4) et prendre une autre tâche.

## 3. Pendant le travail

- **Branches** : `feat/<n>-<slug>`, `fix/<n>-<slug>`, `chore/<n>-<slug>`, `docs/<n>-<slug>`.
- **Push fréquents** (au moins à chaque étape qui compile/passe les tests) : si le
  travail s'interrompt, quelqu'un d'autre doit pouvoir le reprendre depuis la branche.
- **Commits** : style Conventional Commits en français, auteur `scarlaty`
  (`git config user.name scarlaty`), par ex. `feat(compat): ajoute l'étape de conversion`.
  Pas de ligne d'attribution automatique.
- **Périmètre** : uniquement ce que décrit l'issue. Tout besoin supplémentaire = nouvelle
  issue (commentaire dans l'issue d'origine avec le lien).
- **Contenus sous licence** : aucune archive Telmi ni histoire convertie dans le dépôt.
  Les téléchargements et rapports de compatibilité vont dans `.cache/` (ignoré par Git).

## 4. Terminer ou abandonner

- **Terminer** : ouvrir une PR vers `main` avec `Closes #<numéro>`, la description
  (quoi, pourquoi, comment tester) et le résultat des tests. Garder `en-cours` jusqu'à
  la fusion.
- **Abandonner / interrompre** : pousser l'état courant, commenter
  `🔓 Libéré — <état, ce qui reste>` et retirer le label `en-cours`.

## 5. Qualité attendue avant PR

- `test_run.bat` (player, moteur, convertisseur) au vert pour toute modification hors
  `apps/flam-launcher/`.
- `tools/telmi2flam` : `python -m unittest discover -s tools/telmi2flam/tests` au vert ;
  toute correction de compatibilité est accompagnée d'un test de non-régression.
- `apps/flam-launcher` : `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `npm run lint`, `npm run typecheck`, `npm test`.
- Pas de code mort, pas de `TODO` sans issue associée, documentation à jour
  (`README.md`, `AGENT.md`, README du module touché).

## 6. Revue et fusion

Chaque PR est relue (lecture du diff, tests relancés, revue indépendante), puis fusionnée
en **squash** dans `main` par le mainteneur. La branche est supprimée après fusion.
