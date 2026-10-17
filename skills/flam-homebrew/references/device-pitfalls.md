# Pièges device vs simulateur

Le simulateur `flam-player` est plus permissif que la Flam : « marche au simulateur »
ne prouve rien. Chaque point ci-dessous a coûté un aller-retour sur le device.
✅ = confirmé sur device, 🔶 = fortement suspecté.

| # | Piège | Symptôme device | Parade |
|---|---|---|---|
| 1 ✅ | `require` ne cherche que `script/<nom>.lua` (le simulateur cherche aussi la racine) | histoire qui ne charge pas | tous les `.lua` hors `main.lua` dans `script/`, à plat |
| 2 ✅ | `require` au top-level de `main.lua` | ne charge pas | tout `require` dans `setup()` |
| 3 ✅ | UI dessinée hors module `Global.load_module` | écran noir, touches ignorées | jeu = module `create/clean` |
| 4 ✅ | runtime du projet absent (`global.lua`, `button*`, `progressionManager`, `v-scroll`, `v-container`…) | `module 'global' not found` | `flam_build.py` embarque tout `assets/runtime` |
| 5 ✅ | asset UI ou image absente (`img/script/*.lif`, `Global.load_image` sur un fichier manquant) ; le simulateur renvoie `nil` sans erreur | plantage ou gel | `flam_check.py` vérifie les littéraux ; `--strict` au simulateur |
| 6 ✅ | entrée `list-choice` sans `audio` (chaîne) | menu vide puis crash | `audio = "silent.mp3"` |
| 7 🔶 | 1er pixel LIF sans alpha explicite | image opaque affichée noire | encoder avec `lif.py` (1er pixel en opcode 0xFF) |
| 8 🔶 | `audio.load` en échec déclenche aussitôt `"stop"` | enchaînements instantanés | fichiers présents + `.mp3map` ; ne pas enchaîner sur `"stop"` sans garde de logique |
| 9 ✅ | `audio.seek` pendant la lecture | gel complet (écran noir, timers morts) | `audio.pause()` → `audio.seek(s)` → `audio.play()` |
| 10 ✅ | sortie brutale vers la bibliothèque depuis une scène d'histoire (audio + progression en cours) | écran noir figé au **2ᵉ** lancement | voir « Relance » ci-dessous |
| 11 | caractères hors ASCII/Latin-1 | rectangles vides | `font_safe` de `flam_build.py` pour le titre ; vérifier les chaînes du jeu |
| 12 | `io`, `os`, `debug` | erreur Lua (sandbox) | ne pas les utiliser |
| 13 🔶 | image portrait > 64 px de haut (heuristique de transposition du décodeur du simulateur ; non établie sur device, précaution) | image transposée | canevas carré (fait par `flam_build.py`) |
| 14 | `lv.img` plus grand que sa source | image répétée en mosaïque | taille de l'objet = taille de l'image |
| 15 | timer non détruit dans `clean()` | erreurs sur objets supprimés, gel | `lv.timer.del` dans `clean()` (mais pas à la sortie vers la bibliothèque : piège 24) |
| 16 | ZIP compressé / data descriptor / doublon de casse | refus du simulateur ; écart avec les paquets éprouvés ; fichiers écrasés sur la FAT | `flam_build.py` (STORED, noms vérifiés) |
| 18 | runtime différent de celui éprouvé | comportements non testés sur device | garder `assets/runtime` tel quel (version validée sur device avec le casse-briques) |
| 17 | UUID déjà présent sur la Flam | import refusé (« already loaded ») | `uuid_seed` unique ; supprimer l'ancienne version avant réimport |
| 19 ✅ | clic molette attendu en `EVENT_KEY` 10 (la molette est un encodeur : sur un objet non défilable, le clic arrive seulement en `EVENT_CLICKED`) | clic sans effet (simulateur et device) | écouter `EVENT_KEY` **et** `EVENT_CLICKED`, un appui compté une fois (`lua-api.md` §4) |
| 20 ✅ | `audio.stop()` / `audio.pause()` d'une piste en cours | image figée un instant à chaque bruitage qui coupe la musique (invisible au simulateur) | pas de musique pendant l'action ; bruitages qui attendent la fin naturelle (`lua-api.md` §8) |
| 21 ✅ | `audio.load` pendant qu'une piste joue, sans stop | **gel total**, redémarrage obligatoire | `audio.load` seulement si `audio.get_status() == "stop"` |
| 22 ✅ | `audio.load(1, ...)` pris pour un 2ᵉ canal | la musique s'arrête, le nouveau son joue : pas de mixage | une seule piste ; premier argument `0` comme dans les scripts du runtime |
| 23 ✅ | écriture dans `state` (ex. record) | écran noir à la relance, réparé en effaçant la progression | aucune écriture dans `state` pour un jeu (`lua-api.md` §11) |
| 24 ✅ | sortie vers la bibliothèque qui vide l'écran (`clean`, `cleanCurrentModule`) ou supprime le timer | relance = écran noir, ou écran affiché mais touches mortes | à la relance la Flam réaffiche l'état laissé : couper seulement le son, puis `goto_library()` (« Relance » ci-dessous) |

## Relance (pièges n° 10, 23 et 24)

**Fait établi sur device** : relancer un homebrew depuis la bibliothèque ne réexécute
pas `setup()`. La Flam réaffiche l'écran et l'état Lua (timers, callbacks, variables)
laissés au moment de `goto_library()`. D'où :

- sortie qui vide l'écran (`setBackBehavior(function() goto_library() end)`, qui appelle
  `cleanCurrentModule`) → relance sur un écran vide = **écran noir** ;
- sortie qui supprime le timer de jeu → écran réaffiché mais **figé** (les délais comptés
  en ticks n'avancent plus, les touches sont ignorées) ;
- écriture dans `state` → écran noir à la relance, réparé seulement en effaçant la
  progression dans les paramètres de la Flam.

Schéma qui marche (casse-briques, Capi et la rivière) :

- écran d'accueil du jeu : `back_callback` qui coupe le son puis `goto_library()`, sans
  rien nettoyer (ou `Global.setBackToLibrary()` pour un jeu sans son) ;
- partie en cours : retour → écran d'accueil (`Global.setBackBehavior(fn)`), jamais
  directement la bibliothèque ;
- aucune écriture dans `state`.

Les histoires (audio + progression) suivent la même logique : retour scène → menu de
l'histoire → bibliothèque, comme les histoires officielles.

**Un hard reboot masque ces bugs** : le premier lancement après redémarrage marche
toujours. Tester systématiquement : lancer → retour bibliothèque → relancer (2 fois de
suite), **sans** redémarrer la Flam, depuis l'accueil puis depuis une partie.

## Ce que le simulateur ne vérifie pas

- mémoire (tas LVGL du device non mesuré, Lua sans plafond au simulateur) ;
- format réel de `state` (sauvegarde `.save` binaire sur device, `.lua` au simulateur) ;
- comportement à la relance (pièges 10, 23, 24 : le simulateur relance toujours à neuf) ;
- coût réel de l'audio (pièges 20, 21 : stop, pause et load y sont instantanés) ;
- fluidité réelle (le CPU du device est bien plus lent : rester sobre dans `tick`).

Le test sur device reste donc obligatoire avant de déclarer un homebrew fonctionnel.
