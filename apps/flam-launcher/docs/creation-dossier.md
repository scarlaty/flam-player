# Éditeur d'histoires sur dossier : note de conception

> **Statut : implémenté, essais sur la Flam à faire** (2026-11-19). Le mainteneur a validé
> l'objectif (§ 1), les modes recueil et chapitres (§ 5) et les recommandations Q1 à Q8 (§ 12).
> Les lots 1 à 7 et 10 sont faits ; les lots 8 et 9 sont en PR (§ 11.3). Restent les essais sur la Flam (§ 13), qui confirmeront
> les limites (§ 9). Guide utilisateur : [guide-editeur.md](guide-editeur.md).

Légende :

- **Décidé** : demandé ou validé par le mainteneur ;
- **Proposé** : recommandation de cette note ;
- **Fait** : implémenté dans `main` (ou dans une PR indiquée) ;
- **Existant** : déjà présent dans `main` (fichier cité).

## Sommaire

1. [Objectif](#1-objectif)
2. [Ce qui existe déjà](#2-ce-qui-existe-déjà)
3. [Le dossier, source de vérité](#3-le-dossier-source-de-vérité)
4. [Éditeur visuel](#4-éditeur-visuel)
5. [Deux modes : recueil et chapitres](#5-deux-modes--recueil-et-chapitres)
6. [Réglages (`_histoire.json`, `_dossier.json`)](#6-réglages-_histoirejson-_dossierjson)
7. [Compléments automatiques](#7-compléments-automatiques)
8. [Créer, actualiser, envoyer](#8-créer-actualiser-envoyer)
9. [Limites](#9-limites)
10. [Sécurité et intégrité des fichiers](#10-sécurité-et-intégrité-des-fichiers)
11. [Architecture et découpage](#11-architecture-et-découpage)
12. [Décisions](#12-décisions)
13. [Essais sur la Flam](#13-essais-sur-la-flam)

## 1. Objectif

**Décidé** (mainteneur, 2026-11-19) :

- un **outil visuel**, en glisser-déposer, pour fabriquer une histoire Flam à partir de fichiers
  audio et d'images : créer des menus, ranger, renommer, changer les images ;
- les modifications portent sur un **dossier** du PC ; on peut aussi ouvrir un **dossier déjà
  fait** (à la main ou lors d'une session précédente) et **actualiser** l'histoire ;
- une option **recueil** (menu de petites histoires) ou **chapitres** (chapitres qui
  s'enchaînent, avec un menu des chapitres) ;
- la compatibilité avec STUdio Pack Generator (« SPG ») n'est **pas** un objectif : le but n'est
  pas d'échanger des histoires.

Hors périmètre de la première version : histoires interactives à choix multiples, import RSS
(les podcasts ont leur parcours), extraction d'une histoire installée en dossier (§ 12).

## 2. Ce qui existe déjà

La création des packs de podcasts fabrique déjà une histoire à partir d'un arbre de menus, avec
un éditeur d'arbre. Le nouveau parcours en réutilise la plus grande partie.

| Étape                                                                       | Existant                                                                       |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Éditeur d'arbre (déplacer, renommer, monter, descendre, image de catégorie) | `src/features/podcasts/pack/` : `PackEditor.tsx`, `PackTree.tsx`, `tree.ts`    |
| Modèle menus / épisodes                                                     | `podcast/archive.rs` : `Pack`, `MenuEntry::{Episode, Category}`, `PackEpisode` |
| MP3 conforme depuis un fichier local                                        | `podcast/audio.rs` : `probe`, `convert`                                        |
| Images 640 × 480 et couverture 512 × 512                                    | `podcast/artwork.rs`                                                           |
| Titres lus (voix Windows)                                                   | `podcast/speech.rs`, `install/pack_speech.rs`, `TitlesAudioField.tsx`          |
| Cache des MP3 convertis                                                     | `podcast/mp3cache.rs` (podcasts.md § 13)                                       |
| Archive Telmi synthétique puis `.plain.pk`                                  | `podcast/archive.rs::write_archive`, puis `telmi2flam`                         |
| Surveillance d'un dossier                                                   | `watch.rs`                                                                     |
| Écoute sur le PC                                                            | `player.rs`                                                                    |
| Lecture prudente d'un dossier (liens, bornes)                               | `folder.rs`                                                                    |

Contrainte constatée : le dépôt de fichiers natif est **désactivé** sur toutes les fenêtres
(`tauri.conf.json`, `dragDropEnabled: false`, test `src/tauriConfig.test.ts`, #300). Sous
Windows, l'activer fait disparaître les événements HTML5 (`draggable`, `ondrop`) dont dépendent
`PackTree.tsx`, `DeviceStoryOrder.tsx` et `SourcesScreen.tsx`. Voir § 4.3.

## 3. Le dossier, source de vérité

**Fait** : l'histoire **est** le dossier. L'éditeur n'a pas de projet caché : tout ce qu'il
modifie est écrit dans le dossier, et tout ce qui est fait à la main dans l'Explorateur apparaît
dans l'éditeur (§ 4.5). Un dossier rangé à la main, sans aucun fichier spécial, donne déjà une
histoire.

### 3.1 Arborescence

```
Mes histoires du soir/            ← racine : titre par défaut de l'histoire
├── _histoire.json                réglages de l'histoire, identifiant (§ 6) ; créé par l'éditeur
├── _couverture.png               couverture
├── _titre.mp3                    titre lu de l'histoire (facultatif)
├── 01 Animaux/                   sous-dossier = menu (recueil) ou partie (chapitres)
│   ├── _dossier.json             ordre, titres affichés, mode (§ 6) ; facultatif
│   ├── _image.png                image de l'option qui ouvre ce menu
│   ├── _titre.mp3                titre lu de cette option
│   ├── 01 Le loup.mp3            fichier audio = une histoire (ou un chapitre)
│   ├── 01 Le loup.jpg            image de l'histoire : même nom, extension d'image
│   ├── 01 Le loup.titre.mp3      titre lu de l'histoire
│   └── 02 Le chat.m4a
└── 02 Comptines/
```

- Un dossier = un menu ; ses sous-dossiers et ses fichiers audio = ses options, mélangeables.
- Fichiers spéciaux : préfixe `_` pour le dossier qui les contient, même nom de base (avec
  `.titre` pour le titre lu) pour un fichier audio. Ils sont lisibles dans l'Explorateur et
  rangés en tête.
- Dossier caché `.flam/` à la racine : corbeille de l'éditeur (§ 4.4). Ignoré par la lecture.

### 3.2 Ordre et titres

- Ordre : celui de `_dossier.json` (`ordre`) s'il existe ; les éléments qui n'y figurent pas
  (ajoutés à la main) viennent ensuite, en tri naturel (`2` avant `10`, sans casse ni accents).
  Sans `_dossier.json` : tri naturel seul, donc les préfixes numériques d'un dossier fait à la
  main suffisent.
- Titre : celui de `_dossier.json` (`titres`) s'il existe, sinon le nom du fichier sans
  extension ni préfixe de tri (`^\d+\s*[-._)]?\s*` : « 01 - Le loup » → « Le loup »).
- **Décidé** (Q3, 2026-11-19) : réordonner ou renommer dans l'éditeur ne renomme **pas** les
  fichiers ; seul `_dossier.json` change. Raisons : un seul fichier écrit au lieu de dizaines de renommages
  (fichier ouvert, échec à mi-chemin) ; les noms choisis par l'utilisateur restent. Action
  explicite « Renuméroter les fichiers selon l'ordre » pour retrouver l'ordre dans
  l'Explorateur.
- Libellés affichés sur la Flam : translittérés et coupés à 40 caractères (règle des podcasts,
  `MAX_LABEL_CHARS`) ; l'éditeur prévient au-delà.

### 3.3 Formats acceptés

- Audio : `mp3`, `m4a`, `aac`, `ogg`, `opus`, `wav`, `flac`. La vérification est faite par
  `probe` (ffmpeg), pas par l'extension.
- Images : `png`, `jpg`/`jpeg`, `bmp`, `webp`.
- Autres fichiers : ignorés, signalés une fois dans le rapport. Ignorés sans rien dire : fichiers
  cachés, `desktop.ini`, `Thumbs.db`, `.DS_Store`.

## 4. Éditeur visuel

### 4.1 Écran

- Gauche : l'**arbre** (menus, histoires, chapitres), dérivé de `PackTree.tsx`, avec pour chaque
  élément sa vignette, son titre, sa durée et des pastilles (titre lu, image, problème).
- Droite : le **panneau de l'élément choisi** : titre affiché, image (cliquer ou déposer pour
  changer), titre lu (écouter, voix Windows, fichier, plus tard enregistrement au micro),
  écoute de l'audio, réglages du dossier (mode, fin).
- Haut : titre, couverture, mode, âge, description ; boutons « Ouvrir dans l'Explorateur »,
  « Annuler » / « Rétablir », « Créer l'histoire » ou « Mettre à jour l'histoire ».
- Bas : le **rapport** (problèmes bloquants et avertissements), chaque ligne mène à l'élément.

### 4.2 Opérations et effet sur le disque

| Dans l'éditeur                                              | Sur le disque                                                                                    |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Déposer des fichiers audio depuis l'Explorateur sur un menu | **Copie** dans le dossier du menu ; jamais de déplacement de l'original                          |
| Déposer un dossier depuis l'Explorateur                     | Copie récursive (formats acceptés seulement) ; devient un menu                                   |
| Déposer une image sur un élément                            | Copie sous `<nom>.<ext>` ou `_image.<ext>` ; l'ancienne image va dans la corbeille               |
| Réordonner dans un menu                                     | `_dossier.json` (`ordre`)                                                                        |
| Déplacer vers un autre menu                                 | Déplacement du fichier (et de son image, de son titre lu) ; mise à jour des deux `_dossier.json` |
| Renommer                                                    | `_dossier.json` (`titres`) ; « Renommer aussi le fichier » en option                             |
| Nouveau menu                                                | Création d'un sous-dossier                                                                       |
| Supprimer                                                   | Déplacement dans la corbeille `.flam/` (§ 4.4)                                                   |
| Mode, fin, âge, couverture…                                 | `_histoire.json`, `_dossier.json`, `_couverture.*`                                               |

Ajout aussi possible sans glisser-déposer : bouton « Ajouter des fichiers » (sélecteur natif
existant), indispensable pour le clavier et l'accessibilité.

### 4.3 Glisser-déposer : deux fenêtres

Le dépôt de fichiers depuis l'Explorateur demande `dragDropEnabled: true`, incompatible sous
Windows avec le glisser-déposer HTML5 du reste du launcher (§ 2).

- **Décidé** (Q6, 2026-11-19) : l'éditeur s'ouvre dans **sa propre fenêtre** Tauri, avec `dragDropEnabled: true`.
  Les dépôts de fichiers arrivent par `onDragDropEvent` (chemins absolus et position) ;
  l'élément visé est retrouvé par la position. À l'intérieur de cette fenêtre, réordonner et
  déplacer utilisent des **événements de pointeur** (pas HTML5), avec les mêmes fonctions de
  `tree.ts`. Les autres fenêtres gardent `dragDropEnabled: false` ; le test
  `tauriConfig.test.ts` est adapté pour l'exiger partout sauf dans l'éditeur.
- Autre possibilité : activer le dépôt natif partout et passer les trois listes existantes aux
  événements de pointeur. Plus de risques de régression ; non retenu.
- Une seule fenêtre d'éditeur par dossier ; avertissement si le même dossier est déjà ouvert.

### 4.4 Annuler et corbeille

- Chaque opération est inscrite dans un **journal** de la session avec son inverse : copie ↔
  mise à la corbeille, déplacement ↔ déplacement inverse, écriture JSON ↔ contenu précédent.
  « Annuler » et « Rétablir » parcourent ce journal (Ctrl+Z, Ctrl+Y).
- Rien n'est jamais effacé : suppression et remplacement déplacent vers `.flam/corbeille/`
  (même volume, donc renommage instantané et annulable). **Décidé** (Q4, 2026-11-19) : action
  « Vider la corbeille de l'histoire », qui envoie son contenu dans la Corbeille Windows.
- Une opération qui ne peut pas être annulée (le fichier a changé depuis) le dit et ne fait
  rien.

### 4.5 Modifications faites ailleurs

- Le dossier est surveillé (`watch.rs`) tant que l'éditeur est ouvert. Un changement fait dans
  l'Explorateur ou par un autre programme relance la lecture du dossier et met l'arbre à jour,
  sans perdre la sélection si l'élément existe encore (sinon son menu est sélectionné).
  **Fait** (lot 9, `authoring/editor_watch.rs`) : surveillance récursive sans suivre les
  liens, anti-rebond de 400 ms ; `.flam/`, les temporaires des écritures atomiques et les
  changements faits par l'éditeur lui-même (pendant l'opération et 500 ms après) sont
  ignorés. Dossier supprimé, renommé ou clé retirée : l'éditeur passe en lecture seule avec
  « Choisir le dossier » ; s'il revient, la surveillance reprend.
- Avant d'agir, chaque opération vérifie l'état attendu (fichier présent, taille et date
  inchangées). Sinon : pas d'action, lecture du dossier relancée, message.
- Fichier JSON abîmé ou modifié à la main : lu avec tolérance (§ 6), jamais écrasé sans
  sauvegarde (`.flam/corbeille/`).

## 5. Deux modes : recueil et chapitres

**Décidé** (mainteneur, 2026-11-19) : une option choisit si l'histoire est un **recueil** (un
menu avec plein de petites histoires) ou un **livre à chapitres** (les chapitres s'enchaînent,
avec un menu des chapitres pour s'y retrouver).

### 5.1 Choix du mode

- **Décidé** (Q1) : mode réglé **par dossier**, hérité par les sous-dossiers. Celui de la racine est
  l'option principale de l'éditeur ; un sous-dossier peut en changer. Exemple : un recueil
  « Mes livres » dont chaque sous-dossier est un livre à chapitres.
- À l'ouverture d'un dossier sans réglage, l'éditeur propose **chapitres** si la racine ne
  contient que des fichiers audio numérotés (ou des sous-dossiers qui ne contiennent que cela),
  **recueil** sinon. Le choix est écrit dans `_histoire.json`.

### 5.2 Mode recueil

Comportement actuel des packs de podcasts (podcasts.md § 6) :

- démarrage → menu racine ; une option = une histoire ou un sous-menu ;
- fin d'une histoire, selon `fin` :
  - `menu` (défaut) : retour au menu, focalisé sur l'histoire **suivante** ;
  - `suivante` : l'histoire suivante du même menu démarre, la dernière ramène au menu
    (comportement actuel des podcasts) ;
- retour pendant une histoire → son menu, focalisé sur elle.

### 5.3 Mode chapitres

- **Ordre de lecture** : parcours en profondeur dans l'ordre du § 3.2. Les sous-dossiers sont
  des **parties** (« Partie 1 », « Tome 2 ») qui ne servent qu'au menu des chapitres ; la
  lecture les traverse sans s'arrêter.
- **Démarrage** : « Démarrer l'histoire » lance le premier chapitre (`debut` : `chapitre` par
  défaut, ou `menu` pour ouvrir d'abord le menu des chapitres).
- **Enchaînement** : à la fin d'un chapitre, le suivant démarre, y compris d'une partie à
  l'autre. Annonce du titre avant le chapitre seulement si `annoncer` vaut `true` et que le
  chapitre a un titre lu.
- **Menu des chapitres** : le bouton retour pendant un chapitre l'ouvre, focalisé sur le
  chapitre en cours ; une option par chapitre, un niveau par partie. Valider démarre ce
  chapitre (sans son annonce : le titre lu de l'option vient d'être entendu), puis la
  lecture continue avec les suivants. Retour depuis une partie → niveau du dessus.
- **Décidé** (Q2, révisé le 2026-11-19) : retour depuis le premier niveau du menu des
  chapitres → **sortie de l'histoire** (menu Start de la Flam), comme depuis le menu d'un
  recueil ; un livre placé dans un recueil remonte au recueil. Le chapitre quitté par le
  retour **garde sa position** : « Reprendre l'histoire » (après sortie ou mise en veille
  dans le menu) le reprend à l'endroit quitté, et valider son option aussi ; les autres
  chapitres partent du début. Moteur : champ `role` des scènes (`"chapter"`, `"title"`),
  `tools/telmi2flam/README.md`.
- **Fin du livre** (`fin`) : `menu` (défaut : menu des chapitres, sur le premier) ou `fin`
  (fin d'histoire, retour à la bibliothèque de la Flam).
- **Reprise** : le moteur sauve la scène et la position dans l'audio de chaque scène avec audio
  (`tools/telmi2flam/engine/story.lua`, `saveProgress`) et alimente la jauge du menu
  « Reprendre » avec les chapitres écoutés. Sans indication du graphe, il **efface** cette
  position quand on quitte une scène par le retour et à toute entrée par le graphe : c'est le
  comportement des histoires existantes, inchangé. **Décidé** (2026-11-19) : les scènes d'un
  livre portent un `role` (`"chapter"` : position gardée au retour, chapitre quitté retenu ;
  `"title"` : annonce ou titre lu, hors jauge) que `telmi2flam` transmet ; `story.lua` (gel de
  parité, #661) en tient compte. À confirmer sur la Flam (§ 13).
- **Images** : celle du chapitre, sinon celle de sa partie, sinon la couverture. Elle
  s'affiche avec le titre lu du chapitre (option du menu ou annonce) ; pendant la lecture du
  chapitre, l'écran est noir (retour d'essai sur la Flam, 2026-11-19). L'écran « Reprendre »
  garde l'image du chapitre.

### 5.4 Traduction en graphe Telmi

Les deux modes produisent un graphe Telmi que `telmi2flam` convertit sans modification :

- chapitre = scène `autoplay` dont `ok` mène au chapitre suivant (tous dossiers confondus) et
  `home` à son option dans le menu des chapitres ;
- menu des chapitres = actions à plusieurs entrées, comme les menus du recueil ;
- démarrage : `startAction` → premier chapitre (`debut = chapitre`) ou menu racine.

**Fait** : `Pack` a un mode par menu ; `Graph::build` (`podcast/archive.rs`) choisit
les transitions selon ce mode. Tests sur le JSON produit ; ceux des podcasts ne changent pas.

## 6. Réglages (`_histoire.json`, `_dossier.json`)

```json
{
  "format": 1,
  "uuid": "8c1e…",
  "version": 3,
  "titre": "Le Petit Prince",
  "sousTitre": "Livre audio",
  "description": "Lu par…",
  "age": 6,
  "mode": "chapitres",
  "debut": "chapitre",
  "fin": "menu",
  "annoncer": false,
  "titresLus": { "voix": "…" }
}
```

```json
{
  "format": 1,
  "ordre": ["02 Le chat.m4a", "01 Le loup.mp3", "Bonus"],
  "titres": { "01 Le loup.mp3": "Le grand méchant loup" },
  "mode": "recueil",
  "fin": "suivante"
}
```

- `_dossier.json` : seulement les champs utiles ; `mode`, `fin`, `annoncer` héritent du dossier
  parent quand ils sont absents. À la racine, ses champs sont dans `_histoire.json`.
- Lecture tolérante : champ inconnu ignoré ; valeur invalide → valeur par défaut et
  avertissement ; entrée de `ordre` ou de `titres` qui ne désigne plus rien → ignorée et
  retirée à la prochaine écriture.
- Écriture atomique (fichier temporaire puis remplacement), clés inconnues conservées, taille
  bornée.

## 7. Compléments automatiques

| Manque                                | Complément, dans l'ordre                                                                                                    |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Titre lu                              | fichier `_titre.*` / `<nom>.titre.*`, sinon voix Windows (existant), sinon muet                                             |
| Image d'une histoire ou d'un chapitre | image du même nom, sinon pochette du fichier audio (ffmpeg), sinon image générée avec le titre, sinon image unie (existant) |
| Image d'un menu                       | `_image.*`, sinon image de sa première histoire (règle N5 a des podcasts)                                                   |
| Couverture                            | `_couverture.*`, sinon image de la première histoire, sinon image unie                                                      |

- Les compléments sont calculés dans le dossier de travail du launcher, affichés dans l'éditeur
  (pastille « automatique ») et **ne sont pas écrits** dans le dossier. **Décidé** (Q5) : bouton
  « Garder » sur un complément pour l'écrire dans le dossier et pouvoir le modifier.
- Image générée : titre en blanc sur une couleur dérivée du titre, police du player
  (`authoring/title_card.rs`, nouveau).

## 8. Créer, actualiser, envoyer

- **Créer** : première création, `_histoire.json` reçoit un uuid et `version: 1`. L'histoire
  arrive dans « Mes histoires », qui retient le dossier source.
- **Actualiser** : même uuid, `version + 1`. L'histoire remplace l'ancienne dans « Mes
  histoires », puis « Remplacer sur la Flam » (existant) met la Flam à jour. Seuls les fichiers
  modifiés sont reconvertis (cache des MP3).
- Depuis « Mes histoires », une histoire créée ainsi propose « Modifier » (ouvre l'éditeur sur
  son dossier) et « Actualiser » (recrée sans ouvrir l'éditeur). Dossier introuvable (déplacé,
  clé USB retirée) : « Choisir le dossier ».
- Ouvrir un dossier fait à la main : l'éditeur le lit tel quel, sans rien écrire tant que
  l'utilisateur ne modifie rien ou ne crée pas l'histoire.
- Dossier copié (deux dossiers avec le même uuid) : « Remplacer l'autre » ou « Nouvelle
  histoire » (nouvel uuid).
- Dossier en lecture seule : création possible, uuid gardé dans la configuration du launcher
  (indexé par chemin), éditeur en lecture seule.

## 9. Limites

Les limites des podcasts (`podcast/archive.rs`) sont trop serrées pour des histoires faites à
la main ou des livres audio : `MAX_MENU_DEPTH = 3`, `MAX_EPISODES = 50`,
`MAX_OPTIONS_PER_MENU = 50`, `MAX_STAGES = 256`, 1,25 Gio d'audio au total.

**Fait** : limites propres à cette source, passées en paramètre de `Graph::build`
(`authoring/build.rs`), à confirmer par des mesures sur la Flam (§ 13). Valeurs retenues : 5 niveaux, 300 histoires ou chapitres,
50 options par menu, 3 Gio d'audio (archive zip32 : nettement sous 4 Gio). Une limite dépassée est un problème bloquant du rapport,
visible dans l'éditeur avant toute conversion.

## 10. Sécurité et intégrité des fichiers

L'éditeur **modifie les fichiers de l'utilisateur** : c'est la partie la plus sensible.

- Toutes les opérations restent **dans le dossier de l'histoire** : chemin cible recalculé côté
  Rust à partir d'identifiants d'éléments, jamais un chemin libre venu du front ; refus de tout
  chemin qui sort de la racine après résolution ; liens symboliques et jonctions jamais suivis.
- Seule exception : les **sources** d'un dépôt depuis l'Explorateur, lues (jamais modifiées) ;
  un dossier déposé est copié avec des bornes (profondeur, nombre, taille).
- Aucun effacement définitif (§ 4.4) ; écritures atomiques ; nom déjà pris → suffixe « (2) »,
  jamais d'écrasement.
- Lecture bornée comme `folder.rs` ; noms de fichiers jamais réutilisés dans l'archive
  (fichiers produits nommés par rang, comme les podcasts) ; ffmpeg avec arguments fixes et
  `-protocol_whitelist file` (existant).
- Dossiers interdits comme racine : racine d'un disque, dossier système, profil utilisateur,
  dossier du launcher, lecteur de la Flam.

## 11. Architecture et découpage

### 11.1 Modules

- `src-tauri/src/authoring/tree.rs` : lecture pure du dossier → arbre + rapport (code stable
  par problème, chemin relatif).
- `src-tauri/src/authoring/ops.rs` : opérations sur le disque (§ 4.2), vérification de l'état
  attendu, journal et inverses (§ 4.4).
- `src-tauri/src/authoring/settings.rs` : `_histoire.json`, `_dossier.json`.
- `src-tauri/src/authoring/build.rs` : conversion, images, titres lus, `write_archive`,
  `telmi2flam`, installation ; progression et annulation.
- `install/podcast.rs` : en extraire l'étape commune « épisodes prêts → pack ».
- Front : `src/features/authoring/` (fenêtre de l'éditeur), en partageant `tree.ts` et les
  composants de `features/podcasts/pack/`.

### 11.2 Lots (une PR chacun, revue indépendante obligatoire)

1. Cette note.
2. `tree.rs` + `settings.rs` et leurs tests : lecture du dossier, ordre, titres, rapport.
   Revue adversariale.
3. Graphe : mode chapitres dans `podcast/archive.rs`, limites en paramètre. Revue
   adversariale.
4. Extraction de l'étape commune de `install/podcast.rs`, sans changement de comportement.
5. `build.rs`, commandes Tauri, uuid et version (§ 8) : un dossier fait à la main devient une
   histoire. **Premier résultat utilisable**, sans éditeur.
6. `ops.rs` : opérations, journal, corbeille. Revue adversariale (fichiers de l'utilisateur).
7. Fenêtre de l'éditeur : arbre, panneau, rapport, sélecteur de fichiers.
8. Glisser-déposer : dépôt depuis l'Explorateur, réordonner par pointeur (§ 4.3).
9. Surveillance du dossier (§ 4.5), « Modifier » / « Actualiser » dans « Mes histoires ».
10. Pochette extraite, image générée avec le titre.
11. Essais sur la Flam (§ 13), réglage des limites, guide utilisateur.

### 11.3 État d'avancement

Les PR vont de #662 (note de conception) à #675.

| Lot | Contenu                                    | État                                                                      |
| --- | ------------------------------------------ | ------------------------------------------------------------------------- |
| 1   | Cette note                                 | Fait (#662)                                                               |
| 2   | Lecture du dossier, réglages               | Fait (#663)                                                               |
| 3   | Mode chapitres du graphe, limites          | Fait (#665) ; moteur : `role` des scènes, #667                            |
| 4   | Étape commune de `install/podcast.rs`      | Fait (#664)                                                               |
| 5   | Un dossier fait à la main devient histoire | Fait (#670), suites (#673) : relance, dossier déplacé, cache              |
| 6   | Opérations, journal, corbeille             | Fait (#671)                                                               |
| 7   | Fenêtre de l'éditeur                       | Fait (#672)                                                               |
| 8   | Glisser-déposer                            | PR #674                                                                   |
| 9   | Surveillance du dossier                    | PR #675                                                                   |
| 10  | Pochette extraite, image avec le titre     | Fait (#669)                                                               |
| 11  | Essais sur la Flam, guide utilisateur      | Guide fait ([guide-editeur.md](guide-editeur.md)) ; essais à faire (§ 13) |

### 11.4 Écarts constatés à l'implémentation

- **Couverture** : sans `_couverture.*`, la complétion utilise l'image de la première histoire
  (fournie, sinon sa pochette), sinon une image avec le **titre de l'histoire** (et non une image unie).
- **Écran noir pendant un chapitre** : l'image du chapitre n'est affichée qu'avec le titre lu
  (option du menu ou annonce) ; pendant la lecture, l'écran est noir (retour d'essai, § 5.3).
- **Pochette** : extraite par ffmpeg avec `image2pipe`, flux vidéo copié tel quel, sans filtre
  ni décodage (`authoring/images.rs`).
- **Renuméroter** : l'action « Renuméroter les fichiers selon l'ordre » (§ 3.2, Q3) n'est **pas
  encore faite** ; l'ordre reste noté dans `_dossier.json`.
- **Enregistrement d'un titre lu au micro** : plus tard (Q7), comme prévu.
- **Limites** : les valeurs du § 9 sont dans `authoring/build.rs`
  (`MAX_MENU_DEPTH = 5`, `MAX_STORIES = 300`, `MAX_OPTIONS_PER_MENU = 50`, 3 Gio d'audio).
- **Dossier déplacé** : « Choisir le dossier » existe dans Mes histoires (#673) et dans l'éditeur
  (#675).

## 12. Décisions

| #   | Question                                                                                     | Recommandation                                                                                                                                         |
| --- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Q1  | Mode réglable par sous-dossier, ou seulement pour toute l'histoire ?                         | **Décidé** : par sous-dossier, avec héritage (§ 5.1)                                                                                                   |
| Q2  | Retour depuis le menu des chapitres : reprendre le chapitre quitté, ou rester dans le menu ? | **Décidé** (révisé) : retour = sortie de l'histoire ; « Reprendre l'histoire » et l'option du chapitre quitté le reprennent à l'endroit quitté (§ 5.3) |
| Q3  | Réordonner renomme-t-il les fichiers (préfixes) ?                                            | **Décidé** : non, `_dossier.json`, action « Renuméroter » à part                                                                                       |
| Q4  | Corbeille : `.flam/corbeille/` ou Corbeille Windows ?                                        | **Décidé** : `.flam/corbeille/` (annulable), vidage vers la Corbeille Windows                                                                          |
| Q5  | Écrire les compléments automatiques dans le dossier ?                                        | **Décidé** : non par défaut, bouton « Garder »                                                                                                         |
| Q6  | Fenêtre séparée pour l'éditeur (§ 4.3)                                                       | **Décidé** : oui                                                                                                                                       |
| Q7  | Enregistrer un titre lu au micro                                                             | **Décidé** : plus tard                                                                                                                                 |
| Q8  | Extraction d'une histoire installée en dossier ; histoires interactives                      | **Décidé** : plus tard, à part                                                                                                                         |

## 13. Essais sur la Flam

À faire avec une histoire créée par l'éditeur, avant de fixer les limites et de publier :

1. Recueil de 3 menus × 10 histoires : navigation, retour focalisé, `fin = menu` et `suivante`.
2. Livre de 2 parties × 15 chapitres : démarrage direct, passage d'une partie à l'autre, menu
   des chapitres par le retour, fin du livre.
3. Reprise : quitter au milieu du chapitre 7 (retour à la bibliothèque, mise en veille), puis
   « Reprendre l'histoire » : bon chapitre, bonne position, jauge cohérente.
4. Mesures : nombre de scènes avant ralentissement ou échec (300 chapitres), mémoire, temps de
   lancement.
5. Actualisation : ajouter un chapitre, « Mettre à jour l'histoire », « Remplacer sur la
   Flam ». La reprise de l'ancienne version reste cohérente ou redémarre proprement.

### 13.1 Essais restant à faire

Aucun des essais ci-dessous n'a encore été fait sur une histoire créée depuis un dossier. Le comportement décrit aux § 5.2 et 5.3 (retour,
position gardée, écran noir) reste à confirmer.

- [ ] Recueil 3 × 10 : navigation, retour focalisé, `fin = menu` et `suivante` (essai 1).
- [ ] Livre 2 × 15 : démarrage direct, passage de partie, menu des chapitres, fin du livre
      (essai 2).
- [ ] Reprise au milieu du chapitre 7 : position gardée au retour, « Reprendre l'histoire »,
      jauge (essai 3) ; les autres chapitres repartent du début.
- [ ] Écran noir pendant un chapitre, et image du chapitre avec le titre lu (§ 5.3).
- [ ] Retour depuis le premier niveau du menu des chapitres : sortie de l'histoire ; livre dans
      un recueil : remontée au recueil.
- [ ] Mesures : 300 chapitres, mémoire, temps de lancement ; confirmer ou revoir 5 niveaux,
      300 histoires, 3 Gio (essai 4).
- [ ] Actualisation d'une histoire déjà envoyée (essai 5).
- [ ] Titres lus : voix Windows et fichier, avec et sans « Annoncer le titre avant chaque
      chapitre ».
- [ ] Image avec le titre et pochette extraite : lisibilité sur l'écran de la Flam.
