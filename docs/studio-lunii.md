# Histoires STUdio et packs Lunii sur la Flam 2.x : note de conception

Issue : #518, épopée : #517. Crate : `crates/studio2lunii` (squelette #524, CI #525). Suites :
#519 (détection au scan), #520 (`story.json` et index), #521 (images et sons).

**Avancement** (voir le découpage du § 8) : fusionnés #524 et #525 (crate et CI), #526 (E),
#527 (#519), #531 (F), #534 (A et B), #537 (C et D), #539 (H), #543, #544, #545 (suivis), #546 (I),
#552 (G1), #555 (G2) et #550 (suivis de F) ; J (interface et documentation, #557, #548) en revue.
Il ne reste que la validation sur une vraie Flam 2.x (§ 9, protocole dans `docs/tests-manuels.md`).
Les paragraphes qui suivent décrivent le comportement du code fusionné quand il diffère de la
conception initiale (écarts signalés).

Ce document fixe, **avant tout code de données**, les formats en jeu, le périmètre, les
invariants et le découpage en PR. Il ne contient **aucune clé**, aucun numéro de série, aucun
contenu d'histoire du commerce. Les formats sont décrits en mots, pour l'interopérabilité.

Lunii.QT (GPL v3, révision `c8afe43`, `pkg/api/device_flam.py`, `stories.py`,
`convert_image.py`, `convert_audio.py`, `aes_keys.py`) a été **lu pour comprendre** le
format. Notre code est écrit à partir de cette note, jamais recopié.

## 1. Statuts

| Statut                  | Sens                                                                                       |
| ----------------------- | ------------------------------------------------------------------------------------------ |
| **observé (Lunii.QT)**  | ce que Lunii.QT écrit sur une Flam 2.x ; le format est utilisé, mais rien n'est mesuré ici |
| **STUdio**              | convention du format STUdio, non lue par Lunii.QT                                          |
| **implémenté**          | comportement actuel du launcher (`main`)                                                   |
| **choix de conception** | décision de cette note                                                                     |
| **à confirmer (#236)**  | à vérifier sur une vraie Flam 2.x ; renvoi au tableau du § 9                               |

Aucune Flam 2.x n'a été mesurée dans ce dépôt (voir `docs/flam-device-format.md`, § 10). Tout
ce qui suit sur l'appareil est donc au mieux « observé (Lunii.QT) ».

## 2. Le format STUdio

### 2.1 Archive

| Entrée          | Rôle                                                                     | Statut             |
| --------------- | ------------------------------------------------------------------------ | ------------------ |
| `story.json`    | graphe de l'histoire et métadonnées, à la racine du zip                  | observé (Lunii.QT) |
| `assets/<nom>`  | images et sons référencés par `story.json`, à plat (pas de sous-dossier) | observé (Lunii.QT) |
| `thumbnail.png` | vignette de l'histoire (PNG)                                             | observé (Lunii.QT) |

Les noms d'assets STUdio sont en général l'empreinte SHA-1 du fichier (40 caractères
hexadécimaux) suivie de l'extension (STUdio). Les assets présents mais non référencés sont
ignorés (comme Lunii.QT).

**Choix de conception** : `story.json` est cherché à la racine ; un zip dont **toutes** les
entrées sont sous un même dossier unique (dossier recompressé par l'utilisateur) est accepté
comme si ce dossier était la racine. Tout autre emplacement est refusé.

### 2.2 `story.json`

Champs racine :

| Champ                | Type            | Usage                                                          | Statut             |
| -------------------- | --------------- | -------------------------------------------------------------- | ------------------ |
| `format`             | texte `"v1"`    | version du format ; le nombre après `v` va dans l'en-tête `ni` | observé (Lunii.QT) |
| `version`            | entier          | version du pack ; en-tête `ni`, version de la bibliothèque     | observé (Lunii.QT) |
| `title`              | texte           | titre (bibliothèque, `info`)                                   | observé (Lunii.QT) |
| `description`        | texte           | description (bibliothèque)                                     | observé (Lunii.QT) |
| `nightModeAvailable` | booléen, absent | absent = faux ; vrai : fichier `nm` vide dans le pack          | observé (Lunii.QT) |
| `stageNodes`         | liste           | nœuds de scène, ordre conservé (indice = numéro de nœud)       | observé (Lunii.QT) |
| `actionNodes`        | liste           | nœuds d'action (listes de choix), ordre conservé               | observé (Lunii.QT) |

Nœud de scène (`stageNodes[i]`) :

| Champ                      | Type                                          | Usage                                                       | Statut             |
| -------------------------- | --------------------------------------------- | ----------------------------------------------------------- | ------------------ |
| `uuid`                     | UUID texte                                    | identifiant du nœud ; celui du nœud d'entrée = UUID du pack | observé (Lunii.QT) |
| `squareOne`                | booléen                                       | nœud d'entrée (couverture)                                  | STUdio             |
| `image`                    | nom d'asset ou `null`                         | image affichée                                              | observé (Lunii.QT) |
| `audio`                    | nom d'asset ou `null`                         | son joué                                                    | observé (Lunii.QT) |
| `okTransition`             | `{actionNode, optionIndex}` ou `null`         | transition du bouton OK (ou fin du son si autoplay)         | observé (Lunii.QT) |
| `homeTransition`           | `{actionNode, optionIndex}` ou `null`         | transition du bouton Maison                                 | observé (Lunii.QT) |
| `controlSettings`          | `{wheel, ok, home, pause, autoplay}` booléens | commandes actives sur ce nœud                               | observé (Lunii.QT) |
| `type`, `name`, `position` | divers                                        | éditeur STUdio ; ignorés                                    | STUdio             |

Nœud d'action (`actionNodes[j]`) : `id` (texte unique), `options` (liste d'UUID de nœuds de
scène, ordre significatif) ; `name`, `position` ignorés.

UUID du pack : Lunii.QT prend celui du **premier** nœud de scène. **Choix de conception** : le
nœud `squareOne` doit être unique ; s'il n'est pas en tête, il est placé en tête (indices
recalculés) ; sans `squareOne`, le premier nœud est l'entrée, comme Lunii.QT. Le firmware
démarre au nœud 0 (déduit : `ni` ne désigne aucune entrée ; à confirmer, L1). Effet des
commandes (`controlSettings`) sur les boutons de la Flam : à confirmer (L15).

### 2.3 Assets acceptés

| Nature | Formats d'entrée                             | Traitement                                                                               |
| ------ | -------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Image  | PNG, JPEG, BMP (crate `image`, #524)         | toujours décodée puis réencodée en BMP 4 bits RLE (§ 3.5)                                |
| Son    | MP3 ; OGG (Vorbis, Opus), WAV, FLAC, AAC/M4A | MP3 conforme gardé (étiquettes retirées), sinon transcodé par le ffmpeg embarqué (§ 3.6) |

La nature d'un fichier se lit à son contenu (signature), pas à son extension.

### 2.4 Variantes refusées explicitement

| Variante                                                       | Message (idée)                                               |
| -------------------------------------------------------------- | ------------------------------------------------------------ |
| STUdio en `.7z`                                                | « archive 7z : décompressez-la puis recompressez-la en zip » |
| `format` autre que `v1`                                        | « format STUdio <x> non pris en charge »                     |
| zip contenant `uuid.bin` (`.plain.pk`) et `story.json`         | « archive ambiguë (STUdio et .plain.pk) »                    |
| `story.json` ailleurs qu'à la racine ou dans un dossier unique | « story.json introuvable à la racine »                       |
| asset dans un sous-dossier de `assets/`, ou absent             | « asset <nom> introuvable »                                  |
| pack Lunii (`ni`, `li`, `ri`, `si`) au lieu de STUdio          | voir § 4                                                     |
| entrée chiffrée (zip à mot de passe), lien symbolique          | « archive protégée ou lien non pris en charge »              |

Le tirage au hasard de STUdio (`optionIndex` à `-1`) n'est **plus** une variante refusée : refusé
avec plusieurs options jusqu'au 13/11/2026, il est accepté depuis par décision du mainteneur
(#608), sans avertissement dans l'interface ; son effet sur une Flam 2.x reste à mesurer (Q5,
L11). Tout autre `optionIndex` hors de `[0, nombre de choix)` reste refusé (§ 6.3).

## 3. Le pack Lunii attendu par la Flam 2.x

### 3.1 Arborescence

Écrite par Lunii.QT dans `str/<uuid>/` (UUID du pack en minuscules, 36 caractères) :

```
str/<uuid>/
├── ni                 index des nœuds (en clair)
├── li                 index des listes de choix (chiffré, 512 premiers octets)
├── ri                 index des images (chiffré, 512 premiers octets)
├── si                 index des sons (chiffré, 512 premiers octets)
├── nm                 vide ; présent si mode nuit (en clair)
├── key                32 octets, déduit du numéro de série (en clair)
├── info               titre (en clair)
├── img/thumbnail.lif  vignette (en clair)
├── rf/000/XXXXXXXX    images BMP (chiffrées, 512 premiers octets)
└── sf/000/XXXXXXXX    sons MP3 (chiffrés, 512 premiers octets)
```

Statut : observé (Lunii.QT) ; lecture par le firmware à confirmer (L1). Le launcher reconnaît
déjà ce format (`guard.rs`, `is_lunii_format`) : absence de `main.lsf` et présence d'un de `ni`,
`li`, `ri`, `si`. Lunii.QT n'écrit pas de fichier `version`.

Tous les entiers de `ni`, `li` et du BMP sont **petit-boutistes** (l'en-tête LIF, lui, est
gros-boutiste).

### 3.2 `ni` : index des nœuds (en clair)

En-tête de `0x200` octets :

| Offset | Taille | Contenu                                                                        | Statut             |
| ------ | ------ | ------------------------------------------------------------------------------ | ------------------ |
| `0x00` | 2      | version du format (`1` pour `"v1"`)                                            | observé (Lunii.QT) |
| `0x02` | 2      | version du pack (`version` de `story.json`)                                    | observé (Lunii.QT) |
| `0x04` | 4      | début de la liste des nœuds : `0x200`                                          | observé (Lunii.QT) |
| `0x08` | 4      | taille d'un nœud : `0x2C` (44)                                                 | observé (Lunii.QT) |
| `0x0C` | 4      | nombre de nœuds de scène                                                       | observé (Lunii.QT) |
| `0x10` | 4      | nombre d'images (entrées de `ri`)                                              | observé (Lunii.QT) |
| `0x14` | 4      | nombre de sons (entrées de `si`)                                               | observé (Lunii.QT) |
| `0x18` | 1      | `1` : constante de Lunii.QT (attribut `factory_pack`) ; sens inconnu, recopiée | observé (Lunii.QT) |
| `0x19` | reste  | octets nuls jusqu'à `0x200`                                                    | observé (Lunii.QT) |

Puis un enregistrement de 44 octets par nœud de scène, dans l'ordre de `stageNodes` :

| Offset | Taille | Contenu                                                                        |
| ------ | ------ | ------------------------------------------------------------------------------ |
| `0x00` | 4      | indice de l'image dans `ri` (signé), `-1` sans image                           |
| `0x04` | 4      | indice du son dans `si` (signé), `-1` sans son                                 |
| `0x08` | 4      | OK : position du nœud d'action dans `li` (premier choix), `-1` sans transition |
| `0x0C` | 4      | OK : nombre de choix de ce nœud d'action, `-1` sans transition                 |
| `0x10` | 4      | OK : `optionIndex`, `-1` sans transition ou pour un tirage au hasard (L11)     |
| `0x14` | 12     | Maison : mêmes trois champs pour `homeTransition`                              |
| `0x20` | 2 × 5  | `wheel`, `ok`, `home`, `pause`, `autoplay` : `1` ou `0` (16 bits chacun)       |
| `0x2A` | 2      | `00 00`                                                                        |

Taille de `ni` : `0x200 + 44 × nombre de nœuds`. Lunii.QT complète par des `AA` un nœud sans
`controlSettings` : **choix de conception**, nous le refusons (champ obligatoire).

### 3.3 `li`, `ri`, `si`

| Fichier | Contenu en clair                                                                                                                                                                                                                                                                           |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `li`    | pour chaque nœud d'action, dans l'ordre de `actionNodes`, l'indice (entier signé 32 bits) de chaque nœud de scène de `options`. La « position dans `li` » d'un nœud d'action est la somme des tailles des listes qui le précèdent. Fichier complété par des nuls jusqu'à 8 octets au moins |
| `ri`    | 12 octets ASCII par image, sans séparateur : `000\` (barre oblique inverse) puis le nom `XXXXXXXX` du fichier de `rf/000/`                                                                                                                                                                 |
| `si`    | idem pour les sons et `sf/000/`                                                                                                                                                                                                                                                            |

Ordre des entrées de `ri` et `si` : ordre de **première apparition** dans `stageNodes` ; un
asset utilisé par plusieurs nœuds n'a qu'une entrée. Statut : observé (Lunii.QT). Histoire sans image ou
sans son (`ri` ou `si` vide) : à confirmer (L12).

### 3.4 Nommage `rf/000/XXXXXXXX`, `sf/000/XXXXXXXX`

Lunii.QT prend les **8 derniers caractères** du nom de l'asset sans extension, en majuscules.
Deux défauts : deux assets de même fin de nom s'écrasent, et un nom de moins de 8 caractères
donne une entrée de `ri` de moins de 12 octets (index décalé).

**Choix de conception** (#520) :

- si, pour **chaque** asset, le nom sans extension fait au moins 8 caractères et que ses **8
  derniers caractères** sont des lettres ou chiffres ASCII (seule la fin du nom compte, pas le
  reste), et si ces 8 derniers caractères en majuscules sont **uniques** (à la casse près, parmi
  les images, puis parmi les sons), on garde la règle de Lunii.QT : parité octet pour octet, comparaisons faciles avec une Flam remplie par
  Lunii.QT ;
- sinon, **tout le pack** est nommé par son rang : `00000000`, `00000001`… (8 chiffres
  hexadécimaux majuscules), dans l'ordre de `ri` (et de `si`).

Les noms produits sont toujours des noms courts 8.3 valides, sans extension. Aucun nom de
l'archive ne sert de chemin de sortie. Lecture des deux règles par le firmware : à confirmer
(L10). Détail de Lunii.QT : un nom de moins de 8 caractères est écrit en minuscules sur le
disque, en majuscules dans `ri` (sans effet sur FAT).

### 3.5 Images (BMP 4 bits RLE)

Observé (Lunii.QT, `image_to_bitmap_rle4`) :

| Propriété  | Valeur                                                                                                                                                                                             |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Dimensions | 320 × 240, image étirée sans garder les proportions                                                                                                                                                |
| Couleurs   | 16 niveaux de gris : `(0,299 R + 0,587 G + 0,114 B) / 16`, tronqué ; palette `i × 255 / 16` (0 à 239) ; `biSizeImage` = taille des données RLE ; résolutions, `biClrImportant` et `bfReserved` à 0 |
| En-têtes   | `BITMAPFILEHEADER` (14) + `BITMAPINFOHEADER` (40) : hauteur positive (lignes de bas en haut), 1 plan, 4 bits, compression 2 (RLE4), 16 couleurs ; données à l'offset 118                           |
| Codage     | par ligne, paires (nombre de pixels 1 à 255, couleur répétée dans les deux quartets) ; `00 00` en fin de ligne sauf la dernière ; `00 01` en fin d'image                                           |

Écarts **choisis** avec Lunii.QT :

- Lunii.QT garde tel quel un BMP déjà RLE4 de 320 × 240 ; nous le décodons et le réencodons
  toujours (un BMP hostile n'atteint jamais l'appareil) ;
- Lunii.QT ne retourne pas une source BMP déjà en 320 × 240 mais pas en RLE4 (image à
  l'envers, probablement un défaut ; une BMP redimensionnée, elle, est retournée) ; nous
  écrivons toujours les lignes de bas en haut ;
- transparence : composition sur fond noir (Lunii.QT ignore l'alpha).

Le redimensionnement de Lunii.QT est le bicubique de Pillow ; le crate le refait à l'octet près
(coefficients en virgule fixe, passes horizontale puis verticale ; plus proche voisin pour les
images à palette ou en noir et blanc 1 bit, comme Pillow) : le BMP produit est **identique à
l'octet** à celui de Lunii.QT, sauf pour une source JPEG (le décodeur diffère de libjpeg) où
l'en-tête et la structure RLE sont identiques et les niveaux égaux à ± 1 sur au moins 97 % des
pixels (§ 7). Palette couleur possible sur Flam : à confirmer (L7), hors périmètre.

BMP déjà RLE4 de 320 × 240 (comportement du code fusionné, #544) : il est décodé en **indices
de palette** (16 niveaux préservés) puis réencodé. Sont refusés un saut (delta), une ligne
incomplète et un nombre de lignes différent de 240, là où Lunii.QT recopierait le fichier. La
palette du fichier est ignorée (hypothèse : l'appareil lit les indices comme des niveaux de
gris ; à mesurer, L7).

### 3.6 Sons (MP3)

Observé (Lunii.QT, `convert_audio.py`) :

- un fichier dont l'extension n'est pas `.mp3` est transcodé ;
- un MP3 est transcodé s'il est échantillonné **sous** 44 100 Hz, s'il n'est pas **mono** ou
  si son mode de débit est ABR ; sinon il est gardé ;
- transcodage : `libmp3lame`, 44 100 Hz, mono, qualité VBR 5, sans métadonnées, **sans en-tête
  Xing**, sans ID3v2 ;
- les étiquettes (ID3v1, ID3v2) d'un MP3 gardé sont retirées.

**Choix de conception** (#521) : un MP3 est gardé seulement s'il est MPEG-1 couche III, à
44 100 Hz exactement, mono, que sa première trame commence à l'octet 0 une fois les étiquettes
retirées (des octets quelconques avant elle, par exemple des zéros de bourrage, forcent le
transcodage, comme des octets en fin de fichier) et que ses trames se lisent de bout en bout ; sinon il est transcodé
avec les réglages ci-dessus. Écarts voulus avec Lunii.QT, qui accepte `>= 44 100 Hz` et toute
couche : un MP3 à 48 kHz est transcodé, et un MP3 de couche I ou II aussi ; 44,1 kHz est la seule
fréquence connue de la Flam (`telmi2flam`, `SAMPLERATE`). Étiquettes retirées : ID3v2 en tête
(**y compris plusieurs étiquettes empilées** et pied `3DI` d'une 2.4), ID3v1 (`TAG`, 128 octets)
et APEv2 en fin ; `mutagen` ne retire ni l'APEv2, ni le pied 2.4, ni les étiquettes empilées. Le crate n'appelle pas ffmpeg : il
dit « garder » ou « transcoder » ; le launcher lance son ffmpeg LGPL embarqué (`ffmpeg.rs`,
encodeur `libmp3lame`, décodeurs MP3, Vorbis, Opus, FLAC, AAC, PCM). VBR sans Xing : à
confirmer (L6).

### 3.7 Chiffrement

Observé (Lunii.QT, `__get_ciphered_data`, `aes_cipher`, `reverse_bytes`) :

| Propriété      | Histoire Lunii sur Flam 2.x                                                                                                                                         |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Algorithme     | AES-128-CBC                                                                                                                                                         |
| Clé, IV        | clé (`0x4E`) et IV (`0x5E`) de `.mdf`, **chacun passé par l'inversion par mots** ci-dessous                                                                         |
| Portée         | les **512 premiers octets** seulement ; le reste du fichier est en clair                                                                                            |
| Fichiers       | `li`, `ri`, `si`, `rf/000/*`, `sf/000/*`                                                                                                                            |
| En clair       | `ni`, `nm`, `key`, `info`, `img/thumbnail.lif`                                                                                                                      |
| Fichier court  | moins de 512 octets : tout le fichier est chiffré ; s'il n'est pas multiple de 16, il est d'abord complété par des nuls (sa taille change). Fichier vide : inchangé |
| IV par fichier | le même IV pour chaque fichier (CBC repart de l'IV à chaque fichier)                                                                                                |

**Inversion par mots** (`reverse_bytes`) : les 16 octets sont lus comme 4 mots de 4 octets ;
l'ordre des mots est gardé, l'ordre des octets **dans** chaque mot est inversé (passage d'un
mot 32 bits d'un boutisme à l'autre). Exemple avec des octets inventés :

```
entrée : 00 01 02 03  04 05 06 07  08 09 0A 0B  0C 0D 0E 0F
sortie : 03 02 01 00  07 06 05 04  0B 0A 09 08  0F 0E 0D 0C
```

Comparaison avec notre `crypto.rs` (format Flam, implémenté) :

| Point        | Format Flam (`crypto.rs`)                     | Format Lunii (Lunii.QT)                            | Écart à combler                      |
| ------------ | --------------------------------------------- | -------------------------------------------------- | ------------------------------------ |
| Clé, IV      | octets de `.mdf` tels quels                   | inversés par mots                                  | nouvelle fonction                    |
| Portée       | fichier entier                                | 512 premiers octets                                | nouvelle fonction                    |
| Fichiers     | `*.lsf`, `info`                               | `li`, `ri`, `si`, `rf/*`, `sf/*`                   | table propre au format               |
| Bourrage     | nuls jusqu'au multiple de 16                  | idem, seulement sous 512 octets                    | cas couvert par la nouvelle fonction |
| `key`        | 14 + 10 + 8 octets, depuis le numéro de série | **identique** (même fichier pour les deux formats) | aucun                                |
| `info`       | chiffré, 4 lignes                             | **en clair**, 5 lignes (§ 3.8)                     | écriture en clair                    |
| Firmware 1.x | mode carrier                                  | refusé (« mettez à jour en 2.x »)                  | refus avant écriture                 |

La clé « inversée » ne doit jamais sortir de `device/` : elle est calculée dans `crypto.rs` au
moment du chiffrement et effacée (`zeroize`) comme les autres. Les clés carrier (1.x) ne
servent **jamais** au format Lunii : `DeviceKeys` mémorise sa famille (champ à ajouter, PR E)
et la fonction Lunii refuse `V1` (`UnsupportedFirmware`). Le tout est à confirmer sur appareil (L2, L3).

Un pack Lunii « v2 » est chiffré autrement (XXTEA avec une clé générique publiée par
Lunii.QT) : ce format n'est pas repris (§ 4).

### 3.8 `info`, vignette, `nm`, `key`

| Fichier             | Contenu                                                                                                                                                                      | Statut                                          |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| `info`              | en clair, 5 lignes : titre, titre, `0`, titre, auteur (sans fin de ligne finale). Lunii.QT translittère en ASCII (`unidecode`) ; sous Windows il écrit des CRLF (mode texte) | observé (Lunii.QT) ; affichage à confirmer (L4) |
| `img/thumbnail.lif` | LIF, en clair. Lunii.QT : `thumbnail.png` réduit à 104 × 104 au plus, proportions gardées                                                                                    | observé (Lunii.QT)                              |
| `nm`                | fichier vide si `nightModeAvailable`                                                                                                                                         | observé (Lunii.QT) ; effet à confirmer (L8)     |
| `key`               | celui des histoires Flam en 2.x (`key_file_from_serial`)                                                                                                                     | observé (Lunii.QT), implémenté                  |

**Choix de conception** :

- `info` : fins de ligne LF ; contenu de Lunii.QT pour le premier essai sur appareil (titre de
  `story.json` translittéré en ASCII, ligne d'auteur `No authors.` comme Lunii.QT). Les
  variantes (UTF-8, sans ligne d'auteur) seront essayées ensuite (question Q4). Titre et
  description sont d'abord assainis (contrôles et blancs réduits, caractères Unicode de format
  invisibles retirés : contrôles bidirectionnels U+202A–U+202E, U+2066–U+2069, U+200E/F,
  espaces sans chasse, BOM, #553) ; un titre absent ou sans lettre ni chiffre une fois
  translittéré devient « Sans titre » (écart avec Lunii.QT, qui écrirait une ligne vide) ;
- vignette : notre encodeur LIF (`telmi2flam::lif`, utilisé sur une Flam 1.15.14) au format des
  vignettes Flam, 104 × 136 (`telmi2flam::image`, même remplissage que les histoires Telmi).
  Source : `thumbnail.png`, sinon l'image du nœud d'entrée, sinon pas de vignette (Lunii.QT
  échoue sans `thumbnail.png`). Taille à confirmer (L5). Écarts avec Lunii.QT (#552) : la
  source peut être un PNG, un JPEG ou un BMP (reconnu au contenu, dimensions bornées avant
  décodage) ; un JPEG ou un BMP est décodé puis réencodé en PNG avant la réduction. Une source
  illisible, trop lourde ou trop grande est un **avertissement** (repli sur l'image du nœud
  d'entrée, puis aucune vignette), jamais un refus de l'histoire.

### 3.9 Index de la bibliothèque

Lunii.QT réécrit `etc/library/list` et `list.hidden` en entier (UUID en minuscules, LF) et
supprime `usr/0/library.cache`. Le launcher fait déjà mieux : ajout en fin de `list` par
`write_index` (écriture sûre, reprise), puis suppression du cache (`docs/flam-device-format.md`,
§ 5 et § 8.1). **Rien à changer** pour le format Lunii : `list` ne distingue pas les formats. Reconstruction de `library.cache` avec des histoires des
deux formats : à confirmer (L14).

## 4. Périmètre recommandé

| Format d'entrée                                           | Recommandation        | Raison                                                                                                                                                                                                                             |
| --------------------------------------------------------- | --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **STUdio zip**                                            | **maintenant**        | la demande des utilisateurs ; histoires créées par des particuliers ; format ouvert et documenté par l'éditeur STUdio                                                                                                              |
| STUdio 7z                                                 | refus, message clair  | aucune lecture 7z dans le launcher (nouvelle dépendance et inventaire de licences) ; contournement simple : recompresser en zip                                                                                                    |
| Pack Lunii « plain » (`.plain.pk` avec `ni`, `li.plain`…) | refus pour l'instant  | en pratique des histoires du commerce déchiffrées ; règle du dépôt : on ne manipule pas les histoires officielles. Notre format de bibliothèque en est proche (§ 5.2) : facile à ajouter plus tard si le mainteneur le décide (Q2) |
| Pack Lunii v2 (XXTEA)                                     | refus                 | déchiffrement par une clé générique du fabricant : contournement d'une protection, hors règles du projet                                                                                                                           |
| Pack Lunii v3 (AES, `bt` de 32 octets)                    | refus                 | clés propres à un appareil Lunii ; Lunii.QT ne les importe pas non plus sur une Flam                                                                                                                                               |
| Toute entrée sur une Flam 1.x                             | refus, avant écriture | le firmware 1.x ne lit pas le format Lunii (Lunii.QT : « Please update your Flam with v2.x.x »)                                                                                                                                    |

Messages (idée) : « Pack Lunii non pris en charge : seules les histoires STUdio (story.json)
peuvent être installées » ; « Les histoires STUdio demandent une Flam en firmware 2.x :
cette Flam ne peut pas les lire » (sans inviter à mettre à jour le firmware pendant la bêta).

## 5. Intégration dans le launcher

### 5.1 Vue d'ensemble

```
dossier (scan, #519)        installation (bibliothèque)             envoi (Flam 2.x)
StudioZip ──► studio2lunii ──► library/<uuid>-<v>.lunii.pk ──► prepare_lunii ──► send_with (inchangé)
              (pur, sans clé)   (pack Lunii en clair)            (chiffrement, key)
```

Même découpage que les histoires Telmi : conversion **une fois** à l'installation (lente :
images, ffmpeg), chiffrement **à chaque envoi** (propre à l'appareil, rapide).

### 5.2 Où convertir

- **`crates/studio2lunii`** (pur, #524) : `story` (lecture et validation de `story.json`),
  `index` (`ni`, `li`, `ri`, `si`, noms), `bitmap` (BMP RLE4), `audio` (analyse MP3, retrait des
  étiquettes, décision « garder ou transcoder »). Aucune clé, aucun fichier, aucun processus :
  entrées en octets, sorties en octets.
- **Launcher, `install/studio.rs`** (nouveau) : lecture du zip (bornes, noms), appel de la
  crate, transcodage par `ffmpeg.rs`, vignette (`telmi2flam`), écriture du pack clair dans
  `library/.staging/` puis publication par le chemin habituel (`library.rs`).
- **Launcher, `device/prepare_lunii.rs`** (nouveau) : du pack clair au dossier `str/<uuid>/`
  chiffré, dans le dossier de travail ; produit un `PreparedStory`.

Format de la bibliothèque : `library/<uuid>-<version>.lunii.pk`, zip « stored » :

| Entrée                      | Contenu                                                                                  |
| --------------------------- | ---------------------------------------------------------------------------------------- |
| `uuid.bin`                  | UUID du pack, 16 octets (`status.rs`, `uuid_bin_of`, une fois `Library::checked` étendu) |
| `_metadata.json`            | titre, description, version, format source                                               |
| `_thumbnail.png`            | `thumbnail.png` d'origine, seulement s'il a servi de vignette et est un vrai PNG         |
| `ni`, `nm` (si nuit)        | comme sur l'appareil                                                                     |
| `li`, `ri`, `si`            | **en clair**                                                                             |
| `rf/000/*`, `sf/000/*`      | **en clair**                                                                             |
| `info`, `img/thumbnail.lif` | comme sur l'appareil (en clair)                                                          |

L'extension `.lunii.pk` (et non `.plain.pk`) garantit qu'aucun code qui attend une histoire Flam
(player, `prepare.rs`, scan) ne prend ce fichier pour une histoire Flam.

### 5.3 `StoryKind`

#519 ajoute le type STUdio (nom proposé : `StudioZip`), installation refusée. Les PR suivantes :

- `install/mod.rs` : branche `StudioZip` de `transform` (appel de `install/studio.rs`) et de
  `fetch` (extension `zip`) ;
- `library.rs` : `.plain.pk` y est codé en dur à plusieurs endroits : `file_name_for(uuid,
version, format)`, `Library::checked` (accepter `.plain.pk` et `.lunii.pk` ; il sert à
  `final_path`, à l'envoi, à `archive_uuid` de `status.rs` et au player), réconciliation (UUID
  déduit selon l'extension) ; un champ dérivé « format sur l'appareil » (`flam` ou `lunii`)
  exposé au front ;
- `library_thumb.rs` : suffixe du nom de la vignette ;
- `library_source.rs` : empreinte de la source (`file_digest`, comme `TelmiZip`) ;
- `folder.rs` : lever le refus d'installation posé par #519.

### 5.4 Préparation et transfert

`prepare_lunii::prepare(pack, keys, work_dir, limits, cancel) -> PreparedStory` :

1. relit le pack clair (bornes de `prepare::Limits`, mêmes contrôles de noms : la bibliothèque
   peut avoir été modifiée à la main) ; refuse toute entrée inattendue, y compris un fichier de
   `rf/000/` ou `sf/000/` que `ri` ou `si` ne référence pas ;
2. contrôle la cohérence, **plus strictement que la conception initiale** (toute la structure est
   contrôlée avant la moindre écriture ; le contenu des assets et de la vignette l'est à la copie,
   dans le chantier, supprimé en cas d'échec) :
   - `ni` : en-tête (version de format 1 à `0x00`, début `0x200`, nœuds de `0x2C` octets ; octet
     `0x18` **exigé à 1** et octets `0x19` à `0x1FF` nuls : le sens de `0x18` est inconnu, un
     pack tiers à 0 serait donc refusé), de 1 à 20 000 nœuds, taille exacte,
     nombres d'images et de sons égaux aux entrées de `ri` et `si`, indices d'image et de son
     dans leur plage, transitions cohérentes avec `li`, commandes bien formées ;
   - `li` : entiers de 4 octets, chacun indice d'un nœud (au plus 100 000 entrées) ; `ri` et
     `si` : entrées de 12 octets (`000\` et 8 caractères ASCII alphanumériques), sans doublon,
     au plus 10 000 ; chaque nom présent dans `rf/000/` ou `sf/000/`, et aucun fichier de ces
     dossiers absent de `ri` ou `si` (refus) ;
   - `nm` : s'il est présent, **vide** ; `info` : ASCII imprimable, lignes séparées par LF,
     titre non vide, au plus 64 Kio (variantes UTF-8 non essayées : § 8, « plus tard ») ;
   - entrée zip chiffrée (mot de passe) : refusée (`LuniiInvalid`) avant toute écriture ;
   - dépassement d'une borne (entrées de `li`, `ri`, `si`, tailles déclarées ou lues) :
     `LuniiTooLarge` ; tout autre défaut, y compris un nombre de nœuds hors de 1 à 20 000 :
     `LuniiInvalid` ;
3. écrit `ni`, `nm`, `info`, `img/thumbnail.lif` tels quels ; `li`, `ri`, `si`, `rf/*`, `sf/*`
   avec chiffrement de l'en-tête (§ 3.7) ; `key` depuis l'appareil ;
4. même chantier `.partiel-<uuid>-<aléa>` renommé à la fin, même nettoyage que `prepare.rs`
   (fonctions partagées rendues `pub(crate)`, pas de copie).

Le `PreparedStory` passe **tel quel** par `transfer::send_with` : dossier temporaire
`.flam-transfer/`, relecture SHA-256, promotion par renommage, index, cache. Aucune ligne de
`send_with` ne change.

`send_from_library` choisit la préparation selon le suffixe `.lunii.pk` du nom du fichier de
bibliothèque (casse indifférente) : `prepare_lunii::prepare` pour un pack Lunii,
`prepare::prepare_story` sinon.

### 5.5 Contrôle du firmware

Ordre, pour une histoire Lunii, dans `send_from_library` :

1. `check_write` (lecture seule, inchangé) ;
2. **`permit.family() == V2`, sinon `TransferError::LuniiNeedsV2`** (erreur `lunii_needs_v2`,
   avec son message) : refus juste après `check_write`, avant les clés carrier, la préparation et
   toute écriture, donc pas même la reprise d'un transfert interrompu ;
3. clés de `.mdf`, préparation, `send_with`.

Le front grise « Envoyer » pour une histoire Lunii quand la Flam branchée est en 1.x, avec le
message du § 4. Le contrôle du backend reste la seule garantie.

### 5.6 Lecture sur le PC

flam-player ne lit que le format Flam (Lua). **Recommandation** : bouton « Lire » désactivé pour
une histoire Lunii, avec l'infobulle « Lecture sur le PC indisponible pour une histoire
STUdio ». Garde dans `player.rs` (refus explicite si le fichier n'est pas un `.plain.pk`). Un
lecteur de packs Lunii, ou une conversion STUdio vers le format Flam, serait une autre épopée
(Q1).

### 5.7 Écran « Ma Flam »

Des histoires au format Lunii existent déjà sur des Flam 2.x (installées par Lunii.QT). Statut :
implémenté (#522, #531) ; écarts avec la conception initiale signalés ci-dessous.

| Élément   | Comportement (implémenté)                                                                                                                                                                                                                                |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Liste     | dossiers `str/<uuid>/` et index, format indifférent                                                                                                                                                                                                      |
| Format    | champ `format` (`flam` ou `lunii`) porté par `StoryTitle` (réponse de `device_story_titles`), **pas** par `DeviceStory` ; deux valeurs seulement, pas de valeur `inconnu` ; badge discret côté interface (`lunii`)                                       |
| Critère   | `guard::is_lunii_format`, partagé avec le compte `lunii_format` du contrôle de cohérence : **au moins un** des 4 index `ni`, `li`, `ri`, `si` présent et pas de `main.lsf` (un dossier incomplet est donc quand même Lunii)                              |
| Titre     | dossier Lunii : `info` lu **en clair**, sans clé (donc aussi en 1.x sans clé carrier) : première ligne non vide, UTF-8 ou Latin-1, lecture bornée, espaces réduits, 200 caractères au plus ; ligne avec caractère de contrôle : `None`. Jamais déchiffré |
| Titre     | dossier Flam : `info` déchiffré avec les clés Flam ; échec ou clé absente : titre de repli (bibliothèque, puis UUID court)                                                                                                                               |
| Vignette  | `img/thumbnail.lif`, non chiffrée                                                                                                                                                                                                                        |
| Cohérence | format Lunii écarté (`guard.rs`, `lunii_format`) ; en option plus tard : preuve de clé par le premier bloc de `ri` (`000\` attendu une fois déchiffré avec la clé inversée)                                                                              |

### 5.8 Doublons d'UUID

- Sur la Flam : inchangé (`AlreadyInstalled`, « Remplacer » lié à l'UUID).
- Une histoire Telmi tirée d'un pack STUdio garde souvent **le même UUID** : sa version Flam et
  sa version Lunii visent le même `str/<uuid>/`. « Remplacer » change alors de format : le front
  le dit (« remplace la version au format Flam »). Les progressions `usr/0/<uuid>.*` ne sont pas
  touchées (inchangé) ; leur compatibilité entre formats est inconnue (L13).
- Dans la bibliothèque : **une entrée par identifiant** (règle actuelle de `library.rs`).
  L'identifiant d'une histoire STUdio est l'UUID de son nœud d'entrée sous sa forme canonique
  en minuscules (`8-4-4-4-12`, règle de `studio2lunii`, #553), le même au scan, dans la
  bibliothèque et sur la Flam (`str/<uuid>/`) ; celui d'une histoire Telmi est son champ
  `uuid`, souvent de la même forme quand elle vient d'un pack STUdio. Le remplacement dans la
  bibliothèque n'a lieu que si les deux identifiants coïncident, et seulement après
  confirmation : le front la demande avant l'installation (Q10), le backend refuse un
  changement de format non confirmé (`replace_format`, #554). Sinon les deux entrées
  coexistent et la collision apparaît à l'envoi (`AlreadyInstalled`, même `str/<uuid>/`).

### 5.9 Retrait et ordre

`remove.rs` et `reorder.rs` ne regardent pas le contenu des dossiers ; les tests de retrait
utilisent déjà `ni` et `rf/000/…`. Rien à changer, un test de bout en bout avec un pack complet
suffit.

## 6. Invariants et pannes

### 6.1 Invariants

1. Une histoire Lunii n'écrit **rien** sur une Flam qui n'est pas en 2.x (pas même une reprise).
2. Aucune clé hors de `device/` : la crate et l'installation n'en voient jamais ; la clé inversée
   est effacée après usage ; jamais journalisée.
3. Le pack de la bibliothèque est indépendant de l'appareil et en clair ; le chiffrement n'a lieu
   que dans le dossier de travail de l'envoi.
4. Le transfert garde tous les invariants de `send_with` (dossier temporaire, relecture, index
   jamais vers du vide).
5. Aucun nom de l'archive ne devient un chemin de sortie : les noms produits sont générés.
6. Toute référence du graphe est résolue avant d'écrire quoi que ce soit ; aucun `-1` dans `li`
   ni dans un indice d'image ou de son présent ; `-1` seulement pour une transition absente (ses
   trois champs) ou, dans le seul champ d'option, pour un tirage au hasard entre au moins deux
   options (Q5, L11, #608).
7. Conversion déterministe : mêmes entrées, mêmes octets (tests, empreintes).
8. Coût borné et linéaire : aucune récursion pilotée par l'entrée ; les cycles du graphe sont
   **permis** (menus qui reviennent au début) et n'impliquent aucun parcours.

### 6.2 Bornes (choix de conception, ajustables)

| Borne                                | Valeur                                               |
| ------------------------------------ | ---------------------------------------------------- |
| Entrées du zip (annuaire)            | 100 000                                              |
| Total décompressé (déclaré, puis lu) | 4 Gio                                                |
| `story.json`                         | 16 Mio, profondeur JSON 128 (limite de `serde_json`) |
| Nœuds de scène, nœuds d'action       | 20 000 chacun                                        |
| Entrées de `li`                      | 100 000                                              |
| Images, sons distincts               | 10 000 chacun                                        |
| Image source                         | 32 Mio, 40 mégapixels décodés                        |
| Son source                           | 512 Mio                                              |
| Délai ffmpeg par fichier             | 120 s (comme `telmi2flam`)                           |
| Délai ffmpeg cumulé, tous les sons   | 1 h (`FFMPEG_TOTAL_BUDGET`)                          |
| Pré-analyse des sons                 | 64 Kio lus en tête de chaque MP3                     |
| Sons tenus en mémoire                | un à la fois, sans copie ; non-MP3 copié en flux     |

Les tableaux `stageNodes` et `actionNodes` et les `options` sont lus avec un plafond de
`MAX + 1` éléments (le surplus est compté sans être alloué : mémoire bornée) ; les numéros de
nœud et d'action des messages d'erreur sont comptés à partir de 1.

ffmpeg ne reçoit que le protocole `file` (`-protocol_whitelist`) et les démuxeurs des formats du
§ 2.3 (`-format_whitelist mp3,ogg,wav,flac,aac,mov`) : le format reste détecté au contenu, un
autre (Matroska, par exemple, présent dans notre build pour les podcasts) est refusé.

### 6.3 Tableau des pannes

| Situation                                                                                 | Détection                         | Comportement                                                      |
| ----------------------------------------------------------------------------------------- | --------------------------------- | ----------------------------------------------------------------- |
| Chemin hostile (`..`, absolu, `\`, lecteur, nom réservé Windows, doublon à la casse près) | contrôle des noms avant lecture   | refus de l'archive, rien d'écrit                                  |
| Bombe zip (tailles déclarées fausses, ratio énorme)                                       | bornes sur les tailles **lues**   | arrêt de la lecture, refus                                        |
| Image bombe (petit fichier, énorme surface)                                               | dimensions lues avant décodage    | refus                                                             |
| `story.json` invalide, trop gros, trop profond                                            | analyse bornée                    | refus avec le champ fautif                                        |
| Référence absente (action, nœud, asset)                                                   | tables d'index avant toute sortie | refus nommant la référence                                        |
| UUID de nœud ou `id` d'action en double                                                   | idem                              | refus                                                             |
| `optionIndex` hors de `[0, nombre de choix)`                                              | idem                              | refus ; `-1` accepté : converti en `0` avec une seule option, gardé (`-1` dans `ni`) avec plusieurs (Q5, #608) |
| Transition vers une action sans choix                                                     | idem                              | refus                                                             |
| Plusieurs `squareOne`                                                                     | idem                              | refus                                                             |
| Cycles                                                                                    | aucune                            | acceptés (normaux)                                                |
| Asset non décodable, son refusé par ffmpeg                                                | décodage                          | refus (jamais d'asset sauté en silence)                           |
| ffmpeg absent, empreinte fausse                                                           | pré-analyse, avant la conversion  | refus de l'installation, message                                  |
| Délai de ffmpeg dépassé (par son ou budget cumulé)                                        | `ffmpeg.rs`, `install/studio.rs`  | refus de l'installation (`TimedOut`)                              |
| Vignette illisible, trop lourde ou trop grande                                            | en-tête lu avant décodage         | avertissement, repli (§ 3.8)                                      |
| Arrêt pendant l'installation                                                              | chantier `library/.staging/`      | réconciliation existante de `library.rs` ; bibliothèque inchangée |
| Arrêt pendant la préparation                                                              | chantier `.partiel-`              | supprimé au prochain envoi (`cleanup_stale`)                      |
| Arrêt pendant le transfert                                                                | `.flam-transfer/`                 | reprise existante (`docs/flam-device-format.md`, § 8.1)           |
| Pack de bibliothèque abîmé ou modifié à la main                                           | relecture par `prepare_lunii`     | refus de l'envoi                                                  |
| Firmware 1.x                                                                              | `permit.family()`                 | refus avant toute écriture                                        |
| Firmware 3.x, `.mdf` illisible                                                            | `check_write`                     | refus (inchangé)                                                  |
| Firmware 2.x qui ne lit pas le format (version ancienne ?)                                | aucune côté PC                    | histoire illisible ; retrait possible ; à mesurer (L9)            |
| Même UUID déjà sur la Flam, autre format                                                  | `AlreadyInstalled`                | « Remplacer » explicite seulement                                 |
| Espace insuffisant                                                                        | `send_with`                       | refus (inchangé)                                                  |

## 7. Stratégie de test

- **Unitaires, crate** : vecteurs écrits à la main à partir de cette note (en-tête `ni`, nœud
  de 44 octets, `li` avec plusieurs actions, `ri` et `si`, nommage des deux règles, BMP RLE4 d'une
  image unie et d'un damier, ligne de 255 pixels et plus, MP3 avec ID3v2, ID3v1 et APE, trames
  stéréo ou 48 kHz → « transcoder »). Tous les cas du tableau des pannes ont leur test.
- **Poids des fixtures** : les fichiers sous `crates/studio2lunii/tests/data` pèsent environ
  1,6 Mo ; ce poids est à surveiller.
- **Fixtures STUdio synthétiques** générées par les tests (graphe, PNG et MP3 produits en
  mémoire) ; **aucune histoire du commerce ni pack STUdio tiers dans le dépôt**.
- **Chiffrement** : vecteurs NIST pour AES, vecteurs inventés pour l'inversion par mots,
  fichiers de 0, 15, 16, 511, 512, 513 octets ; contrôle que les octets après 512 sont intacts.
- **Parité avec Lunii.QT (oracle hors dépôt)** : un script Python 3.12 dans le dossier de travail
  de l'agent (jamais dans le dépôt) importe `StudioStory` et `image_to_bitmap_rle4` de Lunii.QT
  en remplaçant PySide6, `requests`, `xxtea` et `py7zr` par des modules vides, avec Pillow et
  pycryptodome dans un environnement virtuel jetable. Il produit `ni`, `li`, `ri`, `si`, les BMP
  et le chiffrement pour les mêmes fixtures synthétiques. Attendus : `ni`, `li`, `ri`, `si`
  identiques à l'octet (règle de nommage de Lunii.QT) ; BMP identiques à l'octet, sauf JPEG (en-tête et
  structure RLE identiques, niveaux à ± 1 sur au moins 97 % des pixels) ; chiffrement
  identique à l'octet ; MP3 : mêmes décisions, aux écarts voulus du § 3.6 près (48 kHz, couches
  I et II, étiquettes vides, APE, empilées ou à pied 2.4). Fixtures MP3 seulement, ou ffmpeg dans le `PATH` de l'oracle (sinon
  `StudioStory` déclare l'histoire incompatible). Les résultats sont donnés dans la PR, pas les
  sources de l'oracle.
- **Launcher** : préparation Lunii sur faux appareil, injection de panne à chaque étape (comme
  `prepare.rs` et `transfer.rs`), refus 1.x sans aucune écriture (arborescence comparée avant et
  après), titre en clair dans « Ma Flam », retrait d'un pack complet.
- **Appareil** : liste L1 à L15 du § 9, par le mainteneur ou un testeur 2.x (#316).

## 8. Découpage en PR

Chaque PR : un thème, cinq changements au plus. Modèle : **opus** pour le code de données, de
format ou de transfert ; **sonnet** ailleurs. #519 (détection au scan, `folder.rs`, `story.rs`)
est fait en parallèle (fusionné, #527) et n'est pas repris ici. Colonne « État » : situation au
7 novembre 2026.

| PR  | État             | Thème                                                                                                                                                          | Fichiers                                                                                              | Dépend de      | Modèle                                                                                         |
| --- | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | -------------- | ---------------------------------------------------------------------------------------------- |
| A   | fusionnée (#534) | `story` : lecture et validation de `story.json` (#520)                                                                                                         | `crates/studio2lunii/src/story.rs`                                                                    | #524           | opus                                                                                           |
| B   | fusionnée (#534) | `index` : `ni`, `li`, `ri`, `si`, nommage (#520)                                                                                                               | `crates/studio2lunii/src/index.rs`                                                                    | A (types)      | opus                                                                                           |
| C   | fusionnée (#537) | `bitmap` : décodage, 320 × 240, gris, RLE4 (#521)                                                                                                              | `crates/studio2lunii/src/bitmap.rs`                                                                   | #524           | opus                                                                                           |
| D   | fusionnée (#537) | `audio` : analyse MP3, étiquettes, décision (#521)                                                                                                             | `crates/studio2lunii/src/audio.rs`                                                                    | #524           | opus                                                                                           |
| E   | fusionnée (#526) | Chiffrement Lunii : inversion par mots, 512 octets, refus carrier                                                                                              | `apps/.../device/crypto.rs`                                                                           | aucune         | opus                                                                                           |
| F   | fusionnée (#531) | « Ma Flam » : titre en clair, champ `format`                                                                                                                   | `device/details.rs`, `device/status.rs`, `src/features/device/` (badge)                               | aucune         | sonnet                                                                                         |
| G1  | fusionnée (#552) | Installation (1/2) : conversion, ffmpeg, vignette, écriture du pack clair `.lunii.pk`                                                                          | `install/studio.rs` (nouveau)                                                                         | A, B, C, D     | opus                                                                                           |
| G2  | fusionnée (#555) | Installation (2/2) : bibliothèque et branchement                                                                                                               | `install/mod.rs`, `library.rs`, `library_thumb.rs`, `library_source.rs`, `folder.rs` (levée du refus) | G1, #519       | opus                                                                                           |
| H   | fusionnée (#539) | Préparation Lunii                                                                                                                                              | `device/prepare_lunii.rs` (nouveau), `device/prepare.rs` (fonctions partagées), `device/mod.rs`       | E, G1 (format) | opus                                                                                           |
| I   | fusionnée (#546) | Envoi : choix de la préparation, refus 1.x, erreurs                                                                                                            | `device/transfer.rs`, `error.rs`, messages du front                                                   | H              | opus                                                                                           |
| J   | en revue (#557)  | Interface et documentation : « Lire » désactivé, envoi grisé en 1.x, README, `apps/flam-launcher/docs/flam-transfert.md`, § 10 de `docs/flam-device-format.md` | `player.rs` (garde), `src/features/library/`, `src/features/device/`, docs                            | G2, I          | sonnet (garde de `player.rs` : exception justifiée dans la PR, aucune écriture ni suppression) |

Parallélisme : A, C, D, E, F tout de suite (fichiers disjoints) ; B après A ; G1 après A à D ;
G2 après G1 et #519 ; H après E et G1 ; I puis J. G1 et H peuvent se recouvrir si le format du
§ 5.2 est pris comme contrat (H teste avec des packs construits à la main). G2 touche les
données de l'utilisateur (bibliothèque) : note de conception dans la PR et test d'injection de
panne obligatoires (CONTRIBUTING § 5).

#519 ajoute une variante à `StoryKind` : les `match` exhaustifs (`install/mod.rs`,
`library_source.rs`, `folder.rs`) y reçoivent une branche de refus ; G2 la remplace.

Plus tard (issues à ouvrir si besoin) : preuve de clé par `ri` dans `guard.rs` ; essai des
variantes de `info` ; STUdio 7z.

## 9. Points à confirmer sur appareil (#236)

| N°  | Point                                                                                    | Attendu (Lunii.QT)  |
| --- | ---------------------------------------------------------------------------------------- | ------------------- |
| L1  | Une Flam 2.x lit un pack `ni`/`li`/`ri`/`si` installé par le launcher, après redémarrage | se lance            |
| L2  | Clé et IV de `.mdf` inversés par mots pour le format Lunii (bruts pour le format Flam)   | lu                  |
| L3  | Chiffrement des 512 premiers octets seulement ; fichiers courts complétés                | lu                  |
| L4  | `info` en clair : titre affiché ; LF accepté ; accents (UTF-8) ; rôle des 5 lignes       | titre lisible       |
| L5  | Vignette 104 × 136 d'une histoire Lunii                                                  | affichée            |
| L6  | MP3 VBR sans en-tête Xing, 44,1 kHz mono : lecture, durée, avance                        | lu                  |
| L7  | Images 16 niveaux de gris ; une palette couleur serait-elle acceptée ?                   | gris affichés       |
| L8  | `nm` vide : mode nuit proposé                                                            | proposé             |
| L9  | Plus ancienne version 2.x qui lit le format Lunii                                        | toute 2.x (supposé) |
| L10 | Noms `rf/000/` par rang (`00000000`…) aussi bien lus que les noms de Lunii.QT            | indifférent         |
| L11 | `optionIndex` à `-1` (tirage au hasard de STUdio) écrit tel quel dans `ni` ; accepté depuis le 13/11/2026 par décision du mainteneur (#608), **non mesuré** sur une Flam 2.x, essai confié à un testeur (fiche M12 de `docs/tests-manuels.md`) | inconnu (une option tirée au hasard, si le firmware suit STUdio) |
| L12 | Histoire sans image ou sans son (`ri` ou `si` vide)                                      | inconnu             |
| L13 | Même UUID passé du format Flam au format Lunii : progressions `usr/0/`                   | inconnu             |
| L14 | `library.cache` reconstruit avec des histoires des deux formats                          | reconstruit         |
| L15 | Commandes `wheel`, `ok`, `home`, `pause`, `autoplay` sur les boutons de la Flam          | conformes           |

## 10. Questions ouvertes

| N°  | Question                                                                                                                                        | Recommandation                                                                                                              |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Q1  | Lecture sur le PC : désactiver, ou convertir STUdio vers le format Flam (graphe proche de celui de Telmi, ce qui ouvrirait aussi la Flam 1.x) ? | désactiver maintenant ; la conversion vers le format Flam est une autre épopée, à ouvrir après le test L1                   |
| Q2  | Accepter les packs Lunii « plain » fournis par l'utilisateur ?                                                                                  | non pour l'instant (histoires du commerce) ; techniquement simple ensuite (§ 5.2)                                           |
| Q3  | Noms `rf/000/` : règle de Lunii.QT quand elle est sûre, rang sinon (§ 3.4) ?                                                                    | oui : parité pour #520, aucun écrasement possible                                                                           |
| Q4  | `info` : copie exacte de Lunii.QT (ASCII, `No authors.`) pour le premier essai ?                                                                | oui, puis essai UTF-8 et ligne d'auteur vide (L4) ; garder ce qui s'affiche le mieux                                        |
| Q5  | `optionIndex` à `-1` : accepter ?                                                                                                               | tranchée le 13/11/2026 par décision du mainteneur (#608) : accepter. Converti en `0` quand le nœud d'action n'a qu'une option (exact, et indépendant du comportement de la Flam) ; avec plusieurs options, gardé et écrit `-1` (`FF FF FF FF`, entier signé de 32 bits) dans le champ d'option de `ni`, position dans `li` et nombre d'options renseignés : c'est ce que fait Lunii.QT, qui recopie la valeur sans contrôle (`StudioStory`, `pkg/api/stories.py`, commit c8afe43). OK et maison sont traités de même (le `writer.js` de STUdio peut produire `-1` pour les deux). Établi par l'éditeur STUdio (`writer.js`, `reader.js`, `ActionNodeModel.js`) : `-1` est le port « Random option » du nœud d'action, option tirée au hasard uniformément. Aucun avertissement dans l'interface. Inconnu (L11) : le comportement de la Flam 2.x devant ce `-1` ; mesure confiée à un testeur (M12) |
| Q6  | MP3 à 48 kHz mono : transcoder (plus strict que Lunii.QT) ?                                                                                     | oui : 44,1 kHz est la seule fréquence connue de la Flam                                                                     |
| Q7  | Extension `.lunii.pk` dans la bibliothèque ?                                                                                                    | oui : aucun code Flam ne la confond avec un `.plain.pk`                                                                     |
| Q8  | Zip avec un dossier unique à la racine : accepter ?                                                                                             | oui (cas fréquent d'une recompression) ; tout autre emplacement refusé                                                      |
| Q9  | STUdio 7z : refuser ?                                                                                                                           | oui, avec le message « recompressez en zip » ; issue « plus tard » si la demande revient                                    |
| Q10 | Même UUID en Telmi et en STUdio dans la bibliothèque : garder la règle « une entrée par UUID » ?                                                | oui, avec confirmation avant le remplacement ; deux entrées demanderaient de revoir `library.rs` (données de l'utilisateur) |
