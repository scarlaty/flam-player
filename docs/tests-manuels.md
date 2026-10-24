# Tests manuels du mainteneur

> Procédure unique, ordonnée par priorité, des tests qui ne peuvent se faire que sur la vraie
> Flam ou sur la machine du mainteneur. Elle remplace les listes éparpillées dans #236, #219,
> #413, #470, #443 et #462 (ces issues restent le lieu où **consigner** les résultats).
> Rédigée le 2026-10-24 d'après `main` (dernière fusion : #480). Les points qui dépendent d'une
> PR non fusionnée sont marqués « après fusion de … ».

## Tableau récapitulatif

Cocher la case une fois le résultat consigné dans l'issue indiquée.

| Fait | N° | Test | Session | Matériel | Durée | Consigner dans |
| :-: | --- | --- | --- | --- | --- | --- |
| [ ] | M0 | Sauvegarde complète de la Flam et préparation | tous | Flam 1.x ou 2.x | 20 min | (aucun) |
| [ ] | M1 | Mise à jour NSIS depuis `launcher-v0.1.0-beta.1` (sans Python) | A | VM ou compte Windows propre | 45 min | #236 |
| [ ] | M2 | Packs de podcasts sur la Flam : carrousel, enchaînement, retour, capacité | B | Flam 1.x | 1 h 30 | #219 |
| [ ] | M3 | Glisser-déposer de l'arborescence du pack (WebView2) | B | installation de M1 ou de B | 10 min | #413 |
| [ ] | M4 | Fumée de l'envoi carrier avec le convertisseur Rust (installateur N) | B | Flam 1.x | 30 min | #236 |
| [ ] | M5 | Voix Windows : test réel, voix « en ligne », sans réseau | A | poste de dev, voix FR | 20 min | #462 |
| [ ] | M6 | Mesure `sync_all` sur clé USB | A | clé USB, poste de dev | 15 min | #443 |
| [ ] | M7 | Index absent ou tronqué, limites d'histoires (#236 points 4 et 6) | C | Flam 1.x, **sauvegarde faite** | 1 h | #236 |
| [ ] | M8 | Application Lunii officielle : synchronisation (#236 point 8) | C | Flam 1.x, **sauvegarde faite** | 30 min | #236 |
| [ ] | M9 | Flam firmware 2.x (#236 points 1 et 2, restes de #326) | D | Flam 2.x | 1 h | #236, #326 |
| [ ] | M10 | Deuxième Flam d'un autre compte (#236 point 7, #234) | D | 2 Flam, 2 comptes | 45 min | #236 |
| [ ] | M11 | Titres lus à voix haute dans un pack, sur la Flam | E | Flam 1.x | 45 min | #462 |

Ordre conseillé : A (PC seul, sans risque pour la Flam), puis B (une installation, une connexion
de la Flam), puis C (risqué, seulement après B et une sauvegarde vérifiée). D et E dépendent de
matériel ou de code qui manquent aujourd'hui.

## Sessions

| Session | Contenu | Ce qui se partage |
| --- | --- | --- |
| A, sur le PC, sans Flam | M1, M5, M6 | M5 et M6 utilisent le même poste de développement ; M1 se fait sur la VM |
| B, une installation, une connexion | M2, M3, M4 (puis M0 avant tout) | **une seule installation** de N (celle de M1, étape 4, ou une installation propre), **une seule connexion** de la Flam : M4 envoie la première histoire, M2 envoie les packs, M3 se fait dans l'éditeur déjà ouvert |
| C, risqué | M7, M8 | une connexion, après sauvegarde vérifiée ; restauration prévue en fin de test |
| D, matériel manquant | M9, M10 | à faire quand le matériel existe |
| E, après fusion | M11 | après fusion de la PR 2 de #462 (voir M11) |

## Règles communes

- **Aucune clé, aucun fichier `.mdf`, aucun numéro de série, aucun contenu d'histoire officielle**
  dans les comptes rendus (critère d'acceptation de #236). Ne coller que des SHA-256, des
  nombres, des durées et des captures sans titre officiel.
- Chaque compte rendu indique : modèle et firmware de la Flam, version du launcher (écran
  « Paramètres », rubrique « À propos »), date, et le résultat de **chaque** point (réussite
  comprise).
- Ne jamais **formater** la Flam. Ne pas la synchroniser avec l'application officielle ni mettre à
  jour son firmware entre M0 et la fin de la session (sauf M8, qui le fait exprès, en C).
- Histoire de test sans contenu nommé : « test46-v2.plain.pk » ou toute petite histoire générée
  par `telmi2flam` ; pour les podcasts, un flux RSS public de votre choix (aucun flux n'est
  imposé).

---

## M0. Sauvegarde et préparation (avant la session B ou C)

**But.** Pouvoir restaurer la Flam. Le launcher ne sauvegarde que le `.mdf`.

**Prérequis.** Flam branchée, mode connexion USB activé, éléments masqués affichés dans
l'Explorateur.

**Étapes.** Suivre [guide-testeur.md](../apps/flam-launcher/docs/guide-testeur.md), section 2
(« AVANT TOUT : copiez le contenu de votre Flam sur le PC ») : copie de tout le disque dans un
dossier daté, puis comparaison « Contient : N fichiers, M dossiers » et « Taille » des deux côtés.
La restauration est décrite dans la section « Restaurer » du même guide.

**Résultat attendu.** Nombres de fichiers et tailles identiques ; `str`, `etc` et `.mdf` présents
dans la copie.

**Durée.** 20 minutes. Rien à consigner (conserver le dossier jusqu'à la fin des tests).

---

## M1. Mise à jour NSIS depuis la pré-version sans Python (#470, #138, #139)

**But.** Vérifier que l'installateur N (sans Python) se pose par-dessus `launcher-v0.1.0-beta.1`
(qui livrait Python) : nettoyage de `telmi2flam\python\` par le désinstallateur de N-1 (hook
`installer-hooks.nsh`, #180), conservation des réglages et de la bibliothèque, disparition de la
clé `python_path`.

**Prérequis.** Machine ou VM Windows 10/11 propre, **compte de test distinct du profil réel**
(sans Python dans le PATH pour l'étape 7). Pas de Flam. Fusionnés : #138 (31378b3) et #139
(02d76f3).

**Préparation.**
1. Récupérer N-1 : `gh release download launcher-v0.1.0-beta.1 -p "*.exe" -p "SHA256SUMS*" -D <dossier N-1>`.
2. Construire N depuis `main` (README du launcher, « Installateur Windows » : `npm run build:installer`
   depuis `apps/flam-launcher/`) ou prendre l'artefact NSIS de la CI de `main`. Vérifier le SHA-256
   avec `Get-FileHash -Algorithm SHA256`.

**Étapes.**
1. Installer N-1 (par utilisateur, dossier par défaut `%LOCALAPPDATA%\Flam Launcher`). Constater
   `telmi2flam\python\python.exe`.
2. Lancer N-1, écran **Paramètres** : saisir un `python_path` (n'importe quel `python.exe`), un
   `cache_dir` personnalisé, thème Sombre, vignettes « Rogner », 3 téléchargements simultanés.
   Installer ou importer une histoire. Fermer l'application.
3. Copier la configuration (`settings.json`, `library`, `sources.json` sous `%APPDATA%\flam-launcher`)
   vers un dossier de comparaison. Déposer aussi un fichier étranger (`etranger.txt`) dans le
   dossier d'installation.
4. Lancer l'installateur N **par-dessus**, en interactif. Répéter plus tard sur une copie de la VM
   en silencieux : `<N>.exe /S`.
5. Dans `%LOCALAPPDATA%\Flam Launcher` : `telmi2flam\telmi2flam.exe` présent ; **plus aucun**
   `telmi2flam\python\`, `telmi2flam\telmi2flam.py`, `runtime\` ; `etranger.txt` conservé ;
   `dir /s /b` identique à celui d'une installation propre de N (hors `etranger.txt`).
6. Lancer N : Paramètres sans champ « chemin de Python » ; thème, vignettes, 3 téléchargements et
   dossier de cache **conservés** ; bibliothèque et sauvegardes intactes ; le catalogue se charge.
7. Dans un shell avec `set PATH=%SystemRoot%\system32`, lancer `flam-launcher.exe`, installer une
   histoire Telmi : la conversion réussit sans Python sur la machine.
8. Changer un réglage (par exemple le thème), puis ouvrir `settings.json` : la clé `python_path`
   a disparu, les autres sont inchangées.
9. Désinstaller depuis « Applications installées », case « supprimer les données » décochée :
   dossier d'installation vide, données utilisateur conservées.

**Résultat attendu.** Chaque point ci-dessus vrai, en interactif et en silencieux.

**Consigner dans #236** : versions N-1 et N, mode (interactif ou silencieux), résultat de chaque
point, écarts, sortie de `dir /s /b` avant et après. **Durée** : 45 minutes (une seule variante :
30 minutes).

*Conseil : garder l'installation obtenue à l'étape 4 pour la session B.*

---

## Session B : une installation, une connexion de la Flam

**Prérequis communs.** Flam firmware 1.x (testée : 1.15.14), mode connexion USB, **M0 fait**,
installateur N installé (celui de M1 convient), une histoire officielle du compte sur la Flam
(source de la clé `bt`, mode carrier : voir
[flam-carrier.md](flam-carrier.md) § 3 ; le launcher demande la clé et un consentement). Ne
fermer ni le launcher ni la Flam entre M4, M2 et M3 ; **éjecter et redémarrer la Flam une seule
fois, à la fin**.

### M4. Fumée de l'envoi carrier avec le convertisseur Rust (#236)

**But.** Les validations 1.x du 2026-10-19 ont précédé le passage du convertisseur par défaut au
Rust (D6, 1337bab) et l'installateur sans Python : vérifier que le chemin complet fonctionne
encore avec l'installateur N.

**Étapes.**
1. Ouvrir **Ma Flam** : la carte « Flam détectée » indique le firmware (noter la ligne entière) ;
   titres et vignettes s'affichent.
2. **Mes histoires** : installer une petite histoire Telmi (catalogue ou dossier local).
3. **Envoyer sur la Flam**, lire l'avertissement carrier, choisir ou saisir la clé, confirmer.
   Noter la durée et la taille de l'histoire. Ne pas débrancher.
4. Dans **Ma Flam** : la nouvelle histoire apparaît avec son titre.
5. Essayer **Modifier l'ordre** (déplacer, enregistrer) : à contrôler au redémarrage.
6. Passer à M2 et M3, puis en fin de session : **Éjecter la Flam**, débrancher, **redémarrer** la
   Flam.
7. Sur la Flam : l'histoire apparaît avec titre et images, se lance, le son passe, l'ordre choisi
   est repris.
8. Rebrancher, **Retirer** l'histoire de test (et les packs de M2 si on ne les garde pas),
   redémarrer : elle a disparu, les autres histoires sont intactes.

**Résultat attendu.** Aucune erreur ; histoire jouable ; ordre repris ; retrait propre.

**Consigner dans #236.** Firmware, version du launcher, durée d'envoi, résultat de chaque étape.
**Durée** : 30 minutes (plus les durées de M2 et M3).

### M2. Packs de podcasts sur la Flam (#219)

**But.** Trancher ce que l'analyse ne peut pas : fluidité du carrousel, lecture, enchaînement
des épisodes et retour au menu, bouton retour d'un sous-menu, capacité, limites N2 (3 niveaux,
50 options) et plafonds D1 (50 épisodes, 200 Mo par épisode, 1 Gio par pack).

**Prérequis.** Un ou plusieurs flux RSS publics de votre choix : au moins un avec des MP3 mono
(copiés tels quels) et un avec des MP3 stéréo (réencodés), pour les mêler dans **un même pack** ;
le message de droits s'affiche à la première création (« J'ai compris »). Aucun pack de test ne
doit être versionné dans le dépôt.

**Étapes.**
1. Écran **Sources** : ajouter chaque flux (source podcast), puis **Podcasts** : cocher des
   épisodes (« Cocher les 10 derniers épisodes » aide).
2. **Composer le pack** et créer, chaque fois depuis la file d'installation (progression et
   annulation dans le panneau des tâches), les packs suivants :
   - a. menu de **1** épisode ;
   - b. menu de **10** épisodes ;
   - c. menu de **50** épisodes (plafond) ;
   - d. **sous-menus sur 3 niveaux** ;
   - e. pack mêlant **MP3 mono et stéréo** ;
   - f. pack **multi-flux** (épisodes d'au moins deux flux) ;
   - g. pack avec des titres de **40 caractères et plus** ;
   - h. pack avec **pochettes** JPEG et WebP (couverture du pack et pochettes des épisodes).
   Un même pack peut cumuler plusieurs de ces points (par exemple e, f et g dans le pack d).
3. Les envoyer sur la Flam depuis **Mes histoires** (chaque envoi : noter durée et taille).
4. Dans **Ma Flam**, relever l'**espace libre** avant le premier envoi et après le dernier :
   capacité utile de la Flam.
5. Éjecter, redémarrer la Flam (fin de session B). Pour chaque pack, noter sur l'appareil :
   - temps de chargement du menu ; fluidité du carrousel (50 options : saccades ?) ;
   - titres de 40 caractères et plus : coupés ou lisibles ;
   - lecture d'un épisode sans coupure ;
   - **enchaînement** : un épisode suit le suivant du même menu ; la fin du dernier épisode
     revient à **ce menu** (N4 a) ;
   - **bouton retour** depuis un sous-menu ;
   - pochettes affichées ;
   - lecture d'un MP3 mono et d'un MP3 stéréo dans le même pack : même volume ? même vitesse ?
6. Depuis le launcher (avant de redémarrer), **Retirer** les packs devenus inutiles.

**Résultat attendu.** Packs lisibles de bout en bout ; carrousel fluide à 50 options ;
enchaînement et retour conformes à la spécification (`podcasts.md` § 4, point 5).

**Consigner dans #219.** Tableau : pack, nombre d'épisodes, taille, durée d'envoi, chargement du
menu, fluidité, enchaînement, retour, capacité de la Flam, mémoire si observable. Proposer les
limites ajustées (N2, D1) si une mesure l'exige : le tableau sera repris dans la documentation du
launcher (critère d'acceptation de #219). **Durée** : 1 h 30.

### M3. Glisser-déposer de l'arborescence du pack (#413)

**But.** Vérifier dans l'application réelle (WebView2) ce que les tests Vitest ne couvrent pas :
`PackTree.tsx`.

**Étapes.**
1. Écran **Podcasts**, **Composer le pack** avec au moins 5 épisodes et 2 catégories.
2. Glisser un épisode d'une catégorie vers une autre, vers la racine, puis vers un sous-menu.
3. Glisser une catégorie entière ; tenter un 4e niveau de menu ; tenter de dépasser 50 options
   dans un menu.
4. Refaire un déplacement au clavier (parcours complet au clavier exigé par la spécification).
5. Fermer et rouvrir l'éditeur : le brouillon est repris dans l'ordre choisi.

**Résultat attendu.** Les déplacements autorisés se font et se voient ; le 4e niveau et le 51e
élément sont refusés avec un message ; l'ordre est conservé après réouverture.

**Consigner dans #413** (case « Mainteneur : vérifier le glisser-déposer »). **Durée** : 10 minutes.

---

## Session A : sur le poste de développement

### M5. Voix Windows : test réel, voix « en ligne », sans réseau (#462)

**But.** Confirmer que le module de synthèse vocale (fusionné : #469) liste et utilise les voix
du poste, et que l'heuristique `french_voices` écarte bien les voix « en ligne » (`podcasts.md`
§ 12.8).

**Prérequis.** Windows avec au moins une voix française (Paramètres, Heure et langue, Voix) ;
idéalement une voix « naturelle » en ligne installée en plus. Rust et Visual Studio 2022 (README
du launcher, « Développement »). Fusionné : #469 (module et test `voix_reelle_du_poste`).

**Étapes.** Depuis `apps/flam-launcher/src-tauri/` :
1. Test de base, réseau actif :
   ```powershell
   cargo test --lib -- --ignored --nocapture voix_reelle
   ```
   Relever la liste « voix installées (N) », « voix françaises locales » et, pour chacune, les
   Hz, canaux, durée et temps de synthèse. Sans voix française, le test le dit et réussit.
2. Écrire les WAV pour les écouter : `$env:FLAM_TTS_OUT = "C:\Temp\tts-test"` (dossier existant,
   hors dépôt), relancer, ouvrir les fichiers.
3. Lecture directe : `$env:FLAM_TTS_PLAY = "1"`, relancer : « Bonjour » est dit par chaque voix
   française locale.
4. **Sans réseau** : couper le Wi-Fi et débrancher l'Ethernet (ou mode avion), relancer
   le test de l'étape 1. Rétablir le réseau ensuite.
5. Comparer les listes : toute voix dont le nom ou l'identifiant contient « online » ou « en
   ligne » doit figurer dans « voix installées » mais **pas** dans « voix françaises locales ».
6. Nettoyer : `Remove-Item Env:FLAM_TTS_OUT, Env:FLAM_TTS_PLAY`.

**Résultat attendu.** Étape 4 : la synthèse réussit sans réseau pour toutes les voix retenues
(sinon, une voix en ligne a échappé au filtre : noter son nom et son identifiant) ; étape 5 :
aucune voix en ligne dans les voix locales ; durées d'audio supérieures à 0,2 s (le test
l'affirme).

**Consigner dans #462.** Version de Windows, liste des voix (nom et identifiant : ce ne sont pas
des secrets), résultat de chaque étape, voix en ligne non filtrée le cas échéant. **Durée** :
20 minutes.

### M6. Mesure `sync_all` sur clé USB (#443, #416)

**But.** La mesure de #416 n'a eu lieu que sur un disque local NTFS ; compléter par un volume
amovible (le `.plain.pk` publié sur la Flam passe par `sync_all`).

**Prérequis.** Une clé USB (idéalement FAT32, comme la Flam), 100 Mo libres, Rust stable.

**Étapes.** Depuis la racine du dépôt :
1. Disque local de référence :
   ```powershell
   cargo test -p telmi2flam --release mesure_sync_all -- --ignored --nocapture
   ```
2. Clé USB (remplacer `E:\` par sa lettre) :
   ```powershell
   $env:T2F_MESURE_DIR = "E:\"
   cargo test -p telmi2flam --release mesure_sync_all -- --ignored --nocapture
   Remove-Item Env:T2F_MESURE_DIR
   ```
   `T2F_MESURE_FICHIERS` (500 par défaut) règle le nombre de fichiers.
3. Relever les lignes `MESURE-V <mode> : <n> fichiers : <ms>` pour les quatre modes (« aucun »,
   « par fichier », « final sequentiel », « final 8 fils »), trois tours chacun. Fermer
   l'antivirus n'est pas demandé : noter s'il est actif.

**Résultat attendu.** Chiffres comparables à PORT-RUST.md § 3.1 (local : 0,34 à 0,40 s sans
vidage, 1,6 s par fichier) ; sur clé, le « par fichier » doit rester de l'ordre de quelques
secondes. Un écart d'un ordre de grandeur justifierait de revoir le vidage par fichier.

**Consigner dans #443** (« Mesure sync_all sur clé USB ») : modèle de clé, système de fichiers,
les quatre moyennes. **Durée** : 15 minutes.

---

## Session C : risqué, après B et une sauvegarde vérifiée

**Règle.** Refaire M0 si la Flam a été modifiée depuis. Prévoir la restauration ciblée
([guide-testeur.md](../apps/flam-launcher/docs/guide-testeur.md), « Restaurer »).

### M7. Index absent ou tronqué, limites d'histoires (#236, points 4 et 6)

**But.** Comportement du firmware quand l'index `etc/library/list` est absent ou tronqué ;
limites observables d'histoires (voir `flam-device-format.md` § 10, T4, T6, T9).

**Prérequis.** Flam 1.x, M0 vérifiée, launcher **fermé** pendant les étapes 2 à 6 (il répare
l'index à l'ouverture de « Ma Flam »).

**Étapes.**
1. Copier `etc\library\list` sur le PC (dossier de test, hors dépôt). Noter le nombre de lignes.
2. *Index tronqué.* Sur la Flam, retirer la dernière ligne de `list` (éditeur qui garde les fins
   de ligne LF), éjecter, débrancher, **redémarrer**. Observer : la Flam démarre-t-elle ? une
   histoire manque-t-elle ? le dossier de l'histoire retranchée subsiste-t-il ?
3. Rebrancher, rouvrir le launcher : « Ma Flam » signale-t-elle l'anomalie ? un dossier `str/<uuid>/`
   sans entrée d'index est-il listé comme orphelin ? Noter les messages. Fermer le launcher.
4. *Index absent.* Renommer `etc\library\list` en `list.test`, éjecter, **redémarrer**. Observer
   (liste vide ? reconstruction ? écran noir ?).
5. Rebrancher, **restaurer** : remettre la copie de l'étape 1 sous le nom `list`, supprimer
   `list.test`, supprimer `usr\0\library.cache`, éjecter, redémarrer : les histoires sont toutes
   revenues. Si ce n'est pas le cas, restauration ciblée du guide testeur.
6. Ouvrir le launcher : plus aucune anomalie.
7. *Limites* (facultatif, long) : envoyer des histoires de test minuscules l'une après l'autre
   jusqu'à un refus ou une anomalie (liste de la Flam, démarrage), en notant le nombre
   d'histoires atteint et l'espace libre. Retirer ensuite toutes les histoires de test.

**Résultat attendu.** Aucune perte d'histoire officielle ; la Flam reconstruit sa bibliothèque ;
après restauration, l'index est identique à la copie (comparer les SHA-256).

**Consigner dans #236** (points 4 et 6) et compléter le tableau T4, T6 et T9 de
`flam-device-format.md` (§ 10) lors de la mise à jour des docs. **Durée** : 1 heure (sans
l'étape 7).

*Hors périmètre : la longueur des noms et la casse (FAT) du point 6 ne sont pas testables avec le
launcher, qui impose des dossiers à UUID minuscules.*

### M8. Application Lunii officielle (#236, point 8)

**But.** Effet d'une synchronisation officielle sur les histoires envoyées par le launcher
(suppression des histoires non officielles ? conservation de l'ordre ?).

**Prérequis.** Application officielle installée et compte relié ; une histoire de test déjà
envoyée et visible sur la Flam ; **M0 vérifiée**. Le guide testeur déconseille cette
synchronisation : c'est ici un test voulu, à ne faire qu'avec une copie fraîche.

**Étapes.**
1. Noter l'état : nombre d'histoires, ordre, SHA-256 de `etc\library\list` (pas du `.mdf`).
2. Ouvrir l'application officielle, laisser synchroniser la Flam. Noter chaque message.
3. Quitter l'application officielle, rouvrir « Ma Flam » : l'histoire de test est-elle
   toujours là, l'ordre est-il conservé, `list` a-t-il changé ?
4. Éjecter, redémarrer : la Flam affiche-t-elle l'histoire de test ?
5. Si la Flam a changé de firmware pendant la synchronisation : **ne pas restaurer** ; le noter.

**Résultat attendu.** Information seulement : consigner ce qui se passe, quel que soit le résultat.

**Consigner dans #236** (point 8). **Durée** : 30 minutes.

---

## Session D : matériel manquant

### M9. Flam firmware 2.x (#236 points 1 et 2, restes de #326)

**Prérequis.** Une Flam en 2.x, **M0** faite. Les testeurs de la bêta (#316) ont ces points
([guide-testeur.md](../apps/flam-launcher/docs/guide-testeur.md), « Points à vérifier en 2.x »,
retour par le modèle « Retour testeur Flam »).

**Étapes.** Suivre le guide testeur en entier (installation, M0, tests 1 à 8, points 1 à 8 de
« Points à vérifier en 2.x »), puis relever en plus, pour clore #326 :
1. Format de `usr\0\library.cache` après un envoi et un redémarrage : le titre de l'histoire
   envoyée y figure-t-il ? (preuve par le cache liée au titre).
2. Règle « précédé de sa longueur » de `key` (octet de poids faible) : taille et en-tête du
   `.mdf` 2.x, forme de la version (sans clé ni numéro de série).
3. Empreinte du `.mdf` : le firmware modifie-t-il les 16 octets à `0x6E` ? Comparer le SHA-256
   du `.mdf` avant et après un redémarrage (SHA-256 seulement dans le compte rendu).

**Résultat attendu.** Réponses aux points T1, T2, T3, T4 et T9 de `flam-device-format.md` § 10
pour la 2.x ; décision go/no-go de #236 pour sortir de l'expérimental.

**Consigner dans #236 et #326.** **Durée** : 1 heure.

### M10. Deuxième Flam d'un autre compte (#236 point 7, #234)

**But.** Confirmer qu'un couple `bt`/`key` d'un autre compte est refusé par la Flam et par la
vérification du launcher (flam-carrier.md § 6, C5, C6).

**Prérequis.** Deux Flam de comptes différents, **M0 de chacune**.

**Étapes.**
1. Sur la Flam A, saisir (ou choisir) la clé d'une histoire de la Flam B : le launcher doit refuser
   (« cette clé ne correspond à aucune histoire officielle de cette Flam »), rien n'est écrit.
2. Si la clé est acceptée (par erreur d'usage), arrêter : ne pas envoyer.
3. Retirer sur la Flam B l'histoire officielle dont la clé a été conservée pour elle, envoyer une
   histoire de test avec le couple conservé : le comportement est-il celui attendu (C5) ?
4. Noter si l'histoire, une fois retirée la source de la clé, reste lisible.

**Consigner dans #236** (point 7). **Durée** : 45 minutes.

---

## Session E : après fusion

### M11. Titres lus à voix haute dans un pack (#462, PR 2)

**Dépend de** la PR 2 de #462 (case « Lire les titres à voix haute » dans l'éditeur de pack,
création du pack avec les audios) : **après fusion de la PR 2 de #462** (non encore ouverte au
2026-10-24 ; seuls le module et les commandes `tts_*` sont fusionnés, #469). Aussi **après
vérification du contrat des voix Microsoft** (décision 4 de #462, avant cette fusion).

**Étapes** (d'après `podcasts.md` § 12.8) :
1. Éditeur de pack : cocher « Lire les titres à voix haute », choisir la voix.
2. Créer un pack de 10 titres, l'envoyer sur la Flam (session B), redémarrer.
3. Sur l'appareil : titre lu **au focus** de chaque option, **coupure au défilement**, volume
   comparable aux épisodes.
4. Sans voix française (VM ou voix retirée) : case grisée avec message ; un pack reste créable.

**Consigner dans #462.** **Durée** : 45 minutes.

---

## Après les tests

Rapporter dans chaque issue la date, le matériel et le résultat de chaque point ; fermer ou
laisser ouverte l'issue selon ses critères d'acceptation (#219 : tableau des résultats publié
dans la documentation ; #236 : décision go/no-go).
