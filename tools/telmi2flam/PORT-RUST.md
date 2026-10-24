# Port de telmi2flam en Rust : note de conception (#22)

But : livrer la surcouche flam-launcher **sans Python ni Pillow**. À l'origine de cette note,
l'installateur embarquait un Python officiel, Pillow et les scripts
(`apps/flam-launcher/scripts/build-converter.ps1`, ressource Tauri `telmi2flam/`), le launcher
lançait `telmi2flam.py --progress-json` en processus enfant
(`apps/flam-launcher/src-tauri/src/install/convert.rs`) et l'annulation passait par
`CTRL_BREAK_EVENT`, émis par un petit assistant **Python**. **État (PR 18, #138, faite)** :
l'installateur ne livre plus que `telmi2flam.exe`, le launcher ne lance plus Python (voir §5.4).

Cette note fixe le périmètre, l'architecture, les invariants, la stratégie de parité, le plan
de bascule et le découpage en PR. Elle ne contient aucun code.

## 1. Périmètre

### À porter (livré avec le launcher)

| Module Python | Rôle | Remarques pour le port |
| --- | --- | --- |
| `telmi2flam.py` : lecture de l'archive | `find_root`, index insensible à la casse et à la normalisation (`_fold` = NFC puis `lower()`), `check_asset_name`, `read_info` (`--info`) | Noms d'entrées non UTF-8 : Python les décode en cp437, le Rust doit faire pareil. |
| `telmi2flam.py` : JSON | `load_json`, `_expect`, validation de `metadata.json`, `nodes.json`, `notes.json` | Cas tranchés dans le tableau §3 : clés dupliquées (la dernière gagne, à la position de la première), `NaN`/`Infinity`, flottants hors plage, entiers hors 64 bits, profondeur, surrogates isolés. |
| `telmi2flam.py` : texte | `_flat`, `_font_safe` (police ASCII + Latin-1, NFKD, `unicodedata.name`), `_choice_label`, troncature `LABEL_MAX` | Mêmes tables de remplacement, même découpage en points de code. `_flat` coupe sur les espaces au sens de `str.split()` de Python (dont `\x1c`-`\x1f` et U+0085), pas au sens de `split_whitespace` : table explicite. Tests sur le sigma final et U+0130. |
| `telmi2flam.py` : moteur Lua | `lua_repr`, génération de `script/nodes.lua`, `_max_chapters_per_run`, copie de `engine/` et du runtime (`runtime/script`, `runtime/img/script`) | `repr(float)` de Python (forme la plus courte, exposant au-delà de 1e16, `1.0` et non `1`) doit être reproduit exactement. |
| `telmi2flam.py` : images | `_open_rgba` (modes PNG, `tRNS`, 16 bits), `png_to_lif`, `png_to_scene_lif`, `png_to_fit_lif`, `png_to_thumb_lif`, `_avoid_transpose`, `_bound` | Le point le plus délicat : voir §4.2. |
| `telmi2flam.py` : audio | copie des MP3 **telle quelle** à 44 100 Hz (hors 44 100 Hz : rééchantillonnage par un ffmpeg externe depuis telmi2flam 1.1.0, #46), `silent.mp3` constant, avertissement `audio_samplerate` | **Aucun décodage ni ré-encodage MP3**. Il n'y a donc ni décodeur ni rééchantillonneur à embarquer : le port lance le même ffmpeg externe (`FLAM_FFMPEG`, sinon `PATH`), mêmes arguments, délai et raisons d'échec (`ffmpeg.rs`, #354 ; parité avec un faux ffmpeg, `tests/ffmpeg.rs`). |
| `telmi2flam.py` : sortie | ordre canonique des entrées, `.plain.pk` (ZIP *stored*), `--plain` / `-o *.plain`, `<sortie>.<pid>.part`, `_commit`, `_clean_orphans` | Voir les invariants §3. |
| `telmi2flam.py` : protocole | options CLI, `--progress-json` (événements `start` avec `version`, `step` avec `phase`/`done`/`total` espacés de 0,1 s, `warning`, `done`, `error` avec `exit`), lignes `  ! …` sur stderr, codes de sortie 0/2/3/4/5/6/130, plus 7 (`unsupported_format`) pour le Rust | Contrats lus par `convert.rs` (événements) et par `compat.py` (lignes `  !` de stderr), à garder tels quels. |
| `lif.py` | `encode`, `quantize_rgba` | `decode` a été déplacé du launcher (`src-tauri/src/lif.rs`) dans la crate (`crates/telmi2flam/src/lif.rs`) : tests aller-retour dans la crate, le launcher l'utilise comme dépendance. |
| `mp3map.py` | `build` (table de seek `.mp3map`) | Analyse d'octets non fiables : bornes strictes. |

### Ce qui reste en Python (outils du dépôt, jamais livrés)

- `compat.py`, `explore.py` : outils de campagne. Ils appellent aujourd'hui
  `telmi2flam.convert()` en Python. À la fin du port, ils lancent le **binaire Rust** en
  processus enfant (PR 20).
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
  introuvable ». Cette liste devient un **manifeste unique** (fichier texte du runtime), lu
  par `telmi2flam.py`, `validate.py` et le Rust : il n'y a plus trois copies à tenir à jour.
  Un test vérifie que le manifeste correspond au contenu du dossier.
- `include_bytes!` embarque les octets de la copie de travail : avec `core.autocrlf`, un
  `.lua` peut passer en CRLF. Il faut ajouter à `.gitattributes`
  `tools/telmi2flam/engine/** eol=lf` et `tools/telmi2flam/runtime/** -text` (PR 1).
- **Workspace Cargo à la racine** (décision D1) : `Cargo.toml` racine avec pour membres
  `apps/flam-launcher/src-tauri` et `crates/telmi2flam`. On a ainsi un seul `Cargo.lock`, une
  seule liste de licences (`build-notices.mjs`) et `cargo build -p telmi2flam --bin telmi2flam`
  pour l'installateur. Coûts, tous dans la PR 1 :
  - `Cargo.lock` et `target/` passent à la racine ;
  - CI : cache et `working-directory` de `launcher.yml`, filtres de chemins de `launcher.yml`
    et `telmi2flam.yml` (ajouter `crates/**`), répertoire cargo de `.github/dependabot.yml` ;
  - scripts : chemin du `Cargo.lock` dans `build-notices.mjs`, `target/` vu par Tauri ;
  - **`[profile.release]` ne s'applique qu'au `Cargo.toml` racine** : il faut le déplacer à la
    racine. Le binaire du convertisseur en hérite (`panic = "abort"`, `opt-level = "s"`). On
    ajoute `[profile.release.package.telmi2flam]` avec `opt-level = 3`, pour la vitesse du
    Lanczos. Les tailles calculées à partir d'entrées non fiables utilisent des opérations
    vérifiées (`checked_*`), car la release n'a pas de contrôle de dépassement.
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
le jeton d'annulation entre deux assets **et dans les boucles de lignes** du décodage PNG et
du rééchantillonnage. Sans cela, une grande image dépasserait le délai de grâce, comme le fait
aujourd'hui un `KeyboardInterrupt` Python levé n'importe où. Il nettoie ensuite ses
temporaires et sort avec 130. Le fil qui lit `stdin` ne doit pas empêcher le processus de se
terminer. L'arrêt
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
| Rééchantillonnage | **port de l'algorithme de Pillow** (pas de crate) | Pillow : MIT-CMU (HPND) | Ce n'est pas une dépendance Cargo : `ALLOWED_LICENSES` ne change pas. En-tête de licence et mention dans les notices (D3, §4.2). |
| Autres formats d'image (JPEG…) | JPEG : `zune-jpeg` (`=0.5.15`, avec `zune-core` `=0.5.3`, sans les fonctions SIMD `x86`/`neon`) ; GIF : port de Pillow (`gif.rs`, HPND, voir la ligne du rééchantillonnage) | MIT OR Apache-2.0 OR Zlib (zune) | D4 : délégation à Python (`unsupported_format`, code 7) pendant la transition. Le support devient obligatoire avant #138 pour chaque format que la campagne trouve : en cours (#459, §4.4 ; PNG atypiques, GIF et JPEG faits, le GIF par un port de `GifDecode.c` sans crate, le JPEG à ±4 près selon D7). |
| Méthodes ZIP bzip2/LZMA/Zstandard | bzip2 et LZMA écrits à la main (`unbzip2.rs`, `unlzma.rs`) ; Zstandard : `ruzstd` `=0.9.1` (MIT) et `twox-hash` `=2.1.5` (MIT) | MIT (dépendances) | **Fait (#458)**, voir §3.1. `bzip2-sys` et `libbz2-rs-sys` : licence bzip2-1.0.6, non autorisée ; `bzip2-rs` (MIT/Apache-2.0) échoue sur un bloc incompressible ; `lzma-rs` (MIT) exige le marqueur de fin. |

Pas de `image` (crate complète) : trop de dépendances, et ses conversions (16 → 8 bits par
arrondi `/257`, Lanczos propre) ne donnent pas le résultat de Pillow.

## 3. Invariants et tableau des pannes

Invariants (repris de Python, à tester côté Rust) :

1. **Jamais de sortie partielle** : la sortie finale (`.plain.pk` ou dossier `.plain`) n'existe
   que complète. L'écriture se fait dans `<sortie>.<pid>.part`, voisin de la destination, puis
   le `.plain.pk` est publié par renommage atomique (`MoveFileExW` avec `REPLACE_EXISTING`,
   avec une relance courte si un antivirus verrouille le fichier). Limite reprise de Python :
   le dossier `.plain` (`--plain`) est publié par `_commit`, **fichier par fichier** dans un
   dossier existant, donc sans atomicité. De plus, les deux publications successives
   (`.plain.pk` puis `.plain`) ne sont pas atomiques ensemble. Le launcher n'utilise que le
   `.plain.pk`. Le comportement du dossier est conservé et testé tel quel.
   Durabilité (#357, #416) : le `.plain.pk` et chaque fichier du `.plain` sont vidés sur disque
   (`sync_all`) avant le renommage. Mesure (#416, 500 fichiers dont 100 de 200 Kio, soit 21 Mio,
   disque local NTFS, antivirus actif, `mesure_sync_all` ignoré par défaut de `output.rs`) :
   sans vidage 0,34 à 0,40 s (jusqu'à 4,5 s en cas d'activité disque concurrente), vidage par
   fichier 1,6 s (environ 2,5 ms par fichier, soit environ 4 fois le temps d'écriture sans vidage), vidage
   final séquentiel 1,3 à 1,9 s (aucun gain : chaque `FlushFileBuffers` coûte autant), vidage
   final sur 8 fils 0,65 à 1,35 s (gain partiel seulement). Le coût absolu reste de l'ordre de
   la seconde pour des centaines de fichiers, et le launcher n'écrit que le `.plain.pk` (un seul
   vidage) : le dossier `.plain` ne sert qu'à la CLI. Le vidage par fichier est conservé, sans
   vidage final. Pas de mesure sur clé USB
   (aucun volume amovible monté pendant la mesure).
   Classement des erreurs : si la sortie échoue (`output_error`) alors que l'archive d'entrée
   relue en flux est elle-même illisible ou modifiée, l'erreur rendue est `archive_unreadable`
   (cause première) : l'archive est relue en entier seulement après un échec d'écriture.
   Bibliothèque : `output::part_path` n'est plus public (`pub(crate)`, le paramètre `call`
   d'#357 n'est pas une API) ; une panique qui se déroule supprime et désinscrit les `.part`
   (garde RAII), `discard_pending` ne servant qu'à `panic = "abort"`.
2. **Aucune écriture hors de la destination** : les noms de sortie sont relatifs, sans `..`,
   sans `\`, sans `:`, sans composant vide (garde zip-slip). Le dossier `.plain` est vérifié
   par chemin canonique. Les entrées de l'archive d'entrée ne sont **jamais extraites** sur
   disque, seulement lues en mémoire.
3. **L'archive d'entrée n'est jamais modifiée.**
4. **Sortie déterministe** : mêmes octets d'entrée et mêmes options donnent la même liste
   ordonnée d'entrées, avec les mêmes contenus. L'horodatage des entrées ZIP est fixé (voir
   §4.1).
5. **Aucune archive acceptée par Python n'est refusée** (décision D5) : les bornes sont
   exactement celles que Python applique réellement. Pour les images, c'est la protection
   anti-bombe de Pillow : au-delà de `2 × MAX_IMAGE_PIXELS` (≈ 179 Mpx), l'image est illisible
   (avertissement `image_unreadable`, `empty.lif`), comme aujourd'hui. Il n'y a aucune borne
   sur la taille décompressée des entrées ZIP (Python n'en a pas), ni de limite mémoire du Job
   Object plus stricte que l'existant. Le Rust réduit sa consommation sans refuser : MP3 écrits
   en flux dans le `.part` (par tours de 256 Kio, #402) au lieu d'être tous gardés en mémoire, copies d'images libérées au
   plus tôt.
6. **Délégation des cas non gérés** (décision D4) : face à une image dans un autre format que PNG,
   GIF ou JPEG (les PNG atypiques, APNG compris, le GIF et le JPEG sont décodés depuis #459,
   §4.4 ; restent délégués les JPEG que libjpeg-turbo lit mais pas `zune-jpeg`), le Rust ne
   convertit pas (les entrées ZIP bzip2, LZMA et Zstandard sont lues depuis #458). Il nettoie son `.part`, émet
   l'événement `error` avec le code `unsupported_format` et sort avec le **code 7**, distinct de
   `internal_error`. Tant que le repli Python existe, le launcher rejoue alors la conversion
   en Python.
7. **Annulation propre** : après une annulation, il ne reste ni `.part` ni sortie, et le code
   de sortie est 130.

| Panne | Étape | État laissé | Comportement attendu / reprise |
| --- | --- | --- | --- |
| Archive absente ou illisible | ouverture | rien | `archive_unreadable`, code 3 |
| Archive corrompue (CRC, en-tête, troncature) | lecture d'une entrée | rien | `archive_unreadable`, code 3. Le CRC doit être vérifié : lire chaque entrée jusqu'au bout. |
| Entrée chiffrée, méthode que Python ne lit pas non plus (deflate64…) | lecture | rien | `archive_unreadable`, code 3 (Python : 3 pour une méthode inconnue, mais `internal_error` 6 pour une entrée chiffrée ; divergence volontaire, car Python échoue aussi) |
| Entrée bzip2, LZMA ou Zstandard | lecture | rien | Lue comme `zipfile` (#458, §3.1). Flux invalide : Python lève `OSError`, `LZMAError` ou `ZstdError` (`internal_error`, code 6), le Rust rend `archive_unreadable` (code 3) : divergence volontaire, Python échoue aussi. Flux tronqué, CRC faux, bombe : voir §3.1. |
| Image dans un autre format que PNG, GIF ou JPEG (BMP…), JPEG lu par libjpeg mais pas par le Rust (arithmétique, sans perte, refus de `zune-jpeg`…) | image | `.part` supprimé | `unsupported_format`, code 7 : délégation à Python (invariant 6), et non `image_unreadable` : Pillow la lirait. Prise en charge format par format (#459, §4.4) |
| JPEG | image | — | décodé par `zune-jpeg`, verdict (lu ou illisible) reproduit de Pillow et libjpeg-turbo, pixels à ±4 près (D7, §4.4) |
| PNG atypique (APNG, `IHDR` absent en tête ou répété, `tRNS` répété, après l'IDAT ou de longueur inattendue) | image | — | décodé comme Pillow, erreurs comprises (#459, §4.4) ; plus de délégation |
| Noms dupliqués dans l'archive | indexation | rien | même entrée retenue que Python (`ZipFile.getinfo` : la **dernière**). `zip` 4.6.1 retient aussi la dernière (`IndexMap::insert`), mais ne liste le nom qu'une fois : `find_root` et l'index replié (`setdefault`, premier nom rencontré) doivent reproduire `namelist()`. Test dédié. |
| Pas une histoire Telmi | `find_root` | rien | `not_telmi`, code 3 |
| JSON invalide / types inattendus | `load_json`, `_expect` | rien | `invalid_content`, code 4 ; `notes.json` illisible : avertissement `notes_ignored` |
| JSON limites : flottant hors plage (`1e400`), entier hors 64 bits (`i64::MIN..=u64::MAX` accepté : `Value::Int` ou `Value::UInt`), imbrication profonde, surrogate isolé | `load_json` | rien | **Divergences assumées** (listées dans `parity.py`). Python accepte `1e400` (→ `inf`) et les grands entiers ; il lève `RecursionError` au-delà de la pile (code 6) ; il accepte `"\ud800"` puis échoue à l'encodage UTF-8 (code 6). Rust : `invalid_content`, code 4, dans tous ces cas. Valeurs proches (`NaN`, `Infinity`, clés dupliquées) : comportement de Python reproduit. Profondeur : le Rust refuse au-delà de 512 niveaux, Python lève `RecursionError` vers 985 niveaux (limite de récursion 1000 moins la pile déjà utilisée) : un document de 513 à environ 985 niveaux est accepté par Python et refusé par le Rust (code 4). Écart voulu, pile bornée sans récursion profonde. **Messages d'erreur** : Python dit « line L column C (char N) » (N en caractères), le Rust « position N » en **octets** UTF-8 ; seuls le code et la catégorie sont comparés, le texte n'est pas une interface. |
| `--info` : fin de ligne | sortie | rien | Le Rust écrit `\n` partout ; Python, en mode texte, écrit `\r\n` sous Windows. Écart assumé : `parity.py` et les tests comparent après `strip()`, et les consommateurs lisent ligne à ligne. Un tube fermé (EPIPE) n'est pas une erreur : l'échec d'écriture est ignoré (pas de panique). |
| ffmpeg (MP3 hors 44 100 Hz, #354) : absent, en échec, trop long (120 s), sortie vide ou illisible, annulation | `ffmpeg.rs` | dossier temporaire `t2f_ff_*` supprimé, ffmpeg tué | Avertissement `audio_samplerate` avec la raison de Python et MP3 recopié ; annulation : `cancelled` (130). **Écarts assumés** : texte des erreurs du système ; échec de suppression du dossier temporaire ignoré (Python perd la sortie et avertit, constat de #274) ; dernière ligne d'erreur de plus de 64 Kio ; bit `x` sous Unix au lieu de `os.access` ; lignes `  i …` et compte des fichiers rééchantillonnés ni affichés ni émis (#131). |
| Asset manquant | graphe | rien | `missing_asset`, code 5 ; avec `--allow-missing` : `empty.lif` / `silent.mp3` et avertissement |
| Image illisible (aussi pour Pillow), ou au-delà de `2 × MAX_IMAGE_PIXELS` | image | rien | avertissement `image_unreadable`, `empty.lif` (pas d'échec). **Messages** : le Rust reformule le détail de l'exception de Python (texte de Pillow, adresse mémoire de `BytesIO`), de même pour `notes.json` (`JSON invalide (…)`), et omet le nom Unicode des caractères remplacés (`U+65E5 -> '?'`) ; `package_parity.rs` compare tous les autres messages d'avertissement à l'identique (#351) |
| MP3 sans trame valide | mp3map | rien | avertissement `mp3map_failed`, MP3 copié sans `.mp3map` |
| Disque plein, droits refusés, chemin trop long | écriture du `.part` | `.part` supprimé | `output_error`, code 6, pas de sortie |
| Échec du renommage final | commit | `.part` supprimé, ancienne sortie intacte | `output_error`, code 6 |
| Annulation (stdin fermé, Ctrl+C, `CTRL_BREAK`) | n'importe laquelle | `.part` supprimé | code 130, événement `error` `cancelled` |
| Arrêt brutal (job terminé, coupure de courant) | n'importe laquelle | `<sortie>.<pid>.part` (ou `<sortie>.<pid>-<n>.part` pour la n-ième écriture du processus, #357 ; jamais produit par la CLI, qui n'écrit qu'une fois, et ignoré par le `_clean_orphans` de Python) orphelin | supprimé à la conversion suivante vers la même sortie (`_clean_orphans` : seulement si le PID n'est plus vivant, via `OpenProcess`/`GetExitCodeProcess` ; notre propre PID s'il n'est pas au registre des temporaires). Un PID réutilisé empêche ce nettoyage : c'est une limite connue. Un `.part` jugé mort à tort (partage réseau) fait échouer l'écriture qui le possède, sans publication partielle. Le launcher jette aussi son `staging`. |
| Panique Rust | n'importe laquelle | rien | Avec `panic = "abort"`, aucun `Drop` ne s'exécute après le crochet. Le **crochet de panique** fait donc lui-même le nettoyage : il supprime les temporaires notés dans un registre global, émet l'événement `error` `internal_error`, puis appelle `process::exit(6)`. Test d'intégration dédié. |
| Mémoire épuisée | image, lecture | `.part` possible | comme Python : pas de refus en amont (D5). Le système tue le processus, le launcher signale l'échec (plantage, donc repli Python pendant la transition) et le nettoyage se fait à la reprise. |
| Chemin long (> 260 caractères) | toutes | — | chemins `\\?\` ou manifeste `longPathAware`, testés |
| Chemins non ASCII (entrée, sortie, profil Windows) | toutes | — | API larges de Windows (`std::fs`, pas de conversion ANSI). Tests avec `é`, idéogrammes et emoji dans les chemins. |
| Noms d'assets non ASCII dans l'archive | lecture, sortie | — | `_fold` NFC + minuscules. Noms de sortie identiques à Python, avec le drapeau UTF-8 (bit 11) posé seulement si le nom n'est pas ASCII, comme `zipfile`. |
| Bombe de décompression (entrée ZIP énorme) | lecture | rien | comme Python : pas de borne propre (D5). Les tailles calculées sont vérifiées (`checked_*`), et un manque de mémoire est traité comme ci-dessus. Les décalages et tailles des en-têtes ZIP, entiers non bornés en Python (négatifs possibles avec un préfixe, ZIP64), sont calculés en `i128` : aucun dépassement n'est possible, le `checked_*` ne sert qu'au passage vers `usize`/`u64`. |
| Décompression deflate : flux que `miniz_oxide` et zlib jugent différemment | lecture | rien | Les vecteurs `deflate_*` (blocs dynamiques, sortie > 64 Kio, flux tronqué, octets après la fin du flux, tables et distances invalides, `file_size` faux) donnent le même verdict, sauf une **divergence assumée** : un bloc dynamique dont la table des longueurs de code est vide (`deflate_code_lengths_empty`, longueurs HCLEN toutes à 0) est refusé tout de suite par `miniz_oxide` (`archive_unreadable`, code 3), alors que zlib attend la suite du flux et que Python, qui ne signale pas un flux tronqué ni ne contrôle `file_size` en fin de lecture, lit un contenu vide si le CRC attendu est celui d'un contenu vide (0). Seuls des flux tronqués en plein en-tête dynamique invalide divergent (Python accepte le préfixe déjà décodé si le CRC attendu est celui de ce préfixe) : aucune archive légitime n'est concernée. |

### 3.1 Méthodes ZIP bzip2, LZMA et Zstandard (#458)

Avant #458, le Rust déléguait ces entrées à Python (D4, code 7). Le retrait de Python (#138)
impose de les lire avec le même résultat que `zipfile` de Python 3.14.8, qui les lit toutes
(le Python embarqué contient `bz2`, `lzma` et `compression.zstd`, libzstd 1.5.7).

**Choix des décodeurs** (licences : chaque terme dans `ALLOWED_LICENSES`) :

| Méthode | Décodeur | Raison |
| --- | --- | --- |
| bzip2 (12) | `src/unbzip2.rs`, écrit d'après le format public | `bzip2-sys` et `libbz2-rs-sys` : licence bzip2-1.0.6, non autorisée. `bzip2-rs` 0.1.2 (MIT/Apache-2.0) échoue (« huffman bitstream truncated ») dès qu'un bloc compressé dépasse la taille de bloc annoncée, ce qui arrive avec des données incompressibles comme un MP3 (vecteur `bzip2_multiblock`). |
| LZMA (14) | `src/unlzma.rs`, d'après la spécification LZMA (`LzmaSpec.cpp`, domaine public) | `lzma-rs` 0.3.0 (MIT) exige le marqueur de fin quand la taille est inconnue (Python lit un flux sans marqueur, vecteur `lzma_no_eos`), ne signale pas la fin du flux en mode incrémental et ne fait pas les contrôles de liblzma. |
| Zstandard (93) | `ruzstd` `=0.9.1` (MIT) pour les blocs, `twox-hash` `=2.1.5` (MIT) pour XXH64, trames et contrôles dans `src/unzstd.rs` | Rust pur ; les contrôles de libzstd absents de `ruzstd` (somme de contrôle, taille annoncée, bit réservé, trame « skippable », fenêtre de `2^27 + 1` au plus) sont faits à côté. |

Licence (décision du mainteneur, revue de #479) : `unbzip2.rs` est écrit d'après le format
public bzip2 et le comportement observable de libbzip2, sans copie de son code ; il n'est
donc pas dérivé de libbzip2 et n'a pas d'entrée dans `DERIVED_NOTICES`.

**Invariants** (`zipread.rs`, `pump_codec`, comme `ZipExtFile.read()`) :

1. Tours de `MAX_N` octets compressés (au moins `MIN_READ_SIZE`) ; chaque tour est décodé **en
   entier**, même au-delà de `file_size` (Python aussi : une erreur du flux y est détectée),
   et seuls les `file_size` premiers octets sont gardés (`data[:self._left]`).
2. Les tours s'arrêtent à la fin du flux (`eof`, les octets suivants sont ignorés), à la fin
   des données compressées ou quand `file_size` octets sont rendus ; le CRC-32 de l'entrée est
   toujours vérifié ensuite.
3. Un flux tronqué n'est pas une erreur du décodeur : il rend ce que la bibliothèque C aurait
   rendu (blocs bzip2 complets, symboles LZMA complets, blocs Zstandard complets et début
   d'un bloc brut), puis le CRC décide. `EOFError` (code 3) si le fichier s'arrête avant les
   données annoncées, comme Python.
4. Mémoire bornée : lecture du tour par morceaux de 1 Mio, sortie par morceaux de 64 Kio,
   blocs bzip2 de 900 000 symboles au plus, fenêtre LZMA qui grandit avec la sortie jusqu'à la
   taille du dictionnaire, fenêtre Zstandard de `2^27 + 1` octets au plus (refus au-delà,
   comme libzstd).
5. Aucune borne ajoutée (D5) : pas de limite de taille décompressée.

**Tableau des pannes** (vecteurs `tests/vectors/archive/{bzip2,lzma,zstd}_*`, produits par
Python 3.14.8, comparés octet à octet ; injection de pannes `injection_de_pannes_bzip2_lzma_zstd`) :

| Panne | Python | Rust |
| --- | --- | --- |
| Archive tronquée (données compressées absentes du fichier) | `EOFError`, code 3 | code 3 |
| Flux tronqué, CRC du contenu complet | contenu partiel, `Bad CRC-32`, code 3 | même contenu partiel, code 3 |
| Flux tronqué, CRC et taille du contenu partiel | lecture réussie | même contenu (`*_truncated_partial`) |
| CRC de l'entrée faux | code 3 | code 3 |
| Flux corrompu (CRC de bloc ou de flux bzip2, somme de contrôle ou taille Zstandard, distance LZMA, état final LZMA non nul, propriétés LZMA invalides, nombre magique, bit réservé, fenêtre trop grande) | `OSError` / `LZMAError` / `ZstdError`, code 6 | `archive_unreadable`, code 3 (Python échoue aussi) |
| Bombe (5 Mo de zéros, `file_size` = 100) | tout le flux décodé, 100 octets gardés | idem, mémoire bornée (`*_bomb`) |
| Bombe dont la fin du flux est corrompue | erreur (tout est décodé) | erreur (`*_bomb_corrupt_tail`) |
| Méthode inconnue (deflate64…) | `NotImplementedError`, code 3 | code 3 (inchangé) |
| Zstandard : fenêtre de plus de `2^27 + 1` octets | refus, sauf raccourci « single-pass » de libzstd : taille annoncée d'au plus 32 Kio (premier tampon de sortie de Python) et trame entière dans le tour | même règle (`zstd_big_window_*`) |
| Zstandard : taille annoncée `0xFFFF_FFFF_FFFF_FFFF` (8 octets) | « inconnue », non contrôlée | idem (`zstd_fcs_unknown`) |
| Zstandard : bloc brut ou compressé plus grand que `Block_Maximum_Size` | refus dès l'en-tête de bloc | idem (`zstd_block_too_big` ; en « single segment », la taille annoncée sans plancher de 1 Kio : `zstd_single_segment_block_too_big`, #478) |
| Bombe démesurée (sortie plus grande que la mémoire) | `MemoryError` (tout le tour est décompressé en mémoire), `internal_error` | lecture en mémoire bornée : le Rust lit l'entrée (sens permis par D5) |

Injection de pannes : `injection_de_pannes_bzip2_lzma_zstd` (archives altérées) et
`decodeurs_sous_mutations` (`zipread.rs`, décodeurs seuls : 500 mutations par flux en CI,
`TELMI2FLAM_FUZZ_ITER=30000` en release en local, soit 210 000 flux, sans panique).

**Écarts connus**, tous sur des flux que les encodeurs n'écrivent pas. Les trois premiers vont
dans le sens « le Rust refuse ce que Python lit » (D5) ; ils sont acceptés par le mainteneur
(revue de #479) parce qu'aucun encodeur n'écrit ces flux :

- bzip2 sans marqueur de fin dont le dernier bloc dépasse le tampon de sortie courant de
  Python (32 Kio au départ) : Python s'arrête dès que l'entrée est épuisée et perd la fin du
  bloc, le Rust rend le bloc entier. Visible seulement si l'en-tête annonce le contenu
  complet avec le CRC du contenu tronqué (vecteur `bzip2_no_eos_big_block`, `RUST_READ_ERROR`).
- Blocs bzip2 « randomised » (bzip2 antérieur à 0.9.5, 1999, avant la méthode ZIP 12) :
  refusés (code 3), Python les lit.
- Table de Huffman bzip2 incomplète menant à un index non rempli : libbzip2 lit une mémoire
  non initialisée (résultat indéfini, le CRC de bloc échoue en pratique), le Rust refuse
  (code 3).
- Dictionnaire LZMA annoncé démesuré (jusqu'à 4 Gio) : liblzma l'alloue d'emblée et peut
  échouer (`MemoryError`), le Rust le fait grandir avec la sortie et lit l'entrée.
- Entrée de plus de `MAX_N` (2 Gio) d'octets compressés en Zstandard : la fenêtre retenue par
  `ruzstd` n'est rendue qu'à la fin de la trame, le test « `file_size` atteint » en fin de tour
  peut donc décoder un tour de plus que Python (seule différence possible : une erreur que
  Python n'aurait pas vue).

## 4. Stratégie de parité

### 4.1 Ce qu'on compare

- **Pas les octets du conteneur `.plain.pk`** : `zipfile.writestr` horodate chaque entrée avec
  l'heure locale, donc deux conversions Python ne donnent déjà pas le même fichier. On compare
  la **liste ordonnée des entrées** : nom, méthode (*stored*), drapeau UTF-8 et contenu octet à
  octet. Le Rust fixe l'horodatage (1980-01-01). Le lecteur du player (`src/formats/pk_reader.c`)
  ignore la date (vérifié dans la PR 10 : extraction complète d'un `.plain.pk` écrit par le Rust).
  À horloge égale, le conteneur est identique à l'octet près : les vecteurs du corpus
  (`gen_package_cases.py`) figent l'horloge de `zipfile` au 1980-01-01, et
  `tests/package_parity.rs` exige le même SHA-256 pour le `.plain.pk` du Rust (écriture ZIP faite
  à la main dans `output.rs`, sans la crate `zip`).
- On compare aussi les résultats : succès ou code d'erreur identique, code de sortie identique,
  **multiensemble des codes d'avertissement** identique (pas le texte), et sortie JSON de
  `--info` identique (`ensure_ascii`, séparateurs compacts).
- Outil : `tools/telmi2flam/parity.py` compare deux `.plain.pk` ou deux dossiers de sortie et
  écrit un rapport JSON. Il passe aussi `validate.py` sur la sortie Rust (celle de l'oracle
  n'y passe pas : elle sert de référence). Avec `--plain`, il compare aussi les fichiers du
  dossier `.plain`, et il exige qu'aucun `.part` ne reste après un échec ni qu'une sortie
  finale subsiste.
- **Écarts admis**, listés une fois pour toutes dans `parity.py` : délégation
  `unsupported_format` (code 7, D4 : comptée à part, voir §4.3), entrée chiffrée (code 3 en
  Rust, 6 en Python), et les cas JSON
  du tableau §3 tranchés comme « divergence assumée ». Deux autres écarts du lecteur ZIP
  suivent le même raisonnement que l'entrée chiffrée : le Rust classe l'archive illisible
  (`archive_unreadable`, code 3) là où Python échoue en `internal_error` (code 6) sur une
  exception non rattrapée. Ce sont le **nom d'entrée UTF-8 invalide avec le bit 11**
  (`UnicodeDecodeError`, règle `invalid_utf8_name`) et la **position d'en-tête locale
  négative** (`OSError`, règle `negative_offset`). Chaque règle est limitée à une étiquette
  de cas du corpus et au couple de codes exact (Python 6 / Rust 3). Dernière règle, d'entrée
  et non de statut : `jpeg_bounded` (D7, §4.4), les `.lif` issus d'un JPEG à ±4 près. Tout
  autre écart fait échouer la comparaison.

### 4.2 Images : le point dur

La sortie LIF dépend du redimensionnement de **Pillow 12.3.0** (version épinglée dans
`tools/telmi2flam/ORACLE-VERSIONS.txt`) :

- `resize(…, LANCZOS)` sur du RGBA suit cet ordre : `convert("RGBa")` (prémultiplication par
  **tables entières**, `MULDIV255`), puis passage horizontal et passage
  vertical de `Resample.c` en **virgule fixe** (coefficients entiers 32 bits,
  `PRECISION_BITS = 32 - 8 - 2 = 22`, arrondi `1 << 21`, écrêtage à 0..255), puis
  `convert("RGBA")` (dé-prémultiplication entière) ;
- `thumbnail()` calcule la taille cible avec ses propres règles d'arrondi (rapport conservé au
  plus près). Son `reducing_gap=2.0` reste sans effet sur du RGBA : `resize` ne le transmet pas
  à l'appel récursif sur l'image `RGBa`, donc `Reduce.c` n'intervient jamais (constaté au port,
  #127) ;
- `_open_rgba` a ses règles : 16 bits → 8 bits par `v/256`, clés `tRNS` en gris 1/2/4/16 bits
  et en RGB 16 bits, palettes.

**Choix retenu : porter ces algorithmes de Pillow en Rust** (`Resample.c`, calcul
de taille de `thumbnail`, conversions `RGBa`), avec les **mêmes constantes** et le même ordre
d'opérations, en code scalaire uniquement. On vise la **parité octet à octet des `.lif`**.

- Licence : Pillow est sous MIT-CMU (HPND, permissive). Un port de code n'est pas une
  dépendance Cargo : `ALLOWED_LICENSES` ne change pas. Il faut un en-tête de licence HPND dans
  le module porté et une mention dans `THIRD-PARTY-NOTICES`. Le mainteneur a validé l'aspect
  juridique du portage (décision D3).
- Option écartée : `fast_image_resize`. Ce crate choisit son code SIMD selon le processeur
  (AVX2, SSE4.1, scalaire), donc ses résultats changent d'une machine à l'autre. Cela casse
  l'invariant 4 (déterminisme) et rend toute tolérance non reproductible entre la CI et un
  poste. Son algorithme diffère aussi de celui de Pillow.

Les images **sans redimensionnement** (taille déjà exacte, `--keep-size` sous 2047 px,
canevas) passent seulement par le décodage PNG, la quantification et l'encodeur LIF. Elles
sont identiques à l'octet près dès la PR 6.

### 4.3 Corpus

1. **Synthétique (CI)** : un générateur Python (`tools/telmi2flam/tests/parity/make_corpus.py`)
   produit des archives qui reprennent les cas de `tests/test_telmi2flam.py` et
   `test_progress.py` : tous les modes PNG, `tRNS`, 16 bits, portraits, noms NFD et cp437, JSON
   limites, assets manquants, archives corrompues, MP3 MPEG1/2/2.5, ID3, trames cassées.
   Ces archives ne contiennent aucun contenu sous licence. Elles sont **générées en CI** et ne
   sont pas commitées. Le job convertit chaque archive avec Python (l'oracle), puis avec Rust ;
   `parity.py` ne doit trouver aucun écart hors de la liste §4.1.
   Le job `parity` de `.github/workflows/telmi2flam-rs.yml` (#134, Python 3.14.8, Pillow 12.3.0,
   lupa 2.8) génère le corpus, compare les sorties (`parity.py corpus`, avec `validate.py` sur
   la sortie Rust) et échoue au premier écart hors liste. Mesure locale (#458) : 92 archives,
   85 identiques, 6 admis, 1 délégué (JPEG), 0 écart ; les entrées bzip2, LZMA et Zstandard
   (`zip_*_entry`, `zip_codecs_all`) ne sont plus étiquetées `delegated`.
2. **Matrice de rééchantillonnage (CI)** : les images des tests sont minuscules et ne prouvent
   rien sur le Lanczos. Le générateur produit donc aussi une matrice :
   - modes PNG × tailles sources impaires ou premières (par ex. 1, 3, 97, 641, 2049) ;
   - toutes les cibles réelles : scène 320×212, vignette 104×136 (mode `bandes` ; `flou` et `rogner` couverts par les vecteurs
     `thumb_flou` et `thumb_rogner` de `gen_resample_vectors.py`, #282), inventaire 128×128,
     titre 89×120, borne 2047 ;
   - agrandissements et réductions de plus de 2× (pour passer par `reduce`) ;
   - contenus variés : bruit pseudo-aléatoire à graine fixe, dégradés, alpha partiel.

   Elle est comparée **`.lif` contre `.lif`, à l'octet près**. Le test ignoré `crates/telmi2flam/tests/matrix_render.rs` écrit les `.lif` du Rust
   (`TELMI2FLAM_MATRIX_DIR`, `TELMI2FLAM_MATRIX_OUT`), puis `parity.py matrix --candidate-lif-dir`
   les compare (1465 cellules, 0 écart en local ; `thumb` en mode `bandes`). Un résultat de 100 % sur cette
   matrice est requis pour fusionner la PR 7.
3. **Campagne (locale)** : les archives réelles de `.cache/telmi/` (ignoré par Git, jamais
   commité). `compat.py --engine rust`, ou un script dédié, convertit chaque archive avec les
   deux moteurs et écrit `.cache/parity/report.json`. **Seuil de bascule** : aucun écart hors
   de la liste §4.1, ni sur les statuts ni sur les entrées (images comprises). Le rapport
   **compte les délégations** `unsupported_format` par format d'image et par méthode ZIP (D4).
   Chaque format présent doit être pris en charge par le Rust, avec la même parité, **avant le
   retrait de Python** (#138). Les machines de
   mesure sont chargées : prévoir des lots et une reprise (PR 15).
4. **Oracle épinglé** : la CI installe aujourd'hui `Pillow>=10.0` sur Python 3.12. Elle doit
   épingler Pillow 12.3.0 et la version de Python de `build-converter.ps1` dans
   `telmi2flam.yml` (PR 3), avec `lupa` à une version exacte (2.8, celle de la CI) : le Lua de
   l'oracle ne doit pas changer sans qu'on le voie. Les tables Unicode de `unicodedata` et d'`unicode-normalization`
   doivent aussi correspondre (versions notées dans la PR 5).

### 4.4 Images non PNG et PNG atypiques (#459)

Décision du mainteneur (24/10) : le Rust lit ce que Pillow lisait, pour retirer Python (#138)
sans perte. Chaque format est livré dans une PR courte et fusionnable seule (PR 1 : PNG
atypiques, PR 2 : JPEG, PR 3 : GIF, puis BMP/WebP si le coût est faible, sinon issue de suivi
motivée). Tant qu'un format n'est pas livré, il reste délégué (code 7).

**Chemin Python reproduit** (`_open_rgba` de `telmi2flam.py`) : `Image.open` puis `load()`
(première image d'une animation), `_trns_mask` (n'agit que sur un PNG : clé `tRNS` brute,
`IHDR` à position fixe), `point(v / 256).convert("L")` pour `I;16`, puis `convert("RGBA")`.
**Pas d'`ImageOps.exif_transpose`** : l'orientation EXIF n'est jamais appliquée (vérifié :
aucun appel dans `telmi2flam.py`), le Rust ne l'applique pas non plus. Ni `draft()` ni
`reduce` : décodage pleine taille.

| Format | Mode à l'ouverture (Pillow 12.3.0) | Conversion vers RGBA | Décodeur Rust | Parité visée |
| --- | --- | --- | --- | --- |
| PNG atypique (PR 1, fait) | modes PNG ; APNG : première image (zone du `fcTL`) | celle des PNG (§4.2, `_trns_mask`) | `png.rs` (fait main) | octet à octet, erreurs comprises |
| JPEG (PR 2, fait) | `L` (1 composante), `RGB` (3 : YCbCr→RGB par libjpeg-turbo, ou RGB brut selon JFIF/Adobe/identifiants), `CMYK` (4, rawmode `CMYK;I` : polarité Adobe toujours supposée ; YCCK converti en CMYK par libjpeg) ; refus (illisible) si précision ≠ 8 bits ou 2/5+ composantes | `L`→gris opaque, `CMYK`→`cmyk2rgb` de Pillow (`255 - k` modulé, entier) | `jpeg.rs` : verdict fait main, pixels par `zune-jpeg` `=0.5.15` (MIT OR Apache-2.0 OR Zlib) en scalaire | **±4 par composante** (D7), alpha exact ; verdicts identiques ou délégués (D4) |
| GIF (PR 3, fait) | première image en `P` (palette locale ou globale ; `L` sans palette), `info["transparency"]` = index du GCE ; image plus petite que l'écran : fond rempli de l'index transparent (sinon 0) | palette → RGBA, index transparent à alpha 0 | `gif.rs` (port de `GifImagePlugin.py` et `GifDecode.c`, code dérivé de Pillow, HPND : notices) ; pas de crate `gif` | octet à octet (pas d'arithmétique flottante) |
| BMP, WebP… | à évaluer : seulement si une histoire réelle en contient (campagne #135) | — | — | issue de suivi sinon |

**Invariants** (en plus de §3) :

1. Une image que Pillow lit donne les mêmes pixels (ou l'écart borné et listé du JPEG) ; une
   image que Pillow refuse donne `image_unreadable` (`empty.lif`), jamais un échec de la
   conversion.
2. Garde anti-bombe de Pillow seulement, pour tous les formats : plus de `2 × MAX_IMAGE_PIXELS`
   pixels annoncés, image illisible (D5). Aucune mémoire engagée sur la seule foi d'un
   en-tête (tampons alloués quand les données arrivent, `try_reserve`) ; exception assumée :
   une zone partielle (cadre APNG, image GIF plus petite que l'écran) engage toute l'image dès
   ses premières données, comme `Image.core.new` de Pillow, dans la limite de la garde.
3. Déterminisme : aucun code SIMD choisi à l'exécution dont le résultat dépend du processeur
   (même règle que `fast_image_resize`, §4.2) ; décodeurs utilisés en scalaire ou avec des
   chemins SIMD prouvés identiques au scalaire.
4. Annulation vérifiée par ligne (ou par bloc de lignes MCU) comme pour le PNG.
5. Aucune panique sur une entrée non fiable (préfixes, octets inversés : tests de troncature
   et d'altération contre l'oracle, comme `alterations.tsv`).

**Tableau des pannes** :

| Panne | Comportement de Pillow (oracle) | Comportement attendu du Rust |
| --- | --- | --- |
| Fichier tronqué (JPEG, GIF) | `OSError: image file is truncated` (`LOAD_TRUNCATED_IMAGES` faux, vérifié : `telmi2flam.py` ne le pose pas) | illisible, sauf si Pillow a déjà toutes les lignes : mêmes cas, testés par troncature (JPEG : 3 bases, chaque longueur) |
| JPEG sans `EOI` | un seul balayage : lu si un marqueur suit les données ; sans marqueur, lu seulement si assez d'octets suivent les données utiles (0 à 7 selon l'image, mesuré) ; plusieurs balayages : illisible (`EOI` exigé) | un seul balayage sans marqueur : données manquantes (image changée selon les bits ajoutés après la fin), illisible ; 8 octets inutiles en fin, lu ; sinon délégué |
| JPEG coupé puis terminé par un marqueur (`EOI`, `DHT`…) | lu, blocs manquants gris uniforme (`insufficient_data`) | lu ; pixels de la partie manquante différents (écart assumé, non borné, vecteurs `fin_*_moitie_*`) |
| Données après `EOI`, octets parasites entre segments, remplissage `FF` | ignorés | ignorés (le flux remis à `zune-jpeg` est réécrit) |
| Données corrompues dans un JPEG (code de Huffman invalide, marqueur `RSTn` manquant) | libjpeg-turbo **avertit et continue** (blocs mis à zéro, resynchronisation) : image lue | lu par `zune-jpeg` avec d'autres pixels (écart **assumé**), ou refusé par `zune-jpeg` : **délégué** (code 7) ; marqueur invalide dans une image à redémarrages (libjpeg resynchronise ou refuse selon sa position) : délégué |
| Bombe (dimensions énormes) | `DecompressionBombError` au-delà de `2 × MAX_IMAGE_PIXELS` ; côté > 65 500 px : `JERR_IMAGE_TOO_BIG` | illisible, avant toute allocation |
| JPEG CMYK / YCCK | `CMYK` inversé (Adobe) puis `cmyk2rgb` ; Adobe 2 ou inconnu sur 4 composantes : YCCK | même formule entière ; vecteurs CMYK, YCCK, Adobe inconnu ; écart mesuré ±1 |
| JPEG RGB sans conversion | Adobe 0 sans JFIF, ou identifiants `R`, `G`, `B` sans JFIF ni Adobe | idem (JFIF prime, comme `default_decompress_parms`) |
| JPEG progressif, restart (`DRI`, `RSTn`), tables optimisées, 4:4:4 / 4:2:2 / 4:2:0 | lus par libjpeg-turbo | lus, à ±4 près |
| Tables de Huffman absentes ou invalides | séquentiel : tables 0 et 1 standard ; progressif : `JERR_NO_HUFF_TABLE` ; index ≥ 4, code « tout à un », symbole DC > 15 : illisible | idem (contrôles de `jpeg.rs`, puis `AVI1` pour les tables standard de `zune-jpeg`) |
| JPEG arithmétique (`SOF9`, `SOF10`), sans perte (`SOF3`, `SOF11`) | lus par libjpeg-turbo 3.1 (arithmétique vérifié sur l'oracle) | **délégués** (code 7, `JPEG arithmétique` / `JPEG sans perte`) : `zune-jpeg` ne les lit pas ; avec le repli Python retiré (#138), c'est une erreur de conversion visible |
| JPEG 12 bits, différentiel (`SOF5`-`7`, `SOF13`-`15`), `DHP`, `EXP`, `JPGn`, `SOF` ou `SOI` répété | `SyntaxError` (précision ≠ 8) ou erreur fatale libjpeg : illisible | illisible |
| Flux de tables seules (`EOI`) suivi d'une image | lu (deux `jpeg_read_header`) | délégué (`JPEG à tables séparées`) ; `EOI` non suivi de `SOI` : illisible (`JERR_NO_SOI`) |
| Balayage entrelacé de plus de 10 blocs par MCU | `JERR_BAD_MCU_SIZE` | illisible |
| EXIF ou MPO malformés | exceptions éventuelles de `getexif` / `_getmp` | non reproduites (écart assumé, aucun cas connu) |
| GIF entrelacé | lu (ordre des passes) | lu |
| GIF animé, image partielle, sans palette globale | première image, fond index transparent ou 0 | idem |
| APNG (PR 1) | première image ; séquence `fcTL`/`fdAT` fausse, `fcTL` court ou hors de l'image : illisible ; zone vide : illisible | idem, testé (`apng_*` de `gen_png_vectors.py`) |
| `IHDR` absent en tête ou répété, `tRNS` répété, après l'IDAT ou de longueur inattendue (PR 1) | dernier `IHDR` (taille ; mode du dernier couple valide), transparence rangée selon le mode courant, `_trns_mask` sur la clé brute et l'`IHDR` à position fixe | idem, y compris les erreurs (`TypeError` d'un triplet sur du gris, palette sur une image d'un autre mode que `L`/`LA`) : 634 combinaisons dans `combos.tsv`, cas `plte_*` du manifeste |

**Écarts attendus** : PNG, dans le sens permissif seulement : texte compressé de 1 Mio exactement
suivi d'un bloc stocké dont le premier octet, dernier de l'entrée, était déjà lu d'avance par
miniz (après un bloc dynamique) : lu par le Rust, `ValueError` pour Pillow (#473). GIF : un
fichier forgé à la fois GIF et IM/IMT ou PCD (polyglotte) est relu par Pillow avec un greffon
sans `_accept` après l'échec de l'ouverture GIF (champ tronqué, aucune image) ; le Rust le
déclare illisible (écart accepté, #473). JPEG : écart d'IDCT/suréchantillonnage borné à ±4
(D7, règle `jpeg_bounded` de `ALLOWED_DEVIATIONS`) ; pixels d'un JPEG corrompu, ou coupé puis
terminé par un marqueur, **non bornés : accepté par le mainteneur** (entrée abîmée ; libjpeg
met les blocs manquants en gris uniforme, `zune-jpeg` non), hors corpus, vecteurs
`fin_*_moitie_*` comparés sur le verdict seulement.

**PR 2 (JPEG, faite, D7)** : `crates/telmi2flam/src/jpeg.rs`, appelé par `png::decode_rgba`
dès la signature `FF D8 FF` (`_accept` de `JpegImagePlugin`).
- Verdict fait main : analyse Python de `JpegImageFile._open` jusqu'au premier `SOS` (octets
  parasites, `FF FF`, `FF 00`, marqueur inconnu : `no marker found` ; segment tronqué ; JFIF ou
  Adobe de moins de 7 octets ; bloc Photoshop tronqué ; profil ICC court avant un `SOF` ; `SOF`
  incomplet, précision ≠ 8, nombre de composantes) ; garde anti-bombe et taille nulle de
  `Image.open` ; refus fatals de libjpeg-turbo (tableau ci-dessus) ; fin du flux (règle des
  marqueurs, mesurée sur l'oracle).
- Pixels : `zune-jpeg` `=0.5.15` en scalaire : fonctions `x86` et `neon` non compilées
  (invariant 3 ; `use_unsafe` faux ne suffit pas, l'AVX2 reste choisi à l'exécution quand
  `x86` est compilée). Le launcher active ces fonctions par la crate `image` : avec le
  resolver 2, elles ne sont absentes que si `telmi2flam` est construit seul (`-p telmi2flam`,
  comme `build-converter.ps1`) ; l'étape « zune-jpeg en scalaire » de `telmi2flam-rs.yml` le
  vérifie (`cargo tree -e features`). Un appel de la bibliothèque depuis le launcher (tests du
  podcast) décode donc les JPEG avec les chemins SIMD : hors parité, sans effet sur le binaire
  livré. Sans limite de
  taille ni de nombre de balayages propre (celles de Pillow s'appliquent avant). `zune-jpeg`
  n'a pas d'option d'IDCT ni de suréchantillonnage : son IDCT entière et son suréchantillonnage
  « fancy » sont gardés tels quels. Il reçoit un flux réécrit (`SOF`, `DQT`, `DHT`, `DRI`,
  `SOS` et données ; `APPn`, `COM`, `DNL` retirés ; paramètres de progression d'un `SOS`
  séquentiel remis à 0/63/0, que libjpeg ignore ; `APP0` `AVI1` en séquentiel pour les tables
  standard) et rend les composantes brutes ; la conversion YCbCr→RGB (JFIF, virgule fixe
  16 bits), YCCK→CMYK et `cmyk2rgb` sont faites dans `jpeg.rs` selon l'espace choisi par
  libjpeg-turbo.
- Mesure (`tests/jpeg_vectors.rs`, 229 vecteurs écrits par Pillow, 196 lus) : écart maximal 4
  en RGB (4:2:0, 4:2:2 et 4:4:4, séquentiel et progressif), 1 en gris et en CMYK, alpha exact ;
  32 vecteurs identiques à l'octet près. Corpus de parité (cas `img_jpeg*`, `pkg_cover_jpeg_*`) :
  après redimensionnement Lanczos et quantification RGB565, chaque `.lif` reste dans un pas de
  quantification (règle `jpeg_bounded` : écart minimal avant quantification compatible ≤ 4,
  alpha exact). Les lobes négatifs du Lanczos peuvent porter un écart source de 4 à 5 ou 6 : la
  règle est alors plus stricte que D7 (faux écart possible en campagne, jamais un écart masqué).
- Verdicts : identiques sur les 233 vecteurs, hors délégations (arithmétique, sans perte, flux
  de tables seules, balayage unique sans `EOI`) ; sur 3 680 troncatures et octets inversés,
  10 écarts, tous des délégations là où libjpeg tranche selon des détails non reproduits
  (`alterations_divergences.tsv`), plus les préfixes qui perdent la signature (délégation
  prudente d'`identify`). Aucun cas lu par Pillow n'est jugé illisible par le Rust : chaque
  incertitude est déléguée (D4), dont tout refus de `zune-jpeg` sur une structure que libjpeg
  accepte (données corrompues, table inutilisée invalide, facteur 3, `DQT` court entre deux
  balayages…).
- Mémoire : l'image RGBA est réservée par `try_reserve` ; le flux réécrit est borné par la
  taille du fichier ; les tampons internes de `zune-jpeg` (coefficients d'une image
  progressive, sortie brute) sont alloués d'après l'en-tête, comme ceux de libjpeg et de
  Pillow, dans la limite de la garde anti-bombe ; un manque de mémoire dans `zune-jpeg` arrête
  le processus (§3, « Mémoire épuisée »). Balayage unique sans `EOI` : jusqu'à trois décodages,
  deux sorties brutes en mémoire à la fois (plus de 500 Mio près de la garde).
- Annulation : avant l'analyse, après le parcours du flux, avant et après chaque décodage de
  `zune-jpeg` (un seul en général, trois pour un balayage unique sans `EOI`), puis à chaque
  ligne de la conversion ; test d'injection à chaque appel du jeton
  (`annulation_avant_et_apres_le_decodage_puis_par_ligne`). `zune-jpeg` ne rend pas la main
  pendant un décodage : pour une image proche de la garde, ou un JPEG progressif fait de très
  nombreux balayages vides (aucune limite, comme libjpeg), l'annulation attend la fin du
  décodage (aussi lent côté Python).
- Parité : `parity.py` admet `jpeg_bounded` seulement sur les cas étiquetés `jpeg_bounded` et
  les entrées `jpeg_entries` du cas (corpus), ou, en campagne, sur les `.lif` dont le nom
  correspond à un JPEG de l'archive (`jpeg_outputs`, approximation du nommage de `alloc`) ;
  statuts, avertissements et autres entrées restent comparés à l'identique.
  `package_parity.rs` applique la même règle aux `.lif` de Python rangés dans
  `package_cases.zip` (`jpeg_lif/`).

**PR 1 (PNG atypiques, faite)** : `png.rs` ne délègue plus que les autres formats (`pas un
PNG`). Détails reproduits de Pillow, tous vérifiés contre l'oracle :
- APNG : `acTL` (court : illisible ; double ou nombre d'images hors bornes : ignoré),
  `fcTL` (court, séquence, cadre hors de l'image : illisible), `fdAT` (court, séquence :
  illisible) avant les données (un `fdAT` peut porter la première image), dans les données
  (suite de l'image) et après (vérifié puis sauté) ; `load_end` d'un APNG animé s'arrête au
  `fcTL` suivant, celui d'un APNG à une image continue (chunks vérifiés) ; zone décodée = cadre
  du dernier `fcTL` lu avant les données, reste au pixel nul du mode ; en entrelacé,
  `ZipDecode.c` ignore le décalage du cadre (zone en haut à gauche) ;
- `IHDR` : taille du dernier lu, mode du dernier couple connu (`KeyError` : mode précédent),
  entrelacement jamais retiré ; un `PLTE` lu sous un mode palette puis un `IHDR` d'un autre
  mode : `Image.load` pose la palette sur le cœur de l'image, accepté pour `L` et `LA` (la
  valeur grise sert d'index, alpha du canal A ; avec transparence : illisible en `L`, masque
  sur la luminance de la palette en `LA`, ou, pour le masque RGB 16 bits, sur sa couleur
  comparée à l'octet haut de la clé, #473), refusé (illisible) pour les autres modes ;
- transparence : valeur de Pillow typée selon le mode courant à chaque `tRNS` (entier, triplet
  ou octets ; le dernier gagne, même après l'image : second passage de conversion), masque de
  `_trns_mask` sur la clé brute de `_png_trns_key` et l'`IHDR` à position fixe de `_png_ihdr`,
  conversions de Pillow quel que soit le mode réellement décodé (`rgb2l`, `I;16` écrêté à 255,
  `convert_transparent` : premier élément d'un triplet sur du gris, entier sur du RGB comparé à
  `(k, 0, 0)`) ; octets sur du gris ou du RGB et triplet sur une palette : illisible ;
- zlib : sortie pleine, zlib s'arrête au premier octet à écrire sans contrôler une distance trop
  lointaine (miniz la refuse au décodage) : échec rejoué dans un tampon jetable à historique
  fictif (constaté sur une longueur d'IDAT altérée, `alterations.tsv`) ;
- textes compressés (`zTXt`, `iTXt`, `iCCP`) de 1 Mio exactement : `ValueError` seulement si
  `unconsumed_tail` n'est pas vide, c'est-à-dire s'il reste de l'entrée après le symbole en
  attente (lu par zlib, distance comprise, mais pas un octet de bloc stocké) ; une entrée qui
  finit sur ce symbole ou au milieu est lue (#473, `ztxt_1mio_*`, `iccp_1mio_*`).

## 5. Plan de bascule

1. **Développement** (PR 1 à 13) : la crate et le binaire existent et sont testés en CI, mais
   ne sont pas livrés.
2. **Parité** (PR 14, 15) : CI synthétique verte, puis campagne conforme au seuil §4.3.
3. **Livraison en double** (PR 16) : l'installateur embarque `telmi2flam.exe` **et** le Python
   actuel. Un réglage caché de `settings.json`, `converter_engine: "python" | "rust"`, vaut
   **`python` par défaut** (jusqu'à #137). Le launcher le relit à chaque tâche (`Toolchain`).
   `convert.rs` gère **les deux moteurs** : fermeture de stdin pour Rust, `CTRL_BREAK_HELPER`
   pour Python (retiré en PR 18). Les tests de `convert.rs` et d'`install/tests.rs` sont
   dédoublés.
4. **Bascule** (PR 17) : défaut `rust`. Pendant une version publiée, le launcher **repasse
   automatiquement sur Python** dans trois cas seulement, et l'indique dans le résultat de
   compatibilité (#20) :
   - l'événement `error` porte `code == "unsupported_format"` (code 7, délégation D4). Ce repli
     est **attendu**, et il est compté à part ;
   - l'événement `error` porte `code == "internal_error"`. Le code de sortie ne suffit pas :
     `output_error` (disque plein, droits) vaut aussi 6, et rejouer en Python échouerait de la
     même façon, en doublant le délai ;
   - le binaire **plante** : code de sortie hors de {0, 2, 3, 4, 5, 6, 7, 130}, ou aucun
     événement `start`. Un code 0 sans événement `done` (ou sans fichier de sortie) est aussi un
     plantage : le binaire annonce un succès qu'il n'a pas produit, donc repli. À l'inverse, une
     erreur de contenu rapportée (`archive_unreadable`, `not_telmi`, `validation`,
     `missing_asset`) n'est jamais un plantage, même sans `start` ou avec un code de sortie hors
     protocole.

   Les erreurs de contenu (codes 3/4/5) et `output_error` ne sont **jamais** rejouées en Python,
   pour ne pas masquer une divergence.

   **État (PR 17, #137, faite)** : le défaut est `rust` (réglage absent, inconnu ou mal typé) ;
   un `settings.json` qui porte `python` le garde. La crate annonce `2.0.0` (D6). Un binaire
   `telmi2flam.exe` **absent** reste `converter_unavailable`, sans repli : le repli ci-dessus ne
   couvre que l'échec d'un binaire présent, et l'installateur le livre toujours.

   **État (PR 18, #138, faite)** : décision du mainteneur du 24/10 : Python est retiré tout de
   suite, **sans version intermédiaire** publiée avec le repli (condition de la PR 18 levée).
   La campagne #135 (30 archives sur 30 identiques, aucune délégation) ayant laissé la liste des
   formats à prendre en charge vide, la condition D4 était satisfaite. Retirés : Python,
   Pillow et les scripts de l'installateur (`build-converter.ps1` ne produit plus que
   `telmi2flam.exe`, `LICENSES.md` et `BUILD.txt`), les contrôles de `launcher.yml` et
   `release-launcher.yml` (qui vérifient aussi l'absence de Python), `CTRL_BREAK_HELPER`,
   `convert_with_fallback`, `EngineTrace::PythonFallback`, `ChosenEngine` et
   `ConverterEngine` (le champ `converter_engine` d'un ancien `settings.json` est ignoré, Rust
   est utilisé). `unsupported_format` (code 7) est une **erreur finale** au message fixe et non
   technique (« Cette histoire utilise un format que le launcher ne sait pas encore convertir
   (image non PNG ou archive compressée en bzip2/LZMA). »), sans nom de fichier de l'histoire
   dans les journaux. Les résultats de compatibilité enregistrés avec `engine: python` ou
   `python_fallback` restent lisibles. Les versions de l'oracle Python (3.14.8, Pillow 12.3.0),
   jusque-là lues dans `build-converter.ps1`, sont dans `tools/telmi2flam/ORACLE-VERSIONS.txt`.
   Restent à faire : le réglage `python_path` et l'écran Paramètres (#139), les outils Python
   de développement (#140).
5. **Suppression** (PR 18 à 20). Condition préalable : tous les formats délégués que la
   campagne (#135) a comptés sont pris en charge par le Rust, puisque sans Python une
   délégation devient un échec.
   - retrait de Python, Pillow et des scripts de l'installateur (`build-converter.ps1`,
     ressource `telmi2flam/`, contrôles de `launcher.yml`, notices), de `CTRL_BREAK_HELPER` et
     du repli ;
   - retrait du réglage `python_path` (backend, écran Paramètres, tests). Un ancien
     `settings.json` qui contient ce champ reste lisible, et le champ est ignoré ;
   - bascule des outils de campagne sur le binaire, puis suppression de `telmi2flam.py`,
     `lif.py` et `mp3map.py`.

**Version du convertisseur** (décision D6) : la bibliothèque et l'outil `compat` enregistrent
la version annoncée (`start.version`, `converter` dans `library.json` et les résultats de
compatibilité). Le Rust annonce `2.0.0` (nouvelle implémentation, décision validée). Même avec
une parité octet à octet, ce changement fait passer tous les résultats de compatibilité
enregistrés avec `1.0.0` au statut « obtenus avec une version antérieure ». C'est voulu : le
programme qui produit les paquets a changé.

**Risque de livraison** : un nouvel `.exe` non signé peut être bloqué ou ralenti par
SmartScreen ou par un antivirus, et un antivirus peut verrouiller le `.part` pendant le
renommage (`ERROR_ACCESS_DENIED`). Il faut une relance courte du renommage, et décider de
la signature du binaire avec celle de l'installateur.

## 6. Découpage en PR

Chaque PR traite un seul thème, avec au plus cinq changements fonctionnels. Le modèle `opus`
est réservé au code de données : formats, analyse d'entrées non fiables, écriture de la
sortie, pipeline d'installation, corrections issues de la campagne.

| # | Issue | PR | Dépend de | Modèle | Zones | Estimation |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | #121 | Squelette de la crate `crates/telmi2flam` (lib + bin vides), workspace racine (D1) avec `[profile.release]` déplacé, CI de la crate, coûts du workspace (§2.1), `.gitattributes` | — | sonnet | telmi2flam, ci | 1 j |
| 2 | #122 | Codecs : `lif` (encode, quantize, decode déplacé depuis le launcher) et `mp3map`, avec de petits vecteurs de test produits par Python et commités | 1 | opus | telmi2flam | 1 j |
| 3 | #123 | Outillage de parité Python : `make_corpus.py` (corpus et matrice de rééchantillonnage), `parity.py` (écarts admis), Pillow et Python épinglés dans la CI | — | sonnet | telmi2flam, ci | 1 j |
| 4 | #124 | Lecture de l'archive : index (`_fold`, cp437, doublons), `find_root`, `check_asset_name`, aucune borne au-delà de Python (D5), bzip2/LZMA délégués (code 7), erreurs `archive_unreadable` / `not_telmi`, `read_info` et sortie `--info` | 1 | opus | telmi2flam | 1 j |
| 5 | #125 | JSON et texte : `load_json` (cas du tableau §3), `_expect`, `_flat` (espaces au sens de Python), `_font_safe`, `_choice_label`, `lua_repr` (flottants façon Python) | 1 | opus | telmi2flam | 1 j |
| 6 | #126 | Décodage d'image avec la sémantique de Pillow : modes PNG, `tRNS`, 16 bits, palettes, garde anti-bombe de Pillow, annulation testée par ligne ; autres formats délégués (`unsupported_format`, code 7) | 2 | opus | telmi2flam | 1,5 j |
| 7 | #127 | Port du rééchantillonnage de Pillow (`Resample.c`, `Reduce.c`, `thumbnail`, `RGBa`) et fonctions `png_to_*_lif`, `_avoid_transpose`, `_bound` ; 100 % sur la matrice | 6 | opus | telmi2flam | 2,5 j |
| 8 | #128 | Graphe vers Lua : stages, actions, conditions, inventaire, `_max_chapters_per_run`, sélecteur, génération de `script/nodes.lua` et `main.lua` | 4, 5 | opus | telmi2flam | 1,5 j |
| 9 | #129 | Assets : allocation des noms, cache d'images, couverture, titre et vignette, `silent.mp3`, `info.plain`, `uuid.bin`, moteur et runtime inclus (manifeste unique), avertissements | 7, 8 | opus | telmi2flam | 1,5 j |
| 10 | #130 | Écriture de la sortie : ordre canonique, ZIP *stored* (horodatage fixe, drapeau UTF-8, MP3 écrits en flux), `--plain`, `.part`, commit, `_clean_orphans`, garde zip-slip ; vérification avec `pk_reader.c` | 9 | opus | telmi2flam | 1,5 j |
| 11 | #131 | Événements et codes : `--progress-json` identique (`start`, `step` espacés de 0,1 s, `warning`, `done`, `error`), lignes `  !` sur stderr, codes de sortie | 10 | opus | telmi2flam | 1 j |
| 12 | #132 | Annulation et panique : jeton d'annulation, `--cancel-on-stdin-eof`, signaux console, crochet de panique qui nettoie le `.part` et sort avec 6 | 11 | opus | telmi2flam | 1 j |
| 13 | #133 | CLI complète : options (`-o`, `--plain`, `--keep-size`, `--allow-missing`, `--selector`, `--info`, `--version`), messages humains sur stderr, code 2 | 12 | sonnet | telmi2flam | 0,5 j |
| 14 | #134 | CI de parité : corpus et matrice, oracle Python contre Rust, `validate.py` sur la sortie Rust | 3, 13 | sonnet | telmi2flam, ci | 0,5 j |
| 15 | #135 | Campagne de parité sur `.cache/telmi/` (`compat.py --engine rust`, rapport, lots avec reprise, comptage des délégations par format) et corrections des écarts ; liste des formats à prendre en charge avant #138 | 14 | opus | telmi2flam | 1 j + mesures |
| 16 | #136 | Launcher : construire et livrer `telmi2flam.exe`, réglage `converter_engine` (défaut `python` jusqu'à #137), `convert.rs` pour les deux moteurs, Job Object sans limite mémoire supplémentaire | 13 | opus | launcher-rust, ci | 1,5 j |
| 17 | #137 | Bascule : défaut `rust`, repli Python sur `unsupported_format`, `internal_error` ou plantage seulement, trace dans le résultat de compatibilité, version selon D6 | 15, 16 | opus | launcher-rust | 1 j |
| 18 | #138 | Retrait de Python de l'installateur et du backend : `build-converter.ps1`, ressources, `launcher.yml`, notices, `CTRL_BREAK_HELPER`, repli (**faite**, sans version intermédiaire) | 15 (formats délégués pris en charge), 17 | opus | launcher-rust, ci | 1 j |
| 19 | #139 | Retrait du réglage `python_path` : backend (ancien champ lu puis ignoré), écran Paramètres, tests | 18 | sonnet | launcher-rust, launcher-ui | 0,5 j |
| 20 | #140 | Outils de campagne sur le binaire Rust (`compat.py`, `explore.py`), suppression de `telmi2flam.py`, `lif.py`, `mp3map.py` et de leurs tests (cas repris par le corpus) | 18 | sonnet | telmi2flam | 1 j |

Ordre conseillé :

1. PR 1 et PR 3 en parallèle (la PR 3 ne dépend de rien).
2. PR 2, 4 et 5 en parallèle.
3. Deux branches en parallèle, puis la PR 9 qui les rejoint :
   - 6 → 7 ;
   - 8.
4. 9 → 10 → 11 → 12 → 13.
5. 14 → 15.
6. PR 16 (possible dès la 13), puis 17, puis 18, puis 19 et 20 en parallèle.

Total : environ 23 jours de développement, hors temps de campagne.

## 7. Décisions

Décisions du mainteneur du **2026-10-16** :

- **D1, validée** : workspace Cargo à la racine (§2.1).
- **D2, validée** : convertisseur lancé en processus séparé (§2.2).
- **D3, validée** : port du code de Pillow sous licence HPND, avec en-tête de licence dans le
  module porté et mention dans `THIRD-PARTY-NOTICES` (§4.2).
- **D4 et D5, tranchées** : **le Rust ne refuse aucune archive que Python accepte.**
  - Bornes alignées sur celles que Python applique réellement : protection anti-bombe de
    Pillow (`2 × MAX_IMAGE_PIXELS`, image illisible et non archive refusée) et rien de plus
    que ce que fait déjà `telmi2flam.py`. Pas de borne de taille décompressée, pas de limite
    mémoire du Job Object plus stricte que l'existant.
  - Formats d'image autres que PNG et entrées ZIP bzip2/LZMA : tant que le repli Python
    existait, le Rust **déléguait à Python** (`unsupported_format`, code 7, distinct de
    `internal_error` ; invariant 6). Le repli est supprimé (#138) : c'est désormais une erreur
    finale montrée à l'utilisateur. **Entrées bzip2, LZMA et Zstandard : support fait
    (#458, §3.1)**, plus de délégation ; `parity.py campaign` compte une délégation de ces
    méthodes comme un écart.
  - La campagne de parité (#135) compte ces délégations par format. La prise en charge de
    **chaque format présent** est une condition préalable du retrait de Python (#138).
- **D6, validée** : le Rust annonce `2.0.0`. Les résultats de compatibilité enregistrés avec
  `1.0.0` passent au statut « obtenus avec une version antérieure ».

Décision du mainteneur du **2026-10-24** :

- **D7, tranchée** : JPEG décodé par `zune-jpeg` (option (a) du point d'étape de #459), avec un
  **écart borné à ±4 par composante** par rapport à Pillow/libjpeg-turbo, inscrit dans
  `ALLOWED_DEVIATIONS` (règle `jpeg_bounded`, §4.4). Pas de décodeur maison reproduisant
  libjpeg-turbo (option (b), licence IJG écartée).

Points encore ouverts :

- ~~**Durée de la double livraison**~~ : tranchée le 24/10 par le mainteneur, aucune version
  intermédiaire avec le repli Python (PR 18 faite, #138).
- **Signature** du binaire `telmi2flam.exe` (SmartScreen, antivirus).
