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
| Autres formats d'image (JPEG…) | `zune-jpeg` ou `jpeg-decoder` si nécessaire | MIT OR Apache-2.0 (OR Zlib) | D4 : délégation à Python (`unsupported_format`, code 7) pendant la transition. Le support devient obligatoire avant #138 pour chaque format que la campagne trouve. |
| Méthodes ZIP bzip2/LZMA | fonctions `zip` (`lzma-rs` MIT ; `bzip2-sys` : licence bzip2, **non autorisée**, un décodeur bzip2 en Rust pur serait à trouver) | — | Même règle (D4) : délégation, puis support obligatoire avant #138 si la campagne en trouve. |

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
6. **Délégation des cas non gérés** (décision D4) : face à une image dans un autre format que PNG
   ou à une entrée ZIP en bzip2/LZMA, le Rust ne convertit pas. Il nettoie son `.part`, émet
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
| Entrée bzip2/LZMA (lue par Python, pas par le Rust) | lecture | `.part` supprimé | `unsupported_format`, code 7 : délégation à Python (invariant 6) |
| Image dans un autre format que PNG (JPEG, GIF…) | image | `.part` supprimé | `unsupported_format`, code 7 : délégation à Python (invariant 6), et non `image_unreadable` : Pillow la lirait |
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
| Décompression deflate : flux que `miniz_oxide` et zlib jugent différemment | lecture | rien | Les vecteurs `deflate_*` (blocs dynamiques, sortie > 64 Kio, flux tronqué, octets après la fin du flux, tables et distances invalides, `file_size` faux) donnent le même verdict, sauf une **divergence assumée** : une table de Huffman dynamique incomplète (`deflate_bad_tables2`) est refusée tout de suite par `miniz_oxide` (`archive_unreadable`, code 3), alors que zlib attend la suite du flux et que Python, qui ne signale pas un flux tronqué ni ne contrôle `file_size` en fin de lecture, lit un contenu vide si le CRC attendu est celui d'un contenu vide (0). Seuls des flux tronqués en plein en-tête dynamique invalide divergent (Python accepte le préfixe déjà décodé si le CRC attendu est celui de ce préfixe) : aucune archive légitime n'est concernée. |

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
  de cas du corpus et au couple de codes exact (Python 6 / Rust 3). Tout autre écart fait
  échouer la comparaison.

### 4.2 Images : le point dur

La sortie LIF dépend du redimensionnement de **Pillow 12.3.0** (version épinglée dans
`build-converter.ps1`) :

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
   la sortie Rust) et échoue au premier écart hors liste. Mesure locale : 88 archives, 79
   identiques, 6 admis, 3 délégués, 0 écart.
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

## 5. Plan de bascule

1. **Développement** (PR 1 à 13) : la crate et le binaire existent et sont testés en CI, mais
   ne sont pas livrés.
2. **Parité** (PR 14, 15) : CI synthétique verte, puis campagne conforme au seuil §4.3.
3. **Livraison en double** (PR 16) : l'installateur embarque `telmi2flam.exe` **et** le Python
   actuel. Un réglage caché de `settings.json`, `converter_engine: "python" | "rust"`, vaut
   **`python` par défaut**. Le launcher le relit à chaque tâche (`Toolchain`). `convert.rs`
   gère **les deux moteurs** : fermeture de stdin pour Rust, `CTRL_BREAK_HELPER` pour Python
   (conservé jusqu'à la PR 18). Les tests de `convert.rs` et d'`install/tests.rs` sont dédoublés.
4. **Bascule** (PR 17) : défaut `rust`. Pendant une version publiée, le launcher **repasse
   automatiquement sur Python** dans trois cas seulement, et l'indique dans le résultat de
   compatibilité (#20) :
   - l'événement `error` porte `code == "unsupported_format"` (code 7, délégation D4). Ce repli
     est **attendu**, et il est compté à part ;
   - l'événement `error` porte `code == "internal_error"`. Le code de sortie ne suffit pas :
     `output_error` (disque plein, droits) vaut aussi 6, et rejouer en Python échouerait de la
     même façon, en doublant le délai ;
   - le binaire **plante** : code de sortie hors de {0, 2, 3, 4, 5, 6, 7, 130}, ou aucun
     événement `start`.

   Les erreurs de contenu (codes 3/4/5) et `output_error` ne sont **jamais** rejouées en Python,
   pour ne pas masquer une divergence.
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
| 16 | #136 | Launcher : construire et livrer `telmi2flam.exe`, réglage `converter_engine` (défaut `python`), `convert.rs` pour les deux moteurs, Job Object sans limite mémoire supplémentaire | 13 | opus | launcher-rust, ci | 1,5 j |
| 17 | #137 | Bascule : défaut `rust`, repli Python sur `unsupported_format`, `internal_error` ou plantage seulement, trace dans le résultat de compatibilité, version selon D6 | 15, 16 | opus | launcher-rust | 1 j |
| 18 | #138 | Retrait de Python de l'installateur et du backend : `build-converter.ps1`, ressources, `launcher.yml`, notices, `CTRL_BREAK_HELPER`, repli | 15 (formats délégués pris en charge), 17, puis une version publiée | opus | launcher-rust, ci | 1 j |
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
    existe, le Rust **délègue à Python** (`unsupported_format`, code 7, distinct de
    `internal_error` ; invariant 6).
  - La campagne de parité (#135) compte ces délégations par format. La prise en charge de
    **chaque format présent** est une condition préalable du retrait de Python (#138).
- **D6, validée** : le Rust annonce `2.0.0`. Les résultats de compatibilité enregistrés avec
  `1.0.0` passent au statut « obtenus avec une version antérieure ».

Points encore ouverts :

- **Durée de la double livraison** : proposition, une version publiée avec le repli Python
  avant la PR 18.
- **Signature** du binaire `telmi2flam.exe` (SmartScreen, antivirus).
