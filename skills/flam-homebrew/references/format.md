# Format `.plain.pk` (homebrew FLAM)

`flam_build.py` produit tout ceci. Ce document sert à comprendre, déboguer, ou
assembler sans le script.

## Conteneur

- Archive **ZIP STORED** (méthode 0, aucune compression), sans *data descriptor*
  (bit 3 des flags). Le device ne lit pas le `.pk` lui-même : Lunii.QT l'extrait
  (module `zipfile`) puis copie les fichiers. Mais le simulateur (`pk_reader.c`) exige
  ce format et tous les paquets éprouvés sur device le respectent : c'est la
  convention à garder. `zipfile.ZipFile(..., "w", zipfile.ZIP_STORED)` + `writestr`
  sur un fichier convient. Pas de `Compress-Archive` (PowerShell), qui compresse.
- Chemins relatifs, séparateur `/`, pas de dossier parent, pas d'entrée de dossier.
- Ordre des entrées (repris des paquets qui marchent) :
  `info.plain`, `version`, `main.lua`, `script/<module_du_jeu>.lua`, puis tout le reste
  trié, `uuid.bin` en **dernier**.

## Entrées

| Entrée | Contenu |
|---|---|
| `info.plain` | UTF-8, exactement 4 lignes sans `\n` final : `TITRE\nSOUS-TITRE\n000000\nTITRE`. Ligne 1 = titre de la bibliothèque. Titre/sous-titre sur une ligne, caractères ASCII + Latin-1 seulement. |
| `version` | `1` (octet ASCII, sans retour ligne). Le device accepte aussi `2`. |
| `uuid.bin` | 16 octets bruts = `md5(uuid_seed)`. Identifie l'histoire sur le device (dossier `str/<UUID>/`, sauvegardes). |
| `main.lua` | Bootstrap : définit `function setup()` (appelée par le firmware). |
| `script/*.lua` | Runtime Lua du projet + modules du jeu. `require("x")` ne cherche que `script/x.lua`. **À plat** : pas de sous-dossier. |
| `img/script/*.lif` | Icônes UI du framework (pause, flèches…). Réservé au runtime. |
| `img/empty.lif` | Image transparente utilisée par défaut par plusieurs modules. |
| `img/thumbnail.lif` | Vignette de la bibliothèque (104×136), optionnelle. |
| `img/**.lif` | Images du jeu. `Global.load_image("x.lif")` charge `img/x.lif`. |
| `sounds/*.mp3` + `.mp3map` | Audio. `Global.requestAudioPlay({path="x.mp3"})` lit `sounds/x.mp3`. |
| `sounds/silent.mp3` (+map) | MP3 silencieux (~0,6 s) : audio de remplissage exigé par certains modules (ex. `list-choice`). |

## UUID

- Unique par jeu : Lunii.QT refuse d'importer un UUID déjà présent (« already loaded »).
  `new_project.py` met un `uuid4` aléatoire dans `flam.json` → `uuid_seed`.
- Stable entre deux builds du même jeu (même sauvegarde). Pour réinstaller une nouvelle
  version : supprimer l'ancienne de la Flam (Lunii.QT) puis importer.
- Ne pas dériver l'UUID du seul titre (deux jeux homonymes se marcheraient dessus).

## Noms de fichiers

- Uniquement `[A-Za-z0-9_.-]` par segment : pas d'espace ni d'accent (FAT du device).
- Pas deux noms identiques à la casse près.
- `.lua` seulement en extension finale, pas de suffixe `.plain` hors `info.plain` :
  à l'import, Lunii.QT renomme `*.lua` → `*.lsf` (remplacement dans tout le nom) et
  chiffre les `.lua` et `.plain`. Le reste (images, mp3) est copié en clair.

## Import sur le device (Lunii.QT, fork `scarlaty/Lunii.QT`)

- **Firmware 2.x** : *Import* classique du `.plain.pk`.
- **Firmware 1.x** : l'import classique donne un titre illisible → utiliser l'**import
  carrier** du fork (emprunte les clés d'une histoire Lunii déjà présente sur la même
  Flam, sans la modifier). Fonction **expérimentale**, validée sur une seule Flam
  (fw 1.15.14) : en cas d'échec, le signaler plutôt que d'incriminer le paquet. Rien
  d'autre à faire côté paquet.
- Lunii.QT ignore `uuid.bin` (il s'en sert pour nommer le dossier) et le reste est
  copié tel quel : un paquet invalide le reste sur le device.
- Hors périmètre : import d'histoires d'un autre compte, modification du firmware.
