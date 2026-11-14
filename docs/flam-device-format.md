# Format de la Flam branchée en USB

Document de référence du format de la Flam (volume FAT du « mode connexion USB »), aligné sur le
code de `main` (module `apps/flam-launcher/src-tauri/src/device/`, état du commit `f4f1dbc`).
Il décrit ce que le code fait : détection (`detect`, #225), index (`index`, #226), écran
« Ma Flam » (`status`, #227), chiffrement et préparation (`crypto`, `prepare`, #228), transfert
(`transfer`, #229), retrait (`remove`, #231), ordre des histoires (`reorder`, #233), éjection
(`eject`, #232) et verrou d'appareil (`lock`). Il sert aussi aux tests sans appareil (faux appareil : dossier temporaire contenant un
`.mdf` synthétique de 126 octets). Issue : #224, épopée : #222.

Les formats sont décrits ici **en mots**, pour l'interopérabilité. Ce document ne contient
**aucune clé, aucun bloc de clé, aucun contenu d'histoire officielle**. Le projet est publié sous
GPL-3.0-or-later.

## 1. Statuts et sources

Chaque affirmation porte un statut :

| Statut                  | Sens                                                                                                            |
| ----------------------- | --------------------------------------------------------------------------------------------------------------- |
| **mesuré**              | constaté sur un appareil réel ; la source et la date sont indiquées                                             |
| **supposé**             | issu d'une source écrite non vérifiée sur appareil (rapport interne, hypothèses inscrites dans le code)         |
| **implémenté**          | comportement du code de `main`, vérifié par ses tests (faux appareil) ; ne dit rien du comportement du firmware |
| **choix de conception** | décision prise par le code, sans mesure sur appareil                                                            |
| **à vérifier**          | inconnu ou contradictoire ; renvoi au tableau du § 10 (#236)                                                    |

Sources citées :

- **[inv]** : [`security/flam-device-inventory.md`](security/flam-device-inventory.md), scan du
  volume d'une Flam le 2026-06-14, par listage de l'arborescence et des tailles. La version
  de firmware de cet appareil n'y est **pas consignée**.
- **[rapport]** : analyse interne du 2026-10-17 (non versionnée), faite sur documentation et sur
  le code existant, **sans appareil**. Rien de ce qui en vient n'est mesuré.
- **[détection]**, **[index]**, **[état]**, **[crypto]**, **[préparation]**, **[transfert]**,
  **[retrait]**, **[ordre]**, **[éjection]**, **[verrou]**, **[garde]** : le module correspondant de `device/`
  (`detect.rs`, `index.rs`, `status.rs`, `crypto.rs`, `prepare.rs`, `transfer.rs`, `remove.rs`,
  `reorder.rs`, `eject.rs`, `lock.rs`, `guard.rs`). Ces modules fixent la disposition de `.mdf` et du fichier `key` ; la mesure sur
  appareil de ces positions n'est pas consignée dans le dépôt : statut **supposé** (côté firmware)
  tant que #236 ne les a pas confirmées.

Couverture des firmwares : **une Flam 1.15.14 a été mesurée** (2026-10-19, lecture seule puis
envois en mode carrier validés) : `.mdf` de 126 octets, en-tête `01 00`, version **nue** à
l'offset 2 (`1.15.14-…`, sans préfixe « main: »). **Aucune Flam 2.x n'a été mesurée dans ce
dépôt** : tout ce qui concerne la 2.x est une hypothèse documentée ou un comportement observé de
la 2.x rapporté sans fixture réelle (voir § 3.3), à confirmer par les testeurs (#316).

## 2. Détection

| Élément              | Statut                          | Détail                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| -------------------- | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Mode connexion USB   | supposé [rapport]               | La Flam n'expose ses fichiers que si l'utilisateur active le « mode connexion USB » dans ses réglages ; sinon elle ne fait que charger. Sans ce mode, aucun volume n'apparaît : message d'aide « Activez le mode connexion USB sur la Flam » ([détection]).                                                                                                                                                                                                                                                                     |
| Volume               | supposé [rapport]               | Stockage de masse en **FAT** (FAT32 attendu). Sous Windows, [détection] retient les lecteurs amovibles dotés d'un média et les lecteurs fixes dont le système de fichiers contient « FAT » (un disque USB peut se déclarer fixe) ; le disque système NTFS n'est jamais sondé. Hors Windows, aucun volume n'est candidat (non implémenté).                                                                                                                                                                                       |
| Marqueur             | implémenté                      | Fichier `.mdf` à la racine du volume. Sa présence suffit à reconnaître une Flam. Un lien nommé `.mdf` n'est pas suivi et ne compte pas (`symlink_metadata`). Aucun VID/PID USB n'est interrogé.                                                                                                                                                                                                                                                                                                                                 |
| Taille de `.mdf`     | mesuré [inv], 1.15.14 ; implémenté | 126 octets (appareil de l'inventaire ; Flam 1.15.14 mesurée). [détection] : moins de 126 octets = avertissement « tronqué » (pas de sauvegarde) ; plus de 126 octets = avertissement `UnexpectedMdfSize` ; plus de 64 Kio = trop grand (ni lu ni sauvegardé). La garde des écritures (§ 8.0) n'exige que 110 octets (fin de l'IV) : une autre taille est signalée « non standard », sans refus. |
| Contenu de `.mdf`    | implémenté                      | L'état de « Ma Flam » (`device_status`, champ `firmware_status`) lit la version, sa forme (avec ou sans « main: »), la famille, la taille, l'en-tête (2 octets) et le VID/PID ; il n'expose ni clé, ni numéro de série, ni empreinte. La clé et l'IV sont lus au moment d'un envoi par `keys_from_mdf` ([transfert], voir § 3).                                                                                                                                                                                                               |
| Dossiers attendus    | mesuré [inv] ; implémenté       | `str/` et `etc/library/` ; leur absence n'est qu'un avertissement (`MissingStrDir`, `MissingLibraryDir`).                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Sauvegarde de `.mdf` | choix de conception [détection] | Copie personnelle dans le dossier de données du launcher (`device-backups`), écrite par la commande d'état de l'écran « Ma Flam » (la commande de détection seule n'écrit rien). Nom : `<volume>-<16 premiers caractères hexadécimaux du SHA-256>.mdf`, volume réduit à `[A-Za-z0-9_-]` (32 caractères au plus, `flam` si rien ne reste). Idempotente, écriture atomique, jamais dans le dépôt ; pas de sauvegarde d'un fichier tronqué ou trop grand. Le numéro de série n'entre pas dans le nom : la détection ne le lit pas. |

## 3. Le fichier `.mdf`

Fichier à la racine du volume, lu par le launcher en **lecture seule**. Il contient, entre
autres, du matériel de clé propre à l'appareil : ne jamais l'afficher, le journaliser ni le
versionner (son empreinte SHA-256 peut l'être).

### 3.1 Disposition (source : [crypto], non mesurée sur appareil)

| Offset (décimal / hex) | Taille | Champ                                                                      | Statut              |
| ---------------------- | ------ | -------------------------------------------------------------------------- | ------------------- |
| 0 / 0x00               | 2      | en-tête : `01 00` mesuré en 1.15.14 ; rôle inconnu, non contrôlé            | mesuré (1.x)        |
| 2 / 0x02               | 48     | texte de version : nue (`1.15.14-…`, mesuré en 1.x) ou « main: x.y.z-… » puis « comm: … » ; octets nuls | mesuré (1.x), supposé (2.x) |
| 50 / 0x32              | 24     | numéro de série : **14 caractères ASCII hexadécimaux puis 10 octets nuls** | supposé, implémenté |
| 74 / 0x4A              | 2      | VID (petit-boutiste) : affiché, journalisé s'il diffère de `0x303A`        | supposé             |
| 76 / 0x4C              | 2      | PID (petit-boutiste) : affiché, journalisé s'il diffère de `0x819E`        | supposé             |
| 78 / 0x4E              | 16     | clé de chiffrement des histoires (fw 2.x)                                  | supposé, implémenté |
| 94 / 0x5E              | 16     | IV de chiffrement des histoires (fw 2.x)                                   | supposé, implémenté |
| 110 / 0x6E             | 16     | **non lu** : rôle inconnu (126 − 110 octets restants)                      | à vérifier          |

Total : 126 octets (`MDF_LEN`, repris de la taille de la détection). Les offsets sont des
constantes du code ; un test vérifie qu'ils se déduisent de l'addition des tailles de champs. Leur
confirmation sur appareil reste à faire (T1).

Lecture par `keys_from_mdf` ([transfert]) : un `.mdf` de moins de 110 octets (fin de l'IV) est
refusé (clés illisibles) ; un `.mdf` plus long est accepté, seuls les premiers octets étant
utilisés. Le texte d'offset 2 à 49 est lu comme UTF-8 tolérant : on y cherche « main: » (n'importe
où, même après « comm: … ») ; sans « main: », la version nue en tête du champ est prise. On saute
les espaces, puis on prend les chiffres et points qui suivent (`2.4.2`). La version doit être de
forme stricte (composantes de chiffres non vides : `2..1` et `02.4` sont refusés). Le préfixe est
**facultatif dans les deux familles** : la 1.15.14 mesurée n'en a pas ; sa présence en 2.x est une
hypothèse.

**Ce qui est mesuré et ce qui ne l'est pas** : taille 126, en-tête `01 00`, version nue et
position du numéro de série sont mesurés sur une 1.15.14 seulement. En 2.x, la position de la clé
(0x4E) et de l'IV (0x5E), en clair, et le fichier `key` déduit du numéro de série (§ 6.2) sont
conformes au comportement observé de la 2.x, mais **sans `.mdf` 2.x réel dans ce dépôt** ; la
taille, l'en-tête et la forme de la version d'une 2.x restent à relever (T1).

Valeurs matérielles citées par [rapport] pour une Flam de première génération : VID `0x303A`,
PID `0x819E` (supposé). Le code n'en tient aucun compte.

### 3.2 Numéro de série

Le champ fait 24 octets : **exactement 14 caractères ASCII hexadécimaux** (majuscules ou
minuscules), suivis de **10 octets nuls**, soit un numéro de 7 octets une fois décodé
(`parse_serial_field`, [crypto]). Tout autre contenu est refusé : moins de 14 caractères (un nul
avant le quatorzième), caractère non hexadécimal, octet non nul dans les 10 derniers, champ de
taille inattendue. Le message d'erreur ne reprend pas le contenu du champ (donnée propre à
l'appareil). Cette lecture correspond à la structure du fichier `key` (§ 6.2). Statut : **supposé**
côté appareil, à confirmer sur un `.mdf` réel (T2) ; la formulation « 24 caractères hexadécimaux »
d'une documentation antérieure était incohérente (24 caractères feraient 12 octets) et n'est pas
retenue.

### 3.3 Différences entre firmwares

| Sujet                            | Firmware 1.x                                                                                                                                                                    | Firmware 2.x                                                              |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Matériel de clé dans `.mdf`      | 32 octets bruts à 0x4E, sans séparation clé / IV (supposé [rapport])                                                                                                            | clé 16 octets puis IV 16 octets à 0x4E, en clair (comportement observé de la 2.x) |
| Histoires officielles            | `bt` (clé et IV du compte) et `key` du compte                                                                                                                                  | `key` du compte, chiffrées avec la clé du compte : **les clés de `.mdf` ne les déchiffrent pas** (comportement observé de la 2.x) |
| Import « classique »             | produit une histoire illisible (titre illisible, ne se lance pas), constaté sur 1.15.14 d'après [rapport]                                                                       | fonctionne : clés lues dans `.mdf`, `key` déduit du numéro de série (comportement observé, jamais mesuré dans ce dépôt) |
| Prise en charge dans le launcher | **mode carrier** (#234, livré et validé sur 1.15.14) : clés d'une histoire officielle de la Flam, consentement explicite, contrôle avant copie, relecture après (`docs/flam-carrier.md`) | **en test** : clés de `.mdf`, sans confirmation, comme Lunii.QT (§ 8.0) |

Règle exacte du code ([crypto], `firmware_family`) : la version doit être de forme stricte
(composantes de chiffres non vides) et sa **première composante** exactement `1` (1.x, mode
carrier) ou `2` (2.x, clés de `.mdf`) ; comparaison de texte. Sont refusés `3.x` et les suivants,
jamais observés, et toute forme non canonique (`02.1`, `+2`, `2..1`, composante vide). Ordre des
contrôles dans `keys_from_mdf` : taille de `.mdf`, version lisible, **numéro de série valide**,
puis famille 2.x. La position des 32 octets de clé en 1.x n'est pas connue : **à vérifier** (T1).

## 4. Arborescence

Mesurée par [inv] (2026-06-14) sauf mention contraire ; les dossiers du launcher sont des choix de
conception du code :

```
<volume>/
├── .mdf                       126 octets
├── .flam-transfer/            travail d'un envoi (launcher) : <uuid>.in/, <uuid>.old/ ; supprimé quand vide
├── .flam-trash/               corbeille d'un retrait (launcher) : <uuid>-<32 hexadécimaux>/ ; supprimée quand vide
├── etc/
│   ├── library/
│   │   ├── list               index des histoires visibles (370 octets pour 10 histoires)
│   │   ├── list.hidden        index des histoires masquées (0 octet : aucune masquée)
│   │   ├── list.new / .bak / .restore   fichiers de travail d'une réécriture d'index (launcher, éphémères)
│   │   ├── list.hidden.new / .bak / .restore   idem pour list.hidden
│   │   └── list.corrupt       copie d'un index abîmé avant sa reconstruction confirmée (launcher ; puis list.corrupt.2, .3, jamais écrasées)
│   ├── bluetooth/  onboarding/  update/  wifi/   réglages de l'appareil : ne pas toucher
├── System Volume Information/ créé par Windows (présent dans l'inventaire) : ignoré, ne pas toucher
├── str/<uuid>/                une histoire installée et visible
├── str.hidden/<uuid>/         histoires masquées (supposé [rapport] ; absent de [inv])
├── tmp/bluetooth/scan/        temporaire de l'appareil
└── usr/0/
    ├── library.cache          185 829 octets sur l'appareil inventorié
    ├── <uuid>.prog            4 à 6 octets : progression
    ├── <uuid>.save            6 à 12 831 octets : état sauvegardé
    └── <uuid>/chaps.save      chapitres écoutés
```

- `<uuid>` : UUID en minuscules au format 8-4-4-4-12 (36 caractères). Mesuré [inv]. Le code
  accepte les majuscules à la lecture et écrit toujours des minuscules.
- `.flam-transfer/` et `.flam-trash/` sont **hors de `str/`**, que le firmware énumère peut-être
  (T3, T4), et ne sont pas `tmp/`, dossier de travail du firmware. Ce sont des dossiers inconnus du
  firmware : son comportement face à eux est **à vérifier** (T3, T9). Pendant une reprise, un
  fichier sonde `.flam-sonde-<32 hexadécimaux>` est créé puis supprimé dans `<uuid>.in/` ou
  `<uuid>.old/` pour détecter deux noms du même dossier.
- `library.cache` : cache binaire reconstruit par le firmware quand il est absent (supposé
  [rapport]) ; le launcher le supprime après chaque réécriture d'index et après chaque transfert
  ([index], [transfert]). Il contient les titres déchiffrés (supposé). Statut de la
  reconstruction : **à vérifier** (T4).
- `str.hidden/` n'existe pas dans l'inventaire : le launcher ne le crée jamais ; il le lit s'il
  existe. Création à la demande, ou présence conditionnée par un état de l'appareil ? **À
  vérifier** (T6).
- Des sauvegardes (`.prog`, `.save`, dossier) existent pour des UUID sans histoire installée
  (mesuré [inv] : 7 UUID orphelins). Le launcher ne touche jamais à `usr/0/`, hormis la
  suppression de `usr/0/library.cache`.
- Système de fichiers FAT : noms insensibles à la casse, pas de journal. Les noms d'entrées d'une
  histoire sont contrôlés par [préparation] : ni `..` ni chemin absolu, aucun caractère interdit
  sous Windows (`< > : " | ? * \` et caractères de contrôle), pas de nom finissant par un point ou
  une espace, pas de nom réservé Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM0`–`COM9`,
  `LPT0`–`LPT9`), pas de doublon à la casse près.

## 5. L'index `etc/library/list` et `list.hidden`

| Propriété    | Valeur                                          | Statut                                                                                                                                                                                                                                                                                                                 |
| ------------ | ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Contenu      | un UUID par ligne ; `list.hidden` : même format | supposé [rapport]                                                                                                                                                                                                                                                                                                      |
| Fin de ligne | `\n` (LF), y compris après le dernier UUID      | **mesuré** [inv] : 370 octets pour 10 histoires, soit 37 octets (36 + LF) ; avec CRLF on aurait 380. Confirmé sur une Flam 1.15.14 le 2026-10-19 (lecture seule) : 703 octets pour 19 histoires, LF seul, sans BOM, UUID en minuscules distincts, mêmes UUID que les dossiers de `str/` ; `list.hidden` vide (0 octet) |
| Encodage     | ASCII (UUID hexadécimaux et tirets), sans BOM   | supposé                                                                                                                                                                                                                                                                                                                |
| Ordre        | ordre d'affichage sur la Flam                   | supposé [rapport]                                                                                                                                                                                                                                                                                                      |
| Liste vide   | fichier vide (0 octet)                          | **mesuré** [inv] pour `list.hidden`                                                                                                                                                                                                                                                                                    |
| Casse        | minuscules                                      | supposé ; mesuré pour les noms de dossiers [inv]                                                                                                                                                                                                                                                                       |

Écriture par le launcher ([index]) : UUID normalisés en minuscules, sans doublon (première
occurrence gardée), ordre conservé, LF final. À la lecture : BOM UTF-8, CRLF, espaces autour de
l'UUID, lignes vides et majuscules sont tolérés ; lignes invalides et doublons sont ignorés et
signalés (index « abîmé ») ; un fichier de plus de 1 Mio, contenant un octet nul ou non vide sans
aucun UUID est jugé illisible. Un index illisible, abîmé ou absent n'est jamais réécrit sans le
consentement explicite de l'utilisateur ; seul un `list.hidden` absent est réécrit sans
consentement. Les transferts, retraits et réordonnancements ne passent que par l'écriture sans
consentement.

Procédure d'écriture sûre (`write_index`), dont chaque état intermédiaire est reconnaissable par
`recover` :

1. `list.new` écrit, synchronisé sur le disque, relu et comparé ;
2. `list.bak` : copie de `list` (fichier vide si `list` était absent), synchronisée et relue ; sa
   présence garantit que `list.new` est complet ;
3. `list.new` renommé en `list` (remplacement ; sous Windows `MoveFileExW` avec remplacement et
   écriture directe, puis synchronisation de `list`) ;
4. `list` relu et comparé ;
5. `usr/0/library.cache` supprimé ;
6. `list.bak` supprimé : la transaction est terminée.

`list` n'est jamais supprimé ni tronqué avant l'étape 3. `recover` décide d'après la présence de
`list.new`, `list.bak` et l'état de `list` : rien à faire, retour en arrière (index d'origine
gardé), reprise en avant (transaction terminée), restauration depuis `list.bak`, ou **décision de
l'utilisateur requise** si aucun candidat n'est exploitable (rien n'est touché ; reconstruction
possible depuis les dossiers de `str/` avec consentement). Un `list.restore` résiduel n'est jamais
une source : il est supprimé d'abord. À appeler à chaque détection, avant toute lecture ou écriture
d'un index. Il n'y a pas de verrou entre processus : les écritures sont sérialisées par le verrou
d'appareil (§ 8.5).

Lecture par l'écran « Ma Flam » ([état]) : en lecture seule, sans reprise ni création de dossier.
Chaque histoire est « visible » (dans `list`), « masquée » (dans `list.hidden`) ou « non listée »
(dossier de `str/` absent des deux index) ; une histoire de `list` sans dossier `str/<uuid>/` est
marquée « manquante ». Seuls les dossiers de `str/` dont le nom est un UUID sont comptés. La liste
est coupée à 5000 histoires. Titres et vignettes sont lus ensuite, à la demande (`details`, #276) :
première ligne de `info` déchiffré (4 Kio au plus ; fw 2.x : clés de `.mdf` ; fw 1.x : clé carrier
déjà conservée sur le PC) et `img/thumbnail.lif` (1 Mio au plus), sans verrou d'appareil et en
s'arrêtant dès qu'une opération d'écriture est en cours.

Comportement du firmware face à un index absent, tronqué ou contenant un UUID sans dossier :
**à vérifier** (T4).

## 6. Dossier d'une histoire (`str/<uuid>/`)

### 6.1 Contenu

| Chemin                          | Chiffré ?       | Statut                                   | Note                                                                                                                                                                                      |
| ------------------------------- | --------------- | ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `info`                          | **oui**         | présence mesurée [inv]                   | titre et sous-titre ; en clair, 4 lignes (titre, sous-titre, `000000`, titre) : **supposé** (jeu de test de [crypto])                                                                     |
| `main.lsf`                      | **oui**         | présence mesurée [inv]                   | script principal                                                                                                                                                                          |
| `script/*.lsf`                  | **oui**         | présence mesurée [inv]                   | modules du jeu                                                                                                                                                                            |
| `key`                           | non (32 octets) | présence mesurée [inv] ; taille supposée | autorisation de l'histoire, voir § 6.2                                                                                                                                                    |
| `version`                       | non             | présence mesurée [inv]                   | texte ASCII court ; valeurs 1 à 4 relevées [inv]                                                                                                                                          |
| `img/*.lif`, `img/script/*.lif` | non             | supposé ; `img/thumbnail.lif` mesuré     | images au format LIF ; vignette `img/thumbnail.lif` présente dans les 21 histoires d'une Flam 1.15.14 (2026-10-19) : officielles 104 × 136 (12 à 22 Kio), converties 128 × 96             |
| `sounds/*.mp3`, `*.mp3map`      | non             | supposé                                  | audio ; dossier absent d'une histoire sans son [inv]                                                                                                                                      |
| `bt`                            | —               | supposé [rapport]                        | fichier de 32 octets présent dans certaines histoires officielles ; le code ne le traite pas à part (copié tel quel s'il figure dans l'archive) ; **ne jamais le lire ni le journaliser** |

**Vignette `img/thumbnail.lif`** : **104×136** (portrait), mesurée [inv] sur les 19 histoires
officielles d'une Flam 1.x réelle. Les vignettes produites par `telmi2flam` et `flam_build.py`
ont cette taille (remplissage `flou`, `rogner` ou `bandes`, option `--thumb-fit`) ; l'ancienne
taille 128×96 s'affichait mal sur l'appareil (#279). À confirmer visuellement : #236.

### 6.2 Fichier `key`

32 octets (supposé, [crypto]) : le numéro de série de l'appareil en hexadécimal ASCII
**minuscule** (14 caractères, quelle que soit la casse dans `.mdf`), 10 octets nuls, puis les 8
premiers caractères de ce même hexadécimal (14 + 10 + 8). Ce fichier est donc **déduit du numéro de
série** (fw 2.x) et ne contient pas la clé de chiffrement. Statut : **supposé**. Sur les histoires
officielles d'un même compte lié, `key` est identique d'une histoire à l'autre (supposé [rapport]) ;
en 2.x, ce `key` est **celui du compte**, différent du `key` déduit (comportement observé de la
2.x). Seules les histoires installées par un outil tiers portent le `key` déduit. Sur fw 1.x, un
`key` fabriqué n'est pas accepté (§ 3.3).

### 6.3 Renommages depuis un `.plain.pk` ([préparation], d'après [rapport])

Entrée : `library/<uuid>-<version>.plain.pk` de la bibliothèque du launcher (archive ZIP).

| Entrée du `.plain.pk`                          | Dans `str/<uuid>/`         | Traitement                                   |
| ---------------------------------------------- | -------------------------- | -------------------------------------------- |
| `uuid.bin`                                     | aucune                     | donne le nom du dossier (UUID en minuscules) |
| `_metadata.json`, `_thumbnail.png`, `bt`       | aucune                     | ignorées (un `bt` n'est jamais recopié)      |
| `info.plain` (et tout `*.plain`)               | `info` (nom sans `.plain`) | chiffré                                      |
| `main.lua` et tout `*.lua` (`script/*.lua`)    | `main.lsf`, `script/*.lsf` | chiffré                                      |
| tout le reste (`version`, `img/…`, `sounds/…`) | même nom                   | copié tel quel                               |
| aucune                                         | `key`                      | fabriqué depuis l'appareil                   |

Entrées obligatoires : `uuid.bin`, `info.plain`, `main.lua`, `version`. Sont refusés : un lien
symbolique, un nom d'entrée non sûr (§ 4), deux entrées au même nom à la casse près, deux entrées
donnant le même nom cible, une entrée dont le nom cible est `key` (à la casse près). Bornes
(`Limits`) : 100 000 entrées (nombre **déclaré** par l'annuaire de l'archive, sans lecture) ; 4 Gio
cumulés et 16 Mio par fichier à chiffrer (lu en mémoire), contrôlés d'après les tailles
**réellement lues**. Les tailles déclarées par l'archive sont aussi additionnées dès le plan : un
total déclaré au-delà de 4 Gio est refusé avant toute écriture. Sont refusés dès le plan un nom
de plus de 255 unités UTF-16 (FAT, noms longs) et un nom à la fois fichier et dossier (`a` et `a/b`) ; l'extension
`.lua` et les fichiers propres à l'archive se reconnaissent sans tenir compte de la casse. Le nom
**final** sur la Flam est recontrôlé composant par composant après le retrait de `.plain` et le
passage de `.lua` à `.lsf` (#577) : `img/a..plain` donnerait `img/a.` (point final), `a .plain`
`a ` (espace final), `.plain` un nom vide ; ces entrées sont refusées comme un nom d'entrée non sûr.

La préparation **n'écrit rien sur l'appareil**. Elle construit l'histoire dans un dossier de
travail du launcher : tout est écrit dans `<travail>/.partiel-<uuid>-<aléa>/`, renommé en
`<travail>/str/<uuid>/` à la fin ; le dossier final est donc absent ou complet. Une préparation
précédente de la même histoire est écartée par renommage (jamais supprimée en place) puis
supprimée. En cas d'erreur ou d'annulation, le chantier est supprimé ; ceux qu'un arrêt brutal a
laissés (préfixe `.partiel-`), comme les préparations terminées restées dans `str/`, sont supprimés
au début du prochain envoi. Pour un envoi, le dossier de travail est le sous-dossier `flam-transfer`
du dossier de cache du launcher (sous-dossier de l'application, marqué : « Vider le cache » le compte
et le vide, et le nettoyage des restes n'agit que s'il est marqué), et la préparation y est supprimée
après l'envoi, réussi ou non.

## 7. Chiffrement

| Propriété                           | Valeur                                                                                                     | Statut                                                                                                                                  |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Algorithme                          | AES-128 en mode CBC                                                                                        | supposé [rapport] ; primitive testée par les vecteurs NIST SP 800-38A dans [crypto]                                                     |
| Fichiers concernés                  | `*.lsf` et `info`, sur toute leur longueur                                                                 | supposé                                                                                                                                 |
| Bourrage                            | octets nuls jusqu'au multiple de 16 suivant ; aucun bloc ajouté si déjà aligné ; fichier vide : reste vide | supposé ; implémenté ; la taille d'origine n'est pas récupérable, les octets nuls finaux font partie du clair (sans effet sur du texte) |
| IV                                  | 16 octets, identique pour tous les fichiers de l'appareil                                                  | supposé [crypto] ; à vérifier (T1)                                                                                                      |
| Source de la clé et de l'IV, fw 2.x | `.mdf` de l'appareil (offsets 0x4E et 0x5E)                                                                | supposé ; implémenté                                                                                                                    |
| Source de la clé, fw 1.x            | clé d'une histoire officielle du même compte (« carrier ») de la Flam                                      | **mesuré** sur 1.15.14 (envois validés) ; implémenté (#234, `docs/flam-carrier.md`)                                                     |
| Non chiffrés                        | images, audio, `version`, `key`                                                                            | supposé                                                                                                                                 |

Règles du dépôt : jamais de clé réelle dans le code, les tests, les journaux ni les documents ;
tests par vecteurs publics ou inventés ; pas d'export d'histoires officielles. Dans le code, les
clés sont masquées par `Debug` et effacées de la mémoire à la destruction (`zeroize`) ; les
messages d'erreur ne contiennent jamais de clé.

## 8. Procédures

L'ordre protège contre un débranchement : FAT n'a pas de journal. Toutes les opérations sur
l'appareil (transfert, retrait, éjection) sont sérialisées par le verrou d'appareil (§ 8.5).

### 8.0 Garde commune des écritures ([garde], #317, implémentée)

Toute écriture (envoi, reprise au branchement, retrait, réordonnancement) passe d'abord par
`guard::check_write`, qui n'écrit rien sur la Flam. Refus, sans rien écrire :

- racine qui n'est pas un lecteur détecté ; `.mdf` absent, illisible, de plus de 64 Kio ou de
  moins de 110 octets ;
- version illisible ou hors 1.x et 2.x (3.x et suivants, forme non canonique) ;
- en 2.x : numéro de série invalide, clé ou IV triviaux (tout à `00` ou tout à `FF`).

Une taille autre que 126 octets ou un en-tête autre que `01 00` **ne refuse rien** : l'état de
« Ma Flam » signale « fichier .mdf non standard ». Un VID/PID inattendu est journalisé.

**Empreinte de l'appareil** (#609). Toutes les Flam portent le même nom de volume : sous une
même lettre de lecteur, rien d'autre que `.mdf` ne les distingue. `device_status` rend donc une
empreinte opaque de la Flam détectée (`fingerprint`, absente si `.mdf` n'a pas été lu : plus de
64 Kio ; un `.mdf` qui ne s'ouvre pas rend la Flam non détectée) : SHA-256
d'un préfixe, d'un sel tiré au hasard à chaque lancement du launcher et du SHA-256 de `.mdf`. Elle
ne révèle ni le numéro de série ni les clés, n'est ni stable d'un lancement à l'autre, ni
journalisée, ni écrite sur disque, ni reprise dans un message d'erreur ou dans le diagnostic.
Le SHA-256 nu de `.mdf` n'est transmis à l'interface par aucune réponse ; il ne subsiste que dans
le nom de la sauvegarde personnelle de `.mdf` (16 chiffres, dossier de données du launcher, sur le
disque de l'utilisateur), choix antérieur conservé.
L'interface renvoie l'empreinte avec chaque demande d'écriture (envoi, reprise au branchement,
retrait, ordre), dans `expected` : celle de la Flam **affichée au moment du clic**. `check_write`
la compare, avant tout autre contrôle, à celle du `.mdf` qu'il vient de lire, celui que retient le
permis d'écriture (aucune fenêtre entre la comparaison et le permis) ; différente, refus
`device_changed` sans rien écrire : ni nettoyage du dossier de travail, ni préparation, ni reprise,
ni corbeille. Une Flam affichée **sans empreinte** (`expected` présent, `fingerprint` nul) n'est
jamais reconnue : même refus, pour qu'une autre Flam, lisible, qui reprendrait la lettre ne
reçoive pas l'écriture. Sans `expected` (appels d'avant l'empreinte, reprise qui suit un
changement de Flam), rien n'est comparé. L'éjection n'en reçoit pas (elle n'écrit aucun fichier et
doit rester possible quand `.mdf` est illisible), « Retirer du PC » non plus (rien n'est écrit sur
la Flam, et le `.mdf` branché est déjà comparé à celui de l'envoi).

**Hypothèse, non confirmée sur deux appareils réels** (§ 10) : deux Flam différentes ont des
`.mdf` différents (numéro de série, clés). Si elle était fausse, l'empreinte serait la même pour
les deux et ne distinguerait rien, sans dégât supplémentaire : les écritures se feraient comme
avant elle. Les relectures de `.mdf` pendant l'envoi (§ 8.1, #577) reposent sur la même hypothèse.

En **1.x**, rien d'autre : les clés restent celles du mode carrier, vérifiées à chaque envoi.

En **2.x**, rien d'autre (#326). Un audit de conformité avec Lunii.QT (`o-daneel/Lunii.QT`,
révision `c8afe43`, `pkg/api/device_flam.py`) a montré que clés et IV lus dans `.mdf`, fichier
`key` déduit du numéro de série, chiffrement, chemins et index sont identiques octet pour octet,
et que Lunii.QT écrit sur une 2.x sans aucune confirmation. Le launcher ne demande donc ni
confirmation de sauvegarde, ni preuve par `usr/0/library.cache` ; un ancien fichier
`device-trust.json` du dossier de données (versions précédentes) est simplement ignoré.

**Cohérence** (informative, affichée dans « Ma Flam », jamais contrôlée à l'écriture) : seules
comptent les histoires **au format Flam** (`main.lsf` présent, `info` non lisible en clair), non
installées par le launcher, au `key` déduit. Une d'elles déchiffrée : cohérent. Aucune déchiffrée
mais une au `key` déduit : **contradiction**, signalée par un **avertissement** qui désigne les
histoires en cause, sans rien refuser (faux positifs possibles : `info` en Latin-1 ou de moins de
4 caractères). Écartées : les histoires au format Lunii (`ni`, `li`, `ri`, `si` sans `.lsf`),
celles dont `info` est en clair, les histoires officielles (autre `key`). Un `info` lisible avec
un autre `key` confirme la clé et l'IV sans rien conclure sur le `key`.

**Racine reçue du front et lectures** (#577) : toute commande qui reçoit une racine accepte
seulement un lecteur détecté (amovible, ou FAT fixe) : les écritures (garde), mais aussi
l'éjection, l'état du mode carrier, les titres et les vignettes de « Ma Flam ». `device_list_stories`
reçoit aussi la racine et la Flam attendue (`root`, `expected` : empreinte de `device_status`), que
l'interface transmet : avec deux Flam branchées, la liste est celle de la racine affichée, jamais
celle de la première ; une autre Flam à sa place est refusée (`device_changed`, l'interface relit
alors l'état sans afficher d'erreur) ; sans `root`, première Flam, comme avant. Les dossiers de
`str.hidden/` absents des deux index sont montrés comme orphelins (placement non listé), à retirer
comme ceux de `str/`.
Argument `expected` : absent et `null` sont indiscernables côté IPC et valent « aucune
comparaison » ; seul un objet `{ fingerprint }` désigne une Flam (`{ fingerprint: null }` : affichée
sans empreinte, toujours refusée).

### 8.1 Installation ([transfert], #229, implémentée)

Préalable : garde commune (§ 8.0), lecture de `.mdf` (au plus 64 Kio ; au-delà, refus), clés de l'appareil (§ 3), puis préparation hors
appareil (§ 6.3). Un `.mdf` absent donne « Flam introuvable » sans rien écrire. Ensuite
(`send_with`) :

0. **Flam du clic** (#609) : si l'interface a transmis l'empreinte de la Flam affichée au moment
   du clic (§ 8.0), la garde refuse d'abord une Flam qui n'est pas celle-là (`device_changed`),
   avant le nettoyage du dossier de travail, la préparation et la reprise : rien n'est écrit.
1. **Vérifications, rien d'écrit** (hormis la fin d'opérations interrompues) : `.mdf` présent et
   **relu, identique à celui contrôlé par la garde** avant la préparation (#577 : sinon une autre
   Flam a pu prendre la même lettre de lecteur pendant la préparation ; refus, rien n'est écrit,
   reprise comprise) ; UUID
   canonique (minuscules) ; reprise des envois interrompus (tableau ci-dessous) puis des index
   (`recover`) ; refus si un résidu `<uuid>.in` ou `<uuid>.old` subsiste après la reprise ; `list`
   doit être **valide** et `list.hidden` valide ou absent (sinon « réparez l'index ») ; refus si
   l'UUID est dans `list.hidden` ou si `str.hidden/<uuid>/` existe ; refus si l'histoire est déjà
   listée et présente, sauf mise à jour demandée (`replace`) ; un dossier `str/<uuid>/` présent mais
   non listé (orphelin) est remplacé sans condition, comme une mise à jour ; espace libre suffisant :
   chaque fichier arrondi à un cluster supposé de 32 Kio, plus un cluster par dossier, plus une
   marge de 32 Mio. Espace insuffisant : la corbeille d'un retrait précédent (`.flam-trash/`,
   § 8.2) est d'abord vidée (#577), puis l'espace relu ; elle n'est vidée que dans ce cas (durée
   d'un envoi normal inchangée). Garde-fous : seules les entrées nommées
   `<uuid>-<32 hexadécimaux>` sont supprimées, `.mdf` est relu juste avant (autre Flam ou absent :
   refus `device_changed` au message exact, rien n'est supprimé ; illisible après les nouveaux essais :
   erreur d'entrée/sortie, pas un changement de Flam), le verrou de l'appareil est tenu. La corbeille
   est regardée d'abord (lecture seule) : sans entrée reconnue, refus d'espace tout de suite, sans
   relire `.mdf`.
   Si l'espace manque encore, refus `NotEnoughSpace` : la seule écriture faite est ce vidage, qui
   ne touche ni histoire, ni index, ni cache.
2. **Copie** dans `.flam-transfer/<uuid>.in/` à la racine de l'appareil (hors de `str/`, § 4),
   fichier par fichier, par blocs de 1 Mio, chaque fichier synchronisé (`sync_all`). Empreinte
   SHA-256 calculée pendant la lecture de la source ; un fichier source qui change de taille
   pendant la copie fait échouer l'envoi.
3. **Vérification** : chaque fichier est relu sur l'appareil (sous Windows sans le cache du
   système, `FILE_FLAG_NO_BUFFERING`, avec repli sur une lecture ordinaire si l'ouverture échoue) ;
   taille et empreinte comparées. La progression compte la copie puis la relecture (total = deux
   fois la taille).
4. **Dernier point d'annulation**, puis `.mdf` **relu** (#577) : différent, absent ou encore
   illisible, refus ; rien n'est installé (`str/`, index et cache inchangés) et `<uuid>.in/` est
   laissé tel quel (il peut être sur l'autre Flam), supprimé par la reprise du prochain
   branchement ou du prochain envoi ; le launcher relance aussitôt la reprise sur la Flam
   branchée. Mise à jour : l'ancien `str/<uuid>/` est mis de côté par renommage en
   `.flam-transfer/<uuid>.old/` (rien n'est supprimé).
5. **Promotion** : renommage de `<uuid>.in/` en `str/<uuid>/` (déplacement sur le même volume,
   aucune donnée copiée). En cas d'échec, l'ancienne version est remise en place.
6. **Index** : `.mdf` **relu** d'abord (#577). Différent, absent ou encore illisible : pour une
   nouvelle histoire, l'index n'est **pas** écrit (la liste lue sur la Flam d'origine ne doit
   jamais remplacer celle d'une autre) et l'envoi est refusé ; l'histoire reste complète hors
   index sur la Flam d'origine (orphelin, et `<uuid>.old/` si un orphelin était remplacé), la
   reprise termine la mise de côté et un nouvel envoi de la même histoire l'ajoute à l'index.
   Pour une mise à jour déjà listée, l'envoi n'est réussi que si les renommages ont été forcés
   sur le support (`dir_synced`) : il est alors signalé (`device_changed` dans le rapport : la
   file d'envoi s'arrête et la reprise est relancée, ni relecture carrier ni suite « Mes
   histoires »), et les étapes 7 et 8 sont laissées à la reprise de la Flam d'origine ; sans
   renommage forcé, la mise à jour a pu ne pas atteindre cette Flam (FAT sans journal) : refus
   « mise à jour peut-être faite ; rebranchez la Flam d'origine pour vérifier ». Sinon, l'UUID est ajouté **à la fin** de `list` par `write_index` (§ 5), s'il n'y
   figure pas déjà.

   Chaque relecture de `.mdf` (étapes 1, 4 et 6 ; 126 octets, sans coût mesurable) réessaie un
   verrou passager (antivirus) pendant environ 1 s : seul un `.mdf` différent, absent ou encore
   illisible fait refuser. Une histoire installée et listée n'est jamais présentée comme un
   échec ; une histoire installée hors index l'est toujours (refus avant l'index). Fenêtre
   résiduelle : un changement d'appareil entre la dernière relecture de `.mdf` et l'écriture de
   l'index (quelques millisecondes) n'est pas vu. L'identité de la Flam repose sur le contenu de
   `.mdf`, supposé propre à chaque appareil (numéro de série, clés) : à confirmer sur plusieurs
   Flam.
7. **Cache** : suppression de `usr/0/library.cache`, même si l'index n'a pas changé (le contenu,
   donc peut-être le titre, a changé).
8. **Nettoyage** : suppression de l'ancienne version, puis du dossier `.flam-transfer/` s'il est
   vide.

Annulation, échec ou espace insuffisant avant l'étape 5 : `str/` et les index sont inchangés et le
dossier de copie est supprimé (espace insuffisant : seule la corbeille d'un retrait précédent a pu
être vidée). Seuls `str/<uuid>/` de cet UUID, `.flam-transfer/`, `list` et
`library.cache` sont modifiés. Durée : environ 4 minutes pour 80 Mo (supposé [rapport], non
mesuré).

Reprise (`recover`, appelée à chaque envoi et à appeler à la détection), selon le contenu de
`.flam-transfer/` :

| Coupure après…                         | État laissé                                      | Reprise                                                     |
| -------------------------------------- | ------------------------------------------------ | ----------------------------------------------------------- |
| (refus) `.mdf` changé, à l'entrée      | rien de modifié, reprise comprise                | aucune                                                      |
| les vérifications                      | rien de modifié                                  | aucune                                                      |
| (refus) espace insuffisant             | corbeille d'un retrait précédent vidée ; index et `str/` intacts | aucune                                      |
| (refus) `.mdf` changé avant promotion  | `<uuid>.in/` complet non promu                   | `<uuid>.in/` supprimé (jamais promu)                        |
| une copie partielle ou la vérification | `<uuid>.in/` non promu                           | `<uuid>.in/` supprimé (copie à refaire)                     |
| la mise de côté (mise à jour)          | `<uuid>.old/`, `<uuid>.in/`, pas de `str/<uuid>` | `<uuid>.old/` remis en `str/<uuid>/`, `<uuid>.in/` supprimé |
| le renommage final                     | `str/<uuid>/` complet, peut-être `<uuid>.old/`   | `<uuid>.old/` supprimé, cache supprimé                      |
| (installation) avant l'index           | `str/<uuid>/` complet hors index (orphelin)      | aucune ; un nouvel envoi le remplace                        |
| (refus) `.mdf` changé avant l'index    | idem, peut-être `<uuid>.old/`                    | `<uuid>.old/` supprimé, cache supprimé ; un nouvel envoi l'ajoute à l'index |
| `.mdf` changé avant l'index (mise à jour listée) | nouvelle version listée, `<uuid>.old/`, cache périmé | `<uuid>.old/` supprimé, cache supprimé (renommage non écrit : ancienne version remise) |
| une étape de l'index                   | ancien ou nouvel index                           | `recover` de l'index (§ 5)                                  |
| l'index, avant le cache                | `library.cache` périmé                           | supprimé si `<uuid>.old/` reste ; sinon à l'envoi suivant   |

Avant de supprimer un `<uuid>.old` ou `<uuid>.in` dont l'histoire est aussi dans `str/`, la reprise
vérifie par un fichier sonde que ce ne sont pas deux noms du même dossier (coupure pendant un
renommage FAT) ; si c'est le cas, rien n'est supprimé et l'entrée est laissée (`chkdsk`
recommandé). Toute entrée inconnue de `.flam-transfer/` est laissée en place.

Un débranchement avant l'étape 6 laisse au pire un dossier non listé (orphelin) ; pour une
**installation**, jamais un index qui pointe vers du vide. Lors d'une **mise à jour**, une coupure
entre le renommage `str/<uuid>` → `.old` et `.in` → `str/<uuid>` laisse brièvement un UUID listé
sans dossier : `recover` le répare à la détection suivante. Une histoire partielle n'est donc pas
visible, sous réserve de T3 et T4 :
si le firmware réagit à un dossier inconnu à la racine, la copie en cours pourrait être ignorée ou
supprimée par lui (une copie perdue, sans autre effet).

Entre le renommage final et la suppression de `<uuid>.old`, les dossiers `str/<uuid>`, `str/` et `.flam-transfer/` sont forcés sur le disque (au mieux : un échec est journalisé), car une mise à jour d'une histoire déjà listée n'écrit aucun index. Le dossier racine reçu du front doit être un lecteur détecté (amovible, ou FAT fixe) : un dossier quelconque contenant un `.mdf` est refusé, pour l'envoi comme pour la reprise, le retrait, le réordonnancement et l'oubli des clés carrier d'une Flam. Une lettre de lecteur sans séparateur (`E:`) est refusée (#577) : sous Windows elle désigne le dossier courant du lecteur, pas sa racine ; la détection rend toujours `E:\`. Une entrée de `.flam-transfer/` qui ne peut pas être reprise (dossier verrouillé) n'empêche pas l'envoi d'une autre histoire, mais elle est signalée dans le rapport (`recovery_failures`) ; l'envoi de la même histoire est refusé tant qu'elle reste. À la reprise au branchement, elle n'empêche pas non plus la reprise des index (#577) : celle-ci a lieu, puis la première erreur est rendue. Après un refus pour Flam changée (erreur `device_changed`), le launcher arrête la file d'envoi : les histoires en attente passent à « annulée », jamais envoyées sur une Flam que l'utilisateur n'a pas choisie. « Remplacer » est lié à l'UUID visé : une archive dont `uuid.bin` désigne une autre histoire est refusée sans rien écrire.

### 8.2 Retrait ([retrait], #231, implémenté)

Ordre inverse de l'installation : index d'abord, dossier ensuite (`remove_story`).

1. **Préalables** : confirmation explicite de l'utilisateur (sinon rien n'est lu ni écrit) ; UUID
   valide (36 caractères, tirets aux positions 8, 13, 18, 23) ; racine absolue contenant un `.mdf`
   qui est un vrai fichier ; verrou d'appareil ; garde commune (§ 8.0, empreinte de la Flam du
   clic comprise) ; vidage de la corbeille d'un retrait précédent ; reprise des index ; `list` et
   `list.hidden` doivent être valides ou absents.
   **Flam relue pendant le retrait** (#577) : `.mdf` est relu (même mécanisme et mêmes nouveaux
   essais que l'envoi, § 8.1) **juste avant chaque écriture** des étapes ci-dessous (corbeille,
   chaque index, relecture des index, chaque renommage, chaque suppression de la corbeille : 6
   relectures de 126 octets pour un retrait ordinaire) ; différent, absent ou encore illisible, le
   retrait s'arrête là (`device_changed`) sans plus rien écrire sur la Flam branchée.
2. **Cas particuliers** : ni indexée ni présente sur l'appareil = introuvable ; un dossier non
   indexé (orphelin) n'est supprimé que si `list` est valide, sans toucher aux index ; la corbeille
   doit être utilisable avant toute modification.
3. **Index** : l'UUID est retiré de chaque index qui le contient (`list`, `list.hidden`), chacun
   par `write_index`.
4. **Relecture** : aucun dossier n'est touché tant que l'UUID est encore lisible dans un index.
5. **Corbeille** : `str/<uuid>/` et `str.hidden/<uuid>/` sont renommés en
   `.flam-trash/<uuid>-<32 hexadécimaux>/` (même volume : aucune donnée copiée). Un verrou
   passager (antivirus, indexeur) est réessayé pendant environ 1 s (#577) avant de laisser le
   dossier en orphelin ; une source absente au nouvel essai, cible en place, compte comme un
   renommage réussi.
6. **Suppression** des dossiers de la corbeille, avec les mêmes nouveaux essais (comme le vidage
   de la corbeille au début de chaque retrait) ; un échec persistant (fichier verrouillé) n'annule
   rien et la corbeille sera vidée au prochain retrait, ou par un envoi qui manque de place (§ 8.1) ;
   jusque-là elle reste sur la Flam (dossier caché, place occupée). Seules les entrées nommées
   `<uuid>-<32 hexadécimaux>` sont supprimées, jamais les autres, et une corbeille qui n'est pas un
   vrai dossier est laissée telle quelle.

Les sauvegardes de `usr/0/` de cet UUID (`.prog`, `.save`, dossier) ne sont **jamais** touchées.
Après une coupure : soit le dossier est entier dans `str/` et hors index (orphelin, retrait
possible sur confirmation), soit il est dans `.flam-trash/` (vidée au retrait suivant, ou par un envoi qui manque de place).

Refus parce que la Flam a changé (#577), état laissé **sur la Flam d'origine** et reprise :

| Refus avant…                                    | État laissé                                                                 | Reprise                                                   |
| ----------------------------------------------- | --------------------------------------------------------------------------- | --------------------------------------------------------- |
| la corbeille ou le premier index                | rien de modifié, hors vidage de la corbeille d'un retrait précédent et reprise d'une écriture d'index interrompue (faits après la garde) | aucune                                                    |
| l'index de `list.hidden` (UUID dans les deux)   | UUID retiré de `list`, encore dans `list.hidden` ; dossiers intacts         | nouveau retrait de l'histoire encore listée               |
| un renommage vers la corbeille                  | UUID absent des index ; dossiers non déplacés entiers (orphelins), les autres dans `.flam-trash/` | orphelin signalé, retrait sur confirmation ; corbeille vidée au prochain retrait, ou envoi sans place |
| une suppression de la corbeille                 | histoire retirée ; dossier dans `.flam-trash/`                              | corbeille vidée au prochain retrait, ou envoi sans place ; rapport `device_changed`, retrait présenté comme réussi |

Une histoire encore listée n'est jamais supprimée (aucun dossier n'est touché avant la relecture des
index sur la bonne Flam) ; une histoire sortie de l'index n'y revient jamais, même à moitié. La
fenêtre résiduelle est celle de l'envoi : un échange pendant une écriture elle-même (quelques
millisecondes) n'est pas vu ; la relecture des index (étape 4) borne de toute façon ce qui suit.

### 8.3 Masquage (conception, **non implémenté**)

Le launcher ne déplace aucune histoire vers `str.hidden/` et n'écrit pas `list.hidden` en dehors du
retrait. Conception proposée : masquer une histoire avant une synchronisation avec l'application
officielle, qui pourrait supprimer les histoires qu'elle ne connaît pas (supposé [rapport], T7) :
déplacer `str/<uuid>/` vers `str.hidden/<uuid>/`, retirer l'UUID de `list` et l'ajouter à
`list.hidden`. Ordre proposé pour qu'aucun état intermédiaire ne perde l'histoire : ajouter à
`list.hidden`, déplacer le dossier, puis retirer de `list` (`str.hidden/` créé au besoin, T6).
**Risque** : un débranchement entre le déplacement et le retrait de `list` laisse `list` pointer
vers un dossier absent de `str/` (un état que seule la reprise d'une mise à jour tolère, § 8.1) ; le comportement du firmware est
inconnu (T4) et la reprise (`recover`) ne traite aujourd'hui que chaque index séparément. Le
déplacement est un renommage FAT entre deux dossiers du même volume. L'envoi et le retrait tiennent
déjà compte des histoires masquées : un envoi est refusé pour une histoire masquée, un retrait la
supprime des deux index.

### 8.4 Éjection ([éjection], #232, implémentée) et fin de session

Éjection logicielle sans jamais forcer (Windows) :

1. **Préparation du volume** : vidage des tampons, verrouillage, démontage, puis autorisation du
   retrait. Sans droit d'ouvrir le volume en écriture, l'étape est sautée. Si le verrouillage est
   refusé (une poignée est ouverte : antivirus, indexation, fenêtre de l'Explorateur…), rien
   n'est démonté et on passe quand même à l'étape 2 (#619) : contrairement au verrouillage, la
   demande d'éjection prévient les processus, qui peuvent fermer leurs poignées.
2. **Demande d'éjection** (`CM_Request_Device_EjectW`, sans option de force) sur le parent USB du
   disque ; un refus (« veto ») du gestionnaire Plug and Play est respecté (repli). Sans
   démontage préalable, la réussite n'est annoncée qu'une fois le volume disparu (`.mdf`
   inaccessible, attente bornée à 2 s). Le motif du refus est journalisé, jamais le nom du
   programme ni un chemin.

Tout échec (fichier ouvert, refus, appareil introuvable, autre système) donne un **repli** : message
invitant à éjecter depuis l'Explorateur. Une éjection est **refusée** si la racine reçue du front n'est pas un lecteur détecté (#577, comme
pour les écritures), si elle n'a pas de `.mdf` ou si une autre opération est en cours : un transfert ou un retrait n'est jamais coupé.
Aucune option de retrait forcé n'est utilisée : la Flam peut exécuter des tâches à l'éjection
(supposé [rapport] ; T5).

### 8.5 Verrou d'appareil ([verrou])

Une seule opération à la fois sur l'appareil : retrait, transfert, réordonnancement (§ 8.6),
éjection. Le verrou ne bloque jamais : une opération qui le trouve pris est refusée aussitôt, avec
le nom de l'opération en cours. Il est interne au processus ; un second launcher est empêché par le
plugin d'instance unique.

**Fermeture de la fenêtre** (#577, `device::close`) : fermer le launcher pendant un envoi, un
retrait, un changement d'ordre ou une reprise couperait l'opération en plein milieu (tableaux de
reprise ci-dessus). La première demande de fermeture pendant une telle écriture est donc
**retenue** : l'événement `device://close-retained` (`{ operation }`) fait afficher un bandeau non
modal (« Écriture sur la Flam en cours. La fenêtre se fermera à la fin… »), et un fil referme la
fenêtre **de lui-même quand l'écriture est terminée** (c'est ce que l'utilisateur a demandé).
**Aucun délai** : un envoi peut durer plus de vingt minutes, rien ne le coupe sans geste explicite.
Une deuxième demande pendant l'écriture ferme tout de suite (choix de couper : la reprise au
prochain branchement termine ou défait ce qui était en cours). Le bouton « Ne pas fermer »
(`device_cancel_close`) lève la retenue : la fenêtre ne se fermera pas à la fin de l'écriture
(événement `device://close-cleared`, le bandeau disparaît ; il disparaît aussi à la fin de
l'écriture). Aucun chemin n'empêche de fermer : la décision ne dépend que du verrou et d'un
drapeau « déjà retenue », la deuxième demande passe toujours (même si le fil d'attente meurt ou si
le verrou n'est jamais relâché). L'aperçu des clés du mode carrier et « Retirer du PC » (lectures,
`Operation::Read`) et l'éjection ne retiennent rien.

Un seul point de décision, côté Rust (`CloseGate::request`). Quand l'interface a un gestionnaire
de fermeture (l'éditeur de pack vide son brouillon), Tauri empêche lui-même la fermeture et le
gestionnaire de `@tauri-apps/api` détruit la fenêtre sauf s'il appelle `preventDefault()` : le
gestionnaire JavaScript appelle donc toujours `preventDefault()`, fait son travail, demande le
verdict mémorisé (`device_close_verdict`, `allow` ou `retain`) et ne détruit la fenêtre que si
c'est `allow` ; sans verdict ou si la commande échoue hors application, il ferme comme avant.

Texte du bandeau selon l'opération, d'après ce que fait la reprise : envoi (la copie temporaire est
supprimée au prochain branchement ou envoi, l'histoire n'est pas installée ; installée mais hors
liste, un nouvel envoi l'y ajoute), retrait (l'histoire peut rester à moitié retirée : orphelin à
retirer de nouveau), changement d'ordre (la liste reste l'ancien ou le nouvel ordre, réparée au
branchement).

Arrêt de Windows ou fermeture de session : **non vérifié**. Windows termine les processus qui ne
répondent pas assez vite ; la retenue ne peut pas l'empêcher et il n'est pas établi que Tauri
transmette cette demande comme une fermeture de fenêtre.

### 8.6 Ordre des histoires ([ordre], #233, implémenté)

L'ordre d'affichage est celui de `list` (supposé [rapport]) ; un envoi ajoute à la fin.
Réordonner revient à réécrire `list` (`reorder_stories`) ; aucun dossier n'est déplacé.

1. **Demande** : l'ordre complet souhaité, liste d'UUID. Refus sans rien lire sur l'appareil si
   elle est vide, contient un UUID invalide (position signalée) ou un doublon (à la casse près).
2. **Préalables** : racine absolue contenant un `.mdf` qui est un vrai fichier ; verrou
   d'appareil (`Reorder` : refus si une autre opération est en cours, sans reprise ni lecture) ;
   reprise des deux index (`recover`) ; `list` doit être **valide** et `list.hidden` valide ou
   absent, sinon refus (« réparez l'index »).
3. **Validation stricte** : la demande doit contenir exactement les UUID de `list`. Un UUID de
   `list.hidden` est refusé (une histoire masquée n'a pas d'ordre), un UUID inconnu aussi, ainsi
   qu'une demande où manque un UUID de `list` (la liste a changé depuis son affichage :
   l'interface la relit). Rien n'est écrit en cas de refus. Une histoire listée sans dossier
   garde sa place comme les autres.
4. **Écriture** : ordre identique à `list` = rien n'est écrit ; sinon `.mdf` est **relu** juste
   avant (#577, même mécanisme et mêmes nouveaux essais que l'envoi : une Flam échangée depuis la
   garde, absente ou illisible fait refuser, `device_changed`, sans rien écrire sur la Flam
   branchée ; l'ordre se redemande à la Flam d'origine), puis `write_index` (§ 5) et ses six
   étapes, dont la suppression de `usr/0/library.cache`. Les UUID sont écrits en minuscules.

Seuls `list`, ses fichiers de travail (`list.new`, `.bak`, `.restore`, idem pour `list.hidden`) et `library.cache` sont
modifiés ; `list.hidden`, `str/`, `str.hidden/`, `usr/0/` et les réglages ne le sont jamais
(vérifié par les tests, qui comparent toute l'arborescence hors de ces fichiers).

| Coupure ou échec après…                 | État laissé                               | Reprise (`recover`)                |
| --------------------------------------- | ----------------------------------------- | ---------------------------------- |
| la validation                           | rien de modifié                           | aucune                             |
| (refus) `.mdf` changé avant l'écriture  | rien de modifié, hors la reprise d'une écriture d'index interrompue faite après la garde | aucune |
| `list.new` (complet ou partiel)         | ancien `list`, `list.new` sans `list.bak` | `list.new` supprimé : ancien ordre |
| `list.bak` (complet ou partiel)         | ancien `list`, `list.new`, `list.bak`     | ancien ordre gardé                 |
| le renommage, cible perdue              | `list` absent, `list.new`, `list.bak`     | `list.new` promu : nouvel ordre    |
| le remplacement, la relecture, le cache | nouveau `list`, `list.bak`                | nouvel ordre, cache supprimé       |

L'index est donc toujours l'ancien ou le nouvel ordre complet. Après l'écriture, l'interface
rappelle de redémarrer la Flam pour qu'elle reconstruise son cache et affiche le nouvel ordre
(reconstruction **à vérifier**, T4).

## 9. Empreintes de référence

Aucune empreinte n'est consignée à ce jour (aucun appareil disponible pour cette rédaction).
À renseigner par le mainteneur, SHA-256 uniquement, de fichiers **sans clé en clair** ni contenu
d'histoire officielle :

| Fichier            | Firmware     | Date         | SHA-256      |
| ------------------ | ------------ | ------------ | ------------ |
| `etc/library/list` | à renseigner | à renseigner | à renseigner |
| `.mdf` (entier)    | à renseigner | à renseigner | à renseigner |

## 10. Points à confirmer sur appareil réel (#236)

| N°  | Point                                                                                                                                                                                                                                   | Section  | Attendu                         | Mesure (firmware, date, résultat) |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- | ------------------------------- | --------------------------------- |
| T1  | Disposition complète de `.mdf` (en-tête, offsets 50 à 78, clé et IV à 0x4E et 0x5E, rôle des 16 derniers octets) en fw 2.x et fw 1.x ; IV unique pour tous les fichiers ; acceptation par la Flam de fichiers chiffrés avec ces valeurs | 3, 7     | tableau 3.1 confirmé            |                                   |
| T2  | Numéro de série : 14 caractères + 10 nuls (lecture du code), majuscules ou minuscules dans `.mdf` ; forme du `key` fabriqué (14 + 10 + 8)                                                                                               | 3.2, 6.2 | 7 octets                        |                                   |
| T3  | Dossiers de travail à la racine (`.flam-transfer/`, `.flam-trash/`) tolérés par le firmware, puis renommage vers `str/<uuid>/`                                                                                                          | 4, 8.1   | accepté, ignoré par le firmware |                                   |
| T4  | Index absent, tronqué, UUID sans dossier ; reconstruction de `library.cache` ; titre lisible ; relances successives sans redémarrage                                                                                                    | 4, 5     | reconstruction automatique      |                                   |
| T5  | Éjection logicielle, effet pendant une mise à jour                                                                                                                                                                                      | 8.4      | sans effet de bord              |                                   |
| T6  | `str.hidden/` : création à la demande ou présence à l'état neuf ; limites (nombre d'histoires, taille, longueur des noms, casse)                                                                                                        | 4, 8.3   | à mesurer                       |                                   |
| T7  | Synchronisation officielle : suppression des histoires non officielles, effet du masquage                                                                                                                                               | 8.3      | masquage préservé               |                                   |
| T8  | Valeurs de `version` acceptées (1 à 4 relevées) ; firmwares 1.x et 2.x couverts ; version 3.x et suivantes (refusées par le code)                                                                                                       | 3.3, 6.1 | à mesurer                       |                                   |
| T9  | Fichiers de travail de l'index (`list.new`, `.bak`, `.restore`, et `list.hidden.*`, copie `list.corrupt`) tolérés dans `etc/library/` ; fichier sonde `.flam-sonde-*` toléré                                                                                                      | 4, 5     | ignorés par le firmware         |                                   |
| T10 | Deux Flam différentes ont des `.mdf` différents (numéro de série, clés) : hypothèse de l'empreinte de l'appareil (#609) et des relectures de `.mdf` pendant l'envoi (#577) ; à comparer sans publier leur contenu (fiche M13, étape 13) | 8.0, 8.1 | `.mdf` différents               |                                   |

Mesures acquises : une Flam 1.15.14 (taille 126, en-tête `01 00`, version nue, index en LF, mode
carrier). Aucune mesure 2.x. Points à faire relever par les testeurs 2.x (#316) : taille, en-tête
et forme de la version d'un vrai `.mdf` 2.x (T1) ; `key` et déchiffrement des histoires officielles
(T2) ; lecture sur l'appareil d'une histoire envoyée, après redémarrage (T4) ;
tolérance de `.flam-transfer/`, `.flam-trash/` et `list.new` (T3, T9).
