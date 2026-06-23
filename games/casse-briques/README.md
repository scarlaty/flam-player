# Casse-briques (jeu FLAM)

Casse-briques jouable sur le lecteur FLAM (sim + device), **adapté** d'un jeu LÖVE2D.
100 % **contenu Lua** : aucun changement firmware, packagé comme une histoire (`.plain.pk`).

## Contrôles
- **Molette ← / →** : déplace la raquette (pas-à-pas, `STEP` px par clic).
- **Centre / ENTER** : lance la balle ; rejoue après une fin de partie.
- **Retour / Échap** : retour à la bibliothèque.

## Verrou d'accès (contrôle parental)
Le jeu démarre **verrouillé** (écran « Accès parent ») et le code est **redemandé à chaque
partie**.
- Code = **suite de directions molette uniquement** (le device n'a pas de clic centre dédié).
  Validation **automatique** par match de préfixe (bonne touche avance, mauvaise remet à zéro).
- Témoins : N points se remplissent selon le préfixe correct (le code n'est pas affiché).
- Défaut : **→ → ← ← → ←**. Modifiable via `LOCK_CODE` en haut de
  `script/breakout_1_0_0.lua` (`19` = droite, `20` = gauche).

## Build
```
python build.py            # -> _out/casse-briques.plain (sim) + .plain.pk (device)
python build.py -o sortie.plain.pk --plain
```
Le builder réutilise le framework runtime de `tools/telmi2flam/runtime` (global.lua, modules,
assets UI) et y ajoute `main.lua` + `script/breakout_1_0_0.lua` + métadonnées.

## Test
- **Simulateur** : `build/flam-player.exe "games/casse-briques/_out/casse-briques.plain"`
- **Device** : flasher `_out/casse-briques.plain.pk`, hard reboot.

## Architecture
- `main.lua` : bootstrap, `setup()` → `Global.load_module("breakout","1_0_0").create({})`.
- `script/breakout_1_0_0.lua` : le jeu **en module** (`create()`/`clean()`), deux phases
  `lock` puis `play`. Briques/raquette/balle = objets `lv.obj` (pas de canvas), boucle de jeu
  via `lv.timer` (~25 ms), collisions AABB.

> **Important (device)** : le jeu DOIT être un **module** chargé par `Global.load_module`.
> Le firmware ne rend/route que le module courant — une branche qui dessine directement en
> `lv.*` marche au simulateur mais pas sur device (cf. `tools/telmi2flam/DEVICE_VS_SIM.md` §3).

## Limites / suite possible
- Pas d'audio (sons de rebond/perte = étape suivante, mp3 + `.mp3map`).
- Contrôle raquette **pas-à-pas** (choisi pour la molette) ; à ajuster à l'usage (`STEP`).
- Pas d'écrans menu/fin dédiés (message in-game).
