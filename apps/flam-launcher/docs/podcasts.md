# Podcasts : spécification produit (modèle « pack »)

> **Statut : décisions prises** (issue #208, épopée #207). Ce document est la référence des
> issues #209 à #221 et des issues #361 à #363.
>
> Le 2026-10-19, le mainteneur a abandonné le modèle « 1 podcast = 1 histoire + Mettre à jour »
> pour le **modèle « pack »** décrit ici (§ 2). Le 2026-10-23, il a **adopté les recommandations
> N1 à N10** du § 9 (N1 b, N2 b, N3 b, N4 a, N5 a, N6 b, N7 a, N8 a, N9 a, N10 a). Les tableaux
> d'options du § 9 sont conservés pour l'historique. Les points qui restent ouverts sont listés
> à la fin du § 9.

Légende :

- **Décidé** : décision du mainteneur (#207, issues filles, décision du 2026-10-19 ou du
  2026-10-23) ;
- **Livré** : déjà présent dans `main` (fichier cité) ;
- **À décider** : point ouvert (§ 9, « Points restant à décider »).

## Sommaire

1. [Objectif et périmètre](#1-objectif-et-périmètre)
2. [Décisions prises](#2-décisions-prises)
3. [Contraintes constatées dans le code et sur l'appareil](#3-contraintes-constatées-dans-le-code-et-sur-lappareil)
4. [Parcours utilisateur](#4-parcours-utilisateur)
5. [Architecture A : préparation puis conversion](#5-architecture-a--préparation-puis-conversion)
6. [Histoire générée (archive Telmi synthétique)](#6-histoire-générée-archive-telmi-synthétique)
7. [Licences et provenance](#7-licences-et-provenance)
8. [Abandonné](#8-abandonné)
9. [Décisions du 2026-10-23 (N1 à N10)](#9-décisions-du-2026-10-23-n1-à-n10)
10. [Ordres de grandeur](#10-ordres-de-grandeur)
11. [Impact sur les issues](#11-impact-sur-les-issues)

## 1. Objectif et périmètre

Permettre à un parent de composer, à partir d'épisodes de podcasts (flux RSS), un **pack** :
une histoire jouable sur la Flam et dans le player du launcher, avec un menu carrousel organisé
comme il le souhaite.

**Priorité basse (Décidé, #207)** : l'épopée est « plus-tard », à faire après la version
fonctionnelle du launcher ; elle peut être préparée dès maintenant.

**Périmètre MVP (Décidé)** :

- ajout de sources « podcast » par URL de flux RSS 2.0 avec extensions iTunes ; épisodes listés ;
- sélection d'épisodes, éventuellement de **plusieurs flux** ;
- rangement dans une **arborescence libre** (catégories et sous-menus, glisser-déposer) ;
- saisie des métadonnées du pack (défauts : D16 et N1), choix de la couverture (N6) ;
- brouillon unique sauvegardé automatiquement (N3) ;
- plafonds de taille et de nombre, avec estimation avant création ;
- « Créer le pack » : téléchargement, conversion audio (MP3 44 100 Hz), pochettes, archive Telmi
  synthétique, conversion `telmi2flam`, entrée dans la bibliothèque ;
- erreurs claires ; un épisode en échec est retiré et signalé.

**Hors MVP** (#221, voir § 11) : recherche de podcasts et liste de flux conseillés, synthèse
vocale des titres, titre incrusté ou pochette générée, compositions nommées réutilisables (N3 c),
cache des MP3 convertis (N8 b), image de catégorie choisie par le parent (N5 c), normalisation du
volume, chapitres, reprise par épisode, génération directe d'un paquet flam (option B).

## 2. Décisions prises

### 2.1 Décisions de #207 toujours valables

| #   | Décision                                                                                                                                                                                                    | Issues     |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| A1  | **Architecture A** : le launcher écrit une **archive Telmi synthétique** dans son cache, puis l'envoie au pipeline existant (`telmi2flam`). Aucune modification du format flam ni du moteur Lua.            | #216, #217 |
| A2  | **ffmpeg LGPL embarqué**, lancé comme **processus externe** (jamais lié), sans option GPL ni nonfree, empreinte SHA-256 vérifiée, licence et sources dans `LICENSES.md`. Jamais de build « full » GPL.      | #209, #214 |
| A3  | **Menu sans voix au MVP** : les libellés sont les titres (épisodes, catégories), affichés en texte. Synthèse vocale : évolution (#221).                                                                     | #216, #221 |
| A4  | **Provenance** : salle blanche, aucune reprise de code, de texte, de ressource ni de structure littérale de TelmiSync (§ 7).                                                                                | toutes     |
| A5  | **Lecture du flux tolérante et bornée** : flux de 16 Mio au plus, DTD refusée, date absente laissée vide (jamais 1970).                                                                                     | #210       |
| A6  | **Libellés** : 40 caractères au plus, polices du player (ASCII et Latin-1, translittération des autres caractères).                                                                                         | #216       |
| A7  | **Pipeline** : un seul convertisseur à la fois, espace disque vérifié avant de commencer, annulation possible à chaque étape avec nettoyage, reprise propre après un arrêt brutal.                          | #217       |
| A8  | **Droits** : usage personnel, sans redistribution ; aucun paquet converti versionné. Mention courte sur l'écran d'ajout et message de droits à la première installation ; textes validés par le mainteneur. | #212, #220 |

### 2.2 Modèle « pack » (Décidé, 2026-10-19)

| #   | Décision                                                                                                                                                                                                                                                                                   |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| P1  | **Une source = un flux RSS** ajouté par son URL ; ses épisodes sont listés.                                                                                                                                                                                                                |
| P2  | Le parent **coche des épisodes**, éventuellement de **plusieurs flux**, les range dans une **arborescence libre** (catégories, sous-menus, glisser-déposer), saisit les métadonnées, puis « Créer le pack ».                                                                               |
| P3  | **Chaque pack est une nouvelle histoire** : uuid **aléatoire** généré par le launcher, **version fixe**, **aucune mise à jour**, aucune identité liée au flux. Refaire un pack donne une nouvelle histoire.                                                                                |
| P4  | **Supprimer une source ne touche pas aux histoires créées.**                                                                                                                                                                                                                               |
| P5  | **Épisodes écartés de la liste** : sans enclosure audio, ou de type bande-annonce (`itunes:episodeType` = `trailer`). Liste triée par **date décroissante** par défaut.                                                                                                                    |
| P6  | **Images** : image d'épisode = image de l'épisode, sinon du podcast. Image du podcast = image du canal, sinon image iTunes du canal, sinon image du premier épisode.                                                                                                                       |
| P7  | **Épisode en échec** (téléchargement ou conversion) : retiré du pack et signalé ; la description du pack liste les échecs.                                                                                                                                                                 |
| P8  | **Menu** : carrousel des épisodes (dans chaque menu de l'arborescence). À la fin d'un épisode, l'épisode **suivant** se joue ; la fin du dernier **retourne au menu**.                                                                                                                     |
| P9  | **Pièges écartés** (comportements observés ailleurs, à ne pas reproduire) : TLS non vérifié, redirections illimitées sans contrôle d'adresse, redirection par page HTML « refresh », flux XML lu sans plafond, URL à jeton en clair, date de repli en 1970, nettoyeur HTML maison peu sûr. |

### 2.3 Points autrefois ouverts, tranchés le 2026-10-19

La numérotation reprend celle de la version précédente du document.

| #   | Décision (2026-10-19)                                                                                                                                                                                                                                                                                                                                                              |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | **Plafonds fixes par pack** : 50 épisodes, 200 Mio par épisode, 1 Gio au total. Appliqués dans l'interface et dans le backend ; création impossible au-delà.                                                                                                                                                                                                                        |
| D5  | **Estimation** : taille annoncée de l'enclosure, sinon durée × débit, affichée « environ » ; espace disque nécessaire sur le PC affiché aussi (jusqu'à quatre fois la taille du pack, § 10, plus une marge). Les plafonds restent vérifiés pendant le téléchargement.                                                                                                              |
| D6  | **Audio** : **copie** si le fichier est un MP3 à 44 100 Hz, mono ou stéréo, entre 64 et 192 kbps ; sinon **réencodage** en MP3 44 100 Hz **stéréo**, au débit source arrondi au multiple de 16 supérieur et borné à [64, 192] kbps (192 si le débit est inconnu). Métadonnées, chapitres et flux vidéo (pochettes intégrées) retirés.                                              |
| D7  | **Ordre** : celui que le parent donne dans l'arborescence. Liste de sélection par défaut : date décroissante, bandes-annonces écartées (P5).                                                                                                                                                                                                                                       |
| D8  | **Épisodes explicites** (`itunes:explicit`) : signalés dans la liste et décochés par défaut.                                                                                                                                                                                                                                                                                       |
| D9  | **Publicité** : signalée dans la mention d'usage et dans le README (rien n'est filtré).                                                                                                                                                                                                                                                                                            |
| D10 | **Réseau** : refus des adresses privées et locales **partout** (flux, enclosures, images : bouclage, privées, lien-local, partagées 100.64/10, multidiffusion, non spécifiées et équivalents IPv6), contrôlé **après résolution DNS** et **à chaque redirection**, connexion à l'adresse vérifiée ; TLS vérifié ; nombre de redirections borné ; aucune redirection par page HTML ; noms à une étiquette et suffixes `.local`, `.lan`, `.home.arpa`, `.internal`, `.localhost`, `.test`, `.invalid` (entre autres) refusés, identifiants `user:pass@` refusés ; **mandataires ignorés** (`no_proxy`, décision du 2026-10-23 : un mandataire résoudrait le nom hors contrôle) ; **résidu** : l'adresse IP publique de la box de l'utilisateur, saisie littéralement ou obtenue par un nom public multi-étiquettes qui la résout, reste joignable (rebouclage NAT). |
| D11 | Décidé le 2026-10-23 : **URL de flux affichée en clair dans l'interface** ; jamais journalisée ni recopiée dans les erreurs ou le diagnostic. Les identifiants dans l'URL (`user:pass@`) restent refusés (#211).                                                                                                                                                                   |
| D13 | **Image de repli** : chaîne de P6, puis couleur unie `#122e2c`. Une image manquante ou illisible ne retire **pas** l'épisode.                                                                                                                                                                                                                                                      |
| D14 | **Message de droits** à la première installation, fermé par « J'ai compris ». Texte proposé au § 4, à valider.                                                                                                                                                                                                                                                                     |
| D15 | **Suppression d'une source** : les histoires créées sont gardées (P4) ; le cache de la source (flux, fichiers téléchargés) est purgé.                                                                                                                                                                                                                                              |
| D16 | **Métadonnées modifiables** par le parent, avec ces défauts : titre = titre du flux ; sous-titre = copyright du flux ; âge = 0 ; description = liste des épisodes (et des échecs, P7).                                                                                                                                                                                             |

Remarques de cohérence :

- **D6 et le mono** : un MP3 mono conforme est **copié** tel quel (mono), alors qu'un fichier
  réencodé devient stéréo. Les deux cas peuvent coexister dans un même pack ; le MP3 mono reste à
  vérifier sur l'appareil (#219).
- **D6 et `telmi2flam`** : la règle est plus stricte que celle du convertisseur, qui copie tout
  MP3 à 44 100 Hz quel que soit son débit (§ 3.1). Pas d'incohérence : le launcher livre des
  fichiers déjà conformes.
- **D16 et l'appareil** : `telmi2flam` affiche comme sous-titre le champ `category` de
  `metadata.json` (§ 3.3). Le sous-titre du pack est donc écrit dans `category`. Avec plusieurs
  flux, « titre du flux » et « copyright du flux » sont ambigus : **N1 b** (décidé le
  2026-10-23) : ces défauts ne valent que si **un seul** flux est utilisé ; sinon le titre est à
  saisir (obligatoire) et le sous-titre est vide.
- **D1 et l'arborescence** : les plafonds portent sur le pack entier, toutes catégories
  confondues ; N2 b ajoute 3 niveaux de menus et 50 options par menu au plus.

## 3. Contraintes constatées dans le code et sur l'appareil

### 3.1 Audio

- Le firmware attend du **MP3 à 44 100 Hz** (`SAMPLERATE` dans `tools/telmi2flam/telmi2flam.py`,
  #46). Le convertisseur ne lit que du MP3 : AAC, Ogg ou WAV doivent être transcodés **par le
  launcher** avant l'archive Telmi (#214).
- Filet de sécurité : un MP3 qui n'est pas à 44 100 Hz est rééchantillonné par `telmi2flam`
  avec le ffmpeg embarqué (variable `FLAM_FFMPEG` posée par le launcher, `install/convert.rs` ;
  jamais un ffmpeg du `PATH` ; sans ffmpeg embarqué, la variable est vide et le
  rééchantillonnage est désactivé). Ce rééchantillonnage garde le nombre de canaux, vise un
  débit proche de la source entre 64 et 192 kbps, et a un **délai de 120 s par fichier** : un
  long épisode laissé au convertisseur risque de le dépasser. Le launcher livre donc des MP3 déjà
  conformes (D6).
- ffmpeg embarqué (**Livré**, #252) : FFmpeg 8.1.3 minimal, audio seulement, statique, LGPL,
  construit par `tools/ffmpeg/build.sh` (versions et empreintes dans `tools/ffmpeg/pins.env`) :
  `ffmpeg.exe` et `ffprobe.exe` ; décodeurs MP3, AAC, Opus, Vorbis, FLAC, PCM ; démultiplexeurs
  MP3, MOV/MP4, Ogg, FLAC, WAV, AAC, Matroska ; encodeurs `libmp3lame` (LAME 3.100) et `pcm_s16le` ; protocole
  `file` seul. Reste à faire (#214) : module Rust de lancement (empreinte, arguments fixes,
  annulation, délai), avec `-nostdin` et `-vn` (ou `-map a`), sans quoi le muxeur MP3 réclame un
  encodeur d'image pour les pochettes intégrées.

### 3.2 Images

- Images de scène : PNG 640 × 480 dans l'archive Telmi, ramenées par `telmi2flam` à la zone
  d'affichage des scènes, **320 × 212**.
- Vignette de bibliothèque : **104 × 136**, portrait, calculée par `telmi2flam` depuis la
  couverture, avec un **remplissage paramétrable** (réglage `ThumbFit`, option `--thumb-fit`) :
  `flou` (défaut), `rogner`, `bandes`. Une pochette carrée de 512 × 512 perd environ 24 % de sa
  largeur en `rogner` ; en `bandes`, elle devient 104 × 104 avec deux bandes de 16 px.
- Carte de titre : la couverture est placée dans un emplacement d'environ 89 × 120.
- Pochettes (**Livré**, #215, `src-tauri/src/podcast/artwork.rs`) : JPEG (dont progressif), PNG
  et WebP reconnus au contenu ; garde-fous de 20 Mio par fichier, 8 000 px par côté, 256 Mio
  décodés ; transparence aplatie sur fond blanc ; image de repli unie, couleur en paramètre
  (`FALLBACK_COLOR` = `#122e2c`, valeur retenue par D13). Sorties : 640 × 480 (épisode) et
  512 × 512 (couverture).

### 3.3 Graphe, textes et moteur

- `telmi2flam` garde les **identifiants de scène Telmi tels quels** dans `script/nodes.lua`.
- Libellés de choix : 40 caractères au plus (`LABEL_MAX`), tirés de `notes.json` (titre, sinon
  texte, sinon notes ; un titre égal à l'identifiant du nœud est ignoré) ; caractères hors police
  remplacés, avec un avertissement.
- Sélecteur par défaut : `carousel` (trois vignettes et un libellé).
- Titre et sous-titre sur l'appareil (`info.plain`) : `title` et `category` de `metadata.json`
  (sous-titre = titre si `category` est vide).
- Reprise sur l'appareil (`tools/telmi2flam/engine/story.lua`) : la sauvegarde retient le dernier
  nœud joué ; s'il n'existe plus, l'histoire redémarre au début.
- Mémoire : `telmi2flam` assemble **tout le paquet en mémoire** avant d'écrire le `.plain.pk`.
  La taille d'un pack se paie aussi en mémoire vive pendant la conversion ; le plafond de 1 Gio
  (D1) la borne aussi.

### 3.4 Identité sur l'appareil

- Le dossier de l'histoire sur la Flam (`str/<uuid>/`) est nommé d'après `uuid.bin`, que
  `telmi2flam` calcule comme l'**empreinte MD5 de la chaîne `uuid` de `metadata.json`**. Un uuid
  aléatoire par pack (P3) donne donc un dossier distinct par pack.
- Le fichier `version` du paquet flam vaut toujours `1`. Dans la bibliothèque du launcher, un
  pack a une version fixe : `is_outdated_by` (`library.rs`), qui signale une mise à jour quand la
  version diffère pour un même uuid, ne se déclenche donc jamais pour un pack.
- Les sauvegardes de `usr/0/` ne sont jamais touchées par le launcher ; un nouveau pack a une
  progression vierge.
- Avant un transfert, l'espace libre est vérifié (chaque fichier arrondi à 32 Kio, plus un cluster par dossier,
  plus 32 Mio de marge, `docs/flam-device-format.md` à la racine du dépôt, § 8.1). La capacité de la Flam n'est
  pas consignée dans le dépôt : à relever en #219.
- Appareil de test du mainteneur : **Flam en firmware 1.x**, dont le transfert passe par le mode
  « carrier » (`docs/flam-carrier.md`). Aucun résultat consigné pour le firmware 2.x.

### 3.5 Lecture du flux, sources, réseau

- Lecture du flux (**Livré**, #210, `src-tauri/src/podcast/rss.rs`) : 16 Mio au plus, DTD
  refusée, 5 000 épisodes au plus, encodages UTF-8, UTF-16, ISO-8859-1 et Windows-1252 ;
  bandes-annonces et épisodes sans enclosure audio écartés (P5) ; doublons de `guid` ignorés ;
  chaîne de repli des images conforme à P6 ; champs lus : titre, description (HTML retiré),
  date, durée, auteur, image, URL, type et taille annoncée de l'enclosure ; copyright du canal.
  `itunes:explicit` (D8) est lu depuis #361 : canal et épisode, l'épisode héritant du canal.
- `strip_html` (`rss.rs`) extrait du texte brut (balises retirées, entités décodées) ; ce n'est
  pas un assainisseur HTML : descriptions et titres sont toujours affichés comme texte, jamais
  injectés comme HTML.
- Sources : `Catalog`, `Folder`, `Podcast` (**Livré**, #211, `src-tauri/src/podcast/source.rs` ;
  schéma v2 de `sources.json`, cache `<cache>/podcasts/<id>/`) et la bibliothèque. La commande
  `sources_remove` modifie `sources.json`, arrête la surveillance et purge le cache d'une source
  catalogue ou podcast (`purge_catalog_cache`, `purge_podcast_cache` : D15) ; les histoires
  installées ne sont pas touchées (conforme à P4).
- Client de téléchargement (`install/download.rs`, comme celui du catalogue) : TLS par rustls
  (certificats vérifiés), **5 redirections au plus**, jamais de https vers http ; seules les
  redirections HTTP sont suivies (pas de page HTML « refresh ») ; reprise par plages ; 4 Gio au
  plus par fichier. **Aucun filtrage des adresses privées** aujourd'hui : c'est le reste à faire
  de D10 (#213).
- Pipeline d'installation : `install_start(source_id, entry_id, uuid?)` installe une entrée
  **d'une source**. Un pack n'est l'entrée d'aucune source : son point d'entrée est à créer
  (#217, **N7 a** : nouvelle commande « créer un pack », sans pseudo-source).

## 4. Parcours utilisateur

1. **Ajouter un flux** (Sources) : URL http(s), longueur bornée, sans identifiants (#211).
   Erreurs distinctes : flux invalide, aucun épisode audio, flux trop grand, réseau, adresse
   refusée.
2. **Choisir des épisodes** : liste par flux, triée par date décroissante ; épisodes explicites
   signalés et décochés (D8) ; épisodes de plusieurs flux possibles (un épisode une seule fois
   dans le pack, N9 a).
3. **Composer le pack** : arborescence libre (catégories, sous-menus, glisser-déposer, 3 niveaux
   de menus et 50 options par menu au plus, catégorie vide refusée : N2 b) ; métadonnées
   modifiables (D16, N1 b) ; couverture (N6 b) ; estimation de taille et plafonds affichés,
   création impossible au-delà (D1, D5). La composition est enregistrée automatiquement dans un
   **brouillon unique**, repris à l'ouverture, effacé après la création ; les épisodes devenus
   indisponibles sont signalés (N3 b).
4. **Créer le pack** : téléchargement, conversion audio, pochettes, archive, conversion,
   bibliothèque ; progression et annulation ; récapitulatif des épisodes en échec (P7). Après la
   publication, tous les fichiers de travail sont supprimés (N8 a). Si l'application s'arrête
   pendant la création, celle-ci **reprend** au redémarrage (N10 a).
5. **Lecture** : carte de titre, menu (carrousel), sous-menus ; un épisode enchaîne sur le
   suivant **du même menu**, la fin du dernier épisode d'un menu retourne à **ce menu** (P8,
   N4 a).
6. **Transfert** sur la Flam : comme toute histoire de la bibliothèque.

Textes en français, dans le fichier de textes de l'interface ; parcours complet au clavier.

**Mention d'usage** (A8, D9, D14 ; texte proposé, à valider) :

> Les podcasts appartiennent à leurs auteurs. Les histoires créées ici sont réservées à un usage
> personnel, dans le cadre familial : ne les partagez pas et ne les redistribuez pas. L'audio est
> téléchargé tel que l'éditeur le diffuse, publicité comprise.

## 5. Architecture A : préparation puis conversion

```text
URL du flux ─► téléchargement du flux (cache hors ligne du dernier flux valide, #211)
            ─► lecture RSS (#210, livré)
            ─► sélection (plusieurs flux) et arborescence (#212, éditeur à créer)
            ─► « Créer le pack » → préparation (#217) :
                 téléchargements bornés et annulables (#213)
                 audio : copie ou MP3 44 100 Hz stéréo via ffmpeg (#214)
                 pochettes : PNG 640 × 480 et 512 × 512 (#215, livré)
                 archive Telmi synthétique dans le cache (#216)
            ─► pipeline existant : telmi2flam → .plain.pk → library.json (#217)
            ─► transfert sur la Flam (pipeline existant)
```

- La préparation est une nouvelle étape de la file d'installation persistée (états préparation
  en cours, préparé, erreur), avec la même discipline que l'existant : dossier de travail par
  tâche, nettoyage à l'annulation et à l'échec, reprise ou nettoyage après un arrêt brutal. Le
  code qui touche à la bibliothèque et au journal d'installation suit `CONTRIBUTING.md` § 5
  (note de conception, test d'injection de panne, `modele:opus`).
- Reprise d'une création interrompue (arrêt de l'application ou de la machine) : **N10 a**. La
  composition (arborescence, épisodes, métadonnées) et l'uuid sont enregistrés avec la tâche ;
  l'uuid est tiré **une seule fois**, à la création de la tâche ; la reprise reconstruit le même
  pack et **revalide** les épisodes (flux changé entre-temps).
- Point d'entrée : commande « créer un pack » (**N7 a**), qui reçoit la composition, la
  revalide entièrement dans le backend (épisodes retrouvés dans les flux en cache) et crée une
  tâche de la file existante.
- Tout accès réseau passe par le téléchargeur existant (D10) ; `rss` et `artwork` restent purs.
- L'archive Telmi peut être stockée sans compression (MP3 déjà compressés).
- **Limite connue** : l'audio existe en plusieurs copies pendant la création (originaux, MP3
  convertis, archive Telmi, `.plain.pk`) ; la génération directe d'un paquet flam (option B) est
  une évolution (#221).
- **Après la publication** : tout est supprimé (originaux, MP3 convertis, archive Telmi) ; refaire
  un pack retélécharge tout (N8 a). Le cache des MP3 convertis est une évolution (N8 b, #221).

## 6. Histoire générée (archive Telmi synthétique)

Le format Telmi est pris de sa description publique (wiki Telmi), jamais du code d'un autre
projet.

**Fichiers** : `metadata.json`, `nodes.json`, `notes.json`, couverture (`cover.png` ou
`title.png`, référencée par `metadata.image`), `images/`, `audios/`. Pas de `title.mp3`.

**Métadonnées** :

- `uuid` : aléatoire (proposé : UUID v4, écrit en minuscules avec tirets, forme unique puisque
  `telmi2flam` hache la chaîne telle quelle, § 3.4) ;
- `version` : fixe (1) ;
- `title`, `category` (= sous-titre), `age`, `description` : valeurs de D16, saisies ou par
  défaut.

**Graphe** :

- une scène d'entrée, puis le menu racine ;
- chaque menu de l'arborescence est un choix affiché en carrousel ; une option mène soit à un
  sous-menu (catégorie), soit à un épisode ;
- un épisode est une scène de lecture ; à sa fin, l'épisode suivant se joue ; la fin du dernier
  retourne au menu (P8) ; « suivant » s'entend **dans le même menu**, et la fin du dernier
  épisode d'un menu retourne à **ce menu** (N4 a) ; le bouton retour depuis un sous-menu ramène
  au menu parent (à vérifier sur l'appareil, #219) ;
- libellés : titres d'épisodes et noms de catégories (40 caractères, translittérés) ;
- images : P6 puis D13 pour les épisodes ; image d'une catégorie = image du **premier épisode**
  de la catégorie (N5 a) ; couverture du pack = image du podcast du premier flux utilisé par
  défaut, modifiable parmi les images des flux et épisodes du pack (N6 b) ;
- profondeur : 3 niveaux de menus au plus (racine + 2), 50 options par menu, catégorie vide
  refusée (N2 b).

**Taille du graphe** : menus de 1, 10 et 50 épisodes testés (#216) ; limite pratique sur
l'appareil mesurée en #219 (D1, N2 à revoir après #219).

## 7. Licences et provenance

- **TelmiSync est sous GPL-3.0.** Il a été étudié en **salle blanche** : son **comportement**
  inspire le modèle pack (§ 2.2), décrit avec des mots propres, sans extrait de code, sans nom de
  fonction ou de variable, sans texte d'interface. **Aucun code, texte, ressource (images, voix,
  listes, polices) ni structure littérale n'en est repris.** Toute implémentation part du format
  Telmi documenté publiquement et des règles de flam-launcher (`AGENT.md`, « Surcouche
  flam-launcher »).
- **ffmpeg** : build LGPL sans options GPL, SHA-256 vérifié, `LICENSES.md` à jour, sources ou
  offre de sources (`ffmpeg/BUILD.txt`).
- **Nouvelles crates** : licence compatible (MIT, Apache-2.0 ou MPL-2.0), ajoutées à
  `LICENSES.md`.
- **Synthèse vocale (évolution)** : piper dépend d'espeak-ng (GPL-3.0) ; pas d'embarquement sans
  analyse de licence dédiée.
- **Contenus** : aucun flux réel, aucune archive Telmi, aucune histoire convertie dans le dépôt ;
  essais réels hors dépôt (`.cache/`).

## 8. Abandonné

Décision du 2026-10-19 : le modèle « 1 podcast = 1 histoire + Mettre à jour » est abandonné. Sont
sans objet :

- **1 podcast = 1 histoire** et **uuid dérivé de l'URL du flux** : remplacés par P2 et P3 ;
- **mise à jour par version et bouton « Mettre à jour »** : un pack ne se met jamais à jour ;
- **D2** (épisodes retenus à la mise à jour, sort des anciens) ;
- **D3** (réutilisation des épisodes préparés entre deux mises à jour) ;
- **D4** (progression de lecture après mise à jour) ;
- **D12** (flux qui change d'adresse : plus aucune identité n'est liée au flux) ;
- **lien source → installation** (uuid de l'histoire = uuid de la source, #211) ;
- **une installation d'un podcast déjà en cours est refusée ou mise en file** (#217) : chaque
  pack est une histoire distincte ;
- **mise à jour automatique et fenêtre glissante** (#221) ;
- le graphe « fin d'épisode → retour au menu » : remplacé par l'enchaînement de P8 ;
- le choix « N derniers épisodes, 10 par défaut » de #212 n'est plus une règle du produit (la
  sélection est libre) ; il peut rester comme raccourci de sélection (voir § 11, #212).

## 9. Décisions du 2026-10-23 (N1 à N10)

Questions soulevées par le modèle pack. Le 2026-10-23, le mainteneur a **adopté toutes les
recommandations**. Chaque question garde ses options, leurs conséquences et la recommandation
pour l'historique ; la décision est en tête de chaque sous-section.

| #   | Sujet                                   | Décision (2026-10-23)                                       |
| --- | --------------------------------------- | ----------------------------------------------------------- |
| N1  | Métadonnées par défaut d'un multi-flux  | b : défauts seulement si un seul flux, sinon titre à saisir |
| N2  | Profondeur et largeur de l'arborescence | b : 3 niveaux de menus, 50 options par menu                 |
| N3  | Brouillon de pack                       | b : brouillon unique sauvegardé automatiquement             |
| N4  | Enchaînement « épisode suivant »        | a : suivant dans le même menu, retour à ce menu             |
| N5  | Image d'une catégorie                   | a : image du premier épisode de la catégorie                |
| N6  | Couverture du pack                      | b : premier flux par défaut, modifiable                     |
| N7  | Point d'entrée dans le pipeline         | a : nouvelle commande « créer un pack »                     |
| N8  | Fichiers après la création              | a : tout supprimer                                          |
| N9  | Même épisode plusieurs fois             | a : interdit                                                |
| N10 | Reprise d'une création interrompue      | a : reprendre (composition et uuid enregistrés)             |

### N1. Métadonnées par défaut d'un pack multi-flux — Décidé le 2026-10-23 : b

D16 donne les défauts « titre du flux » et « copyright du flux », non définis quand le pack
mélange plusieurs flux.

| Option                                                                                                            | Conséquences                                                                |
| ----------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| a. Défauts pris du **premier flux** utilisé (dans l'ordre de l'arborescence)                                      | Toujours prérempli ; peut être trompeur si le premier flux est minoritaire. |
| b. Défauts pris seulement si un **seul** flux est utilisé ; sinon titre à saisir (obligatoire) et sous-titre vide | Jamais trompeur ; un champ de plus à remplir dans le cas multi-flux.        |
| c. Titre « Ma sélection de podcasts », sous-titre = copyrights concaténés                                         | Prérempli ; sous-titre long, tronqué sur l'appareil.                        |

**Recommandation : b** (adoptée le 2026-10-23). La description par défaut (liste des épisodes) reste valable dans tous
les cas ; elle peut regrouper les épisodes par flux.

### N2. Profondeur et largeur de l'arborescence — Décidé le 2026-10-23 : b

Aucune limite du moteur n'est connue, mais chaque niveau est un écran de carrousel de plus pour
un enfant, et D1 (50 épisodes) borne déjà le total.

| Option                                                              | Conséquences                                                                 |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| a. Aucune limite propre (seulement D1)                              | Simple ; des arbres profonds peu utilisables par un enfant.                  |
| b. **3 niveaux** de menus au plus (racine + 2), 50 options par menu | Couvre « catégories → sous-catégories → épisodes » ; facile à tester (#219). |
| c. 2 niveaux au plus (racine + 1)                                   | Le plus simple pour l'enfant et pour l'éditeur ; moins de liberté.           |

**Recommandation : b** (adoptée le 2026-10-23), à revoir après #219 ; catégorie vide refusée à la création.

### N3. Brouillon de pack — Décidé le 2026-10-23 : b

| Option                                                                                                           | Conséquences                                                                                         |
| ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| a. Aucun : la composition vit en mémoire jusqu'à « Créer le pack »                                               | Simple ; tout est perdu si l'on ferme la fenêtre.                                                    |
| b. **Un brouillon unique** sauvegardé automatiquement (fichier de données du launcher), effacé après la création | Pas de perte ; un fichier de plus à migrer ; épisodes devenus indisponibles à signaler à la reprise. |
| c. Compositions nommées, conservées après création (pour refaire ou dériver un pack)                             | Permet de « refaire un pack » sans tout ressaisir ; plus d'écrans (liste, renommage, suppression).   |

**Recommandation : b** (adoptée le 2026-10-23) au MVP ; c en évolution (#221).

### N4. Périmètre de l'enchaînement « épisode suivant » — Décidé le 2026-10-23 : a

P8 dit « à la fin d'un épisode, le suivant ; la fin du dernier retourne au menu ». Avec des
sous-menus, « suivant » et « le menu » sont à préciser.

| Option                                                                                             | Conséquences                                                                       |
| -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| a. **Suivant dans le même menu** ; la fin du dernier épisode de ce menu retourne à **ce menu**     | Une catégorie s'écoute d'un bout à l'autre ; comportement prévisible.              |
| b. Suivant dans l'ordre de l'arborescence aplatie ; la fin du tout dernier retourne au menu racine | Écoute continue de tout le pack ; on passe d'une catégorie à l'autre sans le voir. |
| c. Enchaînement au menu racine seulement, pas dans les sous-menus                                  | Contredit P8 dans les sous-menus.                                                  |

**Recommandation : a** (adoptée le 2026-10-23). Le bouton retour depuis un sous-menu ramène au menu parent (à vérifier
sur l'appareil, #219).

### N5. Image d'une catégorie (sous-menu) dans le carrousel — Décidé le 2026-10-23 : a

| Option                                                                                 | Conséquences                                                      |
| -------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| a. Image du premier épisode de la catégorie                                            | Automatique ; peut ressembler à un épisode et prêter à confusion. |
| b. Image unie de repli (`#122e2c`) avec le nom en libellé                              | Distingue les catégories ; visuellement pauvre.                   |
| c. Image choisie par le parent parmi celles des épisodes de la catégorie, a par défaut | Le plus clair ; un réglage de plus dans l'éditeur.                |

**Recommandation : a** (adoptée le 2026-10-23) au MVP, c en évolution.

### N6. Couverture du pack (et vignette 104 × 136) — Décidé le 2026-10-23 : b

| Option                                                                            | Conséquences                                        |
| --------------------------------------------------------------------------------- | --------------------------------------------------- |
| a. Image du podcast (P6) du premier flux utilisé                                  | Automatique ; un seul flux représenté.              |
| b. a par défaut, modifiable : choix parmi les images des flux et épisodes du pack | Le parent reconnaît son pack ; un réglage de plus.  |
| c. Image unie de repli                                                            | Tous les packs se ressemblent dans la bibliothèque. |

**Recommandation : b** (adoptée le 2026-10-23) (a si le temps manque) ; le remplissage de la vignette suit le réglage
`ThumbFit` existant.

### N7. Point d'entrée de la création dans le pipeline — Décidé le 2026-10-23 : a

`install_start` installe une entrée d'une source ; un pack n'en est pas une (§ 3.5).

| Option                                                                                                                            | Conséquences                                                                                                    |
| --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| a. Nouvelle commande « créer un pack », qui reçoit la composition (validée par le backend) et crée une tâche de la file existante | Réutilise file, conversion, publication ; le backend revalide tout (épisodes retrouvés dans les flux en cache). |
| b. Pseudo-source « Packs » dont chaque composition serait une entrée                                                              | Réutilise `install_start` tel quel ; mélange une composition locale et une source distante.                     |

**Recommandation : a** (adoptée le 2026-10-23). Le front ne transmet que des identifiants d'épisodes (source + `guid`, ou URL d'enclosure à défaut, à traiter
comme un secret, D11),
comme il ne transmet aujourd'hui ni URL ni chemin.

### N8. Fichiers téléchargés après la création — Décidé le 2026-10-23 : a

Sans mise à jour, rien n'oblige à garder les épisodes téléchargés.

| Option                                                                      | Conséquences                                                                  |
| --------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| a. **Tout supprimer** après publication (originaux, MP3 convertis, archive) | Disque libéré ; refaire un pack retélécharge tout.                            |
| b. Garder les MP3 convertis par source, purgés avec la source (D15)         | Refaire un pack est rapide ; coûte jusqu'à la taille des packs sur le disque. |

**Recommandation : a** (adoptée le 2026-10-23) au MVP ; b plus tard si le besoin apparaît.

### N9. Même épisode plusieurs fois — Décidé le 2026-10-23 : a

| Option                                                        | Conséquences                                                         |
| ------------------------------------------------------------- | -------------------------------------------------------------------- |
| a. Interdit : un épisode apparaît une seule fois dans le pack | Simple ; pas de double comptage dans D1.                             |
| b. Autorisé dans plusieurs catégories, audio stocké une fois  | Souplesse ; libellés et enchaînement à gérer pour chaque occurrence. |

**Recommandation : a** (adoptée le 2026-10-23).

### N10. Reprise d'une création interrompue — Décidé le 2026-10-23 : a

#217 et A7 demandent qu'un arrêt brutal pendant la préparation « reprenne ou nettoie
proprement », sans choisir.

| Option                                                                                                                                   | Conséquences                                                                                                                   |
| ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| a. Reprendre : composition (arborescence, épisodes, métadonnées) et uuid enregistrés avec la tâche ; la reprise reconstruit le même pack | Rien n'est perdu ; un état de plus à persister et à migrer ; la reprise doit revalider les épisodes (flux changé entre-temps). |
| b. Nettoyer et marquer la tâche « échec, à relancer »                                                                                    | Simple ; le parent recompose ou relance (avec N3 b, le brouillon peut servir à relancer).                                      |

**Recommandation : a** (adoptée le 2026-10-23), l'uuid étant tiré une seule fois, à la création de la tâche.

### Points restant à décider

Aucune décision n'est inventée ici : ces points restent ouverts ou à valider.

- **D14 et mention d'usage (§ 4)** : textes proposés, **à valider** par le mainteneur (#212,
  #220).
- **#212, raccourci « N derniers épisodes »** : à retirer ou à garder comme simple raccourci de
  sélection, **au choix du mainteneur**.
- **N2 b** : limites (3 niveaux, 50 options) **à revoir après le test sur appareil** (#219).
- **N4 a** : bouton retour depuis un sous-menu **à vérifier sur l'appareil** (#219).
- **D6 et le MP3 mono** copié tel quel : **à vérifier sur l'appareil** (#219).
- **Capacité de la Flam** : à relever en #219.

## 10. Ordres de grandeur

Débits en kbit/s ; 1 Mo = 10⁶ octets ; audio seul, CBR.

| Débit    | Par minute | Par heure | 15 min  | 30 min  | 1 h     |
| -------- | ---------- | --------- | ------- | ------- | ------- |
| 64 kbps  | 0,48 Mo    | 28,8 Mo   | 7,2 Mo  | 14,4 Mo | 28,8 Mo |
| 96 kbps  | 0,72 Mo    | 43,2 Mo   | 10,8 Mo | 21,6 Mo | 43,2 Mo |
| 128 kbps | 0,96 Mo    | 57,6 Mo   | 14,4 Mo | 28,8 Mo | 57,6 Mo |
| 192 kbps | 1,44 Mo    | 86,4 Mo   | 21,6 Mo | 43,2 Mo | 86,4 Mo |

- Règle D6 : un fichier AAC à 96 kbps est réencodé à 96 kbps ; à 100 kbps, à 112 kbps ; à débit
  inconnu, à 192 kbps (86,4 Mo par heure). Le débit inconnu est donc le cas le plus coûteux pour
  l'estimation D5.
- Plafond D1 : 200 Mio par épisode ≈ 2 h 19 à 192 kbps, 3 h 28 à 128 kbps ; 1 Gio (1,074 Go)
  ≈ 12,4 h à 192 kbps, 18,6 h à 128 kbps, 37 h à 64 kbps.
- 10 épisodes de 30 min : 144 Mo à 64 kbps, 288 Mo à 128 kbps, 432 Mo à 192 kbps.
- Disque du PC pendant la création : originaux + MP3 convertis + archive Telmi + `.plain.pk`,
  jusqu'à environ quatre fois le volume téléchargé si rien n'est supprimé en cours de route
  (les originaux peuvent être plus gros que les MP3 convertis).

## 11. Impact sur les issues

Changements **appliqués sur GitHub le 2026-10-23**, après adoption de N1 à N10.

| Issue | Changement appliqué                                                                                                                                                                                                 |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #207  | Décisions et périmètre MVP remplacés par le modèle pack ; liste, dépendances, ordre et parallélisme mis à jour.                                                                                                     |
| #208  | Fermée par la fusion de la PR de cette spécification : D1 à D16 et N1 à N10 sont décidés ; restent les points « à valider » ci-dessus.                                                                              |
| #209  | Inchangée (reste : cache CI, `BUILD.txt` ; module de lancement en #214).                                                                                                                                            |
| #210  | Fermée. Suivi : #361 (`itunes:explicit`, en-tête de `rss.rs`).                                                                                                                                                      |
| #211  | Source « Podcast » (flux RSS, persistance, cache) : sans identifiant stable ni lien source vers installation ; URL traitée comme un secret (D11) ; suppression sans effet sur les histoires, cache purgé (P4, D15). |
| #212  | Sélection des épisodes : liste par flux, plusieurs flux, explicites signalés et décochés (D8), estimation et plafonds par pack (D1, D5), mention d'usage ; dépend de #361.                                          |
| #213  | D10 complet (adresses privées contrôlées après DNS et à chaque redirection), plusieurs flux par tâche, suppression des fichiers après création (N8 a).                                                              |
| #214  | Règle D6 et critères associés ; mono accepté si MP3 conforme.                                                                                                                                                       |
| #215  | Fermée (`FALLBACK_COLOR` = `#122e2c`, D13).                                                                                                                                                                         |
| #216  | Réécrite : uuid aléatoire, version 1, graphe arborescent, P8 et N4 a, catégories (N5 a), couverture (N6 b), métadonnées D16 et N1 b.                                                                                |
| #217  | Commande « créer un pack » (N7 a), reprise de la création (N10 a), plusieurs flux par tâche ; dépend de #362.                                                                                                       |
| #218  | Fermée, abandonnée (pas de mise à jour).                                                                                                                                                                            |
| #219  | Enchaînement, sous-menus, retour, mono et stéréo mélangés, packs multi-flux, transfert carrier firmware 1.x, capacité de la Flam.                                                                                   |
| #220  | README « Créer un pack », mention d'usage avec la publicité (D9), message « J'ai compris » (D14) ; dépend de #217 et #363.                                                                                          |
| #221  | Évolutions : recherche, synthèse vocale, compositions nommées (N3 c), cache des MP3 convertis (N8 b), image de catégorie choisie (N5 c), pochettes ; dépend de #219.                                                |
| #361  | Nouvelle : lecture de `itunes:explicit` (D8) et correction de l'en-tête de `rss.rs`.                                                                                                                                |
| #362  | Nouvelle : modèle de composition du pack (types, validation N1, N2, N9, D1) et brouillon unique (N3 b).                                                                                                             |
| #363  | Nouvelle : éditeur de pack (arborescence, métadonnées, couverture).                                                                                                                                                 |

### Ordre et parallélisme conseillés

Le label `plus-tard` est retiré des issues du MVP ; il reste sur #221.

1. **Vague 1, en parallèle** (fichiers disjoints) : #361 (`rss.rs`), #213 (`install/download.rs`),
   #214 (nouveau module audio), #216 (nouveau module d'archive), #211 (`sources.rs`).
2. **Vague 2** : #362 (après #211) ; #212 (après #211 et #361, interface) ; #217 (après #211,
   #213, #214, #216 et #362).
3. **Vague 3** : #363 (après #212 et #362, interface, en série avec #212 car mêmes fichiers) ;
   #219 (après #217).
4. **Fin** : #220 (après #217 et #363), puis #221.

Pistes sans fichier commun : Rust (#361, #213, #214, #216, #211, #362, #217) et interface (#212,
#363). Brancher le contrôle D10 sur le téléchargement du flux (#211) dans celle des deux issues,
#211 ou #213, qui est fusionnée en second.
