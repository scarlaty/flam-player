# Issue : le simulateur `flam-player` est trop permissif vs le device réel

Contexte : en développant un convertisseur TELMI → FLAM, j'ai produit des histoires
qui **fonctionnent parfaitement dans le simulateur** mais **échouent sur le device réel**
(ne chargent pas, écran noir, gel). Chaque divergence a coûté un aller-retour de test
sur device. Ce document liste les restrictions du device que le simulateur **n'applique
pas**, pour rendre le simulateur fidèle (et détecter ces problèmes en local).

Légende : ✅ confirmé sur device · 🔶 fortement suspecté · ⬜ en cours d'investigation.

---

## 1. ✅ `require()` n'est résolu que depuis `script/` (pas la racine)

- **Device** : le searcher Lua ne cherche les modules que dans `script/`.
- **Simulateur** (`src/main.c`, `custom_lua_searcher`) : cherche dans `script/` **ET**
  à la racine (`dirs[] = { "script", "." }`).
- **Symptôme** : un `require("nodes")` qui résout `nodes.lua` à la racine fonctionne au
  sim, échoue sur device → l'histoire ne charge pas.
- **Correctif sim** : retirer `"."` de la liste des dossiers de recherche (ne garder
  que `script`).

## 2. ✅ Aucun `require()` au top-level de `main.lua`

- **Device** : le searcher `require` n'est installé qu'**après** le chargement de
  `main.lua`. Un `require(...)` exécuté au chargement du module (top-level) échoue →
  `main.lua` n'est jamais défini → l'histoire ne charge pas.
- **Simulateur** : `set_lua_package_path()` est appelé **avant** `load_script(main.lua)`,
  donc un `require` top-level fonctionne.
- **Symptôme** : `main.lua` avec `local X = require("...")` en tête charge au sim, pas
  sur device. Les histoires officielles ne font jamais de `require` top-level (tout est
  dans `setup()`/fonctions).
- **Correctif sim** : installer le searcher **après** un premier `pcall(loadfile main.lua)`
  / reproduire l'ordre du firmware (charger main.lua, PUIS configurer require, PUIS
  appeler `setup()`).

## 3. ✅ L'UI doit passer par les modules `Global` (`Global.current_module`)

- **Device** : le firmware rend l'écran et route les entrées **uniquement** via le
  module courant chargé par `Global.load_module(...)`. Dessiner directement en `lv.*`
  ne s'affiche pas.
- **Simulateur** : rend tout l'arbre LVGL directement → un moteur qui dessine en `lv.*`
  fonctionne visuellement au sim mais reste invisible sur device.
- **Symptôme** : moteur custom basé sur `lv.obj/img/btn` → OK sim, écran noir device.
- **Correctif sim** : difficile à simuler fidèlement ; a minima documenter que seul le
  contenu créé via les modules standard est « officiellement » rendu.

## 4. ✅ Bibliothèque standard Lunii obligatoire dans `script/`

- **Device** : `global.lua` (+ `button`, `progressionManager`, `v-scroll`,
  `v-container`, `button-theme*`) et les modules (`title-card`, `audio-player`,
  `list-choice`, `image-choice`, …) doivent être présents dans `script/`. Le firmware /
  `main.lua` fait `Global = require("global")`.
- **Simulateur** : fournit les primitives (`lv`, `audio`, `window`, `document`, `state`,
  `progression`, `context_menu`) mais aucune erreur si ces scripts manquent (tant qu'on
  ne les `require` pas).
- **Correctif sim** : aucun (comportement attendu) — mais utile à documenter.

## 5. ✅ Assets UI requis dans `img/script/`

- **Device** : les modules chargent des sprites via `Global.load_image("script/...")`
  résolu en `img/script/...` :
  - `list-choice` → `script/arrow-right-ui-000.lif`
  - `global`/`audio-player` → `script/audio-player-pause.lif`, play/pause, etc.
- **Simulateur** : `lv.img_src.load()` (cf. `lif_decoder.c`) renvoie `NULL`
  **sans erreur** si le fichier manque → le sim continue ; le device, lui, plante ou
  gèle (ou `Global.load_image` lève une erreur sur `get_width(nil)`).
- **Symptôme** : menu `list-choice` sans la flèche → `get_coords(nil)` → crash.
- **Correctif sim** : faire échouer/avertir bruyamment quand un asset référencé est
  absent (au lieu de renvoyer `nil` silencieusement), pour reproduire le comportement
  device.

## 6. ✅ `list-choice.addEntry` exige `audio` (string) par entrée

- **Device + module** : `addEntry(label, img, audio, cb, slider)` **rejette** l'entrée
  si `label`/`img`/`audio`/`cb` ne sont pas tous des bons types (audio = **string**).
  Sans `audio`, l'entrée est ignorée → menu vide → `enableSelection()` →
  `get_coords(nil)` → crash.
- **Simulateur** : reproduit le crash (c'est dans le module), mais seulement si on
  atteint ce code — facile à manquer sans tester un menu.
- **Correctif** : documenter que toute entrée de menu doit avoir un `audio` (un MP3
  silencieux suffit). (Ce n'est pas une divergence sim/device mais un piège fréquent.)

## 7. 🔶 Initialisation de l'alpha dans le décodeur LIF

- **Device (suspecté)** : initialise l'alpha courant à `0` (ou différemment du repo).
  Une image opaque encodée avec le 1er pixel en `0xFE` (couleur sans alpha → garde
  l'alpha courant) devient **transparente → noire** à l'affichage.
- **Simulateur** (`lif_decoder.c`) : initialise `ca = 255` → l'image s'affiche.
- **Correctif** : encoder le **1er pixel en `0xFF`** (couleur + alpha explicites). À
  vérifier côté device pour confirmer la divergence d'init.

## 8. 🔶 `audio.load()` d'un fichier absent/échoué déclenche le callback `"stop"`

- **Device (suspecté)** : un `audio.load` qui échoue semble déclencher immédiatement le
  callback audio `"stop"` → les scènes en autoplay « défilent » instantanément
  (racing).
- **Simulateur** (`sdl_audio.c`) : un `audio.load` échoué laisse l'état à `STOP` et ne
  déclenche **pas** le callback `"stop"` (émis uniquement en fin de lecture réelle dans
  `sdl_audio_pump`).
- **Correctif sim** : émettre un callback `"stop"`/erreur si `audio.load` échoue, pour
  reproduire le racing.

## 9. ⬜ Persistance de `state` incohérente / non fidèle

- **Device** : `state` (dont `current_fun`, `visited_funs`, `current_choice`, `inv`)
  semble parfois persisté entre deux lancements, parfois rechargé vide. Format binaire
  `.save` (ex. `state.save`, `prog_chaps.save`, `debug.save`) ≠ format texte Lua du sim.
- **Simulateur** : persiste `state` en `.lua` (texte) de façon fiable
  (`fw_globals.c`, `save_lua_table`), fichiers `state.lua` / `prog_<key>.lua`.
- **Symptôme** : la logique de reprise (`isStoryStarted`, `current_fun`) se comporte
  différemment device vs sim.
- **Correctif sim** : aligner le format/chemin de sauvegarde sur le device (`.save`,
  `/usr/0/{uuid}/`) ou au moins documenter la différence.

## 10. ✅ Relance d'une histoire : écran noir figé — RÉSOLU

**Symptôme (device)** : 1ʳᵉ ouverture = tout marche (title-card, son, narration, choix).
On quitte (bouton retour) → on relance → **écran noir figé, sans son, sans input**, dès
le démarrage (la title-card ne s'affiche même pas). **Reset de la progression** sur le
device → ça remarche (comme une 1ʳᵉ ouverture). Reset hard (extinction) → remarche aussi.

**Ce qui est établi (ce n'est NI le firmware NI LVGL)** :
- Les histoires **officielles** (Cluedo) relancent correctement → pas le firmware.
- Le **simulateur** relance correctement → pas LVGL.
- D'**autres** histoires gardent le son après le noir → l'audio device n'est pas corrompu.
- ⇒ C'est l'interaction **état sauvé de l'histoire convertie ↔ relance** côté device.

**Instrumentation (logs SD `debug.save`)** — à la relance :
- `setup()` s'exécute jusqu'au bout (Lua tourne), le pump audio tourne,
- mais **aucun `tick` heartbeat** → `lv_timer_handler` ne tourne PAS → écran jamais
  rafraîchi (noir) + timers LVGL morts + `"stop"` audio prématuré (`seconds≈0`) qui,
  sans la garde, fait défiler toutes les scènes (racing).

**Pistes ÉLIMINÉES (chaque correctif testé sur device → toujours noir)** :
- logging massif (saturation écritures SD) — testé sans logging.
- `audio.stop()` au démarrage (nettoyer un état "pause") — sans effet.
- sortie propre `setBackBehavior` (stop audio + cleanCurrentModule au quit) — sans effet.
- garde anti-racing (ignorer `"stop"` à `seconds<0.5`) — élimine le racing mais reste noir.
- désactivation de la sauvegarde de progression — sans effet.
- **réplication complète du resume officiel (Cluedo)** : architecture branch
  (`script/story.lua` + `loadBranch` + `current_branch[fn]()`), champs d'état STANDARD
  (`current_fun` + `currentBranchName` via `setProgression`), chapitres (`ischapter`)
  donnant `.prog > 0` — **toujours noir**.

**Comparaison fichiers de save** (`/usr/0/{uuid}.save` + `{uuid}.prog`) :
- Mon état (après réplication) : `current_fun`, `currentBranchName="story"`,
  `visited_funs`, `inv={}`, `registeredChoices={}`, `.prog=1`.
- Cluedo : état riche (`achievements`, `inventory` imbriqués…), `.prog=78`.
- Le **contenu** diffère encore (Cluedo a des données métier riches), mais aucune
  différence structurelle réplicable restante n'a corrigé le bug.

**Hypothèses restantes (non vérifiées, hors portée du Lua de l'histoire)** :
- `library.cache` (registre device, ~195 Ko) : l'entrée de l'histoire convertie y serait
  mal formée → le device mishandle au 2ᵉ lancement.
- Désérialisation de `{uuid}.save` par le firmware au 2ᵉ lancement qui corrompt l'état
  runtime (la 1ʳᵉ écriture est OK, la 2ᵉ lecture casse le rendu).
- Une donnée d'état spécifique (ex. `seekposition` flottant écrit par `audio-player`,
  ou un champ custom) qui passe le 1er cycle mais casse au rechargement.

### RÉSOLUTION (debug sur device, fichiers `/usr/0/{uuid}.save` + `.prog` lus directement)

Le symptôme global recouvrait **trois** problèmes distincts. Deux étaient côté histoire et
sont **corrigés** ; le troisième est **firmware** et reste ouvert.

**`.prog` = FAUX coupable.** Le test « forcer `.prog=50` → ça boote » était trompeur : on
faisait un **hard reboot** pour tester, or le 1er lancement après hard reboot marche
**toujours**, quel que soit `.prog`. Vérifié ensuite : `.prog=50` → toujours noir au 2e
lancement soft. Les histoires officielles ont `.prog>0` simplement parce qu'elles utilisent
des **chapitres grossiers** (Cluedo : `totalChapters=14`) ; le converti utilisait
`totalChapters` = nb de scènes → 1ʳᵉ scène = 0 %. Cosmétique, pas la cause du noir.

**Bug A — seek pendant la lecture gèle le device (CORRIGÉ).** `audio.seek()` appelé alors
que l'audio **joue** fige l'appareil (écran noir, `lv_timer_handler` mort). Deux sites dans
`audio-player_1_0_0.lua` : seek au curseur (`inactivityCb`) et seek de reprise
(`audioFeedback`, restauration de `seekposition`). C'est ce **seek de reprise** qui figeait
au 2e lancement quand on reprenait au milieu d'une scène. **Fix** : séquence validée device
`audio.pause()` → `audio.seek()` → `audio.play()` aux deux endroits.

**Bug B — fin de scène ne passait pas à la suite (CORRIGÉ).** Une garde anti-racing ajoutée
(`if seconds < 0.5 then return` sur le `"stop"`) mangeait le `"stop"` de **fin légitime**.
Le « racing » qu'elle combattait venait en fait du seek-en-lecture (Bug A). **Fix** : garde
retirée (= comportement Cluedo) → la fin de scène déclenche bien la transition.

**Bug C — écran noir au 2e lancement (RÉSOLU — CAUSE RACINE).**
Notre `story.lua` forçait `Global.setBackBehavior(function() goto_library() end)` dans chaque
scène/choix → le bouton **retour** sortait **directement** d'une scène active vers la
bibliothèque. Les histoires officielles ne font **jamais** ça : retour → d'abord le **menu de
l'histoire** (Start), puis un 2e retour → bibliothèque (scène → menu → bibliothèque). Cette
sortie **brutale** (scène active → `goto_library`) laisse, sur device, un état runtime qui
**fige le lancement suivant en écran noir**.

**Comment isolé** : un **Cluedo trimmé** (tout le Lua officiel INCHANGÉ, assets réduits au
chemin intro+2 branches, re-zippé + réinstallé via le même outil, nouvel uuid) relance
**sans problème**. ⇒ le déclencheur est dans **NOTRE Lua généré**, pas l'install, le
packaging, les modules moteur ni les assets (tous testés et éliminés un par un). La seule
divergence comportementale visible = la gestion du retour (officiel = retour en 2 étapes ;
converti = retour direct bibliothèque) — pointée à l'œil sur device.

**Fix** : retirer les `setBackBehavior(goto_library)` de `enterStage`/`showChoice`. Le retour
suit alors le comportement posé par `setProgression` (→ menu Start, car `progression.create`
a `backBehavior = Start`), exactement comme les officielles. `exitCb` de l'image-choice
pointe sur `back_callback` (menu) au lieu de `goto_library`. **Confirmé sur device** : plus
d'écran noir, et le retour fait bien scène → menu → bibliothèque.

**Bonus conservé** : `engine/main.lua` enregistre une entrée de menu contextuel
**« Reprendre l'histoire »** (`context_menu.set_entries`), absente avant. Plus nécessaire
comme contournement (le bug est corrigé) mais c'est une amélioration utile, alignée sur les
histoires officielles.

**Pistes ÉLIMINÉES au passage** (toutes testées sur device, toujours noir tant que Bug C
présent) : taille (`nodes.lua` monolithique), structure interpréteur vs branches officielles,
`.mp3map`, modules moteur (= Cluedo), images LIF, audio (= Cluedo), packaging/ZIP, `.prog`
(faux coupable — le hard reboot des tests masquait tout). Voir l'historique des `MiniTest*`.

---

## Métadonnées / divers (informatif)

- `version` : le device accepte `"1"` et `"2"`.
- `info.plain` : format `titre\nsous-titre\n000000\ntitre` (1ʳᵉ ligne = titre lu par la
  bibliothèque).
- `uuid.bin` : 16 octets bruts.
- Conteneur : ZIP **stored** (méthode 0). Le device lit les **local file headers**
  séquentiellement (cf. `pk_reader.c`) — pas de data descriptors (flag bit 3).
- Save dir device : `/usr/0/{uuid}/` (et chemins de save en `.save`).
- `title-card` avance en appelant `back_callback()` à la fin de son audio (pas un `cb`
  direct) — le caller doit donc `setBackBehavior(suite)` avant de l'afficher.
- Bouton retour = sortie propre vers la bibliothèque (`goto_library` /
  `setBackToLibrary`), pas la transition « home » TELMI.

---

## Recommandation

Rendre le simulateur **strict par défaut** (mêmes contraintes que le device) pour que
« marche au sim » ⇒ « marche sur device ». Prioriser : #1, #2, #5, #8 (les plus
trompeuses et faciles à corriger côté sim).
