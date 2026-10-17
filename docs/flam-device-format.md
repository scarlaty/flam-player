# Format de la Flam branchée en USB

Document de référence du format observé sur la Flam (volume FAT du « mode connexion USB »).
Il sert de base à `device::detect` (#225), `device::index` (#226), `device::crypto` et
`device::prepare` (#228), au transfert (#229), au retrait (#231), et aux tests sans appareil
(faux appareil : dossier temporaire contenant un `.mdf` synthétique). Issue : #224, épopée : #222.

Les formats sont décrits ici **en mots**, pour l'interopérabilité. Ce document ne contient
**aucune clé, aucun bloc de clé, aucun contenu d'histoire officielle**.

## 1. Statuts et sources

Chaque affirmation porte un statut :

| Statut | Sens |
|---|---|
| **mesuré** | constaté sur un appareil réel ; la source et la date sont indiquées |
| **supposé** | issu d'une source écrite non vérifiée sur appareil (rapport interne, doc des PR) |
| **à vérifier** | inconnu ou contradictoire ; renvoi au tableau du § 10 (#236) |

Sources citées :

- **choix de conception** : décision prise par le code d'une PR, sans mesure.
- **[inv]** : `flam-device-inventory.md`, scan du volume d'une Flam le 2026-06-14, par listage de
  l'arborescence et des tailles. La version de firmware de cet appareil n'y est **pas consignée**.
- **[rapport]** : analyse interne du 2026-10-17 (non versionnée), faite sur documentation et sur
  le code existant, **sans appareil**. Rien de ce qui en vient n'est mesuré.
- **[#241]**, **[#246]**, **[#247]** : code et documentation des PR ouvertes `device::detect`,
  `device::index`, `device::crypto` + `device::prepare`. Ces PR décrivent la disposition de
  `.mdf` et le fichier `key` ; la mesure sur appareil de ces positions n'est pas consignée dans
  le dépôt : statut **supposé** tant que #236 ne les a pas confirmées.

Couverture des firmwares : **aucune version mesurée n'est documentée pour une fw 2.x ni pour une
fw 1.x**. Les firmwares 1.15.14 et 2.4.2 sont cités par [rapport] comme versions rencontrées,
sans mesure dans ce dépôt. Les critères de #224 ne sont donc pas tous satisfaits tant que le
mainteneur n'a pas rempli la colonne « Mesure » du § 10.

## 2. Détection

| Élément | Statut | Détail |
|---|---|---|
| Mode connexion USB | supposé [rapport] | La Flam n'expose ses fichiers que si l'utilisateur active le « mode connexion USB » dans ses réglages ; sinon elle ne fait que charger. Sans ce mode, aucun volume n'apparaît : message d'aide « Activez le mode connexion USB sur la Flam » ([#241]). |
| Volume | supposé [rapport] | Stockage de masse en **FAT** (FAT32 attendu). Sous Windows, [#241] retient les lecteurs amovibles et les lecteurs fixes dont le système de fichiers contient « FAT » (un disque USB peut se déclarer fixe) ; le disque système NTFS n'est jamais sondé. |
| Marqueur | supposé | Fichier `.mdf` à la racine du volume. Sa présence suffit à reconnaître une Flam. Aucun VID/PID USB n'est interrogé : VID et PID sont lus **dans** `.mdf`. |
| Taille de `.mdf` | mesuré [inv] | 126 octets (appareil de l'inventaire, firmware non consigné). [#241] traite moins de 126 octets comme tronqué et plus de 126 comme un avertissement ; lecture bornée à 64 Kio. |
| Dossiers attendus | mesuré [inv] | `str/` et `etc/library/` ; leur absence n'est qu'un avertissement dans [#241]. |
| Sauvegarde de `.mdf` | choix de conception [#241] | Copie personnelle dans le dossier de données du launcher (`device-backups`), nommée d'après le volume et les 16 premiers caractères hexadécimaux du SHA-256. Jamais dans le dépôt. |

## 3. Le fichier `.mdf`

Fichier à la racine du volume, lu par le launcher en **lecture seule**. Il contient, entre
autres, du matériel de clé propre à l'appareil : ne jamais l'afficher, le journaliser ni le
versionner (son empreinte SHA-256 peut l'être).

### 3.1 Disposition supposée (source : [#247], non mesurée ici)

| Offset (décimal / hex) | Taille | Champ | Statut |
|---|---|---|---|
| 0 / 0x00 | 2 | en-tête (valeur à consigner) | supposé |
| 2 / 0x02 | 48 | texte : « main: x.y.z-… » puis « comm: … », complété par des octets nuls | supposé |
| 50 / 0x32 | 24 | numéro de série en caractères ASCII hexadécimaux, complété par des octets nuls | supposé, **voir 3.2** |
| 74 / 0x4A | 2 | VID (petit-boutiste) | supposé |
| 76 / 0x4C | 2 | PID (petit-boutiste) | supposé |
| 78 / 0x4E | 16 | clé de chiffrement des histoires (fw 2.x) | supposé |
| 94 / 0x5E | 16 | IV de chiffrement des histoires (fw 2.x) | supposé |
| 110 / 0x6E | 16 | **non décrit** : 126 − 110 octets restants | à vérifier |

[#247] ne donne que la taille de chaque champ et l'offset 0x4E de la clé ; les offsets 50, 74, 76
et 94 sont **déduits** par addition des tailles et restent à confirmer avec le reste.

Valeurs matérielles citées par [rapport] pour une Flam de première génération : VID `0x303A`,
PID `0x819E` (supposé). Un matériel inconnu est toléré avec avertissement.

### 3.2 Numéro de série : ambiguïté à lever

[#247] décrit 24 caractères hexadécimaux « dont le numéro décodé fait 7 octets », ce qui est
incohérent (24 caractères hexadécimaux donnent 12 octets). Lecture la plus plausible : le champ
fait 24 octets dont **14 caractères hexadécimaux** (7 octets) suivis de **10 octets nuls**, ce
qui correspond aussi à la structure du fichier `key` (§ 6.2). Statut : **à vérifier** sur un
`.mdf` réel (T2 du § 10).

### 3.3 Différences entre firmwares

| Sujet | Firmware 1.x | Firmware 2.x |
|---|---|---|
| Matériel de clé dans `.mdf` | 32 octets bruts, sans séparation clé / IV (supposé [rapport]) | clé 16 octets puis IV 16 octets à 0x4E (supposé [#247]) |
| Import « classique » | produit une histoire illisible (titre illisible, ne se lance pas), constaté sur 1.15.14 d'après [rapport] | fonctionne : clés lues dans `.mdf` (supposé, jamais mesuré dans ce dépôt) |
| Prise en charge dans le launcher | hors du cœur ; issue séparée #234 (« carrier », consentement explicite) | oui ([#247] : `UnsupportedFirmware` hors du 2.x) |

Le firmware est identifié par le champ « main » (nombre avant le premier point). [#247]
n'accepte que la version majeure 2 : une 3.x serait refusée (choix prudent, à documenter dans
l'interface). La position des 32 octets de clé en 1.x n'est pas connue : **à vérifier** (T1).

## 4. Arborescence

Mesurée par [inv] (2026-06-14) sauf mention contraire :

```
<volume>/
├── .mdf                       126 octets
├── etc/
│   ├── library/
│   │   ├── list               index des histoires visibles (370 octets pour 10 histoires)
│   │   └── list.hidden        index des histoires masquées (0 octet : aucune masquée)
│   ├── bluetooth/  onboarding/  update/  wifi/   réglages de l'appareil : ne pas toucher
├── str/<uuid>/                une histoire installée et visible
├── str.hidden/<uuid>/         histoires masquées (supposé [rapport] ; absent de [inv])
├── tmp/bluetooth/scan/        temporaire de l'appareil
└── usr/0/
    ├── library.cache          185 829 octets sur l'appareil inventorié
    ├── <uuid>.prog            4 à 6 octets : progression
    ├── <uuid>.save            6 à 12 831 octets : état sauvegardé
    └── <uuid>/chaps.save      chapitres écoutés
```

- `<uuid>` : UUID en minuscules au format 8-4-4-4-12 (36 caractères). Mesuré [inv].
- `library.cache` : cache binaire reconstruit par le firmware quand il est absent (supposé
  [rapport] ; [#246] le supprime après chaque réécriture d'index). Il contient les titres
  déchiffrés (supposé). Statut de la reconstruction : **à vérifier** (T4).
- `str.hidden/` n'existe pas dans l'inventaire : création à la demande, ou présence
  conditionnée par un état de l'appareil ? **À vérifier** (T6).
- Des sauvegardes (`.prog`, `.save`, dossier) existent pour des UUID sans histoire installée
  (mesuré [inv] : 7 UUID orphelins). Les ignorer, ne jamais les supprimer.
- Système de fichiers FAT : noms insensibles à la casse, pas de journal. Noms d'entrées
  conseillés : `[A-Za-z0-9_.-]` par segment ([#247] refuse les caractères interdits sous
  Windows et les doublons à la casse près).

## 5. L'index `etc/library/list` et `list.hidden`

| Propriété | Valeur | Statut |
|---|---|---|
| Contenu | un UUID par ligne ; `list.hidden` : même format | supposé [rapport] |
| Fin de ligne | `\n` (LF), y compris après le dernier UUID | **mesuré** [inv] : 370 octets pour 10 histoires, soit 37 octets (36 + LF) ; avec CRLF on aurait 380 |
| Encodage | ASCII (UUID hexadécimaux et tirets), sans BOM | supposé |
| Ordre | ordre d'affichage sur la Flam | supposé [rapport] |
| Liste vide | fichier vide (0 octet) | **mesuré** [inv] pour `list.hidden` |
| Casse | minuscules | supposé ; mesuré pour les noms de dossiers [inv] |

Écriture par le launcher ([#246]) : UUID normalisés en minuscules, sans doublon (première
occurrence gardée), ordre conservé, LF final. À la lecture : BOM, CRLF, espaces, lignes vides et
majuscules sont tolérés ; lignes invalides et doublons sont ignorés et signalés ; un fichier de
plus de 1 Mio, contenant un octet nul ou sans aucun UUID est jugé illisible. Un index illisible, abîmé (lignes invalides ou doublons) ou absent
n'est jamais réécrit sans l'accord explicite de l'utilisateur (seul un `list.hidden` absent est
réécrit sans accord).

Comportement du firmware face à un index absent, tronqué ou contenant un UUID sans dossier :
**à vérifier** (T4).

## 6. Dossier d'une histoire (`str/<uuid>/`)

### 6.1 Contenu

| Chemin | Chiffré ? | Statut | Note |
|---|---|---|---|
| `info` | **oui** | présence mesurée [inv] | titre et sous-titre ; en clair, 4 lignes (titre, sous-titre, `000000`, titre) : **supposé** (jeu de test de #247) |
| `main.lsf` | **oui** | présence mesurée [inv] | script principal |
| `script/*.lsf` | **oui** | présence mesurée [inv] | modules du jeu |
| `key` | non (32 octets) | présence mesurée [inv] ; taille supposée [#247] | autorisation de l'histoire, voir § 6.2 |
| `version` | non | présence mesurée [inv] | texte ASCII court ; valeurs 1 à 4 relevées [inv] |
| `img/*.lif`, `img/script/*.lif` | non | supposé | images au format LIF |
| `sounds/*.mp3`, `*.mp3map` | non | supposé | audio ; dossier absent d'une histoire sans son [inv] |
| `bt` | — | supposé [rapport] | fichier de 32 octets présent dans certaines histoires officielles ; hors périmètre du cœur 2.x ; **ne jamais le lire, le copier ni le journaliser** |

### 6.2 Fichier `key`

32 octets (supposé [#247]) : le numéro de série de l'appareil en hexadécimal minuscule ASCII
(14 caractères), 10 octets nuls, puis les 8 premiers caractères de ce même hexadécimal. Ce
fichier est donc **déduit du numéro de série** (fw 2.x) et ne contient pas la clé de
chiffrement. Statut : **supposé**. Sur les histoires officielles d'un même compte lié, `key` est
identique d'une histoire à l'autre (supposé [rapport]). Sur fw 1.x, un `key` fabriqué n'est pas
accepté (§ 3.3).

### 6.3 Renommages depuis un `.plain.pk` ([#247], d'après [rapport])

| Entrée du `.plain.pk` | Dans `str/<uuid>/` | Traitement |
|---|---|---|
| `uuid.bin` | aucune | donne le nom du dossier (UUID en minuscules) |
| `_metadata.json`, `_thumbnail.png` | aucune | ignorées |
| `info.plain` (et tout `*.plain`) | `info` (nom sans `.plain`) | chiffré |
| `main.lua`, `script/*.lua` | `main.lsf`, `script/*.lsf` | chiffré |
| `version`, `img/…`, `sounds/…` | même nom | copiés tels quels |
| aucune | `key` | fabriqué depuis l'appareil |

## 7. Chiffrement

| Propriété | Valeur | Statut |
|---|---|---|
| Algorithme | AES-128 en mode CBC | supposé [rapport] ; primitive testée par les vecteurs NIST SP 800-38A dans [#247] |
| Fichiers concernés | `*.lsf` et `info`, sur toute leur longueur | supposé |
| Bourrage | octets nuls jusqu'au multiple de 16 suivant ; aucun bloc ajouté si déjà aligné ; fichier vide : reste vide | supposé ; la taille d'origine n'est pas récupérable, les octets nuls finaux font partie du clair (sans effet sur du texte) |
| IV | 16 octets, identique pour tous les fichiers de l'appareil | supposé [#247] ; à vérifier (T1) |
| Source de la clé et de l'IV, fw 2.x | `.mdf` de l'appareil (offsets 0x4E et 0x5E) | supposé |
| Source de la clé, fw 1.x | clé d'une histoire officielle du même compte (« carrier »), fournie par l'utilisateur | supposé [rapport] ; **hors cœur**, issue #234 |
| Non chiffrés | images, audio, `version`, `key` | supposé |

Règles du dépôt : jamais de clé réelle dans le code, les tests, les journaux ni les documents ;
tests par vecteurs publics ou inventés ; pas d'export d'histoires officielles.

## 8. Procédures

L'ordre protège contre un débranchement : FAT n'a pas de journal.

### 8.1 Installation (conception de #229, **non encore implémentée**)

1. Préparer l'histoire **hors appareil** ([#247] : chantier `.partiel-<uuid>-<aléa>` renommé en
   `str/<uuid>/` du dossier de travail une fois complet).
2. Vérifier : UUID absent des deux index et de `str/` et `str.hidden/`, espace libre suffisant,
   firmware pris en charge.
3. Copier dans un dossier temporaire **sur l'appareil**, avec progression et annulation
   (environ 4 minutes pour 80 Mo, supposé [rapport]). Emplacement (`str/.tmp-<uuid>` ou `tmp/`) :
   **à vérifier** (T3), le firmware pouvant énumérer `str/`.
4. Vérifier (tailles, relecture), puis renommer en `str/<uuid>/`.
5. Réécrire l'index ([#246]) : `list.new` écrit et relu, `list.bak` copié, remplacement de `list`,
   relecture, suppression de `usr/0/library.cache`, suppression de `list.bak`. Après
   interruption, `recover` rétablit un état sain au branchement suivant.

Un débranchement avant l'étape 5 laisse au pire un dossier non listé (orphelin) ; jamais un
index qui pointe vers du vide. Une histoire partielle n'est donc pas visible, sous réserve de T3 et T4 (si le firmware énumère `str/`, un dossier temporaire placé dans `str/` pourrait apparaître).

### 8.2 Retrait (conception de #231)

Ordre inverse : retirer l'UUID de l'index d'abord (§ 8.1, étape 5), puis supprimer le dossier
`str/<uuid>/` (ou `str.hidden/<uuid>/`). Les sauvegardes de `usr/0/` de cet UUID peuvent être
conservées ou supprimées selon le choix de l'utilisateur (à décider).

### 8.3 Masquage (conception, **non implémentée**)

Masquer une histoire avant une synchronisation avec l'application officielle, qui pourrait
supprimer les histoires qu'elle ne connaît pas (supposé [rapport], T7) : déplacer
`str/<uuid>/` vers `str.hidden/<uuid>/`, retirer l'UUID de `list` et l'ajouter à `list.hidden`.
Ordre proposé pour qu'aucun état intermédiaire ne perde l'histoire : ajouter à `list.hidden`,
déplacer le dossier, puis retirer de `list` (`str.hidden/` créé au besoin, T6). **Risque** : un
débranchement entre le déplacement et le retrait de `list` laisse `list` pointer vers un dossier
absent de `str/` (contraire au principe du § 8.1) ; le comportement du firmware est inconnu (T4)
et la reprise (`recover`, [#246]) ne traite aujourd'hui que chaque index séparément. Le déplacement
est un renommage FAT entre deux dossiers du même volume.

### 8.4 Ordre et fin de session

L'ordre d'affichage est celui de `list` : le réordonnancement réécrit l'index (§ 8.1, étape 5).
Éjecter proprement avant de débrancher, sans forcer : la Flam peut exécuter des tâches à
l'éjection (supposé [rapport]). Redémarrer la Flam pour qu'elle reconstruise son cache.

## 9. Empreintes de référence

Aucune empreinte n'est consignée à ce jour (aucun appareil disponible pour cette rédaction).
À renseigner par le mainteneur, SHA-256 uniquement, de fichiers **sans clé en clair** ni contenu
d'histoire officielle :

| Fichier | Firmware | Date | SHA-256 |
|---|---|---|---|
| `etc/library/list` | à renseigner | à renseigner | à renseigner |
| `.mdf` (entier) | à renseigner | à renseigner | à renseigner |

## 10. Points à confirmer sur appareil réel (#236)

| N° | Point | Section | Attendu | Mesure (firmware, date, résultat) |
|---|---|---|---|---|
| T1 | Disposition complète de `.mdf` (en-tête, offsets 50 à 78, clé et IV à 0x4E et 0x5E, rôle des 16 derniers octets) en fw 2.x et fw 1.x ; IV unique pour tous les fichiers ; acceptation par la Flam de fichiers chiffrés avec ces valeurs | 3, 7 | tableau 3.1 confirmé | |
| T2 | Numéro de série : 14 caractères + 10 nuls, ou 24 caractères ? Forme du `key` fabriqué | 3.2, 6.2 | 7 octets | |
| T3 | Dossier supplémentaire (`str/.tmp-*` ou `tmp/`) toléré, puis renommage | 8.1 | accepté | |
| T4 | Index absent, tronqué, UUID sans dossier ; reconstruction de `library.cache` ; titre lisible ; relances successives sans redémarrage | 4, 5 | reconstruction automatique | |
| T5 | Éjection logicielle, effet pendant une mise à jour | 8.4 | sans effet de bord | |
| T6 | `str.hidden/` : création à la demande ou présence à l'état neuf ; limites (nombre d'histoires, taille, longueur des noms, casse) | 4, 8.3 | à mesurer | |
| T7 | Synchronisation officielle : suppression des histoires non officielles, effet du masquage | 8.3 | masquage préservé | |
| T8 | Valeurs de `version` acceptées (1 à 4 relevées) ; firmwares 1.x et 2.x couverts ; version 3.x et suivantes | 3.3, 6.1 | à mesurer | |
| T9 | Fichiers de travail de l'index (`list.new`, `.bak`, `.restore`) tolérés dans `etc/library/` | 5 | ignorés par le firmware | |

Aucun point de ce tableau n'a été mesuré lors de la rédaction de ce document.
