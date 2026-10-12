# Pièges device vs simulateur

Le simulateur `flam-player` est plus permissif que la Flam : « marche au simulateur »
ne prouve rien. Chaque point ci-dessous a coûté un aller-retour sur le device.
✅ = confirmé sur device, 🔶 = fortement suspecté.

| # | Piège | Symptôme device | Parade |
|---|---|---|---|
| 1 ✅ | `require` ne cherche que `script/<nom>.lua` (le simulateur cherche aussi la racine) | histoire qui ne charge pas | tous les `.lua` hors `main.lua` dans `script/`, à plat |
| 2 ✅ | `require` au top-level de `main.lua` | ne charge pas | tout `require` dans `setup()` |
| 3 ✅ | UI dessinée hors module `Global.load_module` | écran noir, touches ignorées | jeu = module `create/clean` |
| 4 ✅ | framework Lunii absent (`global.lua`, `button*`, `progressionManager`, `v-scroll`, `v-container`…) | `module 'global' not found` | `flam_build.py` embarque tout `assets/runtime` |
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
| 15 | timer non détruit dans `clean()` | erreurs sur objets supprimés, gel | `lv.timer.del` dans `clean()` |
| 16 | ZIP compressé / data descriptor / doublon de casse | refus du simulateur ; écart avec les paquets éprouvés ; fichiers écrasés sur la FAT | `flam_build.py` (STORED, noms vérifiés) |
| 18 | runtime Lunii différent de celui éprouvé | comportements non testés sur device | garder `assets/runtime` tel quel (octets du casse-briques validé sur device) |
| 17 | UUID déjà présent sur la Flam | import refusé (« already loaded ») | `uuid_seed` unique ; supprimer l'ancienne version avant réimport |

## Relance (piège n° 10)

Le bug d'écran noir au 2ᵉ lancement venait d'histoires qui sortaient vers la
bibliothèque **depuis une scène active** (audio-player + progression sauvegardée).
Les histoires officielles reviennent d'abord à leur menu, puis à la bibliothèque.

Pour un jeu sans progression ni audio (cas du casse-briques) : `Global.setBackToLibrary()`
posé une fois dans `setup()`, `clean()` qui détruit timers et callbacks. Pour un jeu avec
audio ou plusieurs écrans : faire revenir le bouton retour à l'écran d'accueil du jeu
(`Global.setBackBehavior`), et ne sortir vers la bibliothèque que depuis cet accueil.

**Un hard reboot masque ce bug** : le premier lancement après redémarrage marche toujours.
Tester systématiquement : lancer → retour bibliothèque → relancer (2 fois de suite),
**sans** redémarrer la Flam.

## Ce que le simulateur ne vérifie pas

- mémoire (tas LVGL du device non mesuré, Lua sans plafond au simulateur) ;
- format réel de `state` (sauvegarde `.save` binaire sur device, `.lua` au simulateur) ;
- comportement à la relance (piège 10) ;
- fluidité réelle (le CPU du device est bien plus lent : rester sobre dans `tick`).

Le test sur device reste donc obligatoire avant de déclarer un homebrew fonctionnel.
