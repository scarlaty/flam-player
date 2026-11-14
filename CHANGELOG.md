# Journal des changements

Ce fichier suit le format [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) et décrit les
changements de **Flam Launcher** visibles par les utilisateurs. Les versions du launcher sont
publiées sous des étiquettes `launcher-v…` (voir les
[versions](https://github.com/scarlaty/flam-player/releases?q=launcher)).

## [Non publié]

### Corrigé

- Une archive placée sur une clé ou un partage retiré avant la conversion est gardée, et non mise
  en cause.

## [0.1.0] - 2026-11-14

Première version qui n'est plus une pré-version. Changements depuis `launcher-v0.1.0-beta.6`.
« Télécharger et envoyer », la reconnaissance de la Flam branchée, les contrôles de la Flam pendant
un retrait ou un changement d'ordre et la fermeture retenue pendant un envoi ont été vérifiés par
des tests automatiques, pas encore sur une Flam réelle ; l'envoi, le retrait et l'éjection
eux-mêmes ont été essayés sur une Flam en firmware 1.x avec la version précédente.

### Ajouté

- **« Télécharger et envoyer »** (ou « Installer et envoyer » pour un dossier local) : depuis une
  source d'histoires, un seul bouton installe l'histoire puis l'envoie sur la Flam branchée. Si le
  launcher n'a pas pu surveiller la Flam pendant toute l'installation, l'histoire est marquée
  « Installée, envoi à confirmer » : l'envoi ne part pas tout seul, il se fait d'un clic après
  avoir vérifié que la bonne Flam est branchée.
- **Contrôle de la Flam branchée** : le launcher reconnaît la Flam affichée au moment du clic, puis
  la recontrôle avant d'écrire, avant de valider l'envoi et pendant un retrait ou un changement
  d'ordre. Si une autre Flam a été branchée entre-temps, l'opération s'arrête et les envois en
  attente sont annulés : rien n'est installé ni retiré sur l'autre Flam et sa liste d'histoires
  n'est pas modifiée. Une copie temporaire peut y rester ; elle est supprimée au prochain
  branchement ou au prochain envoi.

### Modifié

- **Fermeture pendant un envoi** : fermer la fenêtre ne coupe plus l'écriture. La fermeture attend
  la fin de l'opération en cours (bandeau avec « Ne pas fermer ») ; une seconde demande interrompt
  l'opération tout de suite. L'arrêt de Windows n'est pas retenu.
- **Place manquante sur la Flam** : les histoires retirées par le launcher et encore gardées de
  côté sur la Flam sont supprimées avant de refuser un envoi faute de place.
- **Cache** : le dossier de travail des envois est compté et vidé avec le cache.
- **Installation** : annulation plus rapide pendant la lecture et le décodage d'une archive.
- **Lecture sur le PC** : le script d'une histoire n'a plus accès aux fonctions qui permettaient de
  lire ou d'écrire des fichiers, de lancer un programme ou de charger du code natif, comme sur la
  Flam ; les images et les sons ne peuvent être pris que dans le dossier de l'histoire. Un script
  qui tourne en boucle est interrompu dans davantage de cas et la mémoire d'une histoire est
  plafonnée. Cette isolation réduit les risques, elle ne les supprime pas.

### Corrigé

- « Vider le cache » ne fige plus l'application : si un téléchargement ou une conversion occupe le
  cache, le vidage est refusé après quelques secondes, avec un message.
- Réglages illisibles au démarrage : le launcher ne touche plus aux dossiers de cache. Un autre
  dossier de cache choisi auparavant n'est alors plus vidé par « Vider le cache » : le supprimer à
  la main ou le choisir de nouveau dans les Paramètres.
- Une histoire en cours de suppression ne peut plus être envoyée en même temps ; la fenêtre de
  confirmation peut être refermée pendant la suppression, qui continue.
- Une archive téléchargée n'est plus supprimée quand Windows refuse de la lire (antivirus, fichier
  verrouillé, support retiré).
- Podcasts : le téléchargement des vignettes ne bloque plus les autres opérations sur le cache.

## [0.1.0-beta.6] - 2026-11-13

Changements depuis `launcher-v0.1.0-beta.5`.

### Modifié

- **Histoires STUdio** : le tirage au hasard entre plusieurs options (option « aléatoire » de
  l'éditeur STUdio) est accepté à l'installation.

### Corrigé

- **La Flam redémarrait dans un long menu** (nombreux choix avec images) : le menu gardait en
  mémoire toutes les images déjà affichées ; il ne garde plus que celles qui sont à l'écran.
  Vérifié sur une Flam en firmware 1.x avec des menus de 18 et de 30 choix. Les histoires déjà
  envoyées gardent l'ancien menu : il faut les réinstaller puis les renvoyer (pour un pack de
  podcasts : recréer le pack).
- **« Éjecter la Flam » était refusé juste après un envoi** quand un autre programme occupait
  encore la Flam : le launcher demande désormais le retrait à Windows, sans jamais forcer, et
  n'annonce « Vous pouvez débrancher » que lorsque la Flam a réellement disparu.

## [0.1.0-beta.5] - 2026-11-13

Changements depuis `launcher-v0.1.0-beta.4`.

### Ajouté

- **« Retirer du PC »** : après un envoi réussi, un bouton retire l'histoire de « Mes histoires » ;
  elle reste sur la Flam. Le launcher revérifie d'abord qu'elle est bien sur la Flam branchée. Un
  réglage « Retirer de Mes histoires après l'envoi sur la Flam » (désactivé par défaut) le fait
  automatiquement, seulement si l'écriture a pu être confirmée sur la Flam.

### Modifié

- **Histoires STUdio** : une histoire dont un choix est marqué « aléatoire » s'installe quand ce
  choix n'a qu'une seule option ; un tirage entre plusieurs options reste refusé.
- **« Vider le cache »** : seuls les dossiers créés par le launcher sont comptés, nettoyés et
  vidés ; un dossier qu'il ne reconnaît pas est laissé tel quel.
- **Podcasts** : conversions audio bornées, fichiers WAV à l'en-tête inexact acceptés, nettoyage
  des vignettes et des MP3 limité aux dossiers créés par le launcher.
- **Interface** : navigation au clavier dans les boîtes de dialogue, listes relues après un vidage
  du cache.

### Corrigé

- **Installation** : une erreur de lecture ou d'écriture du système (disque, antivirus) ne fait
  plus retélécharger l'histoire ; après un arrêt du launcher, la file d'installation reprend sans
  lancer deux fois la même tâche ; une image non reconnue dans une histoire Telmi ne fait plus
  échouer la conversion.
- **Ma Flam** : si la Flam est débranchée ou remplacée pendant le choix d'une histoire officielle
  (mode carrier), la demande est abandonnée au lieu de bloquer les fiches d'histoire.
- **Lecture sur le PC** : un dossier `.plain` fourni par l'auteur d'une histoire n'est plus écrasé
  par l'archive `.pk` voisine.

## [0.1.0-beta.4] - 2026-11-07

Changements depuis `launcher-v0.1.0-beta.3`.

### Ajouté

- **Histoires STUdio (Flam en firmware 2.x seulement)** : une histoire créée avec l'éditeur STUdio
  (archive `.zip` contenant `story.json`) est reconnue dans un dossier source (badge « Format
  STUdio »), s'installe dans « Mes histoires » et s'envoie sur une Flam 2.x. Images et sons sont
  convertis automatiquement. Sur une Flam en firmware 1.x, l'envoi est grisé et expliqué ; la
  lecture sur le PC n'est pas disponible ; si la même histoire est déjà installée au format Telmi,
  le launcher demande confirmation avant de la remplacer.

### Modifié

- **Scan d'un dossier** : au lieu de « pas une histoire Telmi ? », le launcher explique ce qu'il a
  trouvé (histoire STUdio, histoire Flam dans un zip, pack Lunii du commerce, archive inconnue avec
  les premiers noms de fichiers, noms de fichiers Telmi en majuscules à renommer).
- **Ma Flam** : les histoires au format Lunii présentes sur la Flam sont reconnues et affichées
  avec leur titre.

### Corrigé

- **Ma Flam** : les titres ne clignotent plus quand on actualise la liste.
- **Fiabilité** : une annulation est prise en compte immédiatement, même quand une étape attendait
  son tour ; les fichiers temporaires laissés par un arrêt brutal sont nettoyés au démarrage.

## [0.1.0-beta.3] - 2026-10-25

Changements depuis `launcher-v0.1.0-beta.2`.

### Modifié

- **Podcasts** : dans le choix de l'image d'une catégorie, la vignette de chaque pochette est
  désormais téléchargée à la demande, dès la première composition, quand elle approche de l'écran
  (une seule fois : elle est gardée dans le cache de la source et purgée avec elle).
- **Convertisseur** : lecture des JPEG encore plus fidèle à l'ancienne version sur les fichiers
  abîmés ou inhabituels (tables lues comme libjpeg, fichiers coupés, octets de remplissage). Un JPEG
  progressif de plus de 1 000 passes est refusé avec le message habituel au lieu de mobiliser le
  décodeur trop longtemps.

## [0.1.0-beta.2] - 2026-10-24

Changements depuis `launcher-v0.1.0-beta.1`.

### Ajouté

- **Podcasts** : une source « Podcast » (adresse d'un flux RSS) fournit des épisodes. L'écran
  **Podcasts** permet de les cocher (raccourci « Cocher les 10 derniers épisodes » ; les
  épisodes explicites ne sont jamais cochés d'office), puis **Composer le pack** les range en
  catégories (3 niveaux de menus) avec titre, âge, description et couverture. **Créer
  l'histoire** télécharge et convertit l'audio, et l'histoire arrive dans « Mes histoires ». Un
  message rappelle à la première création que ces histoires sont réservées à l'usage personnel
  et familial. Le brouillon du pack est enregistré automatiquement. Chaque catégorie peut
  prendre l'image d'un de ses épisodes (choix dans l'éditeur ; « Automatique », l'image du
  premier épisode, reste le défaut). Si l'épisode choisi disparaît du pack, l'image automatique
  est utilisée et un avertissement s'affiche à la fin de la création. Le choix montre une
  vignette de la pochette de chaque épisode (quand elle a déjà été téléchargée pour un pack) ;
  deux épisodes de même titre se distinguent par leur rang.
- **Titres lus (podcasts)** : option « Lire les titres à voix haute » dans l'éditeur de pack. Le
  titre du pack, les catégories et les épisodes sont lus par une voix française de Windows
  (choix de la voix et essai dans les Paramètres). Sans voix française installée, l'option est
  grisée et un message indique comment en ajouter une.
- **Compositions réutilisables (podcasts)** : un pack créé est conservé comme composition
  nommée, pour le refaire ou en dériver un autre.
- **Cache des MP3 (podcasts)** : les épisodes déjà convertis sont réutilisés pour refaire un pack
  sans tout retélécharger ; le cache est purgé avec la source et sa taille est affichée dans les
  Paramètres.
- **Diagnostic copiable** : en cas d'erreur, un panneau permet de copier les informations utiles
  pour un rapport de bogue.
- **Audio** : les MP3 qui ne sont pas à 44 100 Hz sont rééchantillonnés à la conversion
  (un petit ffmpeg est livré avec l'application).
- **Vitrine** : le README du dépôt présente désormais le launcher, avec captures d'écran,
  parcours animé, installation et FAQ.

### Modifié

- **Firmware 2.x** : la garde du firmware 2.x est retirée, alignement sur Lunii.QT. Plus de
  confirmation « j'ai sauvegardé ma Flam » ni de vérification par la Flam : l'envoi, le retrait,
  l'ordre et la reprise au branchement se font directement, comme avec Lunii.QT (mêmes clés,
  même fichier `key`, même chiffrement, mêmes chemins, même index). Des clés incohérentes avec
  une histoire d'un autre outil ne bloquent plus l'envoi : « Ma Flam » affiche un avertissement
  qui désigne les histoires en cause. Les refus de fond restent (firmware 3.x ou illisible,
  fichier `.mdf` tronqué ou trop grand, clés invalides). Une entrée `x.lua.plain` d'une archive
  donne désormais `x.lsf` chiffré, comme dans Lunii.QT.

- **Convertisseur intégré, sans Python** : le convertisseur TELMI vers Flam est maintenant un
  programme Rust autonome (`telmi2flam.exe`, version 2.0.0), livré dans l'installateur. Il est
  plus rapide, n'exige aucune dépendance et donne des résultats vérifiés identiques à
  l'ancienne version sur 30 archives réelles. L'installateur ne contient plus ni Python ni
  Pillow.
- **Paramètres** : le réglage « chemin de Python » a disparu. Un ancien fichier de réglages qui
  le contient reste lisible : le champ est simplement ignoré.
- **Mise à jour** : l'installation d'une version précédente qui livrait Python supprime l'ancien
  dossier du convertisseur Python.
- **Formats lus** : archives TELMI en ZIP sans compression, deflate, bzip2, LZMA ou Zstandard ;
  images PNG (y compris entrelacées, à palette ou en 16 bits), GIF et JPEG (écart de couleur
  d'au plus 4 sur 255 avec l'ancien convertisseur, invisible). Les autres formats d'image (BMP...)
  et les JPEG arithmétiques ou sans perte sont refusés avec un message explicite.
- **Vignettes** : les modes « flou » et « rogner » donnent le même rendu que l'ancien
  convertisseur.

### Supprimé

- **Convertisseur Python** : l'ancienne version Python du convertisseur (`telmi2flam.py`,
  Pillow), qui ne servait plus que de référence de développement, est retirée du dépôt. Ses
  résultats sont conservés comme références de test du convertisseur Rust, seul convertisseur
  désormais. Rien ne change pour les utilisateurs du launcher.

### Corrigé

- `flam-player` ne montre plus de fenêtre de console au lancement depuis le launcher.
- L'écran du player se redessine après une réinitialisation du pilote graphique (Direct3D).
- « Réessayer » rend toujours le focus au titre de l'écran.
- Divers durcissements du téléchargement et de la lecture de contenus venus d'Internet (adresses
  refusées, tailles bornées).

### Documentation

- README, installation et développement mis à jour : le convertisseur est présenté comme un
  programme intégré. Python 3 reste utile uniquement aux outils de campagne et de parité du
  dépôt (`tools/telmi2flam/`, sans Pillow), pas aux utilisateurs.

[Non publié]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0...HEAD
[0.1.0]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.6...launcher-v0.1.0
[0.1.0-beta.6]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.5...launcher-v0.1.0-beta.6
[0.1.0-beta.5]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.4...launcher-v0.1.0-beta.5
[0.1.0-beta.4]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.3...launcher-v0.1.0-beta.4
[0.1.0-beta.3]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.2...launcher-v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.1...launcher-v0.1.0-beta.2
