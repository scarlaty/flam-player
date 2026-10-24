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
12. [Synthèse vocale des titres (#462)](#12-synthèse-vocale-des-titres-462)
13. [Cache des MP3 convertis (N8 b, #460)](#13-cache-des-mp3-convertis-n8-b-460)
14. [Compositions nommées réutilisables (N3 c, #461)](#14-compositions-nommées-réutilisables-n3-c-461)

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
vocale des titres, titre incrusté ou pochette générée, compositions nommées réutilisables (N3 c,
livré par #461, § 14), cache des MP3 convertis (N8 b, livré par #460, § 13), image de catégorie choisie par le parent (N5 c), normalisation du
volume, chapitres, reprise par épisode, génération directe d'un paquet flam (option B).

## 2. Décisions prises

### 2.1 Décisions de #207 toujours valables

| #   | Décision                                                                                                                                                                                                    | Issues     |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| A1  | **Architecture A** : le launcher écrit une **archive Telmi synthétique** dans son cache, puis l'envoie au pipeline existant (`telmi2flam`). Aucune modification du format flam ni du moteur Lua.            | #216, #217 |
| A2  | **ffmpeg LGPL embarqué**, lancé comme **processus externe** (jamais lié), sans option GPL ni nonfree, empreinte SHA-256 vérifiée, licence et sources dans `LICENSES.md`. Jamais de build « full » GPL.      | #209, #214 |
| A3  | **Menu sans voix au MVP** : les libellés sont les titres (épisodes, catégories), affichés en texte. Synthèse vocale : évolution (#221, #462, § 12).                                                         | #216, #221 |
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

| #   | Décision (2026-10-19)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | **Plafonds fixes par pack** : 50 épisodes, 200 Mio par épisode, 1 Gio au total. Appliqués dans l'interface et dans le backend ; création impossible au-delà.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| D5  | **Estimation** : taille annoncée de l'enclosure, sinon durée × débit, affichée « environ » ; espace disque nécessaire sur le PC affiché aussi (jusqu'à quatre fois la taille du pack, § 10, plus une marge). Les plafonds restent vérifiés pendant le téléchargement.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| D6  | **Audio** : **copie** si le fichier est un MP3 à 44 100 Hz, mono ou stéréo, entre 64 et 192 kbps ; sinon **réencodage** en MP3 44 100 Hz **stéréo**, au débit source arrondi au multiple de 16 supérieur et borné à [64, 192] kbps (192 si le débit est inconnu). Métadonnées, chapitres et flux vidéo (pochettes intégrées) retirés.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| D7  | **Ordre** : celui que le parent donne dans l'arborescence. Liste de sélection par défaut : date décroissante, bandes-annonces écartées (P5).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| D8  | **Épisodes explicites** (`itunes:explicit`) : signalés dans la liste et décochés par défaut.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| D9  | **Publicité** : signalée dans la mention d'usage et dans le README (rien n'est filtré).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| D10 | **Réseau** : refus des adresses privées et locales **partout** (flux, enclosures, images : bouclage, privées, lien-local, partagées 100.64/10, multidiffusion, non spécifiées et équivalents IPv6), contrôlé **après résolution DNS** et **à chaque redirection**, connexion à l'adresse vérifiée ; TLS vérifié ; nombre de redirections borné ; aucune redirection par page HTML ; noms à une étiquette et suffixes `.local`, `.lan`, `.home.arpa`, `.internal`, `.localhost`, `.test`, `.invalid` (entre autres) refusés, identifiants `user:pass@` refusés ; **mandataires ignorés** (`no_proxy`, décision du 2026-10-23 : un mandataire résoudrait le nom hors contrôle) ; **résidu** : l'adresse IP publique de la box de l'utilisateur, saisie littéralement ou obtenue par un nom public multi-étiquettes qui la résout, reste joignable (rebouclage NAT). |
| D11 | Décidé le 2026-10-23 : **URL de flux affichée en clair dans l'interface** ; jamais journalisée ni recopiée dans les erreurs ou le diagnostic. Les identifiants dans l'URL (`user:pass@`) restent refusés (#211).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| D13 | **Image de repli** : chaîne de P6, puis couleur unie `#122e2c`. Une image manquante ou illisible ne retire **pas** l'épisode.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| D14 | **Message de droits** à la première installation, fermé par « J'ai compris » ; le bouton « Annuler » (ou Échap) ferme sans créer ni mémoriser. L'acquittement est mémorisé **par poste** (`localStorage`, clé `flam-launcher.podcastPackRightsAck`) : le message ne revient pas aux créations suivantes. Texte complet du § 4, **validé le 2026-10-23** (#220).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| D15 | **Suppression d'une source** : les histoires créées sont gardées (P4) ; le cache de la source (flux, fichiers téléchargés) est purgé.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| D16 | **Métadonnées modifiables** par le parent, avec ces défauts : titre = titre du flux ; sous-titre = copyright du flux ; âge = 0 ; description = liste des épisodes (et des échecs, P7).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |

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
- Prétraitement du XML (`sanitize_xml`, `rss.rs`) : `&` et `<` isolés échappés, `<br>`, `<hr>`,
  `<img>` et `<wbr>` non fermés rendus auto-fermants, `>` ou `<` entre guillemets d'un attribut
  ignorés. Refus assumé : un `<b>` (ou toute balise non vide) jamais fermé dans du XML non
  échappé reste une erreur de flux, on ne devine pas sa fin.
- `guid` de plus de 512 caractères : tronqué avec une empreinte SHA-256, donc l'`episode_id`
  de ces épisodes diffère de celui calculé sur le `guid` brut. Un brouillon de pack qui
  référence l'ancien identifiant le voit signalé
  « épisode retiré du flux » (validation de pack).
- `podcast_feed` relit et analyse le cache à chaque appel : choix assumé, pas de cache mémoire
  (appel rare, invalidation à chaque rafraîchissement pour un gain négligeable).
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
   indisponibles sont signalés (N3 b). Tant que la tâche issue d'un brouillon est en cours, « Créer
   l'histoire » reste bloqué pour ce brouillon inchangé (même après avoir quitté l'éditeur) ; le modifier
   permet de créer un autre pack. À la création, seuls les épisodes du pack quittent la sélection.
4. **Créer le pack** : téléchargement, conversion audio, pochettes, archive, conversion,
   bibliothèque ; progression et annulation ; récapitulatif des épisodes en échec (P7). Après la
   publication, tous les fichiers de travail sont supprimés (N8 a). Si l'application s'arrête
   pendant la création, celle-ci **reprend** au redémarrage (N10 a).
5. **Lecture** : carte de titre, menu (carrousel), sous-menus ; un épisode enchaîne sur le
   suivant **du même menu**, la fin du dernier épisode d'un menu retourne à **ce menu** (P8,
   N4 a).
6. **Transfert** sur la Flam : comme toute histoire de la bibliothèque.

Textes en français, dans le fichier de textes de l'interface ; parcours complet au clavier.

**Mentions d'usage** (A8, D9, D14 ; décidées par le mainteneur le 2026-10-23, #220) :

- **Écran d'ajout d'un flux** : version courte, affichée en permanence.

  > Usage personnel et familial uniquement : ne partagez pas les histoires créées. L'audio est
  > repris tel quel, publicité comprise.

- **Première installation d'un pack** (D14) : texte complet ci-dessous, fermé par le bouton
  « J'ai compris » ; « Annuler » ferme sans créer ; acquittement mémorisé par poste
  (`localStorage`).

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
  un pack retélécharge tout (N8 a). Depuis #460, les MP3 convertis sont gardés en cache par
  source (N8 b, § 13) : refaire un pack ne retélécharge que ce qui manque.

## 6. Histoire générée (archive Telmi synthétique)

Le format Telmi est pris de sa description publique (wiki Telmi), jamais du code d'un autre
projet.

**Fichiers** : `metadata.json`, `nodes.json`, `notes.json`, couverture (`cover.png` ou
`title.png`, référencée par `metadata.image`), `images/`, `audios/`. Pas de `title.mp3`, sauf synthèse vocale cochée (§ 12).

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
- **Format Telmi** : décrit à partir du wiki public du format ; aucune ressource d'un autre
  projet n'est reprise.
- **Nouvelles crates** (`quick-xml`, `image` et ses décodeurs, `reqwest`, `sha2`, `base64`,
  etc.) : licences permissives (MIT, Apache-2.0, BSD, Zlib, Unlicense), listées automatiquement
  dans `THIRD-PARTY-NOTICES.txt` par `npm run build:notices` (liste d'autorisation dans
  `scripts/notices-lib.mjs`) ; `LICENSES.md` couvre ffmpeg et LAME.
- **Synthèse vocale (évolution)** : piper dépend d'espeak-ng (GPL-3.0) ; pas d'embarquement sans
  analyse de licence dédiée. Voie étudiée pour #462 : voix système Windows, aucune voix
  embarquée (§ 12).
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
  sélection est libre) ; il est **gardé comme simple raccourci de sélection** : 10 par défaut, dans la limite des plafonds, la sélection restant libre (décision du 2026-10-23, #220 ; voir § 11, #212).

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
**b retenue le 2026-10-24** pour les MP3 convertis seulement (#460, § 13).

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

### Décisions du mainteneur du 2026-10-23 (#220)

- **Mention d'usage de l'écran d'ajout** : version courte (§ 4).
- **Message de droits de la première installation (D14)** : texte complet du § 4, fermé par
  « J'ai compris ».
- **Raccourci « N derniers épisodes » (#212)** : **gardé**, 10 par défaut, dans la limite des
  plafonds (les épisodes refusés par les plafonds ne sont pas cochés et l'écran dit combien l'ont
  été) ; la sélection reste libre. Les épisodes explicites ne sont pas cochés d'office (D8).

### Points restant à décider

Aucune décision n'est inventée ici : ces points restent ouverts ou à vérifier.

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

| Issue | Changement appliqué                                                                                                                                                                                                                                    |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| #207  | Décisions et périmètre MVP remplacés par le modèle pack ; liste, dépendances, ordre et parallélisme mis à jour.                                                                                                                                        |
| #208  | Fermée par la fusion de la PR de cette spécification : D1 à D16 et N1 à N10 sont décidés ; restent les points ouverts ci-dessus.                                                                                                                       |
| #209  | Inchangée (reste : cache CI, `BUILD.txt` ; module de lancement en #214).                                                                                                                                                                               |
| #210  | Fermée. Suivi : #361 (`itunes:explicit`, en-tête de `rss.rs`).                                                                                                                                                                                         |
| #211  | Source « Podcast » (flux RSS, persistance, cache) : sans identifiant stable ni lien source vers installation ; URL traitée comme un secret (D11) ; suppression sans effet sur les histoires, cache purgé (P4, D15).                                    |
| #212  | Sélection des épisodes : liste par flux, plusieurs flux, explicites signalés et décochés (D8), estimation et plafonds par pack (D1, D5), mention d'usage courte ; raccourci « 10 derniers » gardé dans la limite des plafonds (#220) ; dépend de #361. |
| #213  | D10 complet (adresses privées contrôlées après DNS et à chaque redirection), plusieurs flux par tâche, suppression des fichiers après création (N8 a).                                                                                                 |
| #214  | Règle D6 et critères associés ; mono accepté si MP3 conforme.                                                                                                                                                                                          |
| #215  | Fermée (`FALLBACK_COLOR` = `#122e2c`, D13).                                                                                                                                                                                                            |
| #216  | Réécrite : uuid aléatoire, version 1, graphe arborescent, P8 et N4 a, catégories (N5 a), couverture (N6 b), métadonnées D16 et N1 b.                                                                                                                   |
| #217  | Commande « créer un pack » (N7 a), reprise de la création (N10 a), plusieurs flux par tâche ; dépend de #362.                                                                                                                                          |
| #218  | Fermée, abandonnée (pas de mise à jour).                                                                                                                                                                                                               |
| #219  | Enchaînement, sous-menus, retour, mono et stéréo mélangés, packs multi-flux, transfert carrier firmware 1.x, capacité de la Flam.                                                                                                                      |
| #220  | README « Créer un pack », mention d'usage avec la publicité (D9), message « J'ai compris » (D14) ; dépend de #217 et #363.                                                                                                                             |
| #221  | Évolutions : recherche, synthèse vocale, compositions nommées (N3 c), cache des MP3 convertis (N8 b), image de catégorie choisie (N5 c), pochettes ; dépend de #219.                                                                                   |
| #361  | Nouvelle : lecture de `itunes:explicit` (D8) et correction de l'en-tête de `rss.rs`.                                                                                                                                                                   |
| #362  | Nouvelle : modèle de composition du pack (types, validation N1, N2, N9, D1) et brouillon unique (N3 b).                                                                                                                                                |
| #363  | Nouvelle : éditeur de pack (arborescence, métadonnées, couverture).                                                                                                                                                                                    |

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

## 12. Synthèse vocale des titres (#462)

> **Statut : décidé le 2026-10-24** (partie de #207, évolution de #221 ; décisions en 12.11,
> toutes conformes aux recommandations). PR 1 (module `podcast/speech.rs`, commandes
> `tts_voices` et `tts_preview`) fusionnée ; PR 2 (intégration dans la création du pack, 12.12)
> faite ; PR 3 (interface) à faire.

### 12.1 But et état actuel

Dans un pack, le carrousel de la Flam affiche le titre de chaque option mais **ne le dit pas** :
les scènes d'option sont créées avec `"audio": null` (`podcast/archive.rs`, `Graph::menu`) et
`telmi2flam` met alors une trame silencieuse (`sounds/silent.mp3`, `package.rs`) ; l'archive
n'a pas de `title.mp3` (§ 6). Un enfant qui ne lit pas ne peut donc pas choisir un épisode. La
synthèse vocale remplit ces audios avec le titre prononcé, **en local, avec la voix système de
Windows**, sans licence tierce.

### 12.2 Ce qui est lu, et où cela s'insère

| Élément lu                                | Où dans l'histoire                                        | Recommandation        |
| ----------------------------------------- | --------------------------------------------------------- | --------------------- |
| Titre de chaque épisode                   | `audio` de la **scène d'option** de l'épisode (carrousel) | oui                   |
| Nom de chaque catégorie / sous-menu       | `audio` de la scène d'option de la catégorie              | oui                   |
| Titre du pack                             | `title.mp3` (carte de titre ; lu par `telmi2flam`, § 6)   | oui                   |
| Intro avant l'épisode (« Épisode 3 : … ») | ajouterait un audio avant l'épisode                       | non (voir ci-dessous) |

- Le fichier audio d'une option est **joué quand l'option prend le focus** dans le carrousel
  (scène `autoplay: false` ; comportement attendu du format Telmi, non vérifié ici). **À
  vérifier sur l'appareil** avant la PR 2 (comme #219) : lecture au focus, coupure au
  défilement, volume comparé à celui des épisodes.
- Pas d'intro avant l'épisode : elle allongerait la scène d'épisode (qui est une copie de la
  source, D6) ou obligerait à une scène de plus par épisode (limite `MAX_STAGES`).
- Texte lu : le **titre d'origine** (une ligne, 120 caractères au plus, coupé proprement), pas
  le libellé translittéré de 40 caractères (A6) : la voix prononce mieux les accents. Les
  titres vides ou sans lettre (« 🎧 », « #12 ») gardent le silence ; les numéros et sigles sont
  lus tels que la voix les lit (pas de normalisation en v1).
- Les noms d'entrées de l'archive restent générés (`audios/t<nnn>.mp3`, `audios/c<nnn>.mp3`),
  jamais tirés du titre (invariant « noms d'entrées sûrs »).
- Une option sans voix (case décochée) garde `audio: null`, comportement actuel inchangé.

### 12.3 API : WinRT ou SAPI 5

| Critère                      | `Windows.Media.SpeechSynthesis` (WinRT)                                                                                                                | SAPI 5 (`ISpVoice`, COM)                                    |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------- |
| Sortie                       | `SynthesizeTextToStreamAsync` : flux WAV PCM en mémoire                                                                                                | `SPFileStream` : fichier WAV, ou flux COM à écrire          |
| Voix                         | voix « OneCore » : celles de Paramètres > Heure et langue > Voix                                                                                       | voix SAPI classiques (Hortense, Julie, Paul sur Windows 10) |
| Voix françaises              | attendues avec le pack de langue français, à vérifier (installé par défaut sur un Windows en français)                                                 | idem, ensemble plus restreint, voix plus anciennes          |
| Rendu                        | plus naturel (voix récentes)                                                                                                                           | plus robotique                                              |
| Accès Rust                   | crate `windows` (Microsoft, **MIT OU Apache-2.0**), fonctionnalités `Media_SpeechSynthesis`, `Storage_Streams`, `Foundation`, `Foundation_Collections` | même crate, `Win32_Media_Speech`, `Win32_System_Com`        |
| Complexité                   | asynchrone (`.get()` bloquant dans un thread dédié), flux mémoire                                                                                      | COM à initialiser, gestion des événements                   |
| Applications non empaquetées | fonctionne (le launcher n'est pas MSIX)                                                                                                                | fonctionne                                                  |

**Recommandation : WinRT seul** (meilleur rendu, sortie en mémoire sans fichier temporaire,
une seule dépendance déjà permissive). Pas de repli SAPI en v1 : il doublerait le code et les
tests pour des voix que l'on souhaite moins (voir Q4). Le launcher utilise aujourd'hui
`windows-sys` (`Cargo.toml`, `cfg(windows)`) ; ajouter `windows` augmente le temps de compilation
et la liste de `THIRD-PARTY-NOTICES.txt` (deux licences permissives, déjà autorisées dans `scripts/notices-lib.mjs` : vérifier que `npm run build:notices` passe), à mesurer dans la PR 1. Le code reste derrière
`#[cfg(windows)]` ; sous les autres systèmes, le module répond « synthèse indisponible ».

**À vérifier sur une machine avant la PR 1** (je n'ai pas pu le faire dans cette note) :

- les noms exacts des voix françaises sur Windows 10 et 11 (attendues : Microsoft Hortense,
  Julie, Paul, et, sur Windows 11, éventuellement d'autres voix « naturelles » qui sont
  **réservées au Narrateur** et peuvent ne pas apparaître dans `SpeechSynthesizer.AllVoices`) ;
- le format WAV réellement produit (fréquence, mono ou stéréo ; 16 bits attendus) ;
- qu'aucune voix listée n'est « en ligne » (toute voix de `AllVoices` doit rester locale).

### 12.4 Voix, langue, réglages

- **Choix de la voix** : liste de `SpeechSynthesizer.AllVoices` filtrée sur la langue `fr`
  (`fr-FR`, `fr-CA`, `fr-BE`, `fr-CH`) ; défaut = `DefaultVoice` si elle est française, sinon la
  première voix française, triée par nom pour que le choix soit stable.
- Les titres de podcasts peuvent être dans une autre langue ; le texte est lu par la voix
  choisie (v1 : une voix par pack). Choisir la voix d'après `<language>` du flux est une
  évolution (Q3).
- **Réglages** : la voix (identifiant `VoiceInformation.Id`) est mémorisée par poste dans les
  réglages du launcher ; si elle n'existe plus, retour au défaut avec un avertissement (voir 12.5 ; erreur `tts_no_voice` seulement s'il n'y a plus aucune voix française). **Pas de réglage de
  débit en v1** (débit 1,0) : un titre court, lu une fois ; on le règlera si l'aperçu montre
  un besoin. Le volume est celui du MP3 produit, sans normalisation (comme les épisodes).

### 12.5 Repli si aucune voix française

| Situation                                                | Comportement proposé                                                                                                                                                                                                                                                                                        |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Aucune voix française à l'ouverture de l'éditeur         | case **désactivée**, message : « Aucune voix française n'est installée dans Windows (Paramètres > Heure et langue > Voix). » ; le pack reste créable, sans voix                                                                                                                                             |
| Voix disparue entre le choix et la création              | **repli sur la voix française par défaut**, avec un avertissement au récapitulatif ; `tts_no_voice` (arrêt avant toute écriture) seulement s'il n'y a plus aucune voix française. Tranché par le mainteneur, **fait en PR 2** (`resolve_voice` rend `VoiceGone`, `pack_speech::choose_voice` fait le repli) |
| Échec de synthèse d'un titre                             | ce titre reste **silencieux** (`audio: null`), le pack est créé, un avertissement par titre est ajouté au récapitulatif (comme P7)                                                                                                                                                                          |
| Plateforme non Windows                                   | case masquée                                                                                                                                                                                                                                                                                                |
| Le premier titre échoue, ou plus de la moitié des titres | la création **s'arrête avant toute écriture** (`tts_failed`) : jamais de pack entièrement muet sans l'avoir demandé                                                                                                                                                                                         |

Le silence actuel reste donc le comportement par défaut ; la voix est toujours un choix
explicite du parent (case décochée par défaut).

### 12.6 Format produit, durée, coût, cache

- Synthèse en mémoire (WAV PCM) → fichier WAV temporaire dans le dossier de travail de la tâche
  → **MP3 44 100 Hz** par le ffmpeg LGPL embarqué (`ffmpeg.rs` et `podcast/audio.rs`, arguments fixes, `-nostdin`,
  empreinte vérifiée, annulation, délai) : mêmes précautions que #214. **Mono, 64 kbps**
  (un titre de 3 s pèse environ 24 Kio). Le mono n'est accepté par D6 que pour une copie ; un
  MP3 mono sur l'appareil est **à valider avec #219** (repli : stéréo 96 kbps, environ 36 Kio
  pour 3 s). Le plan de conversion actuel de `podcast/audio.rs` code `-ac 2` : il faut des
  arguments dédiés (entrée WAV, sortie fixe). Pas de nouvel encodeur : `libmp3lame`
  est déjà dans le build.
- Durée d'un titre : 2 à 10 s (120 caractères au plus). **Plafond : 15 s** ; au-delà, le titre
  reste silencieux avec un avertissement (pas de coupe).
- **Ordre de grandeur** : 50 épisodes au plus (`MAX_EPISODES`, D1) et au plus 100 catégories
  (`MAX_STAGES` = 256 laisse la marge), soit au plus environ 150 options et le titre du pack :
  de 4 à 12 Mo de MP3 au total (24 à 80 Kio par titre). La synthèse est bien plus rapide que le
  temps réel ; le coût dominant est le lancement de ffmpeg par fichier (quelques dizaines de ms) :
  de l'ordre de 5 à 20 s pour un grand pack, **à mesurer sur un poste réel** (la CI n'a pas de
  voix française). L'annulation passe par celle de la création (#217) ; la progression affiche
  une phase « speech » (« lecture des titres à voix haute »), sans pourcentage propre (étape
  courte).
- **Cache** : pas de cache persistant en v1 (le coût est faible et le dossier de travail est
  supprimé après la publication, N8 a). Le cache de #460 est propre aux épisodes (clé = épisode
  d'une source, dossier de la source) : il n'offre pas d'API de clé générale, les titres n'y
  sont donc pas mis. Clé prévue si un cache des titres est ajouté plus tard :
  `sha256(voix, débit, version du format, texte)`.
- **Place dans la création (PR 2)** : **avant les téléchargements**, juste après la revalidation
  et le contrôle d'espace disque (D5). Écart assumé avec la recommandation d'origine (« après la
  conversion des épisodes ») : une voix inutilisable (premier titre ou plus de la moitié en
  échec) arrête alors la création **sans rien télécharger**. Les titres sont lus pour toute la
  composition ; ceux des épisodes retirés ensuite (P7) et des catégories vidées ne vont pas dans
  l'archive (leurs MP3 restent dans le dossier de travail, supprimé avec lui) et leurs
  avertissements sont écartés.

### 12.7 Interface (éditeur de pack, #363)

- Case à cocher **« Lire les titres à voix haute »**, sous les métadonnées du pack ; décochée par
  défaut ; désactivée avec explication si aucune voix (12.5).
- Si plusieurs voix françaises : liste déroulante « Voix », mémorisée par poste.
- Bouton **« Écouter »** (aperçu) : lit le titre du pack avec la voix choisie ; même bouton
  discret à côté d'un titre d'épisode ou de catégorie. Commande `tts_preview(text, voice)` :
  texte borné comme 12.2, résultat WAV court (quelques centaines de Kio). **CSP** : la politique du launcher
  (`tauri.conf.json`) n'a pas de `media-src` ; lire le WAV dans l'élément audio exige un
  `media-src blob:` minimal (changement de sécurité, à justifier dans la PR 3) ; sinon l'aperçu
  est joué côté Rust. Choix à faire dans la PR 1 ; un aperçu à la fois, annulé si le texte change. Aucun appel hors de la machine.
- Textes dans le fichier de textes de l'interface, parcours au clavier, case et liste
  étiquetées pour les lecteurs d'écran.
- L'estimation de taille (D5) ajoute la taille des titres (≈ 80 Kio par titre, borne haute pour 10 s).
- La case et la voix entrent dans le brouillon (N3 b) et dans la composition enregistrée avec la
  tâche (N10 a), pour que la reprise reconstruise le même pack.

### 12.8 Vie privée, licences, tests

**Voix « en ligne ».** L'API WinRT n'indique pas si une voix est locale : `french_voices` écarte
celles dont le nom ou l'identifiant contient « online » ou « en ligne » (voix « naturelles » de
Windows 11). C'est une heuristique, **à confirmer à la main sur un poste sans réseau** (test
`voix_reelle_du_poste`, `#[ignore]`) ; elle reste en l'état d'ici là (suivi #467).

**Vie privée.** Tout est attendu local (voix OneCore, **à confirmer par un test sans réseau**
dans la PR 1) ; les titres ne sortent jamais du poste, rien n'est journalisé (même règle que les
URL de flux, D11 : ni titre ni voix dans les journaux).

**Licences.**

- Aucune voix, aucun modèle ni aucune bibliothèque de synthèse n'est embarqué : on appelle le
  système. Rien à ajouter dans `LICENSES.md` ; la crate `windows` (MIT OU Apache-2.0) va dans
  `THIRD-PARTY-NOTICES.txt`.
- Les voix Microsoft appartiennent à Microsoft. Les packs sont déjà réservés à l'usage personnel
  et familial (A8, D14) ; l'audio produit ne doit pas être partagé.
- **Conclusion du mainteneur (2026-10-24, Q4)** : Microsoft ne publie pas de conditions
  explicites sur l'audio produit par les voix intégrées à Windows. L'usage est donc limité au
  cadre personnel et familial, **sans diffusion** des packs ni de leur audio ; cette mention est
  déjà portée par l'avertissement d'usage des podcasts (A8, D14), rien n'est ajouté à
  l'interface.
- piper et espeak-ng restent exclus (GPL-3.0, § 7).

**Tests.**

- Un trait `Synthesizer` (liste des voix, `synthesize(texte, voix) -> WAV`) ; implémentation
  WinRT derrière `#[cfg(windows)]` ; **voix simulée en CI** qui produit un WAV déterministe (un
  sinus dont la durée dépend de la longueur du texte) et des listes de voix configurables
  (aucune, une, plusieurs, disparue). Les tests de l'archive, de la préparation et de l'interface
  s'appuient dessus ; les tests existants avec un faux ffmpeg (#214) couvrent la conversion.
- **Test réel manuel**, hors CI (les exécuteurs GitHub n'ont en général pas de voix française) :
  un test `#[ignore]` sous Windows qui synthétise « Bonjour » si une voix française existe, et une
  procédure dans la PR 2 : créer un pack de 10 titres, vérifier sur l'appareil la lecture au
  focus, la coupure au défilement et le volume.
- Test d'injection de panne (CONTRIBUTING § 5) pour la PR 2, puisqu'elle écrit dans le dossier de
  travail de la file d'installation : arrêt ou échec à chaque étape, aucun fichier restant,
  reprise cohérente.

### 12.9 Pannes

| Étape / panne                                                             | État laissé                        | Reprise ou message                                                                                                                                                        |
| ------------------------------------------------------------------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Aucune voix FR à l'ouverture                                              | rien                               | case désactivée et explication                                                                                                                                            |
| Option demandée sans aucune voix FR (création de la tâche, `pack_create`) | rien, aucune tâche créée           | erreur `tts_no_voice` (`tts_unavailable` si la synthèse est indisponible), retour à l'éditeur                                                                             |
| Voix disparue avant la préparation                                        | rien                               | repli sur la voix FR par défaut, avertissement au récapitulatif ; `tts_no_voice` (avant tout téléchargement) s'il n'y a plus aucune voix FR                               |
| Énumération des voix bloquée                                              | rien                               | délai global de 12 s (liste et voix par défaut ensemble, #480) : traité comme `tts_unavailable`                                                                           |
| `SpeechSynthesizer` indisponible (service voix, COM)                      | rien                               | `tts_unavailable` ; mêmes cas que « aucune voix »                                                                                                                         |
| Échec de synthèse d'un titre                                              | pas de WAV                         | titre silencieux, avertissement au récapitulatif                                                                                                                          |
| Premier titre ou plus de la moitié en échec                               | dossier de travail supprimé        | erreur `tts_failed` **avant tout téléchargement**, retour à l'éditeur (« premier titre » : premier titre qui a quelque chose à lire ; la moitié se compte sur ces titres) |
| Synthèse qui ne rend pas la main                                          | thread abandonné, WAV jamais écrit | délai par titre (10 s) : échec de ce titre ; annulation possible pendant la synthèse, sans attendre la voix                                                               |
| Titre vide, sans lettre ou trop long                                      | rien                               | silencieux ou coupé, pas d'erreur                                                                                                                                         |
| WAV vide ou corrompu                                                      | WAV supprimé                       | comme un échec de synthèse                                                                                                                                                |
| ffmpeg absent, haché faux ou en échec                                     | temporaires `.part` supprimés      | comme un échec de synthèse (titre silencieux, ou arrêt `tts_failed` selon la règle ci-dessus) ; ffmpeg absent au départ bloque déjà la création (prérequis)               |
| Annulation pendant les titres                                             | dossier de travail supprimé        | comme la création (#217), rien téléchargé                                                                                                                                 |
| Arrêt brutal de l'application                                             | dossier de travail à nettoyer      | reprise N10 a : les titres sont **refaits** avec la voix enregistrée ; si elle a disparu, voix par défaut (avertissement) ou arrêt `tts_no_voice`                         |
| Disque plein                                                              | temporaires supprimés              | erreur d'écriture de la création                                                                                                                                          |
| Lecture au focus absente sur l'appareil                                   | pack valide, sans voix audible     | corrigé dans le graphe avant la PR 2, sinon fonctionnalité réservée                                                                                                       |

### 12.10 Découpage en PR courtes

1. **PR 1 : module `podcast/speech.rs`** (trait, implémentation WinRT, liste des voix filtrée,
   synthèse en WAV mémoire, voix simulée, tests) et commandes Tauri `tts_voices` et `tts_preview`.
   Pas de lien avec l'archive. Mesures : durée par titre, taille de la dépendance.
2. **PR 2 : intégration dans la création du pack** : option `titles_audio` dans la composition
   (#362, avec une valeur par défaut serde pour relire les tâches déjà enregistrées), audios de
   titre dans `archive.rs` (`audio` des options, `title.mp3`), étape de synthèse et conversion
   en MP3 dans la préparation (#217), avertissements, reprise ; **note de conception, test
   d'injection de panne, `modele:opus`** (CONTRIBUTING § 5). Dépend de la PR 1 ; test sur
   appareil ; contrat des voix vérifié avant fusion.
3. **PR 3 : interface** : case, voix, aperçu, estimation, textes, README « Créer un pack » (#220).
   Dépend de la PR 1 ; en série avec toute autre PR sur l'éditeur (mêmes fichiers).

Ordre : PR 1, puis PR 2, puis PR 3. #217 et #363 sont fusionnées. Un **essai sur appareil** de la
lecture au focus d'une option avec audio (un MP3 quelconque dans une archive de test) peut se
faire dès maintenant, avant la PR 1. Les noms d'entrées suivent la convention existante
(`t001.mp3`, `c001.mp3`).

### 12.11 Questions à trancher par le mainteneur

| #   | Question                                                                                                                                      | Recommandation                                                                                                                                                                                  |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Q1  | Que lit-on : seulement les épisodes ? aussi les catégories et le titre du pack ? une intro avant chaque épisode ?                             | Épisodes, catégories **et** titre du pack ; pas d'intro avant l'épisode (12.2).                                                                                                                 |
| Q2  | Sans voix française : bloquer, ou créer un pack silencieux ? Et si un seul titre échoue ?                                                     | Case désactivée avec message, jamais de pack silencieux par surprise ; titre isolé en échec = silencieux et avertissement, mais arrêt si le premier titre ou plus de la moitié échouent (12.5). |
| Q3  | Réglages : voix seule, voix et débit, voix selon la langue du flux ?                                                                          | Voix choisie par poste (liste des voix françaises), **sans débit** en v1 ; voix selon la langue du flux plus tard.                                                                              |
| Q4  | API et licence : WinRT seul (crate `windows`) ou WinRT puis SAPI ; accepte-t-on l'usage des voix Microsoft limité au cadre personnel du § 4 ? | **WinRT seul** ; usage personnel et familial déjà imposé (A8, D14), avec une vérification du contrat des voix avant la fusion de la PR 2.                                                       |

### 12.12 Intégration dans la création du pack (PR 2)

- **Composition** (`podcast/pack.rs`) : `titles_audio` (booléen) et `titles_voice` (identifiant
  de voix, facultatif), tous deux `serde(default)` et omis à l'écriture quand ils valent leur
  défaut : les brouillons et tâches enregistrés avant restent lisibles, et le JSON d'une
  composition sans voix est inchangé.
- **Réglage du poste** (`settings.rs`) : `tts_voice` (identifiant, facultatif, borné à
  512 caractères sans caractère de contrôle). À la création de la tâche, une composition sans
  voix reprend celle du poste ; la voix ainsi fixée est enregistrée avec la tâche, la reprise
  (N10 a) la réutilise.
- **Création de la tâche** (`pack_create`, `InstallService::start_pack`) : option demandée →
  identifiant contrôlé, voix françaises énumérées (délai global de 12 s) ; aucune voix
  française : refus `tts_no_voice`, synthèse indisponible : `tts_unavailable` ; aucune tâche
  créée. Une voix demandée mais disparue n'est pas une erreur (repli à la préparation).
- **Préparation** (`install/pack_speech.rs`) : voix résolue (repli avec avertissement), titres
  dans l'ordre de l'archive (titre du pack, puis options en profondeur), synthèse bornée
  (10 s par titre), conversion MP3 mono 64 kbit/s par le ffmpeg embarqué
  (`title-NNNN.mp3` dans le dossier de travail), règles d'arrêt de 12.5 (la moitié se compte
  sur tous les titres lus, y compris ceux d'épisodes retirés ensuite), plus un arrêt
  `tts_failed` après deux délais (ou indisponibilités) de synthèse consécutifs : un service de
  voix figé n'impose pas 10 s d'attente par titre.
- **Archive** (`podcast/archive.rs`) : `audio` de la scène d'option = `tNNN.mp3` (épisode de
  rang NNN) ou `cNNN.mp3` (catégorie de rang NNN), entrées `audios/…` ; titre du pack =
  `title.mp3` à la racine ; titre muet : `"audio": null` (`silent.mp3` à la conversion). Les
  MP3 de titre suivent les contrôles des épisodes (en-tête MP3, taille, total, fichier jamais
  utilisé deux fois). Aucune scène ajoutée, aucune annonce avant l'épisode.
- **Avertissements** : `PackStatus::warnings` (voix remplacée, titres restés muets, cités par
  leur libellé), joints à la tâche et à `install://done` ; l'affichage est fait en PR 3.
  Les journaux ne citent que le rang d'un titre, jamais son texte ni la voix.
- **Tests** : contenu d'archive (un audio par option, `title.mp3`, `"audio": null` sinon, titre
  lu invalide ou réutilisé refusé sans écriture), injection de pannes sur la préparation (voix
  en échec sur le premier titre, sur plus de la moitié, ffmpeg absent, annulation pendant la
  synthèse, voix disparue, plus aucune voix française) : jamais de téléchargement ni de dossier
  de travail résiduel ; anciens brouillons relus ; délai global de l'énumération des voix.
- **Reste à vérifier sur un poste et sur l'appareil** (procédure du mainteneur) : pack de
  10 titres avec une vraie voix, lecture au focus, coupure au défilement, volume comparé aux
  épisodes, MP3 mono accepté (#219), durée de l'étape pour un grand pack.

## 13. Cache des MP3 convertis (N8 b, #460)

Évolution retenue par le mainteneur le 2026-10-24 (#460, issue de #221) : **refaire un pack sans
retélécharger ni reconvertir**. Elle remplace N8 a pour l'audio converti seulement : les
originaux, l'archive Telmi et le dossier de travail restent supprimés après chaque création.
Le code est dans `src-tauri/src/podcast/mp3cache.rs`, appelé par la préparation
(`src-tauri/src/install/podcast.rs`).

### 13.1 Clé, emplacement, contenu

- **Emplacement** : `<cache>/podcasts/<source_id>/mp3/`, sous le dossier de cache de la source
  (§ 3.5). La suppression de la source purge donc aussi ses MP3 (D15) sans code de plus, et
  « Vider le cache » les compte et les supprime (sous-dossier `podcasts` déjà listé).
- **Nom d'une entrée** : identifiant opaque de l'épisode (`episode_id`, empreinte SHA-256 du
  `guid`, 32 chiffres hexadécimaux). Trois fichiers : `<id>.mp3` (MP3 converti selon D6),
  `<id>.img` (pochette d'origine de l'épisode, facultative), `<id>.json` (métadonnées).
- **Empreinte d'entrée** (dans `<id>.json`) : SHA-256 de la version du format, du **profil de
  conversion** (`AUDIO_PROFILE`), du `guid`, de l'URL, de la taille annoncée et du type de
  l'enclosure. La pochette a sa propre empreinte (SHA-256 de son URL). **Aucune URL en clair**
  sur le disque ni dans les journaux (D11) : seules les empreintes sont écrites, les traces ne
  citent que l'uuid de la source et le rang de l'épisode.
- **Métadonnées** : taille et SHA-256 du MP3 (et de la pochette), dates d'écriture et de
  dernier usage.

### 13.2 Invalidation

- **Épisode modifié** (nouvelle URL d'enclosure, taille ou type annoncés différents) :
  l'empreinte ne correspond plus, l'entrée est supprimée et l'épisode retéléchargé.
- **Règle de conversion changée** : `AUDIO_PROFILE` est une constante à incrémenter à chaque
  changement de D6 ou de `podcast/audio.rs` ; toutes les entrées existantes deviennent alors
  invalides. Il n'existe aucun réglage utilisateur de conversion.
- **Pochette changée** : seule la pochette est retéléchargée (empreinte d'image différente),
  le MP3 reste servi ; l'entrée est remise en cache avec la nouvelle pochette.
- **Publicité** (D9) : l'audio est repris tel quel, publicité comprise ; le cache garde le
  fichier tel qu'il a été téléchargé la première fois. Avec une publicité insérée
  dynamiquement par le serveur, un pack refait reprend donc la publicité du premier
  téléchargement (rien n'est filtré, comportement inchangé sur le fond). Vider le cache ou
  supprimer la source force un nouveau téléchargement.
- **Borne d'un épisode abaissée** : une entrée plus grande que `MAX_AUDIO_BYTES` n'est jamais
  servie ; elle est supprimée avec un avertissement (#466).
- **À observer sur de vrais flux** (#466) : l'empreinte contient l'URL de l'enclosure, paramètres
  compris. Un hébergeur qui fait tourner ces paramètres (jeton signé, identifiant de suivi
  renouvelé à chaque lecture du flux) rend le cache inopérant pour ses épisodes : chaque pack
  les retélécharge, sans autre gêne (l'entrée est simplement remplacée). Aucun cas constaté à ce
  jour ; si cela se présente, retirer de l'empreinte les paramètres reconnus comme variables
  plutôt que toute la requête (une URL d'enclosure changée doit rester une invalidation).

### 13.3 Taille maximale et éviction

- **2 Gio au plus** (`MAX_CACHE_BYTES`, soit deux packs au plafond D1), toutes sources
  confondues, comptés sur les tailles réelles (MP3 et pochettes).
- **Éviction LRU** à chaque préparation de pack : les entrées les moins récemment utilisées sont
  supprimées jusqu'à repasser sous le plafond ; les entrées du pack en cours ne sont jamais
  évincées. Les restes (fichier sans métadonnées valides, `.part`, temporaires) sont supprimés
  au même passage.
- D5 (espace disque avant la création) reste calculé comme sans cache : estimation prudente.

### 13.4 Écriture, lecture, concurrence

- **Écriture atomique** : `<id>.json` est supprimé d'abord (l'ancienne entrée n'est plus
  servie), le MP3 et la pochette sont écrits en `.part` puis renommés, et `<id>.json` est écrit
  **en dernier** (temporaire puis renommage) : il vaut validation de l'entrée. Le MP3 est
  d'abord **lié** (lien physique, aucune place en plus sur le disque) depuis le dossier de
  travail, ou copié si le système de fichiers ne le permet pas (FAT, exFAT).
- **Lecture** : l'entrée n'est servie que si `<id>.json` est lisible, si l'empreinte correspond
  et si la taille **et** le SHA-256 du MP3 correspondent ; le fichier est alors lié (ou copié)
  dans le dossier de travail sous un nom distinct (`cache-<rang>.mp3`) et la date d'usage est
  renouvelée. Aucun fichier du dossier de travail n'est modifié sur place : un lien physique
  ne peut pas altérer l'entrée.
- **Verrou** : toute lecture, écriture ou éviction d'une source se fait sous le **verrou de la
  source** (celui des rafraîchissements et de la purge, `PodcastService`), avec contrôle du
  **compteur de génération** lu avant le chargement des flux : une source supprimée pendant la
  création n'est jamais recréée sur le disque. Le portillon du cache est déjà tenu par la tâche
  d'installation (« Vider le cache » attend la fin de la création) ; le cache ne le reprend
  pas (une seconde prise en lecture bloquerait derrière un vidage en attente).
- **Deux packs simultanés** : les préparations sont déjà sérialisées (une à la fois) ; le
  verrou par source suffit en plus pour qu'une écriture, une lecture et une éviction de la même
  source ne se croisent jamais.
- **Annulation** (#466) : le hachage (SHA-256 complet à chaque lecture) et la copie de repli
  lisent par blocs de 256 Kio et s'arrêtent au bloc suivant quand la tâche est annulée, ce qui
  libère vite le verrou de la source. Une lecture annulée ne juge ni ne supprime aucune entrée ;
  une écriture annulée ne laisse rien de l'entrée (une écriture annulée avant de commencer garde
  l'ancienne). Cas accepté : l'annulation pendant la remise en cache d'un épisode servi dont
  seule la pochette a changé efface aussi son MP3 (l'ancienne entrée est retirée avant
  l'écriture) ; il sera retéléchargé au pack suivant.
- **Liens et jonctions** (#466) : `podcasts/`, `podcasts/<id>/` et `podcasts/<id>/mp3/` doivent
  être de vrais dossiers (contrôle sans suivre de lien, à chaque accès, dossiers manquants créés
  un par un puis recontrôlés) ; sinon la source est ignorée avec un avertissement : rien n'est
  lu, écrit ni supprimé hors du cache. Les fichiers d'une entrée doivent être de vrais fichiers,
  et seuls `<id>.mp3`, `<id>.img` et `<id>.json` comptent comme tels (le reste est supprimé à
  l'éviction).
- **Pochettes des épisodes servis** : celles à retélécharger (absentes du cache ou périmées) le
  sont en parallèle, au plus autant à la fois que les épisodes (`max_downloads`, borné à 4).
- Le cache est **au mieux** : aucune de ses erreurs ne fait échouer un pack ; une entrée
  inutilisable donne un retéléchargement.

### 13.5 Tableau des pannes

| Panne                                                 | Effet                                                                             |
| ----------------------------------------------------- | --------------------------------------------------------------------------------- |
| MP3 tronqué ou modifié (taille ou SHA-256 différents) | Entrée supprimée, épisode retéléchargé et reconverti, puis remis en cache.        |
| `<id>.json` absent, illisible ou d'un autre format    | Entrée ignorée (restes supprimés à l'éviction) ; épisode retéléchargé.            |
| Pochette en cache altérée                             | Pochette retéléchargée ; le MP3 reste servi.                                      |
| Disque plein pendant l'écriture en cache              | `.part` et entrée supprimés, avertissement journalisé ; le pack continue.         |
| Disque plein pendant la lecture (copie de repli)      | Copie supprimée, épisode traité comme absent du cache (retéléchargé).             |
| Arrêt brutal pendant l'écriture                       | Sans `<id>.json` valide, l'entrée n'est jamais servie ; restes supprimés ensuite. |
| Source supprimée pendant la création                  | Génération changée : rien n'est écrit ; la purge supprime tout le dossier.        |
| « Vider le cache » pendant la création                | Attend la fin de la tâche (portillon) ; les liens du dossier de travail restent.  |
| Épisode ou règle de conversion modifiés               | Empreinte différente : entrée remplacée.                                          |
| Plafond de 2 Gio dépassé                              | Entrées les plus anciennes évincées (jamais celles du pack en cours).             |
| Tâche annulée pendant le hachage ou la copie          | Arrêt au bloc suivant ; aucune entrée supprimée, écriture en cours effacée.       |
| `podcasts/` ou `podcasts/<id>/` remplacé par un lien  | Source ignorée (avertissement) : rien lu, écrit ni supprimé hors du cache.        |
| Fichier d'extension inconnue dans `mp3/`              | Compté comme reste, supprimé à l'éviction.                                        |

### 13.6 Interface

Aucun écran dédié : la taille du cache des MP3 est comptée dans la taille du cache affichée par
les paramètres, « Vider le cache » la supprime, et la suppression d'une source purge la sienne.
Un affichage séparé (taille du cache des MP3, bouton dédié) est une évolution possible, non
faite dans #466 (suivi : #489).

## 14. Compositions nommées réutilisables (N3 c, #461)

Évolution retenue par le mainteneur le 2026-10-24 (#461, issue de #221) : après la création
d'un pack, sa **composition** (épisodes choisis et leurs flux, arborescence, titre, sous-titre,
âge, description, couverture) est conservée sous un nom. Le parent peut la **rouvrir** pour
refaire le même pack ou en **dériver** un autre, la **renommer** et la **supprimer**. Le modèle
pack est inchangé : chaque création donne une **nouvelle histoire** (nouvel uuid, P3), jamais
une mise à jour. Le code est dans `src-tauri/src/podcast/compositions.rs`.

### 14.1 Quand une composition est conservée

- **À la création de la tâche** (`pack_create` accepté, composition revalidée), pas à la
  publication. Une création qui échoue ou qu'on annule (réseau coupé, disque plein) se relance
  ainsi sans tout ressaisir ; et la file d'installation (#217) n'a pas à connaître les
  compositions : seule la commande `pack_create` les appelle, après l'ajout de la tâche.
- **Composition identique** à une composition conservée (même arborescence, mêmes épisodes,
  mêmes métadonnées saisies, même couverture) : pas de doublon, l'entrée existante est mise à
  jour (date du dernier pack, nombre de packs). C'est le cas « refaire ».
- **Composition différente** : nouvelle entrée, nommée d'après le **titre effectif** du pack
  (titre saisi, sinon titre du flux, N1 b), avec un suffixe « (2) », « (3) »… si ce nom est
  déjà pris. C'est le cas « dériver » : la composition d'origine reste intacte.
- Un échec d'enregistrement (fichier illisible, disque plein, limite atteinte) **ne bloque
  jamais la création** du pack : la tâche est créée, un message dit que la composition n'a pas
  été conservée et pourquoi.

### 14.2 Rouvrir : refaire ou dériver

- « Ouvrir » **copie** la composition dans le brouillon unique (N3 b), à la révision suivante
  de celui-ci, puis affiche l'éditeur. Une sauvegarde automatique en retard est refusée ; celle
  d'un éditeur déjà fermé n'est alors pas réessayée au-dessus (#484), même si elle arrive
  après les 2 s d'attente de l'ouverture : elle ne peut pas remplacer la composition ouverte.
- Si le brouillon contient **une saisie** (épisode, catégorie, titre, sous-titre,
  description, âge ou couverture choisis), une confirmation le dit : « Le brouillon en cours
  sera remplacé. » Brouillon vide : pas de confirmation. Brouillon illisible : le backend l'a
  déjà mis de côté (copie `*.bak`, N3 b), pas de confirmation, son message est affiché ;
  lecture impossible (erreur) : confirmation, par prudence.
- Pendant l'ouverture, « Ouverture de … » s'affiche et les boutons de la liste sont inactifs.
- La composition conservée n'est jamais modifiée par l'éditeur : seul le brouillon l'est.
  « Créer l'histoire » sans modification la refait (même entrée) ; avec des modifications, une
  nouvelle entrée est créée (14.1).
- Épisodes disparus : voir 14.5. La création reste impossible tant qu'ils sont dans
  l'arborescence (validation existante, N3 b) ; le parent les retire puis crée.

### 14.3 Format et emplacement

- Fichier `pack-compositions.json` du **dossier de configuration**, à côté de
  `pack-draft.json` : ce sont des données du parent, pas un cache (« Vider le cache » n'y
  touche pas).
- JSON versionné (`schema_version` = 1, lu par `storage::load_versioned`) :

  ```json
  {
    "schema_version": 1,
    "compositions": [
      {
        "id": "uuid v4 tiré par le launcher",
        "name": "Contes du soir",
        "created_at": 1761300000,
        "last_used_at": 1761300000,
        "packs": 1,
        "composition": { "metadata": {}, "cover": {}, "menu": [] }
      }
    ]
  }
  ```

  `composition` a exactement la forme du brouillon (`PackComposition`, #362) ; dates en
  secondes Unix ; `packs` : nombre de créations lancées depuis cette composition.

- **Aucune URL** sur le disque (D11) : la composition ne contient que des identifiants
  (source, épisode) et des textes saisis.
- **Couverture référencée, jamais copiée** : la composition garde le choix (`CoverChoice` :
  défaut, flux ou épisode), comme le brouillon. Aucune image à stocker ni à purger ; si l'image
  a disparu du flux, l'ouverture signale « couverture indisponible » et le parent en choisit
  une autre (ou le défaut).

### 14.4 Écriture, lecture, concurrence

- Toute modification (conservation, renommage, suppression) est un cycle
  lecture-modification-écriture **sous le verrou de configuration** (`ConfigState::guard`,
  celui du brouillon et des sources), terminé par une **écriture atomique**
  (`storage::write_atomic` : temporaire voisin, `sync_all`, renommage). Un arrêt brutal laisse
  l'ancien fichier ou le nouveau, jamais un mélange ; un temporaire abandonné est ignoré.
- Rien n'est gardé en mémoire : chaque commande relit le fichier.
- **Entrée écartée** dans un fichier lisible : illisible (type d'option inconnu, champ
  manquant, nom invalide), en double (identifiant ou nom déjà pris) ou au-delà de la limite.
  Le fichier entier est d'abord **copié** en `pack-compositions.json.<horodatage>.bak`, puis
  réécrit avec les seules entrées gardées ; un message le dit une fois, raisons distinguées
  (« 1 illisible(s), 2 en double »). Rien n'est perdu : la copie garde tout. Si la réécriture
  échoue (fichier verrouillé, disque plein), la copie est retirée (l'original, intact, en tient
  lieu) : des lectures répétées n'accumulent pas de copies. Toute écriture suivante (conserver,
  renommer, supprimer) **refait d'abord la copie**, et n'a pas lieu si elle échoue : le
  fichier n'est jamais réécrit sans les entrées écartées tant qu'aucune copie ne les garde.
- **Fichier illisible** (tronqué, JSON invalide, non UTF-8) ou **trop gros** : mis de côté en
  `*.bak` (jamais écrasé), liste vide, message.
- **Copies bornées** : 3 copies `pack-compositions.json.*.bak` au plus (`MAX_BACKUPS`) : celle
  qui vient d'être faite, jamais supprimée (même si l'horloge a reculé), et les plus récentes
  des autres ; un fichier au nom inattendu n'est jamais supprimé. Une copie
  de la même seconde prend un suffixe supérieur à tous les existants
  (`storage::backup_path`) : la plus récente n'est jamais la première supprimée.
- **Schéma plus récent** : comme `settings.json`, le fichier n'est **ni lu ni modifié**
  (erreur « version de schéma plus récente », sans chemin). La liste affiche l'erreur ;
  renommer, supprimer, ouvrir échouent ; la création d'un pack continue sans conserver la
  composition (raison dite). Revenir à la version plus récente retrouve tout.
- **Message dit une fois** (fichier mis de côté, entrées écartées) : transmis par toutes les
  commandes qui lisent le fichier, y compris en échec (joint au message d'erreur, ou dans
  `notice` de la conservation à côté de `error`). La liste reste servie si les sources sont
  illisibles (flux supprimés alors non comptés).

### 14.5 Épisodes disparus, source supprimée

- **Épisode retiré du flux** : la composition est gardée telle quelle. La liste n'analyse pas
  les flux (coût : un flux relu par source) ; à l'ouverture, la validation de l'éditeur
  signale l'épisode comme indisponible, sans erreur.
- **Source supprimée** : la composition est **gardée et marquée** dans la liste (« 1 flux
  supprimé »). Justification : la suppression d'une source ne touche pas aux histoires créées
  (P4) ; par analogie, elle ne détruit pas le travail du parent (arborescence, noms de
  catégories, métadonnées), encore utile pour dériver un pack avec les autres flux. Les
  identifiants de source sont aléatoires : réajouter le même flux ne reconnecte pas la
  composition (évolution possible, issue de suivi). Le parent la supprime s'il n'en veut plus.

### 14.6 Limites

- **50 compositions** au plus (`MAX_COMPOSITIONS`). Au-delà, la création du pack continue mais
  la composition n'est pas conservée, avec un message ; aucune suppression automatique : ce
  sont les données du parent.
- Nom : 1 à 60 caractères (`MAX_NAME_CHARS`), espaces de bord retirés, sans caractère de
  contrôle, **unique** sans tenir compte de la casse.
- Nom : compté en caractères (points de code) côté interface comme côté backend ; le champ
  n'a pas de `maxlength` (compté en unités UTF-16, il couperait les émojis).
- Taille : 256 Kio par composition sérialisée en JSON indenté (une composition valide en fait au plus
  quelques dizaines : 50 épisodes, 16 Kio de description) et 16 Mio pour le fichier ; au-delà,
  l'entrée n'est pas conservée (message), un fichier trop gros est mis de côté.

### 14.7 Rétrocompatibilité

- Le **brouillon** ne change pas (schéma 1, aucun champ ajouté) : les brouillons existants se
  lisent comme avant.
- Fichier des compositions absent (versions précédentes) : liste vide, rien n'est écrit tant
  qu'aucun pack n'est créé.
- Champs ajoutés plus tard à `PackComposition` (voix des titres, #462, § 12.10) : en
  `serde(default)`, une composition conservée avant leur ajout se relit avec la valeur par
  défaut ; ils entrent dans la comparaison de 14.1.
- **Retour à une version antérieure** (#484) : les champs inconnus du premier niveau du
  fichier, d'une entrée et de sa `composition` sont conservés tels quels à la réécriture
  (`serde(flatten)` d'une table `extra`, réinsertion des champs inconnus de la composition).
  Une entrée qui en porte n'est jamais jugée identique (14.1) : refaire son pack depuis une
  version antérieure crée une nouvelle entrée, l'originale reste intacte. « Ouvrir » ne copie
  dans le brouillon que les champs connus.
- **Règle d'évolution** : tout ajout qu'une version antérieure ne saurait pas conserver
  tel quel **incrémente `schema_version`** : nouvelle variante (type d'option, de
  couverture), champ ajouté à l'intérieur de `metadata`, `cover` ou d'une option du menu,
  changement de sens d'un champ. Un champ facultatif ajouté au premier niveau d'une entrée
  ou de sa `composition` peut s'en dispenser (conservé ci-dessus). Une version antérieure
  refuse alors le fichier sans le toucher (14.4) au lieu de perdre des entrées.

### 14.8 Pannes

| Étape / panne                                        | État laissé                                  | Reprise ou message                                                            |
| ---------------------------------------------------- | -------------------------------------------- | ----------------------------------------------------------------------------- |
| Arrêt brutal pendant l'écriture                      | ancien fichier intact (temporaire abandonné) | la composition sera conservée à la prochaine création                         |
| Disque plein, fichier en lecture seule               | ancien fichier intact, temporaire supprimé   | création du pack poursuivie, message ; renommer ou supprimer : erreur         |
| Fichier tronqué ou JSON invalide                     | mis de côté en `*.bak`                       | liste vide, message une fois                                                  |
| Entrée écartée dans un fichier lisible               | copie `*.bak`, entrées lisibles réécrites    | message une fois, raisons distinguées                                         |
| Entrée écartée, réécriture impossible                | fichier intact, copie retirée                | entrées lisibles servies, message ; écriture suivante précédée d'une copie    |
| Plus de 3 copies `*.bak`                             | les plus anciennes supprimées                | aucun                                                                         |
| Fichier trop gros                                    | mis de côté en `*.bak`, jamais écrasé        | liste vide, message                                                           |
| Fichier d'un schéma plus récent                      | intact, ni lu ni réécrit                     | erreur affichée ; création poursuivie sans conservation, raison dite          |
| Champs inconnus (version plus récente, même schéma)  | conservés à chaque réécriture                | aucun                                                                         |
| Sources illisibles pendant la liste                  | inchangé                                     | liste servie, flux supprimés non comptés                                      |
| Limite de 50 atteinte                                | inchangé                                     | création poursuivie, message (et message de lecture éventuel)                 |
| Sauvegarde d'un éditeur fermé refusée                | brouillon (composition ouverte) intact       | pas de nouvelle tentative                                                     |
| Ouvrir : brouillon illisible                         | brouillon mis de côté (`*.bak`, N3 b)        | composition copiée dans un brouillon neuf                                     |
| Ouvrir : écriture du brouillon impossible            | brouillon inchangé                           | erreur affichée, l'éditeur n'est pas ouvert                                   |
| Ouvrir, renommer, supprimer une composition disparue | inchangé                                     | « composition introuvable », liste relue                                      |
| Épisode retiré du flux, flux jamais obtenu           | composition intacte                          | épisode signalé indisponible à l'ouverture, création bloquée tant qu'il y est |
| Source supprimée                                     | composition intacte                          | marquée dans la liste ; épisodes indisponibles à l'ouverture                  |
| Couverture disparue                                  | composition intacte                          | « couverture indisponible » à l'ouverture                                     |

### 14.9 Interface

- Écran Podcasts, section **« Compositions enregistrées »** : nom, nombre d'épisodes, date du
  dernier pack, flux supprimés éventuels ; boutons « Ouvrir », « Renommer », « Supprimer »
  (confirmation). Liste vide : une phrase dit que les compositions apparaissent après la
  création d'un pack.
- Après « Créer l'histoire », le message de lancement dit sous quel nom la composition est
  conservée, ou pourquoi elle ne l'a pas été ; dans ce cas (ou avec un message de lecture),
  il reste affiché jusqu'à ce qu'on le ferme.
- Le message dit une fois par « Ouvrir », « Renommer », « Supprimer » s'affiche de même
  jusqu'à fermeture (il survit au passage à l'éditeur).
- La mention d'usage (§ 4) est inchangée ; refaire un pack passe par le même message de
  droits de la première installation (D14, déjà acquitté sur le poste).

### 14.10 Tests

- Unitaires (`compositions.rs`) : conservation, déduplication, noms uniques, limite, renommage,
  suppression, ouverture dans le brouillon (révision suivante), source supprimée marquée.
- Injection de panne : fichier tronqué, JSON invalide, entrée illisible, schéma plus récent,
  fichier trop gros, disque plein simulé à l'écriture (écrivain injecté) : ancien fichier
  intact, aucune perte, création poursuivie.
- Interface (vitest) : liste, ouverture avec et sans confirmation, renommage, suppression,
  erreurs.
- Suivis #484 : copies bornées et rang des copies, copie retirée si la réécriture échoue
  (lectures répétées), messages transmis en succès et en échec, liste sans sources lisibles,
  schéma plus récent intact, aller-retour des champs inconnus (et disque plein), taille en
  JSON indenté, raisons distinguées ; vitest : brouillon non vide sans épisode, indicateur
  d'ouverture, messages des actions, nom en caractères, sauvegarde refusée d'un éditeur
  fermé non réessayée, message de conservation gardé.
