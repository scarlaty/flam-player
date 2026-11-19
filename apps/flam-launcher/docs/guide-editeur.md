# Guide de l'éditeur d'histoires sur dossier

Ce guide s'adresse à l'adulte qui fabrique une histoire pour la Flam à partir de ses propres
fichiers audio : un livre lu à voix haute, des comptines enregistrées, un recueil de contes. Il
explique comment ranger un dossier, comment le modifier dans l'éditeur du launcher, puis comment
l'envoyer sur la Flam.

Les notes de conception et les décisions sont dans [creation-dossier.md](creation-dossier.md).

## 1. À quoi ça sert

Vous avez des fichiers audio (MP3, par exemple) et, si vous voulez, des images. Le launcher en
fait une histoire que l'enfant lance depuis la Flam, avec ses menus, ses images et ses titres lus.

- Le **dossier** de votre PC est l'histoire. Il n'y a pas de projet caché : tout ce que fait
  l'éditeur est écrit dans ce dossier, et tout ce que vous faites à la main dans l'Explorateur
  apparaît dans l'éditeur.
- Vous pouvez travailler à la main (section 2), avec l'éditeur (section 5), ou mélanger les deux.
- Rien n'est jamais effacé : ce que vous retirez va dans une corbeille de l'histoire, et
  « Annuler » le remet en place.

Cette fonction est faite pour un usage personnel et familial. Vos fichiers restent sur votre PC ;
le launcher n'en modifie aucun original quand vous les ajoutez : il en fait une copie.

## 2. Ranger un dossier à la main

Un dossier rangé à la main, sans aucun fichier spécial, donne déjà une histoire.

### 2.1 Les règles de base

- **Un dossier est un menu.** Ses sous-dossiers et ses fichiers audio sont ses options, que vous
  pouvez mélanger.
- **Un fichier audio est une histoire** (ou un chapitre, voir la section 3).
- Les éléments sont classés dans l'ordre naturel : « 2 » vient avant « 10 », sans tenir compte
  des majuscules ni des accents. Mettez un numéro devant les noms pour fixer l'ordre :
  `01 Le loup.mp3`, `02 Le chat.m4a`.
- Le titre affiché est le nom du fichier sans l'extension et sans le numéro de tête :
  « 01 - Le loup » devient « Le loup ».
- Sur la Flam, un titre est coupé à 40 caractères. Restez plus court : l'éditeur vous prévient.

### 2.2 Les fichiers spéciaux

Tous sont facultatifs. Ceux qui commencent par `_` concernent le dossier qui les contient.

| Fichier               | Où                        | Rôle                                                           |
| --------------------- | ------------------------- | -------------------------------------------------------------- |
| `_couverture.png`     | racine                    | couverture de l'histoire                                       |
| `_image.png`          | sous-dossier              | image de l'option qui ouvre ce menu                            |
| `_titre.mp3`          | racine ou sous-dossier    | titre lu de l'histoire ou de ce menu                           |
| `<nom>.<image>`       | à côté de `<nom>.<audio>` | image de cette histoire (`01 Le loup.jpg`)                     |
| `<nom>.titre.<audio>` | à côté de `<nom>.<audio>` | titre lu de cette histoire (`01 Le loup.titre.mp3`)            |
| `_histoire.json`      | racine                    | réglages de l'histoire (écrit par le launcher)                 |
| `_dossier.json`       | sous-dossier              | ordre, titres affichés, réglages du menu (écrit par l'éditeur) |

Ces fichiers sont rangés en tête dans l'Explorateur. Le dossier caché `.flam/` à la racine est la
corbeille de l'éditeur : n'y touchez pas.

`_histoire.json` et `_dossier.json` sont écrits par le launcher. Vous n'avez pas besoin de les
créer. Si vous les modifiez à la main, un réglage illisible est remplacé par sa valeur par
défaut et signalé dans le rapport.

### 2.3 Formats acceptés

- **Audio** : `mp3`, `m4a`, `aac`, `ogg`, `opus`, `wav`, `flac`. Le launcher convertit au
  besoin.
- **Images** : `png`, `jpg` ou `jpeg`, `bmp`, `webp`.
- Les autres fichiers sont ignorés et signalés une fois dans le rapport. Les fichiers cachés,
  `desktop.ini`, `Thumbs.db` et `.DS_Store` sont ignorés sans rien dire.

### 2.4 Exemple

```
Mes histoires du soir/
├── _couverture.png
├── _titre.mp3
├── 01 Animaux/
│   ├── _image.png
│   ├── _titre.mp3
│   ├── 01 Le loup.mp3
│   ├── 01 Le loup.jpg
│   ├── 01 Le loup.titre.mp3
│   └── 02 Le chat.m4a
└── 02 Comptines/
    ├── 01 Au clair de la lune.mp3
    └── 02 Une souris verte.mp3
```

Sur la Flam, l'enfant voit d'abord deux options, « Animaux » et « Comptines ». « Animaux » ouvre
un menu avec « Le loup » (avec son image et son titre lu) et « Le chat ».

Ce qui manque (image du chat, titre lu des comptines…) est complété automatiquement : voir la
section 5.6.

## 3. Recueil ou livre à chapitres

Chaque dossier est l'un ou l'autre. Le réglage est **par dossier** et **hérité** par les
sous-dossiers. Celui de la racine est le réglage principal. Un sous-dossier peut en changer : un
recueil « Mes livres » peut contenir un livre à chapitres par sous-dossier. En revanche, un
recueil placé **dans** un livre à chapitres n'est pas pris en charge.

Si le dossier n'a aucun réglage, l'éditeur propose « chapitres » quand la racine ne contient que
des fichiers audio numérotés (ou des sous-dossiers qui ne contiennent que cela), « recueil »
sinon.

### 3.1 Recueil : un menu de petites histoires

C'est le bon choix pour des contes ou des comptines indépendants.

- Au démarrage, l'enfant arrive dans le menu racine. Une option est une histoire ou un sous-menu.
- Quand une histoire se termine, selon le réglage « À la fin » : « Retour au menu » (par défaut,
  le menu est positionné sur l'histoire suivante) ou « L'histoire suivante démarre » (la
  dernière ramène au menu).
- Le bouton retour pendant une histoire ramène à son menu, sur cette histoire.

### 3.2 Livre à chapitres : les chapitres s'enchaînent

C'est le bon choix pour un livre audio découpé en chapitres.

- **Ordre de lecture** : tous les fichiers dans l'ordre, dossier après dossier. Les sous-dossiers
  sont des **parties** (« Partie 1 », « Tome 2 ») : elles ne servent qu'au menu des chapitres, et
  la lecture passe de l'une à l'autre sans s'arrêter.
- **Au démarrage**, selon « Au démarrage » : « Le premier chapitre démarre » (par défaut) ou « Le
  menu des chapitres s'ouvre ».
- **Enchaînement** : à la fin d'un chapitre, le suivant démarre tout seul.
- **Titres lus** : si « Annoncer le titre avant chaque chapitre » est coché, le titre lu du
  chapitre est dit avant lui, pour les chapitres qui en ont un.
- **Menu des chapitres** : le bouton retour pendant un chapitre l'ouvre, positionné sur le
  chapitre en cours. Il y a une option par chapitre, un niveau par partie. Valider une option
  démarre ce chapitre, puis la lecture continue avec les suivants.
- **Image** : celle du chapitre, sinon celle de sa partie, sinon la couverture. Elle s'affiche
  avec le titre lu du chapitre. **Pendant la lecture du chapitre, l'écran de la Flam est noir.**
- **Retour = sortie** : depuis le premier niveau du menu des chapitres, le bouton retour **sort
  de l'histoire** (menu de la Flam). Si le livre est dans un recueil, il remonte au recueil.
  Depuis une partie, il remonte au niveau du dessus.
- **Reprise** : le chapitre quitté avec le bouton retour **garde sa position**. « Reprendre
  l'histoire » le reprend à l'endroit où l'enfant s'est arrêté, et valider son option aussi. Les
  autres chapitres repartent du début.
- **Fin du livre**, selon « À la fin » : « Menu des chapitres » (par défaut) ou « Fin de
  l'histoire (retour à la bibliothèque) ».

### 3.3 Conseils pour les enfants de 6 à 10 ans

Ce sont eux qui écoutent, souvent sans lire couramment.

- **Menus courts** : visez 5 à 8 options par menu, 10 au plus. Au-delà, l'enfant se perd, même si
  la limite technique est de 50.
- **Peu de niveaux** : un menu dans un menu suffit presque toujours. 5 niveaux sont possibles,
  mais pas souhaitables.
- **Chapitres de quelques minutes** : l'enfant peut ainsi s'arrêter et reprendre sans mal.
- **Titres courts et parlants** : « Le loup », « La forêt ».
- **Titres lus clairs** : une phrase courte, dite lentement, sans musique de fond. Le titre est
  entendu en parcourant le menu : il doit être compris en une seconde ou deux. La voix Windows
  fait l'affaire ; un fichier enregistré par un proche est plus chaleureux.
- **Une image par option** : les petits se repèrent d'abord à l'image. Une image simple et
  contrastée vaut mieux qu'une photo chargée.
- Dans un livre long, laissez « Annoncer le titre avant chaque chapitre » décoché et gardez les
  titres lus pour le menu des chapitres.

## 4. Créer l'histoire depuis Mes histoires

Dans le launcher, ouvrez **Mes histoires**.

1. Cliquez sur **Créer une histoire depuis un dossier**.
2. Choisissez le dossier (« Dossier de l'histoire »). Prenez un dossier personnel, par exemple
   dans Documents : un disque entier, un dossier système, le dossier du launcher ou la Flam sont
   refusés.
3. L'éditeur s'ouvre dans sa propre fenêtre.
4. Quand le rapport en bas de la fenêtre ne signale plus de point à corriger, cliquez sur
   **Créer l'histoire**.
5. La création se lance. L'histoire arrive dans Mes histoires, dans la fenêtre principale.

Si le dossier est déjà ouvert dans un éditeur, sa fenêtre passe au premier plan.

Si le dossier ressemble à la copie d'un autre dossier d'histoire déjà installé (deux dossiers
portent le même identifiant), choisissez **Nouvelle histoire** pour en faire une histoire à part,
ou **Remplacer « … »** pour mettre à jour l'autre.

## 5. L'éditeur

### 5.1 L'écran

- En haut : la couverture, le mode (Recueil ou Livre à chapitres), l'âge, le nombre d'histoires
  ou de chapitres, et les boutons **Ouvrir dans l'Explorateur**, **Annuler**, **Rétablir**,
  **Créer l'histoire** (ou **Mettre à jour l'histoire** si elle existe déjà).
- À gauche : l'**arbre** de l'histoire. Chaque élément montre sa vignette et son titre. Des
  pastilles signalent un « Titre lu », une « Image automatique » et un point « À vérifier » ou
  « À corriger ».
- À droite : le **panneau** de l'élément choisi.
- En bas : le **rapport**. Chaque ligne mène à l'élément concerné. « À corriger avant de créer
  l'histoire » bloque la création ; « Avertissements » ne la bloque pas.

### 5.2 Ajouter des fichiers

Dans tous les cas, les fichiers sont **copiés** dans le dossier ; vos originaux ne bougent pas.

- Bouton **Ajouter des fichiers** (ou **Ajouter des fichiers ici** dans le panneau d'un menu) :
  choisissez des fichiers audio ou des images.
- **Glisser-déposer depuis l'Explorateur** : déposez des fichiers audio ou un dossier entier sur
  un menu de l'arbre ou entre deux éléments. Sur un menu, ils sont copiés dans ce menu. Un
  bandeau dit ce qui va se passer et l'endroit visé est surligné. Déposez une image sur un
  élément ou sur l'image du panneau pour la changer, un fichier audio sur le « Titre lu » pour
  changer le titre lu (un seul fichier à la fois dans ces deux cas). Les fichiers de format refusé ne sont
  pas copiés et un message le dit ; si aucun n'est accepté, rien n'est copié.
- **Nouveau menu** (ou **Nouveau menu ici**) : crée un sous-dossier. Donnez-lui un nom, puis
  cliquez sur **Créer**.

### 5.3 Réordonner et déplacer

- **Au pointeur** : saisissez la poignée (« Glisser pour déplacer ») d'un élément et déposez-le à
  la place voulue. Un trait montre où il ira. Déposé sur un autre menu ou dans un autre menu, il y
  est déplacé. Échap annule. Un menu ne peut pas aller dans lui-même ni dans un de ses sous-menus.
- **Au clavier** : flèches pour parcourir l'arbre, Alt+Flèche haut ou bas pour réordonner.
- **Dans le panneau**, section « Ranger » : **Monter**, **Descendre**, **Déplacer vers** puis
  **Déplacer**.

Réordonner ne renomme pas les fichiers : l'ordre est noté dans `_dossier.json` du menu (`_histoire.json` pour la racine). Dans
l'Explorateur, les fichiers gardent donc leur nom d'origine. Les numéros de tête ne servent
qu'aux éléments que l'éditeur n'a pas encore classés.

### 5.4 Renommer

Dans le panneau, modifiez le **Titre affiché** puis cliquez sur **Enregistrer le titre** (F2
depuis l'arbre). Par défaut, seul le titre affiché change et le fichier garde son nom. Cochez
**Renommer aussi le fichier** pour que le fichier prenne le nouveau nom.

La racine a ses propres champs : **Titre de l'histoire**, **Sous-titre**, **Description**,
**Âge conseillé** (de 0 à 99, vide si aucun), puis **Enregistrer**.

Le panneau d'un menu a une section **Réglages du dossier** : **Organisation** (« Comme le dossier
parent », « Recueil : un menu de petites histoires » ou « Livre à chapitres : les chapitres
s'enchaînent »), **À la fin**, **Au démarrage** et **Annoncer le titre avant chaque chapitre**.
Cliquez ensuite sur **Appliquer les réglages**.

### 5.5 Corbeille et annulation

- **Mettre à la corbeille** (ou Suppr dans l'arbre) demande une confirmation. L'élément est rangé
  dans la corbeille de l'histoire (`.flam/corbeille/`), pas effacé.
- **Annuler** (Ctrl+Z) et **Rétablir** (Ctrl+Y) parcourent les opérations de la session. Une
  opération qui ne peut plus être défaite, parce qu'un fichier a changé depuis, le dit et ne fait
  rien.
- **Vider la corbeille de l'histoire** envoie son contenu dans la Corbeille Windows. Ce qui en
  dépend ne pourra plus être annulé. Sur une clé USB ou un disque réseau, qui n'ont pas de
  Corbeille Windows, le vidage est refusé.

### 5.6 Images et titres lus automatiques

Quand rien n'est fourni, le launcher complète. Ces compléments sont préparés à la création et
**ne sont pas écrits dans votre dossier**. Le panneau les marque « automatique ».

| Il manque              | Le launcher utilise, dans l'ordre                                                                                                                                                                                |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Le titre lu            | un fichier `_titre.*` ou `<nom>.titre.*` ; sinon la voix Windows ; sinon rien                                                                                                                                    |
| L'image d'une histoire | l'image du même nom ; sinon la pochette du fichier audio ; sinon une image avec le titre ; sinon une image unie (dans un livre à chapitres : l'image de la partie, puis la couverture, passent avant ; voir 3.2) |
| L'image d'un menu      | `_image.*` ; sinon l'image de sa première histoire                                                                                                                                                               |
| La couverture          | `_couverture.*` ; sinon l'image de la première histoire, ou la pochette de son fichier audio ; sinon une image avec le titre de l'histoire                                                                       |

Pour un menu ou la couverture qui reprennent l'image d'une histoire, le bouton **Garder**
(« Copie cette image dans le dossier pour pouvoir la changer ensuite ») écrit cette image dans le
dossier. Les pochettes et les images avec le titre ne sont préparées qu'à la création. Vous pouvez ensuite la remplacer par **Choisir une
image…** ou l'enlever par **Retirer l'image**.

Le titre lu a aussi ses boutons : **Écouter la voix Windows** (puis **Arrêter**), **Choisir un
fichier audio…**, **Remplacer le fichier…** et **Retirer le fichier**. Si la voix Windows n'est
pas disponible sur le PC, un message l'indique.

## 6. Actualiser et envoyer sur la Flam

- **Mettre à jour l'histoire** : après des changements, cliquez sur ce bouton dans l'éditeur. Le
  numéro de version augmente et l'histoire de Mes histoires est remplacée. Seuls les fichiers
  modifiés sont reconvertis.
- Dans **Mes histoires**, une histoire créée depuis un dossier propose **Modifier** (ouvre
  l'éditeur sur son dossier) et **Actualiser** (la recrée sans ouvrir l'éditeur ; le bouton de la
  fenêtre qui s'ouvre est **Actualiser l'histoire**).
- **Envoyer sur la Flam** envoie l'histoire sur la Flam branchée. Si elle y est déjà, **Remplacer
  sur la Flam** la met à jour. Les prérequis, la durée, l'éjection et les risques sont dans
  [flam-transfert.md](flam-transfert.md) ; ils valent pour toutes les histoires.
- N'éjectez pas la Flam et n'y lisez rien pendant un envoi.

## 7. Le dossier a changé ou disparu

- **Modifications faites dans l'Explorateur.** Tant que l'éditeur est ouvert, le dossier est
  surveillé. Si vous ajoutez, retirez ou renommez un fichier à côté, l'arbre se met à jour et un
  message le dit : « Le dossier a changé dans l'Explorateur : l'affichage est à jour. » La
  sélection reste sur l'élément, ou passe à son menu s'il a disparu.
- **Opération sur un état périmé.** Si un fichier a changé juste avant votre action, rien n'est
  fait, l'affichage est mis à jour et vous pouvez réessayer.
- **Dossier déplacé, renommé, supprimé, ou clé USB retirée.** L'éditeur passe en lecture seule et
  affiche « Dossier introuvable ». Rebranchez la clé, ou cliquez sur **Choisir le dossier** et
  indiquez le nouvel emplacement : l'éditeur s'ouvre sur ce dossier. Dans Mes histoires, le même
  bouton apparaît quand vous cliquez sur **Actualiser** et que le dossier est introuvable ; il
  vérifie alors que le dossier contient le fichier `_histoire.json` de cette histoire.
- Sous Windows, tant que l'éditeur est ouvert, il peut empêcher d'éjecter proprement la clé USB
  ou de renommer un dossier parent. Fermez la fenêtre de l'éditeur avant.

## 8. Limites

| Limite                          | Valeur |
| ------------------------------- | ------ |
| Niveaux de dossiers             | 5      |
| Histoires ou chapitres au total | 300    |
| Options dans un même menu       | 50     |
| Audio au total                  | 3 Gio  |

Au-delà, le rapport signale un point **à corriger** et la création est bloquée jusqu'à ce que
vous allégiez le dossier. Ces valeurs seront confirmées après des essais sur la Flam ; en
pratique, restez bien en dessous (voir la section 3.3).

## 9. Questions fréquentes

**Faut-il savoir ce qu'est un fichier JSON ?** Non. Le launcher écrit `_histoire.json` et
`_dossier.json` lui-même.

**Mes fichiers d'origine sont-ils modifiés ?** Non. Ajouter des fichiers en fait une copie dans le
dossier de l'histoire. Seul le dossier de l'histoire est modifié, et rien n'y est effacé
définitivement avant que vous ne vidiez la corbeille.

**J'ai ouvert un dossier fait à la main : l'éditeur a-t-il changé mes fichiers ?** Non. Il le lit
tel quel et n'écrit rien tant que vous ne modifiez rien ou que vous ne créez pas l'histoire.

**« Aucun fichier audio dans ce dossier. »** Le dossier choisi ne contient aucun fichier audio
accepté. Vérifiez les formats (section 2.3) et que vous avez choisi le bon dossier.

**Une image est ignorée.** « Image sans fichier audio du même nom » : donnez-lui le même nom que
son fichier audio (`01 Le loup.mp3` et `01 Le loup.jpg`).

**Un titre est « coupé à 40 caractères ».** Raccourcissez-le : la Flam n'affiche pas davantage.

**Le dossier est dit copié d'un autre.** Vous avez recopié le dossier d'une histoire déjà créée.
Choisissez « Nouvelle histoire » pour une histoire à part.

**Le titre lu manque.** Sans fichier de titre lu, la voix Windows lit le titre à la création. Si
elle n'est pas disponible, l'option est muette : ajoutez un fichier `_titre.*` ou
`<nom>.titre.*`.

**Puis-je mettre un livre à chapitres dans un recueil ?** Oui. L'inverse (un recueil dans un livre)
n'est pas pris en charge.

**Où est la corbeille ?** Dans le dossier caché `.flam/corbeille/` de l'histoire.

**Puis-je ouvrir le même dossier dans deux éditeurs ?** Non : la fenêtre déjà ouverte revient au
premier plan.
