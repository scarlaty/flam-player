# telmi2flam — Convertisseur d'histoires TELMI → FLAM

Convertit une histoire interactive au format **TELMI** (archive `.zip` déclarative)
en une histoire au format **FLAM** (`.plain.pk`)
jouable dans le `flam-player` de ce dépôt.

> 📄 **Spécification du format source** : [`TELMI_FORMAT.md`](TELMI_FORMAT.md)
> (structure des fichiers, `metadata.json`, `nodes.json`, inventaire, conditions…).
> À noter : TELMI **n'a pas de notion de chapitre** ; le convertisseur traite chaque
> **scène avec audio jouée comme scène** comme un chapitre (cf. `engine/story.lua`).
> Une option de choix (carrousel) n'est jamais jouée comme scène : elle ne compte pas
> (sauf option autoplay ou action à conditions).
> **Jauge « Reprendre »** = `chapitres visités / totalChapters`, remise à zéro par la fin
> d'histoire. `totalChapters` = nombre **maximal** de chapitres visitables en **une seule
> partie** (chemin le plus lourd du graphe des scènes, cf. §5) : la jauge n'atteint 100 %
> que sur la branche la plus longue. Avec des branches exclusives (menu de niveaux, fins
> alternatives), les autres plafonnent en dessous. Exemple *Maxicours Anglais* : trois
> niveaux de 31, 36 et 33 chapitres, donc `totalChapters = 36` et une jauge maximale de
> 86 %, 100 % et 91 % (avant : `totalChapters = 98`, somme des trois niveaux, et une
> jauge plafonnant à 31, 36 et 33 %).

> **État** : codec LIF ✅ · mp3map ✅ · moteur Lua ✅ · convertisseur ✅ · 1ʳᵉ lecture device ✅
> · relance device ✅ depuis la correction du Bug C (écran noir à la 2ᵉ ouverture, causé par
> une sortie `goto_library` depuis une scène active, cf. `DEVICE_VS_SIM.md` §10).

---

## 1. Pourquoi c'est non trivial

Les deux formats ne sont **pas** de même nature :

| | TELMI | FLAM (`.plain.pk`) |
|---|---|---|
| Conteneur | `.zip` (compressé) | `.zip` **stored** (sans compression), extension `.plain.pk` |
| Logique | **déclarative** : `nodes.json` (graphe de scènes) | **programme Lua** : `main.lua` + `script/*.lua` |
| Images | PNG 640×480 | `.lif` (Lunii Image Format, QOI-like RGB565) |
| Audio | MP3 44100 Hz | MP3 + table de seek `.mp3map` (cadence interne 88200 Hz) |
| Métadonnées | `metadata.json` | `info.plain`, `version`, `uuid.bin` |

TELMI décrit **des données** ; FLAM exécute **du code**. La conversion consiste donc à :

1. Fournir un **moteur Lua générique** (écrit une fois) qui rejoue un graphe de scènes
   style-TELMI en n'utilisant que les primitives du firmware FLAM.
2. **Transcoder les assets** : PNG→LIF (scènes ajustées à la zone 320×212 sous le bandeau),
   MP3→MP3+mp3map.
3. Émettre les **données** (`nodes.lua`) + les **métadonnées** + **rezip stored**.

---

## 2. Le format FLAM (rétro-ingénierie depuis `flam-player`)

Archive `.plain.pk` = ZIP **stored** (méthode 0, aucune compression). Entrées clés :

```
info.plain          # texte UTF-8 ; 1re ligne = titre de l'histoire (lue par le player)
version             # "1" (valeur écrite par le convertisseur)
uuid.bin            # 16 octets : UUID de l'histoire
main.lua            # programme principal (le firmware appelle setup() après load)
script/*.lua        # modules require()  (optionnels selon l'histoire)
img/thumbnail.lif   # vignette de la bibliothèque
img/*.lif           # images des scènes
sounds/*.mp3        # audio des scènes (base path = sounds/)
sounds/*.mp3map     # table de seek associée (optionnelle)
```

### API runtime exposée au Lua (firmware)

Le firmware fournit **uniquement** ces globales (cf. `src/firmware/`, `src/bindings/`,
`src/platform/sdl_audio.c`) — il **ne fournit pas** la bibliothèque UI Lunii
(`Global`, `button`, etc.) : celle-ci est embarquée dans chaque histoire Lunii.
Notre moteur n'en dépend donc pas.

- `window` — conteneur LVGL de contenu (320×212 dans le sim : écran 320×240 − barre 28px)
- `document` — groupe de focus de l'encodeur
- `lv.*` — bindings LVGL : `obj`, `btn`, `label`, `img`, `img_src`, `style`, `group`,
  `timer`, `anim`, `event`, `color`, constantes `KEY_*`, `EVENT_*`, `ALIGN_*`, `OPA_*`…
- `audio.load(track, path, cb)` / `play()` / `stop()` / `pause()` / `seek(s)` /
  `duration()` / `get_status()`. Le callback `cb(state, seconds)` reçoit
  `"play"` (chaque seconde) et `"stop"` (fin de fichier → sert à l'autoplay).
- `state` — table persistée automatiquement (sauvegarde/chargement par le firmware)
- `progression.save(key, t)` / `load(key)`
- `context_menu.set_entries(t)`, `screen.*` (stubs), `progress`
- `goto_library()` — quitte l'histoire ; `back_callback` — appelé sur ESC (retour)

Mapping touches du simulateur (`src/platform/sdl_driver.c`) :
`←/→` = `LV_KEY_LEFT/RIGHT` (molette), `Entrée/Espace` = `LV_KEY_ENTER` (OK),
`Échap` = retour (`back_callback`), `M` = menu contextuel, `S` = screenshot,
`P` = pause / reprise de l'audio.

**Polices** : les polices du player (`src/fonts/*.c`, Nunito générée avec
`lv_font_conv -r 0x20-0x7F,0xA0-0xFF`) ne couvrent que l'ASCII imprimable et Latin-1
(U+0020–U+007E, U+00A0–U+00FF). Tout autre caractère s'affiche en rectangle vide. Le
titre de la bibliothèque (1re ligne d'`info.plain`) est rendu en `nunito_extrabold_16`,
le bandeau en `nunito_bold_12`, et les modules Lua utilisent les mêmes familles.

### Format `.lif`

Cf. `src/formats/lif_decoder.c` (porté de [Seph29/liff-viewer]). QOI-like :

```
[4]  magic "liff"
[4]  width  (uint32 big-endian)
[4]  height (uint32 big-endian)
[1]  channels = 0xA2
[N]  payload (opcodes)
[8]  end marker = 00 00 00 00 00 00 00 01
```

Espace couleur RGB565 (r5/g6/b5) + alpha 8 bits. Opcodes : `0xFE` couleur RGB565,
`0xFF` couleur+alpha, tag `0x00` index cache (64 entrées, hash `(r*7+g*5+b*3)&0x3F`),
tag `0x40` petit delta, tag `0x80` delta vert étendu, tag `0xC0` run-length.
⚠️ Un run `0xC0|n` ne peut valoir ni `0xFE` ni `0xFF` → **n ≤ 61** (run max 62 px).

### Format `.mp3map`

Cf. `src/formats/mp3map_parser.c` et [scarlaty/mp3map-tool] :

```
header 12 o (little-endian) : total_units(u32), id3_offset(u32), reserved=0(u32)
N enregistrements de 8 o    : byte_offset(u32, absolu), unit_pos(u32)
```

Cadence interne 88200 = 2×44100. `unit_pos` = position cumulée du frame, chaque frame
valant `samples × 88200 / samplerate` unités (= 2304 à 44,1 kHz MPEG1 Layer III, donc
`unit_pos = frame_index × 2304`).
**1 enregistrement par seconde** : premier frame dont `unit_pos ≥ k × 88200`.
`total_units = (N_frames − X/2) × unités_par_frame` avec X≈34 (>100 frames) ou 22
(correction délai décodeur). `duration_s = total_units / 88200`.

[Seph29/liff-viewer]: https://github.com/Seph29/liff-viewer
[scarlaty/mp3map-tool]: https://github.com/scarlaty/mp3map-tool

---

## 3. Le format TELMI

Archive `.zip`. Documentation : <https://wiki.telmi.fr/developments/documentation_du_format_des_histoires/>

```
metadata.json   # title, uuid, image, version, category, description, age
nodes.json      # startAction, inventory[], stages{}, actions{}
notes.json      # (optionnel, éditeur Sync Studio)
title.mp3 / title.png / cover.png
audios/sN.mp3   # narration des scènes
images/sN.png   # visuels 640×480 ; icônes inventaire 128×128
```

### `nodes.json`

- `startAction` : `{ action, index }` — point d'entrée.
- `inventory[]` : `{ name, initialNumber, maxNumber, display, image }`
  (`display` : 0 image+compteur, 1 image+jauge, 2 caché).
- `stages{}` : chaque scène `{ image, audio, ok:{action,index}, home:{action,index},
  control:{ok,home,autoplay}, items[], inventoryReset }`.
- `actions{}` : `aN = [ { stage, conditions[] }, … ]` — **liste de stages candidats**.

### Sémantique de sélection d'une action (le cœur du modèle)

Une **action** est une liste de stages candidats. À la résolution de `{action, index}` :

| Cas | Comportement |
|---|---|
| 1 seul stage visible | lien déterministe : va directement à ce stage |
| conditions présentes | seuls les stages dont **toutes** les `conditions` passent restent candidats |
| N stages visibles, cible `autoplay` | pas de molette (spec TELMI : `←/→` désactivés) : la cible est jouée directement |
| N stages visibles, cible non `autoplay` | **choix molette** : `←/→` change l'option focalisée (image+audio), OK valide |

`index` = stage ciblé / option initialement sélectionnée (0-based, dans la liste non
filtrée ; hors borne ou masqué → 1re option visible). **`index = -1` = tirage aléatoire**
(`math.random`) parmi les stages **visibles** : branche aléatoire si la cible tirée est
`autoplay`, sinon simple présélection aléatoire du choix molette. Vérifié sur les données :
une action à **1 option** est toujours ciblée avec `index 0` (lien) ; une action
**multi-options** est ciblée avec `index = -1` (24×) ou `index = 0` (12×). `indexItem` :
l'index est la valeur d'un item d'inventaire (stage joué directement).

`items` : opérations sur l'inventaire (`type` : 0 `+=`, 1 `-=`, 2 `=`, 3 `*=`, 4 `/=`, 5 `%=`).
`conditions` : `comparator` 0 `<`, 1 `<=`, 2 `==`, 3 `>`, 4 `>=`, 5 `!=`.

`control` (émis dans `nodes.lua` sous `ctrl`, défauts du convertisseur si absent :
`ok=true`, `home=false`, `autoplay=false`), respecté par `engine/story.lua` :

| Scène | `autoplay=true` | `autoplay=false` |
|---|---|---|
| avec audio | transition `ok` à la fin de l'audio | reste sur l'image à la fin de l'audio, attend OK |
| image sans audio | transition `ok` immédiate (nœud de passage) | image affichée, attend OK |
| ni image ni audio | nœud de passage | nœud de passage (rien à afficher) |

- `ctrl.ok` : OK (ENTER) exécute `ok`, **pendant** l'audio (skip) comme après ; `false` →
  OK ignoré.
- `ctrl.home` + `home` défini : le bouton **retour** exécute la transition `home` (dans un
  choix : `home` de l'option focalisée). Sinon (ou si `home` ramène sur le nœud courant),
  retour → menu Start de l'histoire, comme avant.
- `wheel` / `pause` : absents du format TELMI (la molette découle de `autoplay`, la pause
  reste gérée par le firmware).
- `nodes.lua` antérieur sans `ctrl` : comportement historique (`autoplay`, OK = skip, pas
  de home).

---

## 4. Composants

```
tools/telmi2flam/
├── README.md            # ce fichier
├── lif.py               # encodeur PNG/RGBA → .lif (+ décodeur de référence pour self-test)
├── mp3map.py            # générateur de table de seek .mp3map
├── engine/
│   ├── main.lua         # bootstrap (setup → title-card → menu Démarrer/Reprendre)
│   └── story.lua        # BRANCH (logique de scènes ; rejoue nodes.lua via les modules Global)
├── runtime/
│   ├── script/          # bibliothèque standard Lunii (global.lua + deps + modules)
│   └── img/script/      # assets UI Lunii (flèches, play/pause, empty…)
├── telmi2flam.py        # CLI de conversion
├── validate.py          # validation hors-GUI d'un .plain.pk généré
├── tests/               # tests unittest (paquets TELMI synthétiques)
└── DEVICE_VS_SIM.md     # divergences device ↔ simulateur (dont le bug relance #10)
```

### `lif.py`

- `encode(rgba, w, h) -> bytes` : encode une image RGBA8888 row-major en `.lif`.
- `decode(data) -> (rgba, w, h)` : décodeur **miroir exact** du C (pour vérification).
- `quantize_rgba(rgba, w, h)` : applique la quantification RGB565 (vérité terrain).

Encodeur QOI-like : RLE + cache 64 + petit delta + delta vert étendu + couleur pleine.
**Validé** par round-trip `decode(encode(x)) == quantize(x)` sur image réelle (ratio ≈ 0,29),
dégradé+alpha, aplat, blanc, transparent.

### `mp3map.py`

- `build(data, info=None) -> (bytes, duration_s, num_records)` : parse les frames MP3
  (MPEG 1/2/2.5, Layers I/II/III, saut du tag ID3v2) et produit le `.mp3map`.
  `info` (dict optionnel) reçoit `samplerates` et `frames`. Lève `ValueError` si le
  fichier n'est pas du MPEG audio (aucun frame, ou frames couvrant < 50 % du fichier).
- `build_file(path)` : variante fichier.

Un sync n'est accepté en resynchronisation que si les 2 frames suivants sont aussi
valides (évite les faux headers dans des données quelconques). Durée et positions sont
cumulées frame par frame à la fréquence réelle : un MP3 48 kHz ou 32 kHz a une table
juste (avant : +8,8 % / −27 %). Le convertisseur **avertit** hors 44,1 kHz (firmware
calibré 44,1 kHz ; rééchantillonner reste conseillé).

**Validé** : sortie **identique octet-pour-octet** au vrai `.mp3map` de l'histoire
officielle *Cluedo* (`mine == real`) ; en 44,1 kHz la sortie est inchangée (test
`test_44k_unchanged`).

### `engine/main.lua` (bootstrap) + `engine/story.lua` (branch)

Architecture **identique aux histoires Lunii officielles** (Cluedo) : tout passe par les
modules `Global` (le device rend/gère les entrées via `Global.current_module` ; un moteur
en `lv.*` direct marche au sim mais PAS sur device).

`main.lua` (bootstrap) :
- `setup()` : `N = require("nodes")`, `Global = require("global")`, `Global.init()`,
  enveloppe `getProgressionValue` (bornée à 100, plancher à 1 dès que l'histoire est
  commencée), `Global.progression.create{ totalChapters, backBehavior = Start }`,
  `Global.setDefaultAudioPlayerCover("empty.lif", …)`, `IntroCard()`, puis menu
  contextuel `M` → « Reprendre l'histoire » (`LateralResume`).
- `IntroCard()` : `title-card.display{ title, subtitle, audio, img, cb = Start }` si le
  paquet a une couverture (sinon `Start()` directement). La title-card avance via
  `back_callback()`, que `progression.create` a réglé sur `Start`. Sans `title.mp3`, elle
  joue `silent.mp3`.
- `Start()` : menu `list-choice` avec **Démarrer l'histoire** ou, si `isStoryStarted()`,
  **Reprendre l'histoire** (jauge `getProgressionValue()`), plus **Inventaire** si un
  item est visible (`display ≠ 2`).
- `LoadStartFunction()` : `Global.loadBranch("story")` puis `current_branch["__start"]()`.
- `LoadCurrentFunction()` : `Global.loadBranch(state.currentBranchName or "story")` puis
  `story.resume()` (repli : `current_branch[state.current_fun]`, sinon `__start`).

`script/story.lua` (branch, requis par `loadBranch`) :
- `story.clear()`, `story.resume()` + métatable : `story[<nom>]()` → `play(nom)`
  **uniquement** pour `__start` et les ids du graphe (toute autre clé reste `nil`).
- `play(name)` : `__start` → inventaire neuf + `followTransition(start)` ; stage →
  `enterStage(id, true)` ; action → `enterAction(id, nil, true)`.
- `enterStage(id, isResume)` : ops d'inventaire (sauf reprise), puis scène avec audio →
  `setProgression{ currentFunction=id, branch="story", ischapter=true, chapterData }` ;
  nœud de passage → `setProgression{ currentFunction=id, branch }` et `followTransition(ok)`
  (garde anti-boucle) ; sinon `audio-player.create{ audio_path, image_background_path,
  callback (autoplay), okCallback (ctrl.ok) }` et, si `ctrl.home`, `setBackBehavior(home)`.
- `followTransition(tr)` : `indexItem` → stage désigné par l'inventaire ; sinon
  `enterAction(action, index)` : filtre par conditions, 0 entrée → fin, 1 → scène,
  `index = -1` → tirage, cible `autoplay` → scène directe, sinon `showChoice`.
- `showChoice(list, actionId, sel, selIndex)` : `setProgression{ currentFunction=actionId,
  branch }` (pas un chapitre), puis module `carousel` (défaut) ou `image-choice`
  (`--selector image`) avec `choices = {{img, audio, label, cb}}` et l'option `sel`
  présélectionnée (`answerIterator`).
- `endStory()` : `resetProgress()`, efface `current_fun`, `currentBranchName`,
  `visited_funs`, `inv`, `current_kind`, `choice_index`, puis `back_callback()` → menu Start
  (jamais `goto_library` depuis une scène active : Bug C, `DEVICE_VS_SIM.md` §10).
- Sauvegarde : champs standard `state.current_fun` + `state.currentBranchName` (écrits par
  `setProgression`, comme Cluedo), plus `state.current_kind` (`"s"` scène / `"a"` action :
  les ids TELMI sont libres, une scène et une action peuvent partager un id) et
  `state.choice_index` (index TELMI de l'option présélectionnée d'un choix).
- Inventaire : `state.inv`, ops complètes (`number`, `assignItem`, `playingTime`), conditions complètes (`num`, `itemB`), `indexItem` dans les transitions, `inventoryReset`.

Données attendues (`nodes.lua`, généré par le convertisseur) :

```lua
return {
  meta      = { title = "…", subtitle = "…" },                   -- pour la title-card
  totalChapters = 36,                                           -- cf. §5
  selector  = "carousel",                                       -- ou "image" (--selector)
  start     = { action = "a0", index = 0 },
  inventory = { { name=, init=, max=, display=, image= }, … },   -- ou nil
  title     = { image = "title.lif", audio = "title.mp3" },      -- ou nil
  stages = {
    s5 = { image=nil, audio="s5.mp3",
           ok={action="a4",index=0}, home={action="backAction",index=0},
           ctrl={ok=true,home=true,autoplay=true}, items={…}, reset=true,
           text="Le chateau" },                                  -- label (notes.json)
    …
  },
  actions = {
    a5 = { {stage="s7"}, {stage="s8"}, {stage="s9"} },
    a1 = { {stage="s2", cond={ {cmp=2,item=0,num=2} }}, … },
    …
  },
}
```

---

## 5. Utilisation

```bash
python telmi2flam.py <histoire-telmi.zip> [-o sortie] [--plain] [--keep-size]
                     [--allow-missing] [--selector carousel|image]
```

- `-o` : chemin de sortie (défaut : `<Titre>.<UUID8>.plain.pk` dans le dossier courant).
  - se termine par `.plain.pk` → archive ZIP **stored** (format device) ;
  - se termine par `.plain` → écrit **uniquement** le dossier extrait (pas de `.pk`).
- `--plain` : écrit **aussi** le dossier `.plain` extrait à côté du `.plain.pk`
  (pratique pour le simulateur, qui charge un dossier `.plain` directement).
- `--keep-size` : conserve la résolution native des images (sinon ajustement à 320×212),
  bornée à 2047 px (limite du décodeur LIF). L'image n'est pas réduite à l'affichage :
  ce qui dépasse la zone 320×212 n'est pas visible.
- `--allow-missing` : convertit même si des images/audios référencés sont absents
  (sinon **erreur**) ; un audio manquant est remplacé par `silent.mp3` (la scène garde
  sa transition de fin d'audio), une image manquante est omise.
- `--selector` : affichage des choix multiples (`carousel` par défaut, ou `image`).

En cas de paquet invalide (zip illisible, JSON invalide/BOM mal formé, `NaN`, type
inattendu, `metadata.json`/`nodes.json` absents, nom d'asset dangereux...), le CLI
affiche `ERREUR : ...` et sort avec le code 1 (pas de traceback).

Dépendance : `pip install Pillow`.

Le CLI :
1. lit `metadata.json` + `nodes.json` dans le zip
   (racine = dossier le moins profond contenant `metadata.json` **et** `nodes.json`,
   hors `__MACOSX/` et `._*` ; erreur si plusieurs histoires au même niveau) ;
2. transcode images (PNG→LIF) et audio (MP3 copié + `.mp3map` généré), avec cache anti-doublon
   (recherche des fichiers insensible à la casse et à la normalisation Unicode NFC/NFD
   des zips macOS ; image illisible → message avec son nom
   et repli sur `empty.lif`) ;
3. génère `script/nodes.lua` (table de données, dont `totalChapters`, voir plus bas) +
   embarque `engine/main.lua` (→ `main.lua`), `engine/story.lua` (→ `script/story.lua`)
   et le runtime Lunii (`runtime/` → `script/`, `img/script/`) ;
4. écrit `info.plain` (titre/sous-titre aplatis sur une ligne) / `version` / `uuid.bin`
   (= MD5 de l'UUID TELMI ; sans uuid, MD5 du contenu de `nodes.json` + avertissement) /
   `img/thumbnail.lif` ;
5. rezip en **ZIP stored**.

**Texte affiché** (titre et sous-titre de `metadata.json`, labels des choix, noms
d'inventaire) : les polices ne couvrant qu'ASCII + Latin-1 (§2), les autres caractères
sont remplacés à la conversion. `’ ‘` → `'`, `“ ” „` → `"`, `– —` → `-`, `…` → `...`,
`œ Œ` → `oe OE`, `€` → `EUR`. Les espaces Unicode (dont l'espace insécable fine U+202F)
deviennent des espaces simples. Les autres caractères sont approchés par décomposition
(`ő` → `o`, `ﬁ` → `fi`), sinon supprimés (emoji, symboles) ou remplacés par `?` (lettre
sans équivalent). Les accents Latin-1 (`é à ç « »`…) sont gardés. Un avertissement liste
les caractères remplacés (`U+2019 RIGHT SINGLE QUOTATION MARK -> "'"`…).

**`totalChapters`** : graphe des scènes (scène → scènes de l'action de son `ok`, et de
son `home` si `ctrl.home`), conditions ignorées. Chaque composante fortement connexe
atteignable depuis `startAction` pèse son nombre de chapitres, et `totalChapters` est le
poids du chemin le plus lourd du graphe condensé, soit le nombre maximal de chapitres
qu'une même partie peut visiter avant la fin d'histoire. C'est une borne haute (les
conditions peuvent fermer des chemins) : la jauge reste dans [0, 100] et atteint 100 %
sur la branche la plus longue. Une scène non atteignable ne compte pas.
Pourquoi pas une jauge par branche : `totalChapters` est une valeur unique passée à
`progression.create` (bibliothèque Lunii embarquée). Une jauge par composante demanderait
de modifier le moteur et le `progressionManager`, sans garantie de compatibilité avec le
firmware officiel.

**Noms des assets** : les noms de sortie sont canoniques (`[A-Za-z0-9_-]`, extension
`.lif`/`.mp3`), uniques sans tenir compte de la casse et distincts des noms réservés
(`empty.lif`, `title.lif`, `thumbnail.lif`, `silent.mp3`, `title.mp3`) : `s0.png` et
`s0.jpg` donnent `s0.lif` et `s0_2.lif`. Un nom TELMI contenant `..`, `\`, `:`, NUL ou
commençant par `/` est **refusé** (zip-slip), et l'écriture `--plain` vérifie que chaque
fichier reste dans le dossier `.plain`.

**Images** : scènes ajustées sans déformation dans 320×212 (zone d'affichage sous le
bandeau de 28 px) et centrées sur un canevas transparent de 320×212 (une image TELMI
640×480 donne 283×212 avec des bandes de 18 et 19 px) ; icônes d'inventaire ≤ 128×128 (fichier distinct
`<nom>_inv.lif` si l'image sert aussi à une scène) ; vignette 128×96 (une couverture
portrait est centrée sur un canevas paysage) ; une image portrait de plus de 64 px est
complétée en carré transparent pour ne pas déclencher l'heuristique de transposition du
décodeur.
La clé de transparence `tRNS` est appliquée pour toutes les profondeurs (gris 1/2/4/8/16
bits, RGB 8/16 bits, palette) : un fond déclaré transparent reste transparent dans le `.lif`.

**Runtime** : `runtime/script/` et `runtime/img/script/` doivent accompagner
`telmi2flam.py`. S'ils sont absents ou incomplets (`global.lua`, `progressionManager.lua`,
modules d'écran `audio-player_1_0_0`, `carousel_1_0_0`, `image-choice_1_0_0`,
`list-choice_1_0_0`, `title-card`, leurs dépendances et icônes UI), la conversion échoue
au lieu de produire un paquet qui planterait dès `setup()` ; `validate.py` signale aussi
ces entrées manquantes.

### Exemple validé

`7+.Enquete.dans.la.foret.enchantee.zip` →
**139 stages, 140 actions, 93 images, 130 audio + 130 mp3map, 35,2 Mo**, 358 entrées,
toutes *Stored* (0 % compression). Conversion ≈ 12 s.

### Validation hors-GUI

```bash
python validate.py <histoire.plain.pk | dossier .plain>
```

Un dossier `.plain` extrait (`--plain`, `-o X.plain`) subit les mêmes contrôles, sauf la
méthode *stored* (propre à l'archive).
Aucune dépendance obligatoire. Vérifie : entrées requises + méthode *stored* +
`uuid.bin` 16 o + noms d'entrées sûrs + `info.plain` sur 4 lignes ; **syntaxe Lua**
de `main.lua` et `script/*.lua` (compilation sans exécution, via `lupa` s'il est
installé, sinon via un interpréteur Lua externe : variable `FLAM_LUA` ou
`lua54`/`lua5.4`/`lua` dans le `PATH` ; sinon avertissement) ; lecture de `nodes.lua`
par un parseur Python du sous-ensemble généré ; cohérence des références
(`start`/`ok`/`home` → actions, actions → stages, index) ; **indices d'inventaire**
(conditions, opérations, `indexItem`) ; présence de tous les assets image/audio +
`.mp3map` ; **en-têtes LIF** (magic, dimensions ≤ 2047, canal, marqueur de fin) ;
**tables `.mp3map`** (taille, offsets croissants pointant sur un header MPEG) ;
**accessibilité** de tous les stages depuis `start`.

### Tests du convertisseur

```bash
python -m unittest discover -s tools/telmi2flam/tests -v
```

Paquets TELMI synthétiques générés à la volée (zip-slip, image corrompue, audio
manquant/casse, BOM, types inattendus, racine `__MACOSX`, mots réservés Lua, NaN,
collisions de noms, inventaire, portrait, labels, titre multi-lignes, caractères hors
police, `totalChapters` sur branches exclusives, UUID, mp3map 44,1/48/32/22,05/24 kHz,
`validate.py` sur un dossier `.plain`). Définir `FLAM_LUA` (ex. `lua54.exe`) pour vérifier
aussi le chargement de `nodes.lua` par un vrai Lua 5.4.

Résultat de `validate.py` sur *Maxicours Anglais - CP / CE1 / CE2* (199 stages, 110
actions) : **35 OK / 4 warnings / 0 erreur**, en `.plain.pk` comme en dossier `.plain`.
Les 4 warnings sont des actions vides (`backChildAction`) présentes telles quelles dans
le `nodes.json` source.

### Test dans flam-player (GUI)

`test_run.bat` (racine du dépôt) enchaîne les tests unitaires du convertisseur puis un test
de bout en bout `tests/e2e/e2e_telmi.py` : conversion d'un paquet TELMI synthétique puis
lancement du vrai `flam-player`. Manuellement : `flam-player [--strict] <dossier .plain | .plain.pk>` (`--strict` : `require()`
limité à `script/`, comme le device).

---

## 6. Décisions de conception

- **Support complet** d'emblée (inventaire, conditions, jauges, aléatoire), pas seulement linéaire.
- **Images de scène à la taille de la zone d'affichage** (320×212 sous le bandeau de 28 px,
  ratio conservé, bandes transparentes) : le runtime n'applique aucune mise à l'échelle.
- **Texte ramené à ASCII + Latin-1** (couverture des polices du player, `src/fonts/`).
- **Convertisseur en Python** (Pillow pour le décodage/redimensionnement PNG).
- **mp3map généré** (fidèle au format officiel).
- **UUID réutilisé** depuis `metadata.json` de TELMI (à défaut : hash du contenu de
  `nodes.json`, pour ne pas partager la sauvegarde entre deux histoires de même titre).
- **Validation** dans le `flam-player` du dépôt.

---

## 7. Notes device réel vs simulateur

Le device réel est bien plus strict que le simulateur. Pièges rencontrés (tous
nécessaires pour que l'histoire **charge sur le device**) :

- **UI via modules Global obligatoire** : le firmware rend/gère les entrées via
  `Global.current_module`. Dessiner en `lv.*` direct marche au sim mais pas sur device.
- **Aucun `require()` au top-level de `main.lua`** : le firmware n'installe le searcher
  `require` qu'**après** avoir chargé `main.lua`. Un `require` au chargement du module
  échoue → `setup()` jamais défini → l'histoire ne charge pas. ⇒ tous les `require`
  (dont `require("nodes")`) sont faits **dans `setup()`**.
- **`require()` résolu uniquement depuis `script/`** ⇒ `nodes.lua` est en `script/nodes.lua`.
- **Bibliothèque Lunii embarquée** (`global.lua`, `progressionManager.lua` + deps, modules
  `audio-player`, `carousel`, `image-choice`, `list-choice`, `title-card`) dans `script/`.
- **`title-card` avance via `back_callback()`** (fin d'audio) : `progression.create{
  backBehavior = Start }` règle ce callback sur le menu Start avant `IntroCard()`.
- **`title-card` affiche le cover à sa taille native** dans un slot ~89×120 (bas-droite,
  `translate(231,92)`) ⇒ `img/title.lif` est généré ajusté à 89×120 (pas plein écran).
- `version` = `"1"`, `info.plain` = `titre\nsous-titre\n000000\ntitre`.
- **Retour** : jamais `goto_library()` depuis une scène active (Bug C, écran noir au
  lancement suivant). Scène ou choix → retour → menu Start → retour → bibliothèque, comme
  les histoires officielles. La transition `home` TELMI (si `ctrl.home`) remplace le
  premier retour.
- **Reprise** : le firmware persiste `state`. `setProgression` y écrit `current_fun`,
  `currentBranchName` et `visited_funs` (chapitres ; `audio-player` y range la position
  audio `seekposition`) ; `story.lua` y ajoute
  `current_kind` et `choice_index`. Rien n'est repris automatiquement dans `setup()` : après
  la title-card, le menu Start propose **Reprendre l'histoire** dès que
  `progression.isStoryStarted()`, et `LoadCurrentFunction` → `story.resume()` rejoue le
  nœud sauvé **selon son type** :
  - scène (`current_kind = "s"`) → `enterStage(id, true)` : ops d'inventaire non rejouées,
    audio repris à `seekposition` ;
  - choix (`current_kind = "a"`) → `enterAction(id, choice_index, true)` : même filtre par
    conditions, toujours le choix (pas de raccourci autoplay, pas de nouveau tirage pour
    `index = -1`), **option présélectionnée à l'affichage** restaurée, sans rejouer la
    narration précédente ;
  - `current_kind` absent (sauvegarde d'une version antérieure du paquet) → scène, sinon
    action de cet id ; id inconnu du graphe → redémarrage (`__start`).
  La **fin** d'histoire (`endStory`) remet la progression et ces champs à zéro : le menu
  propose de nouveau **Démarrer**. Le menu contextuel `M` (« Reprendre l'histoire »,
  `LateralResume`) reprend ou démarre de la même façon.
- **Stages sans audio** : `autoplay` (ex. `backStage` TELMI) ou sans image → on enchaîne
  directement la transition `ok` (nœud de passage, garde anti-boucle `MAX_PASS`) ; image
  non `autoplay` → affichée par `audio-player` sans audio, en attente de OK.
- **OK / skip** (comme Telmi) : la copie embarquée d'`audio-player_1_0_0.lua` accepte
  `args.okCallback` (OK = ENTER `key=10` via `EVENT_KEY`) et un `args.callback` optionnel
  (fin d'audio ; `nil` = rester sur l'image, seek désactivé). L'ENTER est mémorisé puis
  traité par un timer de 100 ms (`okTimer`, jamais dans l'événement : le callback recharge
  un module), avec anti-rebond d'entrée (~200 ms) et garde anti double appel
  (`audioPlayer.leave` / `audioPlayer.exited`) entre fin d'audio et OK.

## 8. Limites connues / TODO

- `notes.json` (éditeur, optionnel) : sert uniquement au **label des choix** (carrousel) :
  `title` de la scène, sinon `text`, sinon `notes`, aplati sur une ligne et tronqué à
  40 caractères. Le reste est ignoré (non nécessaire à la lecture). Un `notes.json`
  illisible (JSON invalide, non UTF-8, NaN) est ignoré avec un avertissement.
- Pas de gestion des succès/collections (spécifique aux histoires Lunii natives).
- Inventaire : affiché seulement par l'entrée **Inventaire** du menu Start (`list-choice`,
  compteur `valeur/max` ou jauge selon `display`), pas pendant les scènes.
- `playingTime` : approximatif — utilise `Global.audioDuration` (dernière valeur rapportée
  par le callback audio), pas un chronomètre précis de la scène courante.
- Choix molette : `ctrl.ok=false` d'une option n'est pas appliqué (OK valide toujours) ;
  `indexItem` joue le stage désigné sans molette. Une reprise sur un choix revient à
  l'option **présélectionnée à l'affichage** (`choice_index`), pas à celle qui avait le
  focus au moment de quitter ; pas de nouveau tirage pour `index = -1`.
- Jauge « Reprendre » : un seul `totalChapters` pour tout le paquet (chemin le plus long,
  cf. §5). Sur une branche plus courte que la plus longue, la jauge plafonne en dessous
  de 100 %.
- Texte : caractères hors ASCII + Latin-1 translittérés ou supprimés à la conversion
  (cf. §5) ; une police plus complète éviterait cette perte.
- `home` : seule la boucle directe (retour vers le nœud courant) est détectée ; un cycle
  `home` à plusieurs nœuds empêcherait de revenir au menu Start par le bouton retour.
