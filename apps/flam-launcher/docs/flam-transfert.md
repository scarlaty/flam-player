# Envoyer des histoires sur la Flam : prérequis, limites et risques

> **Fonction expérimentale et non officielle.** L'envoi d'histoires sur la Flam n'est ni prévu
> ni garanti par le fabricant de la Flam. flam-launcher n'est ni approuvé ni pris en charge par Lunii.
> Lisez la section [Risques](#risques) avant le premier envoi, et suivez le
> [guide testeur](guide-testeur.md) si vous participez à la bêta (copie de votre Flam
> d'abord).

Ce document décrit ce que fait réellement l'application, d'après son code
(`apps/flam-launcher/src-tauri/src/device/`) et les documents de référence
[`docs/flam-device-format.md`](../../../docs/flam-device-format.md) et
[`docs/flam-carrier.md`](../../../docs/flam-carrier.md). Quand un point n'a pas été vérifié sur
une vraie Flam, il est dit « non vérifié ».

## Prérequis

- Windows (la détection de la Flam n'est pas implémentée sur les autres systèmes).
- La Flam **branchée en USB à l'ordinateur**, avec le **mode connexion USB** activé dans ses
  réglages (sans lui, elle ne fait que charger). Elle doit apparaître dans l'Explorateur comme
  un **disque** (volume FAT). Le launcher la reconnaît à un fichier `.mdf` à la racine de ce
  disque.
- Une seule Flam à la fois : si plusieurs sont branchées, seule la première est affichée
  (débranchez les autres).
- De la place libre sur la Flam : le launcher vérifie l'espace avant de copier et refuse sinon.

## Firmwares pris en charge

| Firmware de la Flam                    | État                                                                         | Ce qui est vérifié                                                                                                                                                                                                               |
| -------------------------------------- | ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **1.x** (testé en 1.15.14)             | Validé sur un appareil réel, le 2026-10-19, avec le « mode carrier »         | Envoi d'une histoire avec saisie de la clé `bt`, lancement sur la Flam, titres et vignettes dans « Ma Flam », ordre des histoires, retrait, éjection, relances successives d'une histoire sans redémarrage, renvoi après retrait |
| **2.x**                                | Pris en charge dans le code, **expérimental : jamais testé sur un appareil** | Seulement par des tests automatiques sur un faux appareil. C'est l'objet de la bêta                                                                                                                                              |
| 3.x et suivants, ou firmware illisible | **Refusé** : l'envoi est impossible                                          | Version jamais observée ; refus volontaire plutôt que d'écrire des fichiers douteux                                                                                                                                              |

Le firmware est lu dans le fichier `.mdf` de la Flam.

- **Firmware 2.x** : le launcher utilise le matériel de chiffrement lu dans le `.mdf` de la
  Flam, exactement comme Lunii.QT (clés, fichier `key`, chiffrement, chemins et index
  identiques), sans confirmation préalable. Cette disposition n'a **pas** encore été vérifiée
  sur un appareil 2.x par ce projet : gardez une copie de votre Flam. Si une histoire d'un autre
  outil ne se déchiffre pas avec ces clés, « Ma Flam » affiche un simple avertissement (l'envoi
  reste possible).
- **Firmware 1.x (mode carrier)** : le firmware 1.x ne lit pas une histoire chiffrée avec les
  clés du `.mdf`. Le launcher réutilise la clé d'une histoire officielle **déjà présente sur
  votre Flam et achetée sur votre compte** (clé `bt`, que vous saisissez ou fournissez). Aucune
  clé n'est fournie par le launcher et aucune histoire officielle n'est copiée ni exportée. Un
  avertissement est affiché et doit être accepté avant la première utilisation. Si l'histoire
  officielle d'origine est retirée de la Flam ou de votre compte, les histoires installées ainsi
  peuvent devenir illisibles (non vérifié sur appareil).

## Limites connues

- **Une opération à la fois** sur la Flam : envoi, retrait, changement d'ordre et éjection ne
  se chevauchent pas ; une opération demandée pendant une autre est refusée.
- **Les envois se font les uns après les autres** (file d'envoi). Une histoire déjà présente
  n'est renvoyée que sur demande (« Remplacer sur la Flam »).
- **Nombre et taille des histoires** : le launcher n'affiche pas plus de 5000 histoires (la liste
  est alors tronquée). Les limites réelles de la Flam (nombre d'histoires, taille) n'ont pas
  été mesurées : non vérifié.
- **Lenteur** : l'envoi copie l'histoire, puis la relit en entier pour la vérifier, donc la
  durée est à peu près **double** de la simple copie. Une estimation de l'ordre de 4 minutes
  pour 80 Mo figure dans les notes de conception, mais elle n'a pas été mesurée. Le launcher
  affiche une progression et un temps restant estimé ; l'envoi peut être annulé.
- **Le launcher n'est pas l'application officielle** : il ne se connecte pas à votre compte,
  et ne peut ni acheter ni télécharger d'histoire officielle.
- Les titres et vignettes de « Ma Flam » sont lus après l'affichage de la liste. Pour les
  titres introuvables sur la Flam, le launcher interroge le catalogue officiel en ligne
  (requêtes HTTPS ; rien n'est envoyé depuis la Flam). Sans titre disponible (par exemple une
  histoire d'un autre compte), elle est affichée avec son identifiant court.

## Après un envoi, un retrait ou un changement d'ordre

- **Redémarrez la Flam** : elle reconstruit alors sa bibliothèque et affiche les nouvelles
  histoires (ou le nouvel ordre). Le launcher supprime pour cela le fichier de cache
  `usr/0/library.cache` ; la reconstruction par la Flam a été constatée sur firmware 1.x.
- **Éjectez la Flam** avec le bouton « Éjecter la Flam » du launcher, ou depuis l'Explorateur
  (clic droit sur le lecteur, puis « Éjecter »), **avant** de la débrancher. Ne la débranchez
  jamais pendant un envoi.
- Le launcher ne force jamais l'éjection : si un programme utilise encore la Flam (une fenêtre
  de l'Explorateur ouverte dessus, par exemple), l'éjection est refusée et un message vous
  invite à éjecter depuis l'Explorateur.

<a id="risques"></a>

## Risques

### Corruption des données de la Flam

La Flam utilise un système de fichiers FAT, qui n'a pas de journal : un débranchement ou une
panne au mauvais moment peut abîmer des fichiers. Le launcher réduit ce risque, sans pouvoir
l'écarter :

- **Écriture dans un dossier temporaire, puis renommage.** L'histoire est copiée dans
  `.flam-transfer/` (à la racine de la Flam, hors de `str/`), chaque fichier est écrit sur le
  disque puis **relu et comparé** (taille et empreinte SHA-256). Ce n'est qu'ensuite que le
  dossier est renommé vers `str/<identifiant>/`. En cas d'échec, d'annulation ou d'espace
  insuffisant avant ce renommage, `str/` et l'index ne sont pas modifiés et le dossier
  temporaire est supprimé.
- **Index réécrit de façon sûre.** La liste des histoires (`etc/library/list`) n'est jamais
  tronquée ni supprimée avant qu'une copie complète du nouvel index soit écrite et relue ; une
  copie temporaire `list.bak` de l'ancien index est faite avant le remplacement (supprimée
  une fois l'écriture terminée : ce n'est pas une sauvegarde à conserver). À chaque détection de
  la Flam, le launcher reprend une opération interrompue (fin du transfert ou retour en arrière)
  et vous le signale par un bandeau.
- **Retrait dans l'ordre inverse** : l'histoire est d'abord retirée de l'index, puis son dossier
  est déplacé dans `.flam-trash/` avant d'être supprimé. Vos fichiers de progression sur la
  Flam (`usr/0/`, hors le cache `library.cache`) ne sont jamais touchés.
- **Aucune écriture sans votre demande** (hors reprise automatique d'une opération du
  launcher interrompue), et rien n'est écrit sur la Flam avant les
  vérifications (clés, index valide, espace libre).
- **Sauvegarde du `.mdf` sur le PC.** À chaque affichage de « Ma Flam », une copie du fichier
  `.mdf` de la Flam est enregistrée dans `%LOCALAPPDATA%\flam-launcher\device-backups`. **Ce
  n'est pas une sauvegarde de vos histoires** : le launcher ne sauvegarde ni les histoires, ni
  l'index, ni vos progressions. C'est pourquoi le guide testeur demande de copier **tout le
  contenu de la Flam à la main avant le premier envoi**.
- Un index illisible, abîmé ou absent n'est jamais réécrit par un envoi, un retrait ou un
  changement d'ordre : l'opération est refusée.

Limites : ces protections ont été vérifiées par des tests sur un faux appareil, et sur une
Flam en firmware 1.x. Le comportement du firmware face à un dossier temporaire à la racine, à
un index absent ou tronqué, ou à des fichiers de travail dans `etc/library/` n'est pas
entièrement vérifié (tests sur appareil restants : index absent ou tronqué, limites). Sur
firmware 2.x, rien n'a été essayé.

### Firmware

- Le launcher écrit dans des fichiers que le fabricant n'a pas prévu de voir modifiés. Une
  **mise à jour du firmware** peut changer leur format ou leur lecture, et rendre le launcher
  inopérant ou les histoires envoyées illisibles. Ne faites pas de mise à jour du firmware au
  milieu d'un test sans avoir lu les retours des autres testeurs.
- Une histoire envoyée peut ne pas se lancer : en cas de doute, retirez-la avec le launcher.

### Garantie

L'envoi d'histoires depuis un logiciel tiers n'est pas prévu par le fabricant : la garantie ou
l'assistance de la Flam peuvent ne pas le couvrir. Renseignez-vous auprès du fabricant si cela
vous préoccupe. Vous utilisez cette fonction à vos risques.

### Usage et légalité

Usage strictement personnel. flam-launcher ne fournit aucune clé, ne copie ni n'exporte aucune
histoire officielle. En firmware 1.x, il réutilise une clé d'une histoire de votre propre
compte, à votre demande, après un avertissement à accepter. N'envoyez que des histoires que
vous avez le droit d'utiliser.

### Synchronisation avec l'application officielle

L'application officielle pourrait, lors d'une synchronisation, supprimer ou écraser des
histoires qu'elle ne connaît pas (hypothèse, **non vérifiée** : aucun test avec l'application
officielle n'a encore été fait).

- **Le masquage des histoires n'est pas implémenté.** Le launcher sait lire l'index des
  histoires masquées (`list.hidden`, dossier `str.hidden/`) : une histoire masquée est affichée
  avec la mention « Masquée », et un retrait la supprime des deux index. Mais il **ne déplace
  aucune histoire vers `str.hidden/`** et n'écrit pas `list.hidden`, hors retrait.
- Conseil, tant que le masquage n'existe pas : **ne synchronisez pas la Flam avec
  l'application officielle sans avoir copié son contenu sur le PC** (voir le guide testeur).
  Les histoires installées avec le launcher restent dans « Mes histoires » (tant que vous ne
  videz pas le cache) et peuvent être renvoyées.

## Dépannage

| Symptôme                                                                  | Piste                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| « Aucune Flam détectée » / « Activez le mode connexion USB sur la Flam »  | Activez le mode connexion USB dans les réglages de la Flam. Utilisez un câble USB qui transmet les données (certains câbles ne servent qu'à charger). Vérifiez que la Flam apparaît comme un disque dans l'Explorateur. Cliquez sur **Réessayer**                                                                                        |
| La Flam n'est pas détectée alors qu'un disque apparaît                    | Le launcher cherche un fichier `.mdf` à la racine d'un disque amovible (ou d'un disque FAT). Si le disque n'en contient pas, ce n'est pas une Flam reconnue                                                                                                                                                                              |
| Plusieurs Flam                                                            | Seule la première est affichée : débranchez les autres                                                                                                                                                                                                                                                                                   |
| Avertissement « À noter » sur `.mdf` (taille inattendue, tronqué)         | Le fichier `.mdf` de la Flam n'est pas celui attendu. Un `.mdf` tronqué ou démesuré n'est ni lu ni sauvegardé et empêche l'envoi. Ne continuez pas : signalez-le (voir le guide testeur)                                                                                                                                                 |
| « Firmware de la Flam non pris en charge »                                | Seuls les firmwares 1.x et 2.x sont pris en charge. Indiquez le firmware affiché sur la carte « Flam détectée » dans votre retour                                                                                                                                                                                                        |
| « Impossible d'éjecter la Flam »                                          | Fermez les fenêtres de l'Explorateur et les programmes qui utilisent la Flam, attendez la fin de toute opération du launcher, puis réessayez. Sinon : clic droit sur le lecteur dans l'Explorateur, « Éjecter », puis débranchez                                                                                                         |
| Une histoire est illisible sur la Flam (titre illisible, ne se lance pas) | Retirez-la avec **Retirer** dans « Ma Flam » puis renvoyez-la. En firmware 1.x, le launcher relit le titre après l'envoi et signale un titre illisible ; vérifiez alors la clé choisie. Si cela se reproduit, signalez-le                                                                                                                |
| Bandeau sur l'index des histoires de la Flam (absent, abîmé ou illisible) | Le launcher refuse alors d'écrire (aucun envoi, retrait ni changement d'ordre) pour ne rien aggraver, et ne propose pas de reconstruction. Les dossiers trouvés sont affichés comme « Non listée ». Suivez la restauration ciblée complète du [guide testeur](guide-testeur.md#restaurer) (ne recopiez pas seulement `etc/library/list`) |
| Bandeau « Un transfert interrompu a été repris »                          | Normal après un débranchement pendant un envoi : le launcher a remis la Flam dans un état cohérent. Relancez l'envoi si besoin                                                                                                                                                                                                           |
| Les nouvelles histoires n'apparaissent pas sur la Flam                    | Redémarrez la Flam. Vérifiez qu'elles figurent bien dans « Ma Flam »                                                                                                                                                                                                                                                                     |
| Disque de la Flam plein                                                   | Retirez des histoires avec le launcher. Un envoi est refusé s'il ne reste pas assez de place                                                                                                                                                                                                                                             |

Pour restaurer la Flam à son état d'origine, recopiez votre copie (voir le
[guide testeur](guide-testeur.md#restaurer)).
