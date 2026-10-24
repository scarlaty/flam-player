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
  et familial. Le brouillon du pack est enregistré automatiquement.
- **Diagnostic copiable** : en cas d'erreur, un panneau permet de copier les informations utiles
  pour un rapport de bogue.
- **Audio** : les MP3 qui ne sont pas à 44 100 Hz sont rééchantillonnés à la conversion
  (un petit ffmpeg est livré avec l'application).
- **Vitrine** : le README du dépôt présente désormais le launcher, avec captures d'écran,
  parcours animé, installation et FAQ.

### Modifié

- **Convertisseur intégré, sans Python** : le convertisseur TELMI vers Flam est maintenant un
  programme Rust autonome (`telmi2flam.exe`, version 2.0.0), livré dans l'installateur. Il est
  plus rapide, n'exige aucune dépendance et donne des résultats vérifiés identiques à
  l'ancienne version sur 30 archives réelles. L'installateur ne contient plus ni Python ni
  Pillow.
- **Paramètres** : le réglage « chemin de Python » a disparu. Un ancien fichier de réglages qui
  le contient reste lisible : le champ est simplement ignoré.
- **Mise à jour** : l'installation d'une version précédente qui livrait Python supprime l'ancien
  dossier du convertisseur Python.
- **Conversion** : les tâches affichent leur progression, peuvent être annulées, et le fichier
  final n'est écrit qu'une fois la conversion terminée (pas de fichier à moitié écrit).
- **Formats lus** : archives TELMI en ZIP sans compression ou deflate, images PNG (y compris
  les PNG atypiques : entrelacés, palettes, 16 bits...). Les images d'autres formats et les ZIP
  compressés autrement sont refusés avec un message explicite.
- **Vignettes** : les modes « flou » et « rogner » donnent le même rendu que l'ancien
  convertisseur.

### Corrigé

- `flam-player` ne montre plus de fenêtre de console au lancement depuis le launcher.
- L'écran du player se redessine après une réinitialisation du pilote graphique (Direct3D).
- Sur un firmware 2.x, retirer une histoire ou changer l'ordre est bloqué tant que vous n'avez
  pas confirmé, et les histoires concernées sont désignées.
- « Réessayer » rend toujours le focus au titre de l'écran.
- Divers durcissements du téléchargement et de la lecture de contenus venus d'Internet (adresses
  refusées, tailles bornées).

### Documentation

- README, installation et développement mis à jour : le convertisseur est présenté comme un
  programme intégré. Python 3 reste utile uniquement aux outils de campagne et de parité du
  dépôt (`tools/telmi2flam/`), pas aux utilisateurs.

[Non publié]: https://github.com/scarlaty/flam-player/compare/launcher-v0.1.0-beta.1...HEAD
