# Podcasts : spécification produit

> **Statut : brouillon à valider par le mainteneur** (issue #208, épopée #207).
> Ce document est la référence des issues #209 à #221. Il reprend les décisions déjà prises
> (§ 2) et liste les décisions qui restent à prendre (§ 9), chacune avec ses options et une
> recommandation. **Une recommandation n'est pas une décision** : tant que le mainteneur n'a
> pas tranché, la section reste marquée « À décider ».

Légende utilisée dans le document :

- **Décidé** : décision du mainteneur consignée dans #207 ou dans une issue fille ;
- **Livré** : déjà présent dans `main` (fichier cité) ;
- **À décider** : point ouvert, voir § 9.

## Sommaire

1. [Objectif et périmètre](#1-objectif-et-périmètre)
2. [Décisions prises](#2-décisions-prises)
3. [Contraintes constatées dans le code et sur l'appareil](#3-contraintes-constatées-dans-le-code-et-sur-lappareil)
4. [Parcours utilisateur](#4-parcours-utilisateur)
5. [Architecture A : préparation puis conversion](#5-architecture-a--préparation-puis-conversion)
6. [Histoire générée (archive Telmi synthétique)](#6-histoire-générée-archive-telmi-synthétique)
7. [Identité, version et mise à jour](#7-identité-version-et-mise-à-jour)
8. [Licences et provenance](#8-licences-et-provenance)
9. [Décisions à prendre](#9-décisions-à-prendre)
10. [Ordres de grandeur](#10-ordres-de-grandeur)
11. [Correspondance avec les issues](#11-correspondance-avec-les-issues)

## 1. Objectif et périmètre

Permettre à un parent de transformer un podcast (flux RSS) en **une histoire** jouable sur la
Flam et dans le player du launcher, puis de la mettre à jour quand de nouveaux épisodes
paraissent.

**Priorité basse (Décidé)** : toute l'épopée est « plus-tard », à faire après la version
fonctionnelle du launcher. Elle peut être préparée dès maintenant (cette spécification, ffmpeg,
modules purs).

**Périmètre MVP (Décidé, #207)** :

- saisie d'une URL de flux RSS 2.0 avec extensions iTunes ;
- aperçu : titre, pochette, nombre d'épisodes ;
- choix des N derniers épisodes ou sélection manuelle ;
- plafonds de taille et de nombre, avec estimation avant installation ;
- téléchargement des enclosures et des pochettes ;
- conversion audio en MP3 44 100 Hz (copie si déjà conforme) ;
- pochettes en PNG 640 × 480 (images d'épisode) et 512 × 512 (couverture, #215) ;
- graphe « menu carrousel → lecture → retour au menu » ;
- installation avec uuid stable, mise à jour, erreurs claires.

**Hors MVP** (liste de rappel #221) : recherche de podcasts, synthèse vocale des titres,
arborescence (saisons, catégories), titre incrusté ou pochette générée, mise à jour automatique
et fenêtre glissante automatique, normalisation du volume, chapitres, reprise par épisode,
génération directe d'un paquet flam (option B).

## 2. Décisions prises

Toutes ces décisions viennent de l'épopée #207 (et des issues filles citées). Elles ne sont
pas rediscutées ici.

| #   | Décision                                                                                                                                                                                                                                                                                                        | Issues       |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| A1  | **1 podcast = 1 histoire.** Le flux produit une seule histoire, avec un menu carrousel des épisodes (pas une histoire par épisode).                                                                                                                                                                             | #211, #216   |
| A2  | **Identité stable** : uuid dérivé de l'URL du flux (URL normalisée, espace de noms fixe, UUID v5 par exemple).                                                                                                                                                                                                  | #211         |
| A3  | **Mise à jour par version** : même uuid, version entière strictement croissante, déclenchée par le bouton « Mettre à jour » ; la bibliothèque (`library.json`) remplace l'installation. Pas de mise à jour automatique au MVP.                                                                                  | #216, #218   |
| A4  | **Architecture A** : le launcher écrit une **archive Telmi synthétique** dans son cache, puis l'envoie au pipeline existant (`telmi2flam`). Aucune modification du format flam ni du moteur Lua.                                                                                                                | #216, #217   |
| A5  | **ffmpeg LGPL embarqué dès le départ**, lancé comme **processus externe** (jamais lié), build sans option GPL ni nonfree, empreinte SHA-256 vérifiée, licence et sources référencées dans `LICENSES.md`. Jamais de build « full » GPL.                                                                          | #209, #214   |
| A6  | **Menu sans voix au MVP** : les libellés du carrousel sont les titres des épisodes, affichés en texte. La synthèse vocale est une évolution (#221).                                                                                                                                                             | #216, #221   |
| A7  | **Provenance** : TelmiSync (GPL-3.0) a été analysé en salle blanche ; aucune reprise de code, de texte, de ressource ni de structure littérale (§ 8).                                                                                                                                                           | toutes       |
| A8  | **Lecture du flux tolérante et bornée** : flux de 16 Mio au plus, DTD refusée, bandes-annonces et épisodes sans enclosure audio ignorés, date absente laissée vide (jamais 1970).                                                                                                                               | #210 (livré) |
| A9  | **Choix par défaut** : « les N derniers épisodes », N réglable, **10 par défaut**, ou sélection manuelle dans la liste triée du plus récent au plus ancien ; le bouton d'installation est désactivé au-delà du plafond.                                                                                         | #212         |
| A10 | **Épisode en échec** (erreur HTTP, délai, fichier illisible) : retiré de la sélection avec un avertissement en fin d'opération ; échec global seulement si aucun épisode n'est récupérable.                                                                                                                     | #213, #214   |
| A11 | **Libellés** : titres d'épisodes tronqués à 40 caractères, limités aux polices du player (ASCII et Latin-1, translittération des autres caractères).                                                                                                                                                            | #216         |
| A12 | **Audio** : métadonnées et chapitres retirés ; normalisation du volume non activée au MVP ; parallélisme borné.                                                                                                                                                                                                 | #214         |
| A13 | **Pipeline** : une seule installation par uuid, un seul convertisseur à la fois, vérification de l'espace disque avant de commencer, annulation possible à chaque étape avec nettoyage, reprise propre après arrêt brutal. L'ancienne installation n'est supprimée qu'après publication réussie de la nouvelle. | #217, #218   |
| A14 | **Droits** : les podcasts sont protégés par le droit d'auteur ; usage personnel, sans redistribution ; aucun paquet converti n'est versionné dans le dépôt.                                                                                                                                                     | #220         |

## 3. Contraintes constatées dans le code et sur l'appareil

Ces faits viennent du code de `main` ; ils encadrent les options du § 9.

### 3.1 Audio

- Le firmware attend du **MP3 à 44 100 Hz** (`SAMPLERATE` dans `tools/telmi2flam/telmi2flam.py`,
  #46). Le convertisseur ne lit que du MP3 : un fichier AAC, Ogg ou WAV doit être transcodé
  **par le launcher** avant d'entrer dans l'archive Telmi (#214).
- Filet de sécurité existant : un MP3 qui n'est pas à 44 100 Hz est rééchantillonné par
  `telmi2flam` lui-même avec le ffmpeg embarqué (variable `FLAM_FFMPEG` posée par le launcher,
  `install/convert.rs` ; le launcher ne laisse jamais le convertisseur chercher un ffmpeg du
  `PATH`). Ce rééchantillonnage garde le nombre de canaux, vise un débit CBR proche de la
  source dans la plage 64 à 192 kbps, et a un **délai de 120 s par fichier**. Un long épisode
  laissé au convertisseur risque donc de dépasser ce délai : le launcher doit livrer des MP3 déjà
  conformes.
- ffmpeg embarqué (**Livré**, #252) : FFmpeg 8.1.3 minimal, audio seulement, statique, LGPL,
  construit depuis les sources par `tools/ffmpeg/build.sh` (versions et empreintes dans
  `tools/ffmpeg/pins.env`) avec `ffmpeg.exe` et `ffprobe.exe`. Décodeurs MP3, AAC, Opus, Vorbis,
  FLAC, PCM ; démultiplexeurs MP3, MOV/MP4, Ogg, FLAC, WAV, AAC, Matroska ; encodeur
  `libmp3lame` (LAME 3.100) ; protocole `file` seul (pas de réseau). Reste à faire (#214) : le
  module Rust de lancement (empreinte, arguments fixes, annulation, délai), avec `-nostdin` et
  `-vn` (ou `-map a`), sans quoi le muxeur MP3 réclame un encodeur d'image pour les pochettes
  intégrées.

### 3.2 Images

- Images de scène : l'archive Telmi porte des PNG 640 × 480 ; `telmi2flam` les ramène à la zone
  d'affichage des scènes du player, **320 × 212** (écran 320 × 240 moins le bandeau de titre).
- Vignette de bibliothèque : **104 × 136**, portrait, calculée par `telmi2flam` depuis la
  couverture, avec un **remplissage paramétrable** (réglage du launcher `ThumbFit`, option
  `--thumb-fit`) : `flou` (défaut : image entière centrée sur un fond fait de la même image
  agrandie et floutée), `rogner` (image qui couvre le cadre, rognée au centre), `bandes`
  (image entière centrée, bandes transparentes). Une pochette de podcast est presque toujours
  carrée : en `rogner`, 512 × 512 devient 136 × 136 puis perd 32 px de large (environ 24 %) ;
  en `bandes`, elle devient 104 × 104 avec deux bandes de 16 px.
- Carte de titre : la couverture est placée dans un emplacement d'environ 89 × 120 (une
  pochette carrée y occupe 89 × 89).
- Conversion des pochettes (**Livré**, #215, `src-tauri/src/podcast/artwork.rs`) : JPEG (dont
  progressif), PNG et WebP, reconnus au contenu ; garde-fous de 20 Mo par fichier, 8 000 px par
  côté, 256 Mio décodés ; transparence aplatie sur fond blanc ; image de repli **unie** dont la
  couleur est un paramètre (valeur provisoire `#122e2c`, à trancher : D13).

### 3.3 Graphe, textes et moteur

- `telmi2flam` garde les **identifiants de scène Telmi tels quels** dans `script/nodes.lua`.
- Libellés de choix : 40 caractères au plus (`LABEL_MAX`), tirés de `notes.json` (titre, sinon
  texte) ; caractères hors police ASCII et Latin-1 remplacés, avec un avertissement.
- Sélecteur par défaut : `carousel` (trois vignettes et un libellé).
- Titre et sous-titre de l'histoire sur l'appareil (`info.plain`) : `title` et `category` de
  `metadata.json` (le sous-titre retombe sur le titre si la catégorie est vide).
- Reprise sur l'appareil (`tools/telmi2flam/engine/story.lua`) : la sauvegarde retient
  l'identifiant du dernier nœud joué (scène ou choix) et, pour un choix, l'**index** de
  l'option présélectionnée. À la relance, si ce nœud n'existe plus, l'histoire redémarre au
  début (`noeud de reprise inconnu`).
- Mémoire du convertisseur : `telmi2flam` assemble **tout le paquet en mémoire** avant d'écrire
  le `.plain.pk` (fichiers de sortie gardés dans un dictionnaire). La taille totale d'un podcast
  se paie donc aussi en mémoire vive pendant la conversion.

### 3.4 Identité sur l'appareil

- Le dossier de l'histoire sur la Flam (`str/<uuid>/`) est nommé d'après `uuid.bin`, que
  `telmi2flam` calcule comme l'**empreinte MD5 de la chaîne `uuid` de `metadata.json`**. Le même
  uuid Telmi donne donc toujours le même uuid d'appareil ; mais une chaîne différente (même
  casse différente) donne un autre uuid. Le générateur écrit toujours l'uuid sous sa forme
  canonique (minuscules, avec tirets).
- Le fichier `version` du paquet flam vaut toujours `1` : la version d'un podcast vit dans la
  bibliothèque du launcher (`library.json`, `<cache>/library/<uuid>-<version>.plain.pk`), pas
  sur l'appareil.
- Une mise à jour sur l'appareil remplace `str/<uuid>/` (transfert avec remplacement,
  `docs/flam-device-format.md` à la racine du dépôt, § 8.1) ; les sauvegardes de `usr/0/` (`.prog`, `.save`,
  `chaps.save`) **ne sont jamais touchées** par le launcher.
- Avant un transfert, l'espace libre est vérifié (chaque fichier arrondi à 32 Kio, plus 32 Mio de
  marge). La capacité totale de la Flam n'est pas consignée dans le dépôt : elle est à relever
  pendant le test réel (#219).
- Appareil de test du mainteneur : **Flam en firmware 1.x**. Aucun résultat n'est encore
  consigné pour le firmware 2.x.

### 3.5 Pipeline et sources existants

- Lecture du flux (**Livré**, #210, `src-tauri/src/podcast/rss.rs`) : 5 000 épisodes au plus,
  encodages UTF-8, UTF-16, ISO-8859-1 et Windows-1252, doublons de `guid` ignorés, `guid` absent
  remplacé par l'URL de l'enclosure. Champs lus par épisode : titre, description, date, durée
  (`itunes:duration`), auteur, image, URL, type et **taille annoncée** de l'enclosure (non
  vérifiée). Ne sont pas lus aujourd'hui : `itunes:explicit`, `itunes:type` (épisodique ou
  feuilleton), `itunes:new-feed-url`, saisons et numéros d'épisode.
- Sources : `Catalog`, `Folder` et la bibliothèque ; la source podcast reste à ajouter (#211).
  Supprimer une source ne touche pas aux histoires installées.
- Téléchargeur (`install/`) : liens http(s) seulement, reprise par plages, 4 Gio au plus par
  fichier. **Aucun filtrage des adresses privées** aujourd'hui (D10).
- Bibliothèque : `is_outdated_by` signale une mise à jour quand la version proposée **diffère**
  de la version installée (même uuid) ; la version doit donc augmenter à chaque génération qui
  change le contenu.

## 4. Parcours utilisateur

1. **Ajouter un podcast** (Sources) : le parent colle l'URL du flux. Contrôles : http(s)
   seulement, longueur bornée, pas d'identifiants dans l'URL (#211). Erreurs distinctes : flux
   invalide, aucun épisode audio, flux trop grand, réseau.
2. **Aperçu** : pochette, titre, description, nombre d'épisodes.
3. **Choix** : les N derniers (10 par défaut) ou sélection manuelle ; estimation de taille et
   plafond affichés ; installation impossible au-delà (D1, D5).
4. **Mention d'usage personnel** sur l'écran d'ajout, message de droits à la première
   installation (D14, #220).
5. **Installation** : préparation (téléchargement, conversion audio, pochettes, archive), puis
   conversion et publication dans la bibliothèque, avec progression et annulation.
6. **Lecture** : l'histoire s'ouvre sur sa carte de titre, puis le menu carrousel des épisodes ;
   un épisode se joue, puis l'histoire revient au menu.
7. **Mise à jour** : quand de nouveaux épisodes paraissent, l'histoire porte l'indicateur
   « Mise à jour disponible » ; le bouton « Mettre à jour » relance la préparation (D2, D3).
8. **Transfert** sur la Flam : comme toute histoire de la bibliothèque.
9. **Suppression de la source** : voir D15.

Textes en français, dans le fichier de textes de l'interface ; parcours complet au clavier
(#212).

## 5. Architecture A : préparation puis conversion

```text
URL du flux ─► téléchargement du flux (cache hors ligne du dernier flux valide, #211)
            ─► lecture RSS (#210, livré)
            ─► choix des épisodes (#212)
            ─► préparation (#217) :
                 téléchargements bornés et annulables (#213)
                 audio : copie ou MP3 44 100 Hz via ffmpeg (#214)
                 pochettes : PNG 640 × 480 et 512 × 512 (#215, livré)
                 archive Telmi synthétique dans le cache (#216)
            ─► pipeline existant : telmi2flam → .plain.pk → library.json (#217)
            ─► transfert sur la Flam (pipeline existant)
```

- La préparation est une **nouvelle étape** de la file d'installation persistée, avec ses états
  (préparation en cours, préparé, erreur) et la même discipline que l'existant : fichiers
  temporaires dans le dossier de travail de la tâche, nettoyés à l'annulation et à l'échec,
  reprise ou nettoyage après un arrêt brutal (#217). Le code qui touche à la bibliothèque et au
  journal d'installation suit les règles de `CONTRIBUTING.md` § 5 (note de conception, test
  d'injection de panne, `modele:opus`).
- Tout accès réseau passe par le téléchargeur existant ; les modules `rss` et `artwork` restent
  purs (octets en entrée, octets en sortie).
- Une archive Telmi contient des MP3 déjà compressés : la stocker sans compression évite un
  travail inutile.
- **Limite connue de l'architecture A** : l'audio est copié plusieurs fois (originaux, MP3
  convertis, archive Telmi, `.plain.pk`). C'est le prix de la réutilisation du pipeline ; la
  génération directe d'un paquet flam (option B) est une évolution (#221), après le port Rust
  de `telmi2flam`.

## 6. Histoire générée (archive Telmi synthétique)

Le format Telmi est pris de sa description publique (wiki Telmi), jamais du code d'un autre
projet.

**Fichiers** : `metadata.json`, `nodes.json`, `notes.json`, couverture (`cover.png` ou
`title.png`, référencée par `metadata.image`), `images/`, `audios/`. Pas de `title.mp3` (menu
sans voix).

**Métadonnées** : `uuid` (§ 7), `version` entière, `title` (titre du flux), `category` (affichée
comme sous-titre sur l'appareil, D16), `age` (D16), `description` sans HTML, auteur.

**Graphe** (Décidé) :

- une scène d'entrée ;
- une scène de choix « menu » : un choix par épisode, affiché en carrousel ;
- chaque choix mène à une scène de lecture audio de l'épisode ;
- à la fin de l'épisode, retour au menu.

**Contenu par épisode** : libellé = titre de l'épisode (40 caractères, translittéré) ; image =
pochette de l'épisode, sinon du podcast, sinon image de repli (D13) ; note = titre de scène et
description de l'épisode ; audio = MP3 conforme (§ 3.1).

**Identifiants de scène et noms de fichiers** : stables d'une génération à l'autre pour un même
épisode (dérivés de son identifiant `guid`), et non de sa position dans la liste. C'est la
condition pour que la reprise de lecture survive à une mise à jour (D4).

**Taille du graphe** : menus de 1, 10 et 50 épisodes testés (#216) ; la limite pratique du moteur
sur l'appareil est mesurée en #219 (D1).

## 7. Identité, version et mise à jour

- **uuid** (Décidé) : UUID v5 calculé sur l'URL normalisée du flux (schéma et hôte en minuscules,
  fragment retiré, règles complètes fixées et testées dans #211), dans un espace de noms fixe
  propre au launcher, écrit en minuscules. Deux URL équivalentes donnent le même uuid ; deux URL
  différentes, deux uuid différents. Le comportement quand le flux change d'adresse est à
  décider (D12).
- **Version** (Décidé) : entier strictement croissant, incrémenté à chaque mise à jour qui change
  le contenu. Comme `is_outdated_by` ne compare que l'égalité des versions, une version qui
  n'augmente pas ne serait jamais proposée ; une version qui change sans contenu nouveau
  produirait une fausse alerte.
- **Détection** (#218) : par comparaison des épisodes installés (identifiants ou enclosures) avec
  le flux, pas seulement par la date du flux. Pas d'indicateur si aucun épisode nouveau.
- **Remplacement** : même uuid, version supérieure ; l'ancienne archive n'est supprimée qu'après
  publication réussie ; un échec laisse l'installation précédente utilisable.
- Ce qui reste à décider : quels épisodes garder à la mise à jour (D2), réutilisation des
  épisodes déjà préparés (D3), progression de lecture (D4).

## 8. Licences et provenance

- **TelmiSync est sous GPL-3.0.** Il a été analysé en **salle blanche** : comportement décrit avec
  des mots propres, sans extrait de code, sans nom de fonction ou de variable, sans texte
  d'interface. **Ne jamais reprendre de code, de texte, de ressource (images, voix, listes,
  polices) ni de structure littérale de TelmiSync.** Toute implémentation part du format Telmi
  documenté publiquement (wiki) et des règles de flam-launcher (`AGENT.md`, section
  « Surcouche flam-launcher »).
- **ffmpeg** : build LGPL sans options GPL, SHA-256 vérifié, `LICENSES.md` à jour, sources ou
  offre de sources disponibles (`ffmpeg/BUILD.txt`). Jamais de build GPL « full ».
- **Nouvelles crates** (images, XML) : licence compatible (MIT, Apache-2.0 ou MPL-2.0), ajoutées
  à `LICENSES.md`.
- **Synthèse vocale (évolution)** : piper dépend d'espeak-ng (GPL-3.0) ; pas d'embarquement sans
  analyse de licence dédiée. Les voix système de Windows sont le premier choix (#221).
- **Contenus** : aucune archive Telmi, aucun flux réel, aucune histoire convertie dans le dépôt ;
  les tests utilisent des flux et médias synthétiques ; les essais réels restent hors dépôt
  (`.cache/`).

## 9. Décisions à prendre

Chaque point indique la question, les options et leurs conséquences, puis une recommandation.
Les chiffres viennent du § 10.

### D1. Plafonds : nombre d'épisodes, taille par épisode, taille totale — À décider

**Question** : quelles limites appliquer (interface et backend, #212), et sont-elles
réglables ?

**Faits** : 10 épisodes de 30 min pèsent environ 144 Mo à 64 kbps et 288 Mo à 128 kbps ; 50
épisodes d'une heure à 128 kbps, environ 2,9 Go. `telmi2flam` garde tout le paquet en mémoire
(§ 3.3). La capacité de la Flam et la limite pratique du menu sur l'appareil ne sont pas encore
mesurées (#219).

| Option                                                                                                          | Conséquences                                                                                                                                             |
| --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| a. Plafonds fixes : 50 épisodes, 200 Mo par épisode, 1 Gio au total                                             | Simple à tester ; 1 Gio couvre environ 18 h à 128 kbps ou 37 h à 64 kbps ; mémoire du convertisseur bornée. Un podcast d'émissions longues sera tronqué. |
| b. Mêmes valeurs par défaut, réglables dans les Paramètres jusqu'à un maximum dur (par ex. 100 épisodes, 2 Gio) | Souplesse pour les usages adultes ; plus d'états à tester ; le maximum dur reste nécessaire pour la mémoire et le temps de conversion.                   |
| c. Plafond total seul (pas de limite de nombre)                                                                 | Le carrousel peut dépasser ce que le moteur supporte sur l'appareil (non mesuré).                                                                        |

**Recommandation : a**, valeurs fixes au MVP, revues après #219 (le gabarit de compatibilité
peut les ajuster) ; un réglage (b) seulement si un besoin réel apparaît. 50 épisodes est la plus
grande taille de menu prévue dans les tests de #216 et #219 ; 200 Mo par épisode couvre plus de
3 h à 128 kbps ; 1 Gio garde la conversion en mémoire raisonnable sur un PC de 8 Go et reste en
dessous de la limite de 4 Gio du pipeline.

### D2. Épisodes retenus à la mise à jour, sort des anciens — À décider

**Question** : quand de nouveaux épisodes paraissent, que contient la nouvelle version ?

| Option                                                                                                                                                                                           | Conséquences                                                                                                                                                                                     |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| a. Fenêtre stricte : toujours les N derniers du flux                                                                                                                                             | Simple, taille stable. Un épisode que l'enfant aime disparaît sans qu'on le choisisse ; la sélection manuelle est perdue. Proche de la « fenêtre glissante » que #221 range dans les évolutions. |
| b. Cumul : épisodes installés conservés, nouveaux ajoutés, jusqu'au plafond (D1) ; au-delà, retrait des plus anciens **annoncé avant la mise à jour** (liste des épisodes retirés, confirmation) | Rien ne disparaît sans le dire ; garde les choix manuels ; taille croissante jusqu'au plafond.                                                                                                   |
| c. Cumul sans retrait : mise à jour refusée au-delà du plafond, le parent retire lui-même des épisodes                                                                                           | Contrôle total ; demande un écran d'édition de la sélection en plus.                                                                                                                             |
| d. Redemander le choix complet à chaque mise à jour (écran de #212 prérempli)                                                                                                                    | Toujours explicite ; une étape de plus à chaque mise à jour.                                                                                                                                     |

**Recommandation : b**, avec l'écran de confirmation qui liste les ajouts et les retraits. Un
épisode installé qui a disparu du flux (retiré par l'éditeur) est conservé tant qu'il est
disponible localement (D3), sinon retiré avec un avertissement.

### D3. Réutilisation des épisodes déjà préparés — À décider

**Question** : à la mise à jour, faut-il retélécharger et reconvertir les épisodes déjà
installés ?

| Option                                                                                 | Conséquences                                                                                                                                     |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| a. Tout retélécharger                                                                  | Rien à stocker ; réseau et temps multipliés ; un épisode retiré du flux est perdu (incompatible avec D2 b).                                      |
| b. Cache des MP3 convertis et des PNG par podcast, dans le cache du launcher           | Mise à jour rapide ; coûte une fois la taille du podcast sur le disque ; cache à purger à la suppression de la source ou de l'histoire.          |
| c. Réextraire l'audio du `.plain.pk` installé (les `sounds/*.mp3` sont déjà conformes) | Aucun stockage en plus ; couple la génération au format de sortie de `telmi2flam` (noms de fichiers alloués par le convertisseur), plus fragile. |

**Recommandation : b**, simple et indépendante du format de sortie ; la purge suit D15.

### D4. Progression de lecture après mise à jour — À décider

**Faits** : sur l'appareil, la sauvegarde est rangée sous l'uuid de l'histoire, qui ne change pas,
et le launcher n'y touche jamais (§ 3.4). Elle retient l'identifiant du dernier nœud joué ; s'il
n'existe plus, l'histoire redémarre au début. Pour le menu, elle retient l'**index** de l'option
présélectionnée.

| Option                                                            | Conséquences                                                                                                                                                                                                                                          |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| a. Identifiants de scène dérivés du `guid` de l'épisode (stables) | La reprise retombe sur le même épisode s'il est toujours là ; sinon redémarrage au début (comportement du moteur, sans message sur l'appareil). Si des épisodes sont ajoutés en tête, la présélection du menu peut se décaler d'un cran (inoffensif). |
| b. Identifiants séquentiels (position dans la liste)              | Plus simple, mais la reprise peut tomber sur **un autre épisode** que celui en cours : à écarter.                                                                                                                                                     |
| c. Réinitialiser la progression à chaque mise à jour              | Impossible sans écrire dans `usr/0/` (interdit) ou changer d'uuid (contraire à A2).                                                                                                                                                                   |

**Recommandation : a**, et un message court dans le launcher au moment de la mise à jour (« la
reprise peut revenir au début de l'histoire »). Comportement à vérifier sur la Flam du mainteneur
(firmware 1.x) en #219, ainsi que l'effet sur la jauge de progression (le nombre de chapitres
change avec le nombre d'épisodes).

### D5. Estimation de taille avant installation — À décider

**Faits** : la taille annoncée de l'enclosure (`length`) est souvent absente, à zéro ou fausse ;
la durée (`itunes:duration`) est souvent présente. La taille finale dépend du débit après
conversion (D6). Pendant la préparation, l'audio existe en plusieurs copies (§ 5).

| Option                                                                                                               | Conséquences                                                                                                                   |
| -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| a. Taille annoncée seule                                                                                             | Gratuit ; souvent faux ou inconnu.                                                                                             |
| b. Taille annoncée, sinon durée × débit cible, sinon valeur forfaitaire marquée « inconnue » ; affichage « environ » | Gratuit et utile dans la plupart des cas ; les plafonds restent vérifiés pendant le téléchargement (#213).                     |
| c. Requête `HEAD` par épisode pour lire `Content-Length`                                                             | Plus juste ; une requête par épisode (lent pour 50 épisodes, certains serveurs refusent `HEAD`, mesure d'audience déclenchée). |

**Recommandation : b**. Afficher deux chiffres : taille de l'histoire (pour la Flam) et espace
disque nécessaire sur le PC pendant la préparation (environ trois fois la taille de l'histoire,
plus une marge), qui sert aussi à la vérification d'espace de #217.

### D6. Débit cible, règle de copie, mono — À décider

**Question** : à quel débit transcoder, quand copier sans réencoder, et que faire du mono (#214) ?

| Option                                                                                                                      | Conséquences                                                                                                                           |
| --------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| a. Copie si MP3 44 100 Hz entre 64 et 192 kbps (plage déjà retenue par `telmi2flam`) ; sinon CBR 128 kbps, canaux conservés | Qualité préservée ; 57,6 Mo par heure pour les épisodes transcodés.                                                                    |
| b. Comme a, mais transcodage à 64 kbps en mono et 96 kbps en stéréo                                                         | Fichiers deux fois plus petits pour la voix parlée ; qualité suffisante pour un haut-parleur de boîtier ; à valider à l'écoute (#219). |
| c. Comme a, mais mono converti en stéréo                                                                                    | Sans effet sur la taille à débit CBR égal ; utile seulement si l'appareil lit mal le mono.                                             |

**Recommandation : b**, mono conservé tant que #219 ne montre pas de problème sur l'appareil
(le MP3 mono 22 kHz fait déjà partie des essais prévus) ; bascule vers c si le mono pose
problème.

### D7. Ordre des épisodes dans le menu — À décider

| Option                                                                          | Conséquences                                                                                        |
| ------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| a. Plus récent en premier                                                       | Le nouvel épisode est sous la main ; un feuilleton se lit à l'envers.                               |
| b. Ordre chronologique (le plus ancien en premier)                              | Naturel pour un feuilleton ; les nouveautés sont au bout du carrousel.                              |
| c. Selon `itunes:type` : `serial` → chronologique, sinon plus récent en premier | Le bon ordre dans la plupart des cas ; demande de lire `itunes:type` (non lu par #210 aujourd'hui). |

**Recommandation : c**, avec a par défaut si le champ est absent.

### D8. Épisodes explicites — À décider

**Faits** : `itunes:explicit` n'est pas lu aujourd'hui. Le public visé est l'enfant de 2 à 8 ans,
mais c'est le parent qui choisit.

| Option                                                                                        | Conséquences                                                                                 |
| --------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| a. Ignorer                                                                                    | Rien à faire ; le parent n'est pas averti.                                                   |
| b. Signaler dans l'aperçu (flux et épisodes marqués), épisodes explicites décochés par défaut | Information sans blocage ; lecture d'un champ de plus (#210).                                |
| c. Refuser les épisodes ou les flux explicites                                                | Protège par défaut ; bloque des usages légitimes ; le marquage est déclaratif et peu fiable. |

**Recommandation : b**.

### D9. Publicité et mesure d'audience — À décider

**Faits** : de nombreux hébergeurs insèrent la publicité dans l'audio au moment du
téléchargement, et passent les enclosures par des redirections de mesure d'audience. Rien ne
permet de la détecter ni de la retirer de façon fiable.

| Option                                                            | Conséquences                                            |
| ----------------------------------------------------------------- | ------------------------------------------------------- |
| a. Ne rien faire                                                  | Le parent découvre la publicité à l'écoute.             |
| b. Le dire : une phrase dans la mention d'usage et dans le README | Honnête, sans coût technique ; rien n'est filtré.       |
| c. Tenter de retirer la publicité                                 | Peu fiable, peut modifier l'œuvre (droits) : à écarter. |

**Recommandation : b**.

### D10. Adresses privées et locales (SSRF) — À décider

**Faits** : l'URL du flux est saisie par le parent, mais les enclosures et images viennent du
flux, donc d'un tiers. Le téléchargeur ne filtre aucune adresse aujourd'hui.

| Option                                                                                                                                                                                                                                                                         | Conséquences                                                                                                    |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| a. Refus des adresses de bouclage, privées, lien-local, partagées (100.64/10), multidiffusion, non spécifiées et IPv6 équivalentes, pour le flux, les enclosures et les images ; contrôle **après résolution DNS** et à **chaque redirection**, connexion à l'adresse vérifiée | Ferme le SSRF, y compris par redirection ou rebond DNS ; un flux hébergé sur un NAS du réseau local est refusé. |
| b. Comme a, avec une exception explicite dans les Paramètres (« autoriser le réseau local »)                                                                                                                                                                                   | Permet l'auto-hébergement ; un réglage de plus, à expliquer.                                                    |
| c. Refus pour les enclosures et images seulement, flux local accepté                                                                                                                                                                                                           | Le flux local reste possible, mais ses enclosures locales seraient refusées : incohérent.                       |

**Recommandation : a** au MVP (c'est la règle par défaut prévue par #213) ; b plus tard si un
besoin réel apparaît.

### D11. Flux privés et authentifiés — À décider

**Faits** : #211 refuse les identifiants dans l'URL (`user:pass@`). Les flux privés courants
(abonnements payants) portent plutôt un jeton dans le chemin ou la requête. L'uuid dérivant de
l'URL, un jeton renouvelé donne une autre histoire.

| Option                                                                                                                          | Conséquences                                                                                |
| ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| a. Refuser `user:pass@`, accepter les URL à jeton sans traitement particulier                                                   | Rien à faire ; le jeton apparaît en clair dans `sources.json`, les journaux et l'interface. |
| b. Comme a, et traiter toute URL de flux comme un secret : jamais journalisée en entier, masquée dans l'interface après l'ajout | Protège le jeton ; quelques règles de journalisation à suivre dans #211 et #213.            |
| c. Gérer l'authentification HTTP (identifiant et mot de passe stockés)                                                          | Couvre plus de flux ; stockage de secrets à sécuriser ; hors MVP.                           |

**Recommandation : b**.

### D12. Flux qui change d'adresse — À décider

**Faits** : un éditeur peut déplacer son flux (redirection permanente, balise
`itunes:new-feed-url`). L'uuid est dérivé de l'URL (A2).

| Option                                                                                                                                                                               | Conséquences                                                                                                                                                                              |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| a. uuid calculé **une fois**, à l'ajout de la source, et enregistré avec elle ; la nouvelle adresse est suivie (redirections, `new-feed-url` après confirmation) sans changer l'uuid | La mise à jour continue ; la sauvegarde de l'enfant est gardée ; « dérivé de l'URL » s'entend « de l'URL d'origine ». Réajouter le podcast avec la nouvelle URL donne une autre histoire. |
| b. uuid recalculé depuis l'URL courante                                                                                                                                              | Règle pure, mais un déménagement du flux crée une deuxième histoire et perd la progression.                                                                                               |

**Recommandation : a**.

### D13. Image de repli sans pochette — À décider

**Faits** : `artwork.rs` sait produire une image unie de la bonne taille ; couleur provisoire
`#122e2c` (surface du thème sombre).

| Option                                                                 | Conséquences                                                                  |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| a. Couleur unie `#122e2c`                                              | Déjà prête ; neutre ; les épisodes se distinguent seulement par leur libellé. |
| b. Autre couleur unie de la palette (par ex. le vert d'eau du boîtier) | Plus gai ; même travail.                                                      |
| c. Image dessinée pour le projet                                       | Plus soigné ; une ressource à créer (pas d'emprunt à un autre projet).        |
| d. Pochette générée avec titre incrusté                                | Prévu en évolution (#221) ; hors MVP.                                         |

**Recommandation : a** au MVP, d en évolution.

### D14. Mention d'usage personnel — À décider (texte à valider)

**Question** : où l'afficher et avec quel texte ? #212 et #220 prévoient une mention sur l'écran
d'ajout et un message de droits à la première installation.

| Option                                                                | Conséquences                                      |
| --------------------------------------------------------------------- | ------------------------------------------------- |
| a. Texte court sur l'écran d'ajout, sans action demandée              | Discret ; peut passer inaperçu.                   |
| b. a + message à la première installation, fermé par « J'ai compris » | Lu au moins une fois ; un clic de plus, une fois. |
| c. Case à cocher obligatoire à chaque ajout                           | Pesant pour un usage répété.                      |

**Recommandation : b**. Proposition de texte, à valider ou réécrire :

> Les podcasts appartiennent à leurs auteurs. Les histoires créées ici sont réservées à un usage
> personnel, dans le cadre familial : ne les partagez pas et ne les redistribuez pas.
> L'audio est téléchargé tel que l'éditeur le diffuse, publicité comprise.

### D15. Suppression d'une source podcast — À décider (#211)

**Faits** : aujourd'hui, supprimer une source ne touche pas aux histoires installées.

| Option                                                                                        | Conséquences                                                                                                                       |
| --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| a. L'histoire installée reste ; le cache du podcast (flux, épisodes préparés de D3) est purgé | Cohérent avec les autres sources ; plus de mise à jour possible ; réajouter la même URL retrouve le même uuid et la même histoire. |
| b. Proposer aussi de retirer l'histoire de la bibliothèque                                    | Plus complet ; un choix de plus dans une boîte de confirmation.                                                                    |
| c. Refuser la suppression tant que l'histoire est installée                                   | Bloquant, sans bénéfice clair.                                                                                                     |

**Recommandation : a**.

### D16. Métadonnées de l'histoire (sous-titre, âge) — À décider

**Faits** : la catégorie Telmi devient le sous-titre sur l'appareil (§ 3.3).

| Option                                           | Conséquences                                          |
| ------------------------------------------------ | ----------------------------------------------------- |
| a. Sous-titre « Podcast », âge laissé vide       | Repère clair dans la bibliothèque de la Flam.         |
| b. Sous-titre = auteur du podcast, âge vide      | Plus informatif ; l'auteur peut être long ou absent.  |
| c. Âge choisi par le parent dans l'écran d'ajout | Un champ de plus, sans effet constaté sur l'appareil. |

**Recommandation : a**.

## 10. Ordres de grandeur

Les débits sont en kbit/s ; 1 Mo = 10⁶ octets ; audio seul, en CBR.

| Débit    | Par minute | Par heure | Épisode de 15 min | Épisode de 30 min | Épisode de 1 h |
| -------- | ---------- | --------- | ----------------- | ----------------- | -------------- |
| 64 kbps  | 0,48 Mo    | 28,8 Mo   | 7,2 Mo            | 14,4 Mo           | 28,8 Mo        |
| 96 kbps  | 0,72 Mo    | 43,2 Mo   | 10,8 Mo           | 21,6 Mo           | 43,2 Mo        |
| 128 kbps | 0,96 Mo    | 57,6 Mo   | 14,4 Mo           | 28,8 Mo           | 57,6 Mo        |
| 192 kbps | 1,44 Mo    | 86,4 Mo   | 21,6 Mo           | 43,2 Mo           | 86,4 Mo        |

- 10 épisodes de 30 min : 144 Mo à 64 kbps, 288 Mo à 128 kbps.
- 50 épisodes de 30 min : 720 Mo à 64 kbps, 1,44 Go à 128 kbps.
- 1 Gio (1,074 Go) : environ 37 h à 64 kbps, 18,6 h à 128 kbps.
- Images : quelques images de 320 × 212 par épisode une fois converties, négligeables devant
  l'audio (à mesurer en #219).
- Disque du PC pendant la préparation : originaux + MP3 convertis + archive Telmi + `.plain.pk`,
  soit jusqu'à trois ou quatre fois la taille de l'histoire si rien n'est supprimé en cours de
  route.

## 11. Correspondance avec les issues

| Issue | Sujet                             | Sections | Décisions à prendre qui la concernent |
| ----- | --------------------------------- | -------- | ------------------------------------- |
| #209  | ffmpeg LGPL embarqué              | 3.1, 8   | (livré en partie par #252)            |
| #210  | Lecture RSS (fermée)              | 3.5      | D7, D8 (champs à lire en plus)        |
| #211  | Source « Podcast »                | 4, 7     | D11, D12, D15                         |
| #212  | Écran d'ajout et choix            | 4        | D1, D5, D8, D14                       |
| #213  | Téléchargements                   | 3.5, 5   | D1, D10, D11                          |
| #214  | Conversion audio                  | 3.1      | D6                                    |
| #215  | Pochettes (fermée)                | 3.2      | D13                                   |
| #216  | Archive Telmi synthétique         | 6        | D4, D7, D13, D16                      |
| #217  | Intégration au pipeline           | 5        | D1, D5                                |
| #218  | Mise à jour                       | 7        | D2, D3, D4                            |
| #219  | Test réel sur Flam (firmware 1.x) | 3, 10    | D1, D4, D6                            |
| #220  | Documentation et droits           | 8        | D9, D14                               |
| #221  | Évolutions                        | 1        | —                                     |
