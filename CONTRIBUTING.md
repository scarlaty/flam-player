# Contribuer à flam-player

Plusieurs personnes et agents travaillent en même temps sur ce dépôt. Les règles
ci-dessous évitent que deux contributeurs fassent le même travail.

## 1. Organisation

- **Toute tâche est une issue GitHub.** Le suivi d'ensemble est dans l'issue épique
  épinglée « Suivi : surcouche flam-launcher ». Sa section **« Prêtes à prendre »**
  liste les issues qu'on peut démarrer tout de suite, par ordre de priorité.
- **GitHub est la seule source de vérité.** Tout ce qui permet de reprendre le travail
  depuis un autre poste se trouve dans l'issue, la branche et la PR : réservation, état
  d'avancement, demandes de revue. Rien ne doit rester seulement dans une conversation
  ou sur un poste.
- **Jalons** = phases : `M0 Organisation`, `M1 Compatibilité`, `M2 Corrections telmi2flam`,
  `M3 Surcouche flam-launcher`.
- **Labels** :
  - `zone:*` : partie du code touchée (`zone:telmi2flam`, `zone:player`,
    `zone:launcher-rust`, `zone:launcher-ui`, `zone:ci`).
  - `modele:sonnet` / `modele:opus` : niveau de complexité (tâche courante / complexe), et
    modèle utilisé par l'auteur. Une issue qui touche aux données de l'utilisateur (voir §5)
    porte `modele:opus` ; la revue peut se faire avec un modèle sonnet.
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
4. Relire les commentaires de l'issue : si un autre 🔒 **antérieur** existe, c'est lui qui
   gagne. Supprimer la branche qu'on vient de pousser (`git push origin --delete <branche>`),
   **ne pas** retirer le label `en-cours` (il appartient au gagnant), ne poster ni 🔓 ni
   commentaire, et prendre une autre tâche.

Une issue `en-cours` n'est **jamais** prise, sauf si son verrou est périmé (§4.1).

**Exceptions** :
- correction de documentation triviale (faute, lien, une phrase) : une PR autonome sans
  issue ni réservation suffit, sur une branche `docs/<slug>` ;
- **`main` cassé** (CI rouge sur `main`) : le correctif est prioritaire sur tout le reste.
  Le mainteneur peut l'ouvrir sans réservation, sur une branche `fix/<slug>`, en expliquant
  la cause dans la PR. Il est fusionné dès que sa propre CI est verte, sans attendre les
  autres PR.

## 3. Pendant le travail

- **Branches** : `feat/<n>-<slug>`, `fix/<n>-<slug>`, `chore/<n>-<slug>`, `docs/<n>-<slug>`.
- **Push fréquents** (au moins à chaque étape qui compile/passe les tests) : si le
  travail s'interrompt, quelqu'un d'autre doit pouvoir le reprendre depuis la branche.
- **Commits** : style Conventional Commits en français, auteur `scarlaty`
  (`git config user.name scarlaty`), par ex. `feat(compat): ajoute l'étape de conversion`.
  Pas de ligne d'attribution automatique.
- **Un worktree par tâche** : `git worktree add ../flam-<numéro> -b <branche> origin/main`,
  pour que deux tâches menées sur un même poste ne se gênent pas.
- **`git add` avec des chemins explicites**, jamais `git add -A` ni `git add .`. Ne jamais
  committer `.claude/`, `.cache/`, `build/`, `node_modules/` ni `target/`.
- **Point d'étape** : à chaque pause longue (fin de session, limite atteinte), pousser
  puis commenter l'issue avec `📍 Point d'étape — fait : … ; reste : … ; prochaine étape : …`.
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

### 4.1 Reprendre le travail de quelqu'un d'autre

- **Issue libérée (🔓)** : la réserver normalement (§2), mais **repartir de la branche
  existante** (`git fetch origin && git worktree add ../flam-<n> <branche>`) au lieu d'en
  créer une nouvelle.
- **Verrou périmé** : l'issue porte `en-cours`, mais on ne voit **depuis 24 h** ni commit
  poussé sur sa branche, ni commentaire sur l'issue ou la PR. Commenter alors
  `🔁 Reprise — verrou de <nom> inactif depuis <date>`, garder le label `en-cours` et
  continuer sur la même branche (et la même PR si elle existe). Ce 🔁 remplace le 🔒
  d'origine. Si plusieurs 🔁 sont postés, seul le **plus ancien** compte, et les autres
  abandonnent comme au §2.4.
- **Avant d'écrire du code**, relire dans l'ordre :
  1. le dernier `📍 Point d'étape` ou `🔓 Libéré` de l'issue ;
  2. les commentaires de la PR, en particulier `Revue du mainteneur` (§6) ;
  3. le diff de la branche par rapport à `origin/main`.

## 5. Qualité attendue avant PR

- `test_run.bat` (player, moteur, convertisseur) au vert pour toute modification de code hors
  `apps/flam-launcher/`.
- `tools/telmi2flam` : `python -m unittest discover -s tools/telmi2flam/tests` au vert ;
  toute correction de compatibilité est accompagnée d'un test de non-régression.
- `apps/flam-launcher` : `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `npm run lint`, `npm run typecheck`, `npm test`.
- `crates/telmi2flam` (port Rust du convertisseur, workspace Cargo à la racine) :
  `cargo test -p telmi2flam`, `cargo fmt --check` et `cargo clippy -p telmi2flam -- -D warnings`.
  Une modification de `crates/**` déclenche aussi la CI du launcher, qui partage le workspace (et en dépendra avec #122).
- Copie de travail extraite avant l'ajout de `.gitattributes` : `tools/telmi2flam/engine/` peut
  garder des fins de ligne CRLF. `git add --renormalize tools/telmi2flam/engine/` les remet en LF
  (`include_bytes!` embarque les octets de la copie de travail).
- Pas de code mort, pas de `TODO` sans issue associée, documentation à jour
  (`README.md`, `AGENT.md`, README du module touché).
- **PR courtes** : un seul thème, au plus cinq changements fonctionnels (hors tests et
  documentation). Au-delà, l'issue est découpée en plusieurs PR successives.
- **Revue préalable par l'auteur** : avant d'ouvrir la PR, l'auteur fait relire son diff
  par un relecteur indépendant et corrige ce qui est bloquant ou majeur.
- **Code qui touche aux données de l'utilisateur** (bibliothèque, sauvegardes, journal
  d'installation : `apps/flam-launcher/src-tauri/src/library.rs`, `.../src/player.rs`,
  `.../src/install/` ; le moteur et le player C de `zone:player` ne sont pas concernés) :
  - avant le code, une courte **note de conception** dans l'issue ou la PR : invariants,
    puis tableau « arrêt ou échec à chaque étape → état laissé → reprise » ;
  - un **test d'injection de panne** qui simule un arrêt ou un échec à chaque étape et
    vérifie les invariants ;
  - exception : un diff qui ne change ni l'écriture ni la suppression de données
    (renommage, commentaire, journalisation) en est dispensé, à condition de le justifier
    dans la PR ;
  - label `modele:opus` sur l'issue (voir §1).

## 6. Revue et fusion

Chaque PR est relue (lecture du diff, tests relancés, revue indépendante), puis fusionnée
en **squash** dans `main` par le mainteneur. La branche est supprimée après fusion.

- Les corrections demandées sont publiées **sur la PR**, dans un commentaire
  `Revue du mainteneur`. Celui qui reprend la PR les traite, pousse, puis répond sur la PR
  point par point.
- Les constats **MINEUR** et **NIT** deviennent des issues de suivi, liées dans la PR
  (voir le seuil ci-dessous).
- **Seuil de fusion** : seuls les constats **BLOQUANT** et **MAJEUR** (perte ou
  corruption de données, blocage durable de l'utilisateur, interblocage, faille) sont
  corrigés dans la PR. **MINEUR** et **NIT** vont toujours dans une issue de suivi.
- **Revue complète, correctifs prescrits** : la première revue cherche en une fois tous
  les constats bloquants ou majeurs, et propose pour chacun une correction précise que
  l'auteur applique telle quelle.
- **Deux tours au plus** : au troisième, le mainteneur tranche. Il impose la solution, ou
  il fusionne et ouvre une issue de suivi pour le reste.
- **Main à jour avant fusion** : juste avant de fusionner, le mainteneur vérifie que la tête
  de la PR contient le `main` courant (`git merge-base --is-ancestor origin/main <tête>`).
  Sinon, `main` est fusionné dans la branche (`git merge origin/main`, sans rebase) et on
  attend la CI verte sur ce nouveau commit. Deux PR vertes séparément peuvent casser `main`
  une fois fusionnées.
