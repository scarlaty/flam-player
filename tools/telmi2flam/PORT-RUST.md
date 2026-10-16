# Port de telmi2flam en Rust : note de conception (#22)

But : livrer la surcouche flam-launcher **sans Python ni Pillow**. Aujourd'hui, l'installateur
embarque un Python officiel, Pillow et les scripts (`apps/flam-launcher/scripts/build-converter.ps1`,
ressource Tauri `telmi2flam/`). Le launcher lance `telmi2flam.py --progress-json` en processus
enfant (`apps/flam-launcher/src-tauri/src/install/convert.rs`). L'annulation passe par
`CTRL_BREAK_EVENT`, émis par un petit assistant **Python**.

Cette note fixe le périmètre, l'architecture, les invariants, la stratégie de parité, le plan
de bascule et le découpage en PR. Elle ne contient aucun code.

## 1. Périmètre

### À porter (livré avec le launcher)

| Module Python | Rôle | Remarques pour le port |
| --- | --- | --- |
| `telmi2flam.py` : lecture de l'archive | `find_root`, index insensible à la casse et à la normalisation (`_fold` = NFC puis `lower()`), `check_asset_name`, `read_info` (`--info`) | Noms d'entrées non UTF-8 : Python les décode en cp437, le Rust doit faire pareil. |
| `telmi2flam.py` : JSON | `load_json`, `_expect`, validation de `metadata.json`, `nodes.json`, `notes.json` | Différences à reproduire ou à décider : clés dupliquées (la dernière gagne, à la position de la première), `NaN`/`Infinity`, entiers hors 64 bits, surrogates isolés (`"\ud800"`). |
| `telmi2flam.py` : texte | `_flat`, `_font_safe` (police ASCII + Latin-1), `_choice_label`, troncature `LABEL_MAX` | Mêmes tables de remplacement et même découpage en caractères (points de code, pas octets). |
| `telmi2flam.py` : moteur Lua | `lua_repr`, génération de `script/nodes.lua`, `_max_chapters_per_run`, copie de `engine/` et du runtime (`runtime/script`, `runtime/img/script`) | `repr(float)` de Python (forme la plus courte, exposant au-delà de 1e16, `1.0` et non `1`) doit être reproduit exactement. |
| `telmi2flam.py` : images | `_open_rgba` (modes PNG, `tRNS`, 16 bits), `png_to_lif`, `png_to_scene_lif`, `png_to_fit_lif`, `png_to_thumb_lif`, `_avoid_transpose`, `_bound` | Le point le plus délicat : voir §4.2. |
| `telmi2flam.py` : audio | copie des MP3 **telle quelle**, `silent.mp3` constant, avertissement `audio_samplerate` | **Aucun décodage ni ré-encodage MP3**. Il n'y a donc ni décodeur ni rééchantillonneur à embarquer. |
| `telmi2flam.py` : sortie | ordre canonique des entrées, `.plain.pk` (ZIP *stored*), `--plain` / `-o *.plain`, `<sortie>.<pid>.part`, `_commit`, `_clean_orphans` | Voir les invariants §3. |
| `telmi2flam.py` : protocole | options CLI, `--progress-json` (événements `start`/`phase`/`warning`/`error`…, champ `version`), codes de sortie 0/2/3/4/5/6/130 | Contrat lu par `convert.rs`, à garder tel quel. |
| `lif.py` | `encode`, `quantize_rgba` | `decode` existe déjà côté launcher (`src-tauri/src/lif.rs`) : à déplacer dans la crate pour les tests aller-retour. |
| `mp3map.py` | `build` (table de seek `.mp3map`) | Analyse d'octets non fiables : bornes strictes. |

### Ce qui reste en Python (outils du dépôt, jamais livrés)

- `compat.py`, `explore.py` : outils de campagne. Ils appellent aujourd'hui
  `telmi2flam.convert()` en Python. À la fin du port, ils lancent le **binaire Rust** en
  processus enfant (PR 15).
- `validate.py` : validateur de paquets, sans dépendance. Il reste en Python et sert
  d'**oracle indépendant** pour les deux implémentations.
- `tests/` : les tests Python restent tant que `telmi2flam.py` existe. Ils sont supprimés avec
  lui, une fois leurs cas couverts par les tests Rust et le corpus de parité.

## 2. Architecture

### 2.1 Crate

- Nouvelle crate **`crates/telmi2flam/`** (racine du dépôt), qui ne dépend pas de Tauri :
  - **bibliothèque** `telmi2flam` : `convert(input, output, &Options, &mut dyn Progress, &Cancel)
    -> Result<Report, ConvError>` et `read_info(input)`. `ConvError` porte le même `code` et
    le même code de sortie que Python ;
  - **binaire** `telmi2flam` (cible `[[bin]]`) : mêmes options et même protocole que
    `telmi2flam.py`, pour la parité, les outils de campagne et le simulateur.
- Modules : `archive`, `json`, `text`, `lua`, `graph` (conversion de `nodes.json`), `image`
  (décodage et rééchantillonnage), `lif`, `mp3map`, `output`, `progress`, `cli`.
- `engine/*.lua` et les fichiers du runtime sont **inclus dans le binaire**
  (`include_bytes!`, liste explicite reprise de `RUNTIME_SCRIPTS` / `RUNTIME_UI_LIF`). Il n'y a
  plus de dossier de ressources à trouver à l'exécution, donc plus d'erreur « runtime
  introuvable ». Un test vérifie que la liste correspond au contenu du dossier.
- **Workspace Cargo à la racine** (décision D1) : `Cargo.toml` racine avec pour membres
  `apps/flam-launcher/src-tauri` et `crates/telmi2flam`. On a ainsi un seul `Cargo.lock`, une
  seule liste de licences (`build-notices.mjs`) et `cargo build -p telmi2flam --bin telmi2flam`
  pour l'installateur. Coût : `Cargo.lock` et `target/` passent à la racine, et il faut
  adapter la CI `launcher.yml` (cache, chemins), `tauri.conf.json` et les scripts.
  Alternative sans workspace : la crate a son propre `Cargo.lock` et le launcher en dépend par
  chemin. Cela fait deux lock files et deux inventaires de licences.

### 2.2 Intégration au launcher : processus enfant (recommandé, décision D2)

Le launcher lance le **binaire Rust** comme il lance aujourd'hui Python, au lieu d'appeler la
bibliothèque dans son propre processus. Raisons :

- le profil `release` du launcher a `panic = "abort"` : une panique dans un décodeur
  (archive non fiable) **fermerait toute l'application**. `catch_unwind` est inopérant dans ce
  profil ;
- une image ou une archive piégée (bombe de décompression) peut épuiser la mémoire. Dans un
  processus enfant, seul le convertisseur meurt, et le Job Object
  (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) le termine proprement ;
- `convert.rs` (protocole JSON, Job Object, lancement suspendu, délai de grâce, suppression
  du `.part`) est réutilisé presque tel quel. Seule la commande change : `telmi2flam.exe
  <entrée> -o <sortie> --progress-json` au lieu de `python telmi2flam.py …`.

**Annulation sans Python** : l'assistant `CTRL_BREAK_HELPER` est en Python et doit
disparaître. Il est remplacé par l'option `--cancel-on-stdin-eof`, passée seulement par le
launcher : celui-ci garde `stdin` en tube et le ferme pour annuler. Le convertisseur vérifie
le jeton d'annulation entre deux assets, nettoie ses temporaires et sort avec 130. L'arrêt
forcé par le job reste le dernier recours. Sans cette option, le binaire ignore `stdin` : en
CI ou en `nohup`, un `stdin` fermé n'annule rien. `SIGINT`/`Ctrl+C`/`CTRL_BREAK` restent gérés
pour l'usage en console.

La bibliothèque reste appelable en processus (tests, outils futurs). On pourra revoir
l'appel en processus plus tard, si le profil passe en `panic = "unwind"` et que les bornes de
§3 sont prouvées. C'est hors périmètre ici.

### 2.3 Dépendances envisagées

La CI n'accepte que les licences de `ALLOWED_LICENSES`
(`apps/flam-launcher/scripts/notices-lib.mjs`). **Chaque terme** d'une expression SPDX doit y
figurer, même avec `OR`.

| Besoin | Crate | Licence | Statut |
| --- | --- | --- | --- |
| Lecture et écriture ZIP | `zip` (déjà `=4.6.1`, `deflate-flate2`) | MIT | Déjà présent. Écriture *stored* seulement. |
| Décodage PNG | `png` (déjà `=0.18.1`) | MIT OR Apache-2.0 | Déjà présent. La conversion vers RGBA est écrite à la main pour reproduire Pillow (§4.2). |
| MD5 (`uuid.bin`) | `md-5` | MIT OR Apache-2.0 | Nouveau. |
| Normalisation NFC | `unicode-normalization` | MIT OR Apache-2.0 | Nouveau. |
| JSON | `serde_json` (déjà), fonction `preserve_order` | MIT OR Apache-2.0 | Fait entrer `indexmap` (Apache-2.0 OR MIT). |
| Flottants façon Python | `ryu` (ou formatage maison) | Apache-2.0 OR BSL-1.0 | Les deux termes sont autorisés. |
| Rééchantillonnage | **port de l'algorithme de Pillow** (recommandé) ou `fast_image_resize` | Pillow : MIT-CMU (HPND) ; `fast_image_resize` : MIT OR Apache-2.0 | Décision D3, §4.2. |
| Autres formats d'image (JPEG…) | `zune-jpeg` ou `jpeg-decoder` si nécessaire | MIT OR Apache-2.0 (OR Zlib) | Seulement si la campagne en trouve (décision D4). |
| Méthodes ZIP bzip2/LZMA | fonctions `zip` | `bzip2-sys` : licence bzip2, **non autorisée** | Python les lit. Ne les activer que si la campagne en trouve ; sinon, divergence documentée (`archive_unreadable`). |

Pas de `image` (crate complète) : trop de dépendances, et ses conversions (16 → 8 bits par
arrondi `/257`, Lanczos propre) ne donnent pas le résultat de Pillow.

## 3. Invariants et tableau des pannes

Invariants (repris de Python, à tester côté Rust) :

1. **Jamais de sortie partielle** : la sortie finale (`.plain.pk` ou dossier `.plain`) n'existe
   que complète. L'écriture se fait dans `<sortie>.<pid>.part`, voisin de la destination, puis
   par renommage atomique (`MoveFileExW` avec `REPLACE_EXISTING` ; dossier : renommage puis
   remplacement, comme `_commit`).
2. **Aucune écriture hors de la destination** : les noms de sortie sont relatifs, sans `..`,
   sans `\`, sans `:`, sans composant vide (garde zip-slip). Le dossier `.plain` est vérifié
   par chemin canonique. Les entrées de l'archive d'entrée ne sont **jamais extraites** sur
   disque, seulement lues en mémoire.
3. **L'archive d'entrée n'est jamais modifiée.**
4. **Sortie déterministe** : mêmes octets d'entrée et mêmes options donnent la même liste
   ordonnée d'entrées, avec les mêmes contenus. L'horodatage des entrées ZIP est fixé (voir
   §4.1).
5. **Mémoire bornée** : dimensions d'image ≤ seuil de Pillow (`MAX_IMAGE_PIXELS` × 2 ≈
   179 Mpx, au-delà : image illisible), taille décompressée d'une entrée et total bornés
   (décision D5 : Python n'a pas de borne).
6. **Annulation propre** : après une annulation, il ne reste ni `.part` ni sortie, et le code
   de sortie est 130.

| Panne | Étape | État laissé | Comportement attendu / reprise |
| --- | --- | --- | --- |
| Archive absente ou illisible | ouverture | rien | `archive_unreadable`, code 3 |
| Archive corrompue (CRC, en-tête, troncature) | lecture d'une entrée | rien | `archive_unreadable`, code 3. Le CRC doit être vérifié : lire chaque entrée jusqu'au bout. |
| Entrée chiffrée, méthode inconnue (deflate64, bzip2/LZMA non activés) | lecture | rien | `archive_unreadable`, code 3 (Python : 3 pour une méthode inconnue, mais `internal_error` 6 pour une entrée chiffrée ; divergence volontaire, documentée) |
| Noms dupliqués dans l'archive | indexation | rien | même entrée retenue que Python (`ZipFile.getinfo` : la **dernière**). À tester, car `zip` peut refuser ou prendre la première. |
| Pas une histoire Telmi | `find_root` | rien | `not_telmi`, code 3 |
| JSON invalide / types inattendus | `load_json`, `_expect` | rien | `invalid_content`, code 4 ; `notes.json` illisible : avertissement `notes_ignored` |
| Asset manquant | graphe | rien | `missing_asset`, code 5 ; avec `--allow-missing` : `empty.lif` / `silent.mp3` et avertissement |
| Image illisible, trop grande, format non géré | image | rien | avertissement `image_unreadable`, `empty.lif` (pas d'échec) |
| MP3 sans trame valide | mp3map | rien | avertissement `mp3map_failed`, MP3 copié sans `.mp3map` |
| Disque plein, droits refusés, chemin trop long | écriture du `.part` | `.part` supprimé | `output_error`, code 6, pas de sortie |
| Échec du renommage final | commit | `.part` supprimé, ancienne sortie intacte | `output_error`, code 6 |
| Annulation (stdin fermé, Ctrl+C, `CTRL_BREAK`) | n'importe laquelle | `.part` supprimé | code 130, événement `error` `cancelled` |
| Arrêt brutal (job terminé, coupure de courant) | n'importe laquelle | `<sortie>.<pid>.part` orphelin | supprimé à la conversion suivante vers la même sortie (`_clean_orphans` : seulement si le PID n'est plus vivant). Le launcher jette aussi son `staging`. |
| Panique Rust | n'importe laquelle | `.part` possible | binaire : crochet de panique, événement `internal_error`, code 6 ; le `.part` est nettoyé à la reprise |
| Mémoire épuisée | image, lecture | `.part` possible | processus tué, le launcher signale l'échec, nettoyage à la reprise |
| Chemins non ASCII (entrée, sortie, profil Windows) | toutes | — | API larges de Windows (`std::fs`, pas de conversion ANSI). Tests avec `é`, idéogrammes et emoji dans les chemins. |
| Noms d'assets non ASCII dans l'archive | lecture, sortie | — | `_fold` NFC + minuscules. Noms de sortie identiques à Python, avec le drapeau UTF-8 (bit 11) posé seulement si le nom n'est pas ASCII, comme `zipfile`. |
| Bombe de décompression (entrée ZIP énorme) | lecture | rien | refus au-delà de la borne D5 (`archive_unreadable` ou code dédié) |

## 4. Stratégie de parité

### 4.1 Ce qu'on compare

- **Pas les octets du conteneur `.plain.pk`** : `zipfile.writestr` horodate chaque entrée avec
  l'heure locale, donc deux conversions Python ne donnent déjà pas le même fichier. On compare
  la **liste ordonnée des entrées** : nom, méthode (*stored*), drapeau UTF-8 et contenu octet à
  octet. Le Rust fixe l'horodatage (1980-01-01). Le lecteur du player (`src/formats/pk_reader.c`)
  ignore la date : on le vérifie dans la PR 9.
- Les résultats aussi : succès ou code d'erreur identique, code de sortie identique, et
  **multiensemble des codes d'avertissement** identique (pas le texte).
- Outil : `tools/telmi2flam/parity.py` compare deux `.plain.pk` ou deux dossiers de sortie et
  écrit un rapport JSON. Il passe aussi `validate.py` sur la sortie Rust.

### 4.2 Images : le point dur

La sortie LIF dépend du redimensionnement de Pillow :

- `resize(…, LANCZOS)` sur du RGBA passe par du **RGBa prémultiplié**, puis un passage
  horizontal et un passage vertical **en virgule fixe** (coefficients 8 bits pour les
  images 8 bits), puis la dé-prémultiplication ;
- `thumbnail()` calcule la taille cible avec ses propres règles d'arrondi et utilise
  `reducing_gap=2.0` : une réduction entière par moyenne de boîte (`reduce`) précède le
  Lanczos ;
- `_open_rgba` a ses règles : 16 bits → 8 bits par `v/256`, clés `tRNS` en gris 1/2/4/16 bits
  et en RGB 16 bits, palettes.

Deux options (décision D3) :

- **A (recommandée)** : porter en Rust les algorithmes de Pillow utilisés (`Resample.c` pour
  Lanczos en virgule fixe, `Reduce.c` pour la boîte, calcul de taille de `thumbnail`,
  prémultiplication). On vise la **parité octet à octet des `.lif`**. Pillow est sous licence
  MIT-CMU (HPND, permissive) : il faut une mention dans `THIRD-PARTY-NOTICES` et ajouter
  `MIT-CMU` à `ALLOWED_LICENSES`, ce qui demande l'accord du mainteneur (§2.3).
- **B** : `fast_image_resize` (MIT OR Apache-2.0), avec une **tolérance** : décodage des deux
  LIF, écart par canal ≤ 1 pas RGB565 (8 en rouge et bleu, 4 en vert) et ≤ 2 en alpha, sur au
  moins 99,9 % des pixels, avec un PSNR minimal mesuré pendant la PR 7. Dans ce cas, les
  octets des `.lif` diffèrent (codage delta/cache), ce qui rend les diagnostics plus lourds.

Dans tous les cas, les images **sans redimensionnement** (taille déjà exacte, `--keep-size`
sous 2047 px, canevas) doivent être identiques à l'octet près : seuls le décodage PNG, la
quantification et l'encodeur LIF entrent en jeu.

### 4.3 Corpus

1. **Synthétique (CI)** : un générateur Python (`tools/telmi2flam/tests/parity/make_corpus.py`)
   produit des archives à partir des cas de `tests/test_telmi2flam.py` et `test_progress.py`
   (tous les modes PNG, `tRNS`, 16 bits, portraits, noms NFD et cp437, JSON limites,
   assets manquants, archives corrompues, MP3 MPEG1/2/2.5, ID3, trames cassées). Ces archives
   ne contiennent aucun contenu sous licence, mais elles sont **générées en CI**, pas
   commitées. Le job convertit chaque archive avec Python (oracle) puis avec Rust, et
   `parity.py` doit donner zéro écart.
2. **Campagne (locale)** : les archives réelles de `.cache/telmi/` (ignoré par Git, jamais
   commité). `compat.py --engine rust`, ou un script dédié, convertit chaque archive avec les
   deux moteurs et écrit `.cache/parity/report.json`. Seuil de bascule : 100 % de statuts
   identiques, entrées non-image identiques à l'octet près, images selon D3. Les machines de
   mesure sont chargées : prévoir des lots et une reprise (PR 13).
3. **Version de Pillow de l'oracle épinglée** : celle de `build-converter.ps1`. Aujourd'hui, la
   CI installe `Pillow>=10.0` sans version fixe, il faut l'épingler dans `telmi2flam.yml`
   (PR 3).

## 5. Plan de bascule

1. **Développement** (PR 1 à 11) : la crate et le binaire existent et sont testés en CI, mais
   ne sont pas livrés.
2. **Parité** (PR 12, 13) : CI synthétique verte, puis campagne conforme au seuil §4.3.
3. **Livraison en double** (PR 14) : l'installateur embarque `telmi2flam.exe` **et** le Python
   actuel. Un réglage caché de `settings.json`, `converter_engine: "python" | "rust"`, vaut
   **`python` par défaut**. Le launcher le relit à chaque tâche (`Toolchain`).
4. **Bascule** (PR 15) : défaut `rust`. Pendant une version, **repli automatique sur Python**
   seulement pour `internal_error` (code 6, panique) ou un plantage du binaire, avec
   l'information dans le résultat de compatibilité (#20). Une erreur de contenu (codes 3/4/5)
   n'est **jamais** rejouée en Python, pour ne pas masquer une divergence.
5. **Suppression** (PR 16, 17) : retrait de Python, Pillow et des scripts de l'installateur
   (`build-converter.ps1`, ressource `telmi2flam/`, contrôles de `launcher.yml`, notices), du
   réglage `python_path` (backend, écran Paramètres, tests ; un ancien `settings.json` qui le
   contient reste lisible et le champ est ignoré), de `CTRL_BREAK_HELPER`. Puis bascule des
   outils de campagne sur le binaire et suppression de `telmi2flam.py`, `lif.py`, `mp3map.py`.

**Version du convertisseur** (décision D6) : la bibliothèque et l'outil `compat` enregistrent
la version annoncée (`start.version`, `converter` dans `library.json` et les résultats de
compatibilité). Proposition : le Rust annonce `2.0.0` (nouvelle implémentation). Les
résultats de compatibilité obtenus avec `1.0.0` restent affichés comme « obtenus avec une
version antérieure », comme pour toute montée de version.

## 6. Découpage en PR

Chaque PR traite un seul thème, avec au plus cinq changements fonctionnels. Le modèle
`opus` est réservé au code de données : formats, analyse d'entrées non fiables, écriture de
la sortie, pipeline d'installation.

| # | Issue | PR | Dépend de | Modèle | Zones | Estimation |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | #ISSUE1 | Squelette de la crate `crates/telmi2flam` (lib + bin vides), workspace racine (D1), CI Rust de la crate (`fmt`, `clippy`, `test`), inventaire des licences | — | sonnet | telmi2flam, ci | 0,5 j |
| 2 | #ISSUE2 | Codecs : `lif` (encode, quantize, decode déplacé depuis le launcher) et `mp3map`, avec des vecteurs de test produits par Python (petits, commités) | 1 | opus | telmi2flam | 1 j |
| 3 | #ISSUE3 | Outillage de parité Python : `make_corpus.py`, `parity.py`, Pillow épinglé dans la CI | — | sonnet | telmi2flam | 1 j |
| 4 | #ISSUE4 | Lecture de l'archive : index (`_fold`, cp437, doublons), `find_root`, `check_asset_name`, bornes de taille (D5), erreurs `archive_unreadable` / `not_telmi`, `read_info` | 1 | opus | telmi2flam | 1 j |
| 5 | #ISSUE5 | JSON et texte : `load_json` (règles Python), `_expect`, `_flat`, `_font_safe`, `_choice_label`, `lua_repr` (flottants façon Python) | 1 | opus | telmi2flam | 1 j |
| 6 | #ISSUE6 | Décodage d'image avec la sémantique de Pillow : modes PNG, `tRNS`, 16 bits, palettes, garde de taille ; formats additionnels selon D4 | 2 | opus | telmi2flam | 1,5 j |
| 7 | #ISSUE7 | Rééchantillonnage (D3) et fonctions `png_to_*_lif` (scène, ajustement, vignette, `_avoid_transpose`, `_bound`) | 6 | opus | telmi2flam | 2 j |
| 8 | #ISSUE8 | Conversion du graphe : stages et actions, `_max_chapters_per_run`, allocation des noms d'assets, couverture et titre, avertissements, moteur et runtime inclus | 4, 5, 7 | opus | telmi2flam | 2 j |
| 9 | #ISSUE9 | Écriture de la sortie : ordre canonique, ZIP *stored* (horodatage fixe, drapeau UTF-8), `--plain`, `.part`, commit atomique, `_clean_orphans`, garde zip-slip ; vérification avec `pk_reader.c` | 8 | opus | telmi2flam | 1 j |
| 10 | #ISSUE10 | Progression et annulation : événements `--progress-json` identiques, jeton d'annulation, `--cancel-on-stdin-eof`, signaux console, crochet de panique, codes de sortie | 9 | opus | telmi2flam | 1 j |
| 11 | #ISSUE11 | CLI complète : options (`-o`, `--plain`, `--keep-size`, `--allow-missing`, `--selector`, `--info`, `--version`), messages humains sur stderr | 10 | sonnet | telmi2flam | 0,5 j |
| 12 | #ISSUE12 | CI de parité sur le corpus synthétique (oracle Python contre Rust, `validate.py` sur la sortie Rust) | 3, 11 | sonnet | telmi2flam, ci | 0,5 j |
| 13 | #ISSUE13 | Campagne de parité sur `.cache/telmi/` (`compat.py --engine rust`, rapport, lots avec reprise) et corrections, ou issues de suivi pour les écarts | 12 | sonnet | telmi2flam | 1 j + mesures |
| 14 | #ISSUE14 | Launcher : construire et livrer `telmi2flam.exe`, réglage `converter_engine` (défaut `python`), `convert.rs` sans Python (commande, annulation par stdin), détection du binaire | 11 | opus | launcher-rust, ci | 1,5 j |
| 15 | #ISSUE15 | Bascule : défaut `rust`, repli Python seulement sur `internal_error` ou plantage, trace dans le résultat de compatibilité, version 2.0.0 (D6) | 13, 14 | opus | launcher-rust | 1 j |
| 16 | #ISSUE16 | Retrait de Python de l'installateur : `build-converter.ps1`, ressources, `launcher.yml`, notices, `CTRL_BREAK_HELPER`, réglage `python_path` (backend et UI, ancien champ ignoré) | 15, puis une version publiée | opus | launcher-rust, launcher-ui, ci | 1 j |
| 17 | #ISSUE17 | Outils de campagne sur le binaire Rust (`compat.py`, `explore.py`), suppression de `telmi2flam.py`, `lif.py`, `mp3map.py` et de leurs tests (cas repris par le corpus) | 16 | sonnet | telmi2flam | 1 j |

Ordre conseillé : 1 et 3 en parallèle, puis 2, 4 et 5 en parallèle, puis 6 → 7 → 8 → 9 → 10 →
11, puis 12 → 13, puis 14 (possible dès 11), 15, 16, 17. Total : environ 18 jours de
développement, hors temps de campagne.

## 7. Décisions attendues du mainteneur

- **D1** : workspace Cargo à la racine (recommandé) ou crate isolée avec son propre `Cargo.lock`.
- **D2** : intégration en processus enfant (recommandé : `panic = "abort"`, entrées non
  fiables) ou appel de la bibliothèque dans le processus du launcher.
- **D3** : port des algorithmes de Pillow et ajout de `MIT-CMU` aux licences autorisées
  (parité octet à octet), ou `fast_image_resize` avec une tolérance de pixels.
- **D4** : formats d'image autres que PNG (JPEG, GIF…) et méthodes ZIP autres que deflate.
  À trancher d'après un comptage sur la campagne : Pillow ouvre tout, le Rust ne prendra que
  ce qui est rencontré.
- **D5** : bornes mémoire (taille décompressée par entrée et totale). Python n'en a pas ; une
  borne peut refuser une archive que Python accepte.
- **D6** : version annoncée par le convertisseur Rust (`2.0.0` proposé) et effet sur les
  résultats de compatibilité déjà enregistrés.
- **Durée de la double livraison** : une version publiée avec le repli Python avant la PR 16
  (proposé).
