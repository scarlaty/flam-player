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

- **[inv]** : `flam-device-inventory.md`, scan du volume d'une Flam le 2026-06-14, par listage de
  l'arborescence et des tailles. La version de firmware de cet appareil n'y est **pas consignée**.
- **[rapport]** : analyse interne du 2026-10-17 (non versionnée), faite sur documentation et sur
  le code existant, **sans appareil**. Rien de ce qui en vient n'est mesuré.
- **[détection]**, **[index]**, **[état]**, **[crypto]**, **[préparation]**, **[transfert]**,
  **[retrait]**, **[ordre]**, **[éjection]**, **[verrou]** : le module correspondant de `device/`
  (`detect.rs`, `index.rs`, `status.rs`, `crypto.rs`, `prepare.rs`, `transfer.rs`, `remove.rs`,
  `reorder.rs`, `eject.rs`, `lock.rs`). Ces modules fixent la disposition de `.mdf` et du fichier `key` ; la mesure sur
  appareil de ces positions n'est pas consignée dans le dépôt : statut **supposé** (côté firmware)
  tant que #236 ne les a pas confirmées.

Couverture des firmwares : **aucune version mesurée n'est documentée pour une fw 2.x ni pour une
fw 1.x**. Les firmwares 1.15.14 et 2.4.2 sont cités par [rapport] comme versions rencontrées,
sans mesure dans ce dépôt. Les critères de #224 ne sont donc pas tous satisfaits tant que le
mainteneur n'a pas rempli la colonne « Mesure » du § 10.

## 2. Détection

| Élément              | Statut                          | Détail                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| -------------------- | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Mode connexion USB   | supposé [rapport]               | La Flam n'expose ses fichiers que si l'utilisateur active le « mode connexion USB » dans ses réglages ; sinon elle ne fait que charger. Sans ce mode, aucun volume n'apparaît : message d'aide « Activez le mode connexion USB sur la Flam » ([détection]).                                                                                                                                                                                                                                                                     |
| Volume               | supposé [rapport]               | Stockage de masse en **FAT** (FAT32 attendu). Sous Windows, [détection] retient les lecteurs amovibles dotés d'un média et les lecteurs fixes dont le système de fichiers contient « FAT » (un disque USB peut se déclarer fixe) ; le disque système NTFS n'est jamais sondé. Hors Windows, aucun volume n'est candidat (non implémenté).                                                                                                                                                                                       |
| Marqueur             | implémenté                      | Fichier `.mdf` à la racine du volume. Sa présence suffit à reconnaître une Flam. Un lien nommé `.mdf` n'est pas suivi et ne compte pas (`symlink_metadata`). Aucun VID/PID USB n'est interrogé.                                                                                                                                                                                                                                                                                                                                 |
| Taille de `.mdf`     | mesuré [inv] ; implémenté       | 126 octets (appareil de l'inventaire, firmware non consigné). [détection] : moins de 126 octets = fichier tronqué (erreur, pas de sauvegarde) ; plus de 126 octets = avertissement `UnexpectedMdfSize` ; plus de 64 Kio = trop grand (ni lu ni sauvegardé). Lecture bornée à 64 Kio + 1 octet.                                                                                                                                                                                                                                  |
| Contenu de `.mdf`    | implémenté                      | La détection **ne décode aucun champ** de `.mdf` : `parse_mdf` rend `FormatUnspecified` et seuls la taille et le SHA-256 sont exposés. Les champs (version, numéro de série, clé, IV) sont lus au moment d'un envoi, par `keys_from_mdf` ([transfert], voir § 3). VID et PID ne sont lus nulle part aujourd'hui.                                                                                                                                                                                                                |
| Dossiers attendus    | mesuré [inv] ; implémenté       | `str/` et `etc/library/` ; leur absence n'est qu'un avertissement (`MissingStrDir`, `MissingLibraryDir`).                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Sauvegarde de `.mdf` | choix de conception [détection] | Copie personnelle dans le dossier de données du launcher (`device-backups`), écrite par la commande d'état de l'écran « Ma Flam » (la commande de détection seule n'écrit rien). Nom : `<volume>-<16 premiers caractères hexadécimaux du SHA-256>.mdf`, volume réduit à `[A-Za-z0-9_-]` (32 caractères au plus, `flam` si rien ne reste). Idempotente, écriture atomique, jamais dans le dépôt ; pas de sauvegarde d'un fichier tronqué ou trop grand. Le numéro de série n'entre pas dans le nom : la détection ne le lit pas. |

## 3. Le fichier `.mdf`

Fichier à la racine du volume, lu par le launcher en **lecture seule**. Il contient, entre
autres, du matériel de clé propre à l'appareil : ne jamais l'afficher, le journaliser ni le
versionner (son empreinte SHA-256 peut l'être).

### 3.1 Disposition (source : [crypto], non mesurée sur appareil)

| Offset (décimal / hex) | Taille | Champ                                                                      | Statut              |
| ---------------------- | ------ | -------------------------------------------------------------------------- | ------------------- |
| 0 / 0x00               | 2      | en-tête, non lu (rôle à consigner)                                         | à vérifier          |
| 2 / 0x02               | 48     | texte : « main: x.y.z-… » puis « comm: … », complété par des octets nuls   | supposé, implémenté |
| 50 / 0x32              | 24     | numéro de série : **14 caractères ASCII hexadécimaux puis 10 octets nuls** | supposé, implémenté |
| 74 / 0x4A              | 2      | VID (petit-boutiste), non lu                                               | supposé             |
| 76 / 0x4C              | 2      | PID (petit-boutiste), non lu                                               | supposé             |
| 78 / 0x4E              | 16     | clé de chiffrement des histoires (fw 2.x)                                  | supposé, implémenté |
| 94 / 0x5E              | 16     | IV de chiffrement des histoires (fw 2.x)                                   | supposé, implémenté |
| 110 / 0x6E             | 16     | **non lu** : rôle inconnu (126 − 110 octets restants)                      | à vérifier          |

Total : 126 octets (`MDF_LEN`, repris de la taille de la détection). Les offsets sont des
constantes du code ; un test vérifie qu'ils se déduisent de l'addition des tailles de champs. Leur
confirmation sur appareil reste à faire (T1).

Lecture par `keys_from_mdf` ([transfert]) : un `.mdf` de moins de 126 octets est refusé (clés
illisibles) ; un `.mdf` plus long est accepté, seuls les 126 premiers octets étant utilisés. Le
texte d'offset 2 à 49 est lu comme UTF-8 tolérant : on y cherche « main: », on saute les espaces,
puis on prend les chiffres et points qui suivent (`2.4.2`) ; sans chiffre en tête, la version est
illisible. Le reste du texte (« comm: … ») n'est pas lu.

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
| Matériel de clé dans `.mdf`      | 32 octets bruts à 0x4E, sans séparation clé / IV (supposé [rapport])                                                                                                            | clé 16 octets puis IV 16 octets à 0x4E (supposé [crypto])                 |
| Import « classique »             | produit une histoire illisible (titre illisible, ne se lance pas), constaté sur 1.15.14 d'après [rapport]                                                                       | fonctionne : clés lues dans `.mdf` (supposé, jamais mesuré dans ce dépôt) |
| Prise en charge dans le launcher | **non implémentée** : refus `UnsupportedFirmware`. Mode « carrier » (clé d'une histoire officielle fournie par l'utilisateur, consentement explicite) : issue #234, **à venir** | oui ([crypto], [transfert])                                               |

Règle exacte du code ([crypto], `check_firmware`) : le firmware est identifié par le champ
« main » ; sa **première composante**, avant le premier point, doit être exactement la chaîne `2`
(comparaison de texte, pas de nombre). Sont acceptés `2` et `2.4.2` ; sont refusés `1.x`, `3.x` et
les suivants, mais aussi toute forme non canonique (`02.1`, `+2`, composante vide). Les versions 3.x
et suivantes n'ont jamais été observées : elles sont refusées explicitement plutôt que de produire
un fichier `key` douteux (choix prudent, à confirmer, T8). Ordre des contrôles dans
`keys_from_mdf` : taille de `.mdf`, version lisible, **numéro de série valide**, puis version prise
en charge : un `.mdf` 1.x au numéro de série illisible est donc refusé comme « clés illisibles » et
non comme « firmware non pris en charge ». La position des 32 octets de clé en 1.x n'est pas
connue : **à vérifier** (T1).

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
│   │   └── list.new / .bak / .restore   fichiers de travail d'une réécriture d'index (launcher, éphémères)
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
officielles d'un même compte lié, `key` est identique d'une histoire à l'autre (supposé [rapport]).
Sur fw 1.x, un `key` fabriqué n'est pas accepté (§ 3.3).

### 6.3 Renommages depuis un `.plain.pk` ([préparation], d'après [rapport])

Entrée : `library/<uuid>-<version>.plain.pk` de la bibliothèque du launcher (archive ZIP).

| Entrée du `.plain.pk`                          | Dans `str/<uuid>/`         | Traitement                                   |
| ---------------------------------------------- | -------------------------- | -------------------------------------------- |
| `uuid.bin`                                     | aucune                     | donne le nom du dossier (UUID en minuscules) |
| `_metadata.json`, `_thumbnail.png`             | aucune                     | ignorées                                     |
| `info.plain` (et tout `*.plain`)               | `info` (nom sans `.plain`) | chiffré                                      |
| `main.lua` et tout `*.lua` (`script/*.lua`)    | `main.lsf`, `script/*.lsf` | chiffré                                      |
| tout le reste (`version`, `img/…`, `sounds/…`) | même nom                   | copié tel quel                               |
| aucune                                         | `key`                      | fabriqué depuis l'appareil                   |

Entrées obligatoires : `uuid.bin`, `info.plain`, `main.lua`, `version`. Sont refusés : un lien
symbolique, un nom d'entrée non sûr (§ 4), deux entrées au même nom à la casse près, deux entrées
donnant le même nom cible, une entrée dont le nom cible est `key` (à la casse près). Bornes
(`Limits`), contrôlées d'après les tailles **réellement lues** : 100 000 entrées, 4 Gio cumulés,
16 Mio par fichier à chiffrer (lu en mémoire).

La préparation **n'écrit rien sur l'appareil**. Elle construit l'histoire dans un dossier de
travail du launcher : tout est écrit dans `<travail>/.partiel-<uuid>-<aléa>/`, renommé en
`<travail>/str/<uuid>/` à la fin ; le dossier final est donc absent ou complet. Une préparation
précédente de la même histoire est écartée par renommage (jamais supprimée en place) puis
supprimée. En cas d'erreur ou d'annulation, le chantier est supprimé ; ceux qu'un arrêt brutal a
laissés (préfixe `.partiel-`) sont supprimés au début du prochain envoi. Pour un envoi, le dossier
de travail est le sous-dossier `flam-transfer` du dossier de cache du launcher, et la préparation y
est supprimée après l'envoi, réussi ou non.

## 7. Chiffrement

| Propriété                           | Valeur                                                                                                     | Statut                                                                                                                                  |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Algorithme                          | AES-128 en mode CBC                                                                                        | supposé [rapport] ; primitive testée par les vecteurs NIST SP 800-38A dans [crypto]                                                     |
| Fichiers concernés                  | `*.lsf` et `info`, sur toute leur longueur                                                                 | supposé                                                                                                                                 |
| Bourrage                            | octets nuls jusqu'au multiple de 16 suivant ; aucun bloc ajouté si déjà aligné ; fichier vide : reste vide | supposé ; implémenté ; la taille d'origine n'est pas récupérable, les octets nuls finaux font partie du clair (sans effet sur du texte) |
| IV                                  | 16 octets, identique pour tous les fichiers de l'appareil                                                  | supposé [crypto] ; à vérifier (T1)                                                                                                      |
| Source de la clé et de l'IV, fw 2.x | `.mdf` de l'appareil (offsets 0x4E et 0x5E)                                                                | supposé ; implémenté                                                                                                                    |
| Source de la clé, fw 1.x            | clé d'une histoire officielle du même compte (« carrier »), fournie par l'utilisateur                      | supposé [rapport] ; **non implémenté**, issue #234 (à venir)                                                                            |
| Non chiffrés                        | images, audio, `version`, `key`                                                                            | supposé                                                                                                                                 |

Règles du dépôt : jamais de clé réelle dans le code, les tests, les journaux ni les documents ;
tests par vecteurs publics ou inventés ; pas d'export d'histoires officielles. Dans le code, les
clés sont masquées par `Debug` et effacées de la mémoire à la destruction (`zeroize`) ; les
messages d'erreur ne contiennent jamais de clé.

## 8. Procédures

L'ordre protège contre un débranchement : FAT n'a pas de journal. Toutes les opérations sur
l'appareil (transfert, retrait, éjection) sont sérialisées par le verrou d'appareil (§ 8.5).

### 8.1 Installation ([transfert], #229, implémentée)

Préalable : lecture de `.mdf` (au plus 64 Kio), clés de l'appareil (§ 3), puis préparation hors
appareil (§ 6.3). Un `.mdf` absent donne « Flam introuvable » sans rien écrire. Ensuite
(`send_with`) :

1. **Vérifications, rien d'écrit** (hormis la fin d'opérations interrompues) : `.mdf` présent ; UUID
   canonique (minuscules) ; reprise des envois interrompus (tableau ci-dessous) puis des index
   (`recover`) ; refus si un résidu `<uuid>.in` ou `<uuid>.old` subsiste après la reprise ; `list`
   doit être **valide** et `list.hidden` valide ou absent (sinon « réparez l'index ») ; refus si
   l'UUID est dans `list.hidden` ou si `str.hidden/<uuid>/` existe ; refus si l'histoire est déjà
   listée et présente, sauf mise à jour demandée (`replace`) ; un dossier `str/<uuid>/` présent mais
   non listé (orphelin) est remplacé sans condition, comme une mise à jour ; espace libre suffisant :
   chaque fichier arrondi à un cluster supposé de 32 Kio, plus un cluster par dossier, plus une
   marge de 32 Mio.
2. **Copie** dans `.flam-transfer/<uuid>.in/` à la racine de l'appareil (hors de `str/`, § 4),
   fichier par fichier, par blocs de 1 Mio, chaque fichier synchronisé (`sync_all`). Empreinte
   SHA-256 calculée pendant la lecture de la source ; un fichier source qui change de taille
   pendant la copie fait échouer l'envoi.
3. **Vérification** : chaque fichier est relu sur l'appareil (sous Windows sans le cache du
   système, `FILE_FLAG_NO_BUFFERING`, avec repli sur une lecture ordinaire si l'ouverture échoue) ;
   taille et empreinte comparées. La progression compte la copie puis la relecture (total = deux
   fois la taille).
4. **Dernier point d'annulation.** Mise à jour : l'ancien `str/<uuid>/` est mis de côté par
   renommage en `.flam-transfer/<uuid>.old/` (rien n'est supprimé).
5. **Promotion** : renommage de `<uuid>.in/` en `str/<uuid>/` (déplacement sur le même volume,
   aucune donnée copiée). En cas d'échec, l'ancienne version est remise en place.
6. **Index** : l'UUID est ajouté **à la fin** de `list` par `write_index` (§ 5), s'il n'y figure
   pas déjà.
7. **Cache** : suppression de `usr/0/library.cache`, même si l'index n'a pas changé (le contenu,
   donc peut-être le titre, a changé).
8. **Nettoyage** : suppression de l'ancienne version, puis du dossier `.flam-transfer/` s'il est
   vide.

Annulation, échec ou espace insuffisant avant l'étape 5 : `str/` et les index sont inchangés et le
dossier de copie est supprimé. Seuls `str/<uuid>/` de cet UUID, `.flam-transfer/`, `list` et
`library.cache` sont modifiés. Durée : environ 4 minutes pour 80 Mo (supposé [rapport], non
mesuré).

Reprise (`recover`, appelée à chaque envoi et à appeler à la détection), selon le contenu de
`.flam-transfer/` :

| Coupure après…                         | État laissé                                      | Reprise                                                     |
| -------------------------------------- | ------------------------------------------------ | ----------------------------------------------------------- |
| les vérifications                      | rien de modifié                                  | aucune                                                      |
| une copie partielle ou la vérification | `<uuid>.in/` non promu                           | `<uuid>.in/` supprimé (copie à refaire)                     |
| la mise de côté (mise à jour)          | `<uuid>.old/`, `<uuid>.in/`, pas de `str/<uuid>` | `<uuid>.old/` remis en `str/<uuid>/`, `<uuid>.in/` supprimé |
| le renommage final                     | `str/<uuid>/` complet, peut-être `<uuid>.old/`   | `<uuid>.old/` supprimé, cache supprimé                      |
| (installation) avant l'index           | `str/<uuid>/` complet hors index (orphelin)      | aucune ; un nouvel envoi le remplace                        |
| une étape de l'index                   | ancien ou nouvel index                           | `recover` de l'index (§ 5)                                  |
| l'index, avant le cache                | `library.cache` périmé                           | supprimé si `<uuid>.old/` reste ; sinon à l'envoi suivant   |

Avant de supprimer un `<uuid>.old` ou `<uuid>.in` dont l'histoire est aussi dans `str/`, la reprise
vérifie par un fichier sonde que ce ne sont pas deux noms du même dossier (coupure pendant un
renommage FAT) ; si c'est le cas, rien n'est supprimé et l'entrée est laissée (`chkdsk`
recommandé). Toute entrée inconnue de `.flam-transfer/` est laissée en place.

Un débranchement avant l'étape 6 laisse au pire un dossier non listé (orphelin) ; jamais un index
qui pointe vers du vide. Une histoire partielle n'est donc pas visible, sous réserve de T3 et T4 :
si le firmware réagit à un dossier inconnu à la racine, la copie en cours pourrait être ignorée ou
supprimée par lui (une copie perdue, sans autre effet).

### 8.2 Retrait ([retrait], #231, implémenté)

Ordre inverse de l'installation : index d'abord, dossier ensuite (`remove_story`).

1. **Préalables** : confirmation explicite de l'utilisateur (sinon rien n'est lu ni écrit) ; UUID
   valide (36 caractères, tirets aux positions 8, 13, 18, 23) ; racine absolue contenant un `.mdf`
   qui est un vrai fichier ; verrou d'appareil ; vidage de la corbeille d'un retrait précédent ;
   reprise des index ; `list` et `list.hidden` doivent être valides ou absents.
2. **Cas particuliers** : ni indexée ni présente sur l'appareil = introuvable ; un dossier non
   indexé (orphelin) n'est supprimé que si `list` est valide, sans toucher aux index ; la corbeille
   doit être utilisable avant toute modification.
3. **Index** : l'UUID est retiré de chaque index qui le contient (`list`, `list.hidden`), chacun
   par `write_index`.
4. **Relecture** : aucun dossier n'est touché tant que l'UUID est encore lisible dans un index.
5. **Corbeille** : `str/<uuid>/` et `str.hidden/<uuid>/` sont renommés en
   `.flam-trash/<uuid>-<32 hexadécimaux>/` (même volume : aucune donnée copiée).
6. **Suppression** des dossiers de la corbeille ; un échec (fichier verrouillé) n'annule rien et la
   corbeille sera vidée à la prochaine opération. Seules les entrées nommées
   `<uuid>-<32 hexadécimaux>` sont supprimées, jamais les autres, et une corbeille qui n'est pas un
   vrai dossier est laissée telle quelle.

Les sauvegardes de `usr/0/` de cet UUID (`.prog`, `.save`, dossier) ne sont **jamais** touchées.
Après une coupure : soit le dossier est entier dans `str/` et hors index (orphelin, retrait
possible sur confirmation), soit il est dans `.flam-trash/` (vidée au retrait suivant).

### 8.3 Masquage (conception, **non implémenté**)

Le launcher ne déplace aucune histoire vers `str.hidden/` et n'écrit pas `list.hidden` en dehors du
retrait. Conception proposée : masquer une histoire avant une synchronisation avec l'application
officielle, qui pourrait supprimer les histoires qu'elle ne connaît pas (supposé [rapport], T7) :
déplacer `str/<uuid>/` vers `str.hidden/<uuid>/`, retirer l'UUID de `list` et l'ajouter à
`list.hidden`. Ordre proposé pour qu'aucun état intermédiaire ne perde l'histoire : ajouter à
`list.hidden`, déplacer le dossier, puis retirer de `list` (`str.hidden/` créé au besoin, T6).
**Risque** : un débranchement entre le déplacement et le retrait de `list` laisse `list` pointer
vers un dossier absent de `str/` (contraire au principe du § 8.1) ; le comportement du firmware est
inconnu (T4) et la reprise (`recover`) ne traite aujourd'hui que chaque index séparément. Le
déplacement est un renommage FAT entre deux dossiers du même volume. L'envoi et le retrait tiennent
déjà compte des histoires masquées : un envoi est refusé pour une histoire masquée, un retrait la
supprime des deux index.

### 8.4 Éjection ([éjection], #232, implémentée) et fin de session

Éjection logicielle sans jamais forcer (Windows) :

1. **Préparation du volume** : vidage des tampons, verrouillage (échoue si un autre processus a un
   fichier ouvert : arrêt, sans forcer), démontage, puis autorisation du retrait. Sans droit
   d'ouvrir le volume en écriture, l'étape est sautée.
2. **Demande d'éjection** (`CM_Request_Device_EjectW`) sur le parent USB du disque ; un refus
   (« veto ») du gestionnaire Plug and Play est respecté.

Tout échec (fichier ouvert, refus, appareil introuvable, autre système) donne un **repli** : message
invitant à éjecter depuis l'Explorateur. Une éjection est **refusée** si la racine n'a pas de
`.mdf` ou si une autre opération est en cours : un transfert ou un retrait n'est jamais coupé.
Aucune option de retrait forcé n'est utilisée : la Flam peut exécuter des tâches à l'éjection
(supposé [rapport] ; T5).

### 8.5 Verrou d'appareil ([verrou])

Une seule opération à la fois sur l'appareil : retrait, transfert, réordonnancement (§ 8.6),
éjection. Le verrou ne bloque jamais : une opération qui le trouve pris est refusée aussitôt, avec
le nom de l'opération en cours. Il est interne au processus ; un second launcher est empêché par le
plugin d'instance unique.

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
4. **Écriture** : ordre identique à `list` = rien n'est écrit ; sinon `write_index` (§ 5) et ses
   six étapes, dont la suppression de `usr/0/library.cache`. Les UUID sont écrits en minuscules.

Seuls `list`, ses fichiers de travail (`list.new`, `.bak`, `.restore`) et `library.cache` sont
modifiés ; `list.hidden`, `str/`, `str.hidden/`, `usr/0/` et les réglages ne le sont jamais
(vérifié par les tests, qui comparent toute l'arborescence hors de ces fichiers).

| Coupure ou échec après…                 | État laissé                               | Reprise (`recover`)                |
| --------------------------------------- | ----------------------------------------- | ---------------------------------- |
| la validation                           | rien de modifié                           | aucune                             |
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
| T9  | Fichiers de travail de l'index (`list.new`, `.bak`, `.restore`) tolérés dans `etc/library/` ; fichier sonde `.flam-sonde-*` toléré                                                                                                      | 4, 5     | ignorés par le firmware         |                                   |

Aucun point de ce tableau n'a été mesuré lors de la rédaction de ce document.
