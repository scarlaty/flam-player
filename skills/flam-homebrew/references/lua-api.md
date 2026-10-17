# API Lua pour un homebrew FLAM

Sommaire : 1. Environnement — 2. Bootstrap — 3. Contrat de module — 4. Entrées molette —
5. Dessin (LVGL) — 6. Boucle de jeu — 7. Images — 8. Audio — 9. Texte et polices —
10. Écrans multiples — 11. Sauvegarde — 12. API sûre (liste blanche)

## 1. Environnement

Lua 5.4, sandbox du device : `lv.*`, globales firmware (`window`, `document`, `audio`,
`state`, `screen`, `goto_library`, `back_callback`, `context_menu`) et la bibliothèque
standard **sans** `io`, `os`, `debug`. Utiliser `string`, `math`, `table`, `pairs`,
`ipairs`, `tostring`, `require` (dans `setup()` ou plus tard), `collectgarbage`.
`math.random(n)` est utilisé par le module officiel `dice-roll` : disponible.

Écran 320×240. Le firmware dessine un bandeau titre de 28 px en haut : le jeu dispose de
`window` = **320×212**, origine en haut à gauche de cette zone
(`Global.visual_width`, `Global.visual_height`).

## 2. Bootstrap (`main.lua`)

```lua
function setup()
    Global = require("global")      -- GLOBALE, pas `local`
    Global.init()
    Global.isMultiBranches = true
    Global.setBackToLibrary()       -- bouton retour -> bibliothèque
    Global.load_module("monjeu", "1_0_0").create({})
end
```

- Rien d'exécutable au top-level hors définitions : surtout **aucun `require`** hors
  fonction (le device installe le chercheur de modules après avoir chargé `main.lua`).
- `Global.load_module(nom, version)` fait `require(nom .. "_" .. version)`, mémorise le
  module courant et nettoie le précédent (`clean()`). Version `""` → `require(nom)`.

## 3. Contrat de module (`script/<nom>_<version>.lua`)

Le device ne rend et ne route les touches **que** pour le module courant chargé par
`Global.load_module`. Tout le jeu vit donc dans un module :

```lua
local game = {}
game.styles, game.events = {}, {}
local container, loopTimer           -- état en upvalues (réinitialisé au rechargement)

function game.clean()                -- appelé par Global.cleanCurrentModule
    if loopTimer then lv.timer.del(loopTimer); loopTimer = nil end
    for obj, ev in pairs(game.events) do
        for _, cb in pairs(ev) do lv.obj.remove_event_cb(obj, cb) end
    end
    Global.requestAudioStop(true, true)
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    container = nil
    game.styles, game.events = {}, {}
end

function game.create(args)
    lv.obj.clean(window)
    lv.group.set_editing(document, true)          -- molette -> EVENT_KEY, clic -> CLICKED
    -- styles, objets, timers...
    container = lv.obj.new(window)
    lv.obj.remove_style_all(container)
    lv.obj.set_size(container, Global.visual_width, Global.visual_height)
    lv.obj.clear_flag(container, lv.OBJ_FLAG_SCROLLABLE)   -- §4 : clic = CLICKED seul
    game.events[container] = {                    -- voir §4 (clic = EVENT_CLICKED)
        key   = lv.obj.add_event_cb(container, onKey, lv.EVENT_KEY),
        click = lv.obj.add_event_cb(container, onClick, lv.EVENT_CLICKED),
    }
    lv.group.add_obj(document, container)
end

return game
```

- `clean()` doit défaire **tout** : timers (`lv.timer.del`), callbacks, groupe, objets.
  Un timer oublié continue d'appeler du code sur des objets détruits. Exception : la
  sortie vers la bibliothèque ne doit **pas** appeler `clean()` (§10, relance).
- Le module est rechargé à neuf après `clean()` (`package.loaded[...] = nil`) : ne pas
  stocker d'état à conserver dans le module ; utiliser `state` (§11) ou `args`.
- `Global.visual_width` etc. peuvent être lus au top-level du **module** (il est chargé
  après `Global`), pas au top-level de `main.lua`.

## 4. Entrées molette

La molette est un **encodeur** LVGL (simulateur et Flam, vérifié sur device) :

- gauche (20) et droite (19) arrivent en `lv.EVENT_KEY` ;
- le **clic molette** (= ENTER) arrive en `lv.EVENT_CLICKED` (précédé de
  PRESSED/RELEASED) sur l'objet focalisé. Sur un objet **non défilable et non éditable**
  (le cas recommandé), il n'arrive **jamais** en `EVENT_KEY` 10 : un code qui n'attend
  que `b == 10` ne voit pas le clic (constaté au simulateur, cohérent avec la Flam).
  Sur un objet défilable, LVGL envoie les deux (d'où l'anti-doublon ci-dessous).

Écouter les deux, et ne compter un appui qu'une fois :

```lua
local ticks, lastEnter = 0, -100          -- ticks incrémenté dans tick()

local function action(b)
    if b == 10 then                       -- clic molette / ENTER
        if ticks - lastEnter < 4 then return end   -- un appui = une fois
        lastEnter = ticks
        -- ...
    elseif b == 19 then                   -- molette droite
    elseif b == 20 then                   -- molette gauche
    end
end

local function onKey(e)
    local k = lv.event.get_key_value(e)
    if k == nil then return end
    action(string.byte(k))
end
local function onClick(e) action(10) end

-- dans create() :
game.events[container] = {
    key   = lv.obj.add_event_cb(container, onKey, lv.EVENT_KEY),
    click = lv.obj.add_event_cb(container, onClick, lv.EVENT_CLICKED),
}
-- dans clean() : for _, cb in pairs(ev) do lv.obj.remove_event_cb(obj, cb) end
```

- Prérequis : objet ajouté au groupe `document` + `lv.group.set_editing(document, true)`,
  objet **non** défilable (`clear_flag(..., lv.OBJ_FLAG_SCROLLABLE)`). Sans le mode
  édition, la molette déplace le focus au lieu d'envoyer des touches.
- Le clic est pris en compte au **relâchement** (`CLICKED`) ; un appui long envoie
  d'abord `LONG_PRESSED`, puis `CLICKED` au relâchement (action retardée).
  `lv.EVENT_PRESSED` serait immédiat mais n'est pas dans la liste blanche (§12).
- Doubler malgré tout les actions par les directions quand c'est naturel (« un cran
  relance après 1 s ») : un enfant tourne plus volontiers qu'il ne clique.
- Retour : bouton retour du device / Échap au simulateur → `back_callback` (posé par
  `Global.setBackToLibrary()` ou `Global.setBackBehavior(fn)`), pas un `EVENT_KEY`.
  Voir §10 pour sortir vers la bibliothèque sans casser la relance.
- Contrôles discrets (1 cran = 1 pas) : prévoir des pas assez grands (casse-briques :
  18 px par cran), pas de « touche maintenue ».
- Test automatisé au simulateur (constaté) : `WScript.Shell.SendKeys` n'envoie pas
  Entrée/Espace à la fenêtre SDL (seulement des NumLock) ; `keybd_event` (user32) marche.

## 5. Dessin (LVGL 8.3 via `lv.*`)

Construire avec des objets, pas de canvas :

```lua
local S = lv.style.new()
lv.style.set_bg_color(S, lv.color.hex(0xfbbd2a))
lv.style.set_bg_opa(S, lv.OPA_COVER)
lv.style.set_radius(S, 4)

local o = lv.obj.new(container)
lv.obj.remove_style_all(o)                 -- partir d'un objet nu
lv.obj.set_size(o, 24, 24)
lv.obj.add_style(o, S, lv.STATE_DEFAULT)
lv.obj.set_pos(o, x, y)                    -- relatif au parent
lv.obj.add_flag(o, lv.OBJ_FLAG_HIDDEN)     -- masquer ; clear_flag pour réafficher
lv.obj.set_style_bg_color(o, lv.color.hex(0xff0000), lv.STATE_DEFAULT)  -- style local
lv.obj.del(o)                              -- détruire (ne plus utiliser `o` ensuite)
```

- Garder les styles dans une table du module (`game.styles`) : ils doivent vivre aussi
  longtemps que les objets qui les utilisent. Les créer une fois dans `create()` (pas
  par frame) ; `clean()` vide la table après `lv.obj.clean(window)`.
- **Ordre d'affichage** = ordre de création entre frères : le dernier créé est dessiné
  au-dessus. Créer le décor d'abord, les sprites ensuite, le HUD (score, messages) en
  dernier.
- **Défilement** : un conteneur LVGL devient défilable dès qu'un enfant dépasse ses
  bords (sprite qui entre par le haut, etc.) → `lv.obj.clear_flag(container,
  lv.OBJ_FLAG_SCROLLABLE)` sur le conteneur du jeu.
- Mettre à jour un label seulement quand sa valeur change (pas à chaque frame).
- Labels : `lv.label.new(parent)`, `lv.label.set_text(l, "...")`,
  `lv.label.set_long_mode(l, lv.LABEL_LONG_WRAP)` + `lv.obj.set_width` pour du texte
  multiligne, `lv.obj.align(l, lv.ALIGN_CENTER, dx, dy)`.
- Couleurs : `lv.color.hex(0xRRGGBB)`, `lv.color.black()`. Opacité 0..255 ou
  `lv.OPA_COVER` / `lv.OPA_TRANSP`.
- Budget : quelques dizaines à ~150 objets (le casse-briques en a ~45). Le tas LVGL du
  device n'est pas mesuré : créer les objets une fois et les déplacer/masquer, plutôt
  que créer/détruire à chaque frame.

## 6. Boucle de jeu

```lua
loopTimer = lv.timer.new(tick, 25, nil)    -- ~40 fps, valeur éprouvée sur device
-- lv.timer.reset(t), lv.timer.set_repeat_count(t, 1) (one-shot), lv.timer.del(t)
```

- `tick()` : intégrer positions, collisions AABB maison, `lv.obj.set_pos`. Pas de
  physique coûteuse ni d'allocation de tables par frame.
- Pause logique : un booléen `running` testé en tête de `tick`, plutôt que
  détruire/recréer le timer.

## 7. Images

```lua
local data = Global.load_image("hero.lif")      -- img/hero.lif ; renvoie data, w, h
local img = lv.img.new(container)
lv.obj.remove_style_all(img)
lv.img.set_src(img, data)
lv.obj.set_pos(img, x, y)
game.images = game.images or {}; game.images.hero = data   -- garder la référence
```

- Chemin relatif à `img/` (sous-dossiers permis : `"sprites/a.lif"`). Image absente :
  le simulateur renvoie `nil` sans bruit, le device plante ou gèle → `flam_check.py`
  vérifie les littéraux `"*.lif"`.
- Un `lv.img` plus grand que sa source **répète** l'image : ne pas agrandir l'objet ;
  `lv.img.set_zoom(img, 256)` = 100 %.
- Décor plein écran : 320×212 exactement.

## 8. Audio

```lua
Global.requestAudioPlay({ path = "boing.mp3", priority = true, showPause = false,
                          AFCb = function(state, seconds) end })   -- sounds/boing.mp3
Global.requestAudioStop(true, true)                                 -- couper
```

- Champs reconnus par le runtime embarqué : `path`, `AFCb`, `priority` (couper la
  piste en cours pour celle-ci : donc un `stop`, à éviter pendant l'action), `showPause` (`false` dans un jeu : pas d'overlay
  pause plein écran si l'audio est mis en pause).
- `requestAudioPlay` charge après un délai de 500 ms (`Global.audioDelayValue`) :
  adapté à une musique ou une narration, pas à un bruitage synchrone.
- Fichier absent : avec ce runtime, le chargement est retenté toutes les 500 ms et
  `"stop"` n'arrive jamais → toujours vérifier que le `.mp3` existe (`flam_check.py`
  contrôle les littéraux `"*.mp3"`).
- `AFCb(state, seconds)` reçoit `"play"` (chaque seconde), `"pause"`, `"stop"` (fin).
- Chaque `.mp3` doit avoir son `.mp3map` (généré par `flam_build.py`).
- Sans besoin d'audio : ne rien faire (le casse-briques n'a pas de son).

### Règles mesurées sur la Flam (jeu Capi et paquet de test, 2026-10)

Constats faits par l'utilisateur sur sa Flam ; le simulateur ne montre aucun de ces
effets (stop, pause et load y sont instantanés).

| Appel | Effet sur device |
|---|---|
| `audio.load` + `play` après la **fin naturelle** de la piste précédente | sans à-coup |
| `audio.stop()` ou `audio.pause()` d'une piste **en cours** | l'image **se fige** un instant |
| `audio.load` pendant qu'une piste joue (sans stop) | **gel total**, redémarrage de la Flam |
| `audio.load(1, ...)` + `play` | pas un 2ᵉ canal : la musique s'arrête, le nouveau son joue (les scripts du runtime passent toujours `0`) |
| `audio.seek` pendant la lecture | gel (piège 9) : `pause()` → `seek()` → `play()` |

Conséquences :

- **Une seule piste, pas de mixage.** Musique **et** bruitages instantanés pendant
  l'action sont impossibles sans à-coup : chaque bruitage doit couper la musique.
- **Jeu d'action** : pas de musique pendant la partie ; des bruitages courts qui
  attendent la fin naturelle du précédent (ou sont oubliés s'ils attendent trop).
  Musique seulement aux écrans calmes (titre, fin), où l'arrêt ne se voit pas : la
  couper au **départ** de la partie (seul `stop` toléré, l'image change de toute façon).
- Bruitages pilotés depuis la boucle du jeu (25 ms) : n'appeler `audio.load` **que si**
  `audio.get_status() == "stop"`, jamais deux chargements rapprochés, et ne pas mélanger
  avec `Global.requestAudioPlay` (son propre timer de 500 ms). Les callbacks passés à
  `audio.load` portent un jeton : ignorer le `"stop"` d'une piste coupée volontairement.
  Exemple complet : `games/capi-riviere/script/capi_1_0_0.lua` (`audioPump`).
- Musique longue sans `seek` : la découper en segments (2 mesures) enchaînés sur
  `"stop"` ; chaque enchaînement naturel est sans à-coup.

## 9. Texte et polices

- Polices disponibles (simulateur ET modules officiels) : `lv.font.nunito_bold_12`,
  `nunito_bold_16`, `nunito_bold_20`, `nunito_extrabold_12`, `nunito_extrabold_14`,
  `nunito_extrabold_16`, `nunito_extrabold_20`. S'en tenir à cette liste.
- Glyphes : U+0020–007E et U+00A0–00FF seulement. Accents français OK (é è à ç ù ô ï,
  « »), **pas** de `’ “ ” – — … œ €`, emoji, flèches Unicode → rectangle vide.
  Écrire `'`, `"`, `-`, `...`, `oe`, `EUR`. Sources Lua en UTF-8 sans BOM.
- Pas de saut de ligne dans les titres de `flam.json`.

## 10. Écrans multiples

Un écran = un module. Passer d'un écran à l'autre :

```lua
Global.load_module("menu", "1_0_0").create({ onPlay = ... })   -- nettoie l'écran courant
Global.setBackBehavior(function() Global.load_module("menu", "1_0_0").create({}) end)
```

- Les données à transmettre passent par `args` de `create` ou par `state`.
- Pour un menu à liste, le module fourni `list-choice` (version `1_0_0`) existe :
  `Global.load_module("list-choice","1_0_0").create({ choices = { { label="Jouer",
  img="empty.lif", audio="silent.mp3", cb=function() ... end } } })` — **chaque entrée
  exige `img` et `audio` (chaîne)**, sinon l'entrée est rejetée et le menu vide plante.
- Le plus simple et le plus éprouvé reste un module unique avec des phases internes
  (le casse-briques : phase `lock` puis `play`).

### Sortie vers la bibliothèque et relance (vérifié sur device)

À la relance depuis la bibliothèque, la Flam **ne réexécute pas `setup()`** : elle
réaffiche l'écran et l'état Lua laissés à la sortie. Donc, au moment de
`goto_library()` :

- **ne pas** vider l'écran ni appeler `cleanCurrentModule` / `clean()` (relance = écran
  noir) ; ne **pas** supprimer le timer de jeu (relance = écran affiché mais touches
  mortes si un délai compte les ticks) ;
- couper seulement le son ; le reste du module reste vivant, comme le casse-briques :

```lua
back_callback = function()             -- sur l'écran d'accueil du jeu seulement
    -- arrêter sa propre machine à sons (demandes en attente, musique) ...
    Global.requestAudioStop(true, true)
    goto_library()
end
```

- Depuis une partie en cours : retour → écran d'accueil du jeu
  (`Global.setBackBehavior(fn)`, qui nettoie et recrée le module), puis seulement de là
  → bibliothèque, comme les histoires officielles (sortir d'une scène active vers la
  bibliothèque donnait l'écran noir : `tools/telmi2flam/DEVICE_VS_SIM.md` §10). Jamais
  de `setBackBehavior(function() goto_library() end)`.
- La musique coupée à la sortie ne reprend pas toute seule à la relance (aucun
  événement ne signale le retour) : l'écran d'accueil reste muet jusqu'à la première
  action.

## 11. Sauvegarde

`state` est une table globale persistée par le firmware (par UUID, fichiers
`/usr/0/{uuid}.save` et `.prog`, voir `flam-device-inventory.md` à la racine du dépôt).

- **Jeu : ne rien écrire dans `state`.** Sur device, un homebrew qui y écrivait
  (`state.best = score`) donnait un écran noir à la relance, réparé seulement en
  effaçant la progression dans les paramètres de la Flam. Le casse-briques n'écrit
  rien. Garder un record pour la session dans une table qui survit au rechargement du
  module, par ex. `Global.monJeuBest`.
- Si une vraie sauvegarde est indispensable : nombres, chaînes, booléens, tables
  simples, puis tester la relance x2 sans redémarrer sur device (le format diffère du
  simulateur).
- Ne pas utiliser `Global.progression` (chapitres d'histoire) pour un jeu.

## 12. API sûre (liste blanche)

Utilisée par le framework officiel ou le casse-briques, donc présente sur le device.
Préférer cette liste ; tout appel hors liste est à tester sur le device.

- `lv.obj.` new, del, clean, remove_style_all, add_style, remove_style, set_size,
  set_width, set_height, set_pos, set_x, set_y, align, set_align, get_width,
  get_height, get_x, get_coords, add_flag, clear_flag, add_state, clear_state,
  get_state, add_event_cb, remove_event_cb, invalidate, get_child, get_child_cnt,
  set_flex_flow, set_flex_align, scroll_by, scroll_to, set_style_bg_color,
  set_style_bg_opa, set_style_radius, set_style_text_color, set_style_text_align,
  set_style_img_opa, set_style_pad_top/bottom/left/row/column,
  set_style_translate_x/y
- `lv.label.` new, set_text, get_text, set_long_mode · `lv.img.` new, set_src,
  set_zoom, set_angle · `lv.btn.new` · `lv.arc.` new, set_angles, set_rotation ·
  `lv.slider.` new, set_range, set_value, get_value
- `lv.style.` new, reset, set_bg_color, set_bg_opa, set_bg_grad_color,
  set_bg_grad_dir, set_radius, set_border_color, set_border_width, set_border_opa,
  set_pad_all/top/bottom/left/right/hor/ver/row/column, set_text_color,
  set_text_font, set_text_align, set_text_opa, set_text_line_space, set_img_opa,
  set_img_recolor, set_img_recolor_opa, set_width, set_height, set_arc_color,
  set_arc_width, set_translate_y
- `lv.timer.` new, del, reset, set_repeat_count · `lv.anim.` new, set_var,
  set_values, set_time, set_exec_cb, set_path_cb, path_ease_in_out, start,
  set_repeat_count, set_playback_time · `lv.anim_var.del`
- `lv.group.` add_obj, remove_obj, remove_all_objs, focus_obj, set_editing, set_wrap
- `lv.event.` get_key_value, get_code, get_target · `lv.color.` hex, black, make ·
  `lv.img_src.` load, get_width, get_height (via `Global.load_image`)
- Constantes : `lv.ALIGN_*` (CENTER, TOP_LEFT, TOP_MID, BOTTOM_MID, LEFT_MID,
  RIGHT_MID, BOTTOM_LEFT, BOTTOM_RIGHT), `lv.OPA_COVER`, `lv.OPA_TRANSP`,
  `lv.STATE_DEFAULT`, `lv.STATE_FOCUSED`, `lv.EVENT_KEY`, `lv.EVENT_CLICKED`,
  `lv.EVENT_FOCUSED`, `lv.OBJ_FLAG_HIDDEN`, `lv.OBJ_FLAG_SCROLLABLE`,
  `lv.LABEL_LONG_WRAP`, `lv.LABEL_LONG_SCROLL_CIRCULAR`, `lv.TEXT_ALIGN_CENTER`,
  `lv.TEXT_ALIGN_LEFT`, `lv.RADIUS_CIRCLE`, `lv.FLEX_FLOW_ROW/COLUMN`,
  `lv.FLEX_ALIGN_CENTER/START`, `lv.ANIM_ON/OFF`, `lv.ANIM_REPEAT_INFINITE`
- `audio.` load (1er argument `0`), play, stop, pause, get_status (utilisés par
  `global.lua` ; règles d'emploi §8)
- `Global.` load_module, load_image, setBackToLibrary, setBackBehavior,
  setBackModule, requestAudioPlay, requestAudioStop, cleanCurrentModule,
  visual_width, visual_height, screen_width, screen_height, header_height
