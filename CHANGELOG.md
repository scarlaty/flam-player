# Journal des changements

Ce fichier suit le format [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) et décrit les
changements de **Flam Launcher** visibles par les utilisateurs. Les versions du launcher sont
publiées sous des étiquettes `launcher-v…` (voir les
[versions](https://github.com/scarlaty/flam-player/releases?q=launcher)).

## [Non publié]

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
  est utilisée et un avertissement s'affiche à la fin de la création.
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
- **Formats lus** : archives TELMI en ZIP sans compression ou deflate, images PNG (y compris
  les PNG atypiques : entrelacés, palettes, 16 bits...). Les images d'autres formats et les ZIP
  compressés autrement sont refusés avec un message explicite.
- **Vignettes** : les modes « flou » et « rogner » donnent le même rendu que l'ancien
  convertisseur.

### Corrigé

- `flam-player` ne montre plus de fenêtre de console au lancement depuis le launcher.
- L'écran du player se redessine après une réinitialisation du pilote graphique (Direct3D).
- « Réessayer » rend toujours le focus au titre de l'écran.
- Divers durcissements du téléchargement et de la lecture de contenus venus d'Internet (adresses
  refusées, tailles bornées).

### Documentation

- README, installation et développement mis à jour : le convertisseur est présenté comme un
  programme intégré. Python 3 reste utile uniquement aux outils de campagne et de parité du
  dépôt (`tools/telmi2flam/`), pas aux utilisateurs.

[Non publié]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.1...HEAD
