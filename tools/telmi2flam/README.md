# telmi2flam — Convertisseur d'histoires TELMI → FLAM

Convertit une histoire interactive au format **TELMI** (archive `.zip` déclarative)
en une histoire au format **FLAM** (`.plain.pk`)
jouable dans le `flam-player` de ce dépôt.

> 📄 **Spécification du format source** : [`TELMI_FORMAT.md`](TELMI_FORMAT.md)
> (structure des fichiers, `metadata.json`, `nodes.json`, inventaire, conditions…).
> À noter : TELMI **n'a pas de notion de chapitre** ; le convertisseur traite chaque
> **scène avec audio** comme un chapitre (cf. `engine/story.lua`).

> **État** : codec LIF ✅ · mp3map ✅ · moteur Lua ✅ · convertisseur ✅ · 1ʳᵉ lecture device ✅
> · **relance device ⛔ (bug ouvert, cf. `DEVICE_VS_SIM.md` #10 — écran noir à la 2ᵉ ouverture,
> contournement : reset de la progression sur le device)**

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
2. **Transcoder les assets** : PNG→LIF (320×240), MP3→MP3+mp3map.
3. Émettre les **données** (`nodes.lua`) + les **métadonnées** + **rezip stored**.

---

## 2. Le format FLAM (rétro-ingénierie depuis `flam-player`)

Archive `.plain.pk` = ZIP **stored** (méthode 0, aucune compression). Entrées clés :

```
info.plain          # texte ; 1re ligne = titre de l'histoire (lue par le player)
version             # "2"
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
`Échap` = retour (`back_callback`), `M` = menu contextuel, `S` = screenshot.

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

Cadence interne 88200 = 2×44100. `unit_pos = frame_index × 2304` (= 1152 samples × 2).
**1 enregistrement par seconde** : `frame_index = ceil(k × 88200 / 2304)`.
`total_units = (2·N_frames − X)·1152` avec X≈34 (>100 frames) ou 22 (correction délai
décodeur). `duration_s = total_units / 88200`.

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
| 1 seul stage | lien déterministe : va directement à ce stage |
| conditions présentes | 1er stage dont **toutes** les `conditions` passent |
| N stages (sans conditions) | **choix molette** : `←/→` change l'option focalisée (image+audio), OK valide |

`index` = option initialement sélectionnée. Vérifié sur les données : une action à
**1 option** est toujours ciblée avec `index 0` (lien) ; une action **multi-options**
est ciblée avec `index = -1` (24×, pas de présélection → option 0) ou `index = 0`
(12×) — **dans les deux cas c'est un choix molette** (et non une branche aléatoire).

`items` : opérations sur l'inventaire (`type` : 0 `+=`, 1 `-=`, 2 `=`, 3 `*=`, 4 `/=`, 5 `%=`).
`conditions` : `comparator` 0 `<`, 1 `<=`, 2 `==`, 3 `>`, 4 `>=`, 5 `!=`.
`control.autoplay` : avance automatiquement à la fin de l'audio.

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

- `build(data) -> (bytes, duration_s, num_records)` : parse les frames MP3
  (MPEG 1/2/2.5, Layers I/II/III, saut du tag ID3v2) et produit le `.mp3map`.
- `build_file(path)` : variante fichier.

**Validé** : sortie **identique octet-pour-octet** au vrai `.mp3map` de l'histoire
officielle *Cluedo* (`mine == real`).

### `engine/main.lua` (bootstrap) + `engine/story.lua` (branch)

Architecture **identique aux histoires Lunii officielles** (Cluedo) : tout passe par les
modules `Global` (le device rend/gère les entrées via `Global.current_module` ; un moteur
en `lv.*` direct marche au sim mais PAS sur device).

`main.lua` (bootstrap) :
- `setup()` : `Global = require("global")`, `Global.init()`,
  `Global.progression.create{ totalChapters, backBehavior = Start }`,
  `Global.setDefaultAudioPlayerCover("empty.lif", …)`, puis `IntroCard()`.
- `IntroCard()` : `title-card.display{ title, subtitle, audio, img, cb = Start }`
  (la title-card avance via `back_callback()` ; `progression.create` l'a réglé sur `Start`).
- `Start()` : menu `list-choice` → **Démarrer** (0%) ou **Reprendre** (`isStoryStarted`).
- `LoadStartFunction` / `LoadCurrentFunction` : `Global.loadBranch("story")` puis
  `Global.current_branch[<noeud>]()` (`"__start"` ou `state.current_fun`).

`script/story.lua` (branch, requis par `loadBranch`) :
- `story.clear()` + métatable : `story[<nom>]()` → `play(nom)`.
- `play(name)` : `__start` → `followTransition(start)` ; stage → `enterStage` ; action → `showChoice`.
- `enterStage(id)` : `audio-player.create{ audio_path, image_background_path=image, callback }`
  + `setProgression{ currentFunction=id, branch="story", ischapter, chapterData }`.
- `showChoice(list, actionId)` : `image-choice.create{ choices={{img,audio,cb}} }`
  + `setProgression{ currentFunction=actionId, branch="story" }`.
- Inventaire (`state.inv`, ops + conditions), `followTransition` (1 / conditions / choix multi).
- Reprise : champs STANDARD `state.current_fun` + `state.currentBranchName` (comme Cluedo).

⚠️ Cette réplication fidèle de Cluedo **ne corrige pas** le bug de relance device (#10) —
cf. `DEVICE_VS_SIM.md`.
- Inventaire : `state.inv`, ops complètes (`number`, `assignItem`, `playingTime`), conditions complètes (`num`, `itemB`), `indexItem` dans les transitions, `inventoryReset`.

Données attendues (`nodes.lua`, généré par le convertisseur) :

```lua
return {
  meta      = { title = "…", subtitle = "…" },                   -- pour la title-card
  start     = { action = "a0", index = 0 },
  inventory = { { name=, init=, max=, display=, image= }, … },   -- ou nil
  title     = { image = "title.lif", audio = "title.mp3" },      -- ou nil
  stages = {
    s5 = { image=nil, audio="s5.mp3",
           ok={action="a4",index=0}, home={action="backAction",index=0},
           ctrl={ok=true,home=true,autoplay=true}, items={…}, reset=false },
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
```

- `-o` : chemin de sortie (défaut : `<Titre>.<UUID8>.plain.pk` dans le dossier courant).
  - se termine par `.plain.pk` → archive ZIP **stored** (format device) ;
  - se termine par `.plain` → écrit **uniquement** le dossier extrait (pas de `.pk`).
- `--plain` : écrit **aussi** le dossier `.plain` extrait à côté du `.plain.pk`
  (pratique pour le simulateur, qui charge un dossier `.plain` directement).
- `--keep-size` : conserve la résolution native des images (sinon resize 320×240).

Dépendance : `pip install Pillow`.

Le CLI :
1. lit `metadata.json` + `nodes.json` dans le zip ;
2. transcode images (PNG→LIF) et audio (MP3 copié + `.mp3map` généré), avec cache anti-doublon ;
3. génère `nodes.lua` (table de données) + embarque `engine/main.lua` ;
4. écrit `info.plain` / `version` / `uuid.bin` (= MD5 de l'UUID TELMI) / `img/thumbnail.lif` ;
5. rezip en **ZIP stored**.

### Exemple validé

`7+.Enquete.dans.la.foret.enchantee.zip` →
**139 stages, 140 actions, 93 images, 130 audio + 130 mp3map, 35,2 Mo**, 358 entrées,
toutes *Stored* (0 % compression). Conversion ≈ 12 s.

### Validation hors-GUI

```bash
pip install lupa
python validate.py <histoire.plain.pk>
```

Vérifie : entrées requises + méthode *stored* + `uuid.bin` 16 o ; **syntaxe Lua**
de `main.lua` et `nodes.lua` (compilation `lupa`, sans exécution) ; cohérence des
références (`start`/`ok`/`home` → actions, actions → stages) ; présence de tous les
assets image/audio + `.mp3map` ; **accessibilité** de tous les stages depuis `start`.

Résultat sur l'exemple : **15 OK / 0 warning / 0 erreur**.

### Test dans flam-player (GUI)

> 🚧 Build complet du player (CMake + VS2022, SDL2/Lua/lvgl compilés depuis `libs/`)
> puis lancement sur le `.plain.pk`. À faire.

---

## 6. Décisions de conception

- **Support complet** d'emblée (inventaire, conditions, jauges, aléatoire), pas seulement linéaire.
- **Images redimensionnées 320×240** (écran natif FLAM), mise à l'échelle finale à la fenêtre au runtime.
- **Convertisseur en Python** (Pillow pour le décodage/redimensionnement PNG).
- **mp3map généré** (fidèle au format officiel).
- **UUID réutilisé** depuis `metadata.json` de TELMI.
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
- **Bibliothèque Lunii embarquée** (`global.lua` + deps + modules `audio-player`,
  `image-choice`, `title-card`) dans `script/`.
- **`title-card` avance via `back_callback()`** (fin d'audio) → on fait
  `Global.setBackBehavior(startStory)` avant de l'afficher.
- **`title-card` affiche le cover à sa taille native** dans un slot ~89×120 (bas-droite,
  `translate(231,92)`) ⇒ `img/title.lif` est généré ajusté à 89×120 (pas plein écran).
- `version` = `"1"`, `info.plain` = `titre\nsous-titre\n000000\ntitre`.
- **Reprise** : le firmware persiste `state` (dont `state.current_fun` / `visited_funs`,
  écrits par `progression.setProgression`). `setup()` teste `progression.isStoryStarted()`
  et **reprend** au dernier stage (`S.enterStage(state.current_fun)`) sans réinitialiser
  l'inventaire ; sinon démarrage normal. Ne pas gérer la reprise = **écran noir** au retour
  dans l'histoire. La **fin** d'histoire réinitialise la progression (repart du début).
- **Reprise sur un choix** : un choix n'est pas un stage → on mémorise `state.current_choice`
  (= l'action) quand on l'affiche, et `setup()` y revient **directement** (sans rejouer la
  narration précédente). Sinon, quitter sur un choix = narration rejouée / écran noir.
- **Stages sans audio** (ex. `backStage` TELMI) : `audio-player` ne déclencherait jamais
  son `callback` → on enchaîne directement la transition `ok` (nœud de passage).
- **Skip audio** (comme Telmi) : la copie embarquée d'`audio-player_1_0_0.lua` est
  patchée (`audioPlayer.skip()`) pour qu'un **clic central** (ENTER `key=10` via
  `EVENT_KEY`, ou `EVENT_CLICKED`) stoppe l'audio et passe à la scène suivante
  (`exitCallback`), avec garde anti-double (`audioPlayer.skipping`).

## 8. Limites connues / TODO

- `notes.json` (éditeur) ignoré — non nécessaire à la lecture.
- Pas de gestion des succès/collections (spécifique aux histoires Lunii natives).
- Rendu de l'inventaire à l'écran (compteur/jauge) : non encore implémenté dans le moteur
  (l'inventaire est géré en logique, pas affiché).
- `playingTime` : approximatif — utilise `Global.audioDuration` (dernière valeur rapportée
  par le callback audio), pas un chronomètre précis de la scène courante.
```
