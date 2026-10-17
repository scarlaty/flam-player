---
name: flam-homebrew
description: Fabrique un jeu, une histoire interactive ou une petite application homebrew pour le lecteur Lunii FLAM, packagé en .plain.pk prêt à importer (Lua + LVGL, images LIF, audio MP3/mp3map). À utiliser dès que l'utilisateur veut créer, porter, modifier, packager ou déboguer un contenu maison pour la Flam ou la Lunii Flam — jeu molette, quiz, mini-jeu, histoire faite maison, écran Lua, fichier .plain/.plain.pk, conversion PNG→.lif ou MP3→.mp3map — même s'il ne prononce pas le mot « homebrew ». Ne couvre pas la conversion d'histoires TELMI (telmi2flam), ni l'import de contenus d'un autre compte, ni toute modification ou contournement du firmware.
---

# Homebrew FLAM (.plain.pk)

Un homebrew FLAM est un paquet `.plain.pk` (ZIP non compressé) contenant un `main.lua`,
le code du jeu en **module Lua**, le runtime Lua du projet embarqué, des images `.lif` et de
l'audio `.mp3`. Le firmware exécute ce Lua dans une sandbox LVGL (`lv.*`) sur un écran
320×240 piloté par une molette.

Le simulateur PC accepte beaucoup de choses que le device refuse. Ce skill encode les
règles qui font qu'un paquet **marche aussi sur la vraie Flam** : suivre le flux et
garder les scripts fournis plutôt que réassembler à la main.

Périmètre : contenus maison légitimes. Pas d'exploit, pas de contournement du
firmware, pas d'import de contenus d'un autre compte.

## Contenu du skill

| Fichier | Rôle |
|---|---|
| `scripts/new_project.py` | crée un projet (flam.json, main.lua, module de démo) |
| `scripts/flam_build.py` | assemble le `.plain.pk` (+ dossier `.plain` pour le simulateur) et lance le check |
| `scripts/flam_check.py` | valide un `.plain.pk` (structure, Lua, assets, pièges device) |
| `scripts/lif.py`, `scripts/mp3map.py` | encodeurs image et table audio (utilisés par le build) |
| `assets/runtime/` | runtime Lua du projet à embarquer, version validée sur device avec le casse-briques (copié par le build, ne pas modifier ni remplacer par une autre version) |
| `assets/template/` | gabarits de `main.lua` et du module |
| `references/lua-api.md` | **à lire avant d'écrire du Lua** : contrat de module, molette, dessin, timers, images, audio, polices, liste blanche d'API |
| `references/device-pitfalls.md` | pièges device vs simulateur, protocole de test |
| `references/format.md` | format du paquet, UUID, noms, import Lunii.QT |
| `references/assets.md` | images (tailles, LIF) et audio (MP3, mp3map) |

Prérequis : Python 3.8+ ; Pillow si le projet contient des PNG
(`python -m pip install pillow`). Pour vérifier la syntaxe Lua : `python -m pip install
lupa`, ou un `luac`/`lua` 5.4 dans le PATH, ou `FLAM_LUA=<chemin>`. Si rien de tout ça
n'est installable (ex. Python MSYS/mingw), le check le signale en WARN et le lancement
au simulateur `--strict` (étape 6) sert de contrôle de syntaxe.

## Flux de travail

### 1. Cadrer le jeu pour la molette

Trois entrées seulement : molette gauche (20), droite (19), **clic molette** (= ENTER,
reçu en `EVENT_CLICKED`, pas en `EVENT_KEY` 10 : voir `lua-api.md` §4), plus le bouton
retour géré par le framework. Pas de touche maintenue : un cran = un pas. Doubler les
actions du clic aux directions quand c'est naturel (ex. un cran relance après 1 s).
Zone de jeu 320×212. Texte court, accents Latin-1 seulement.

**Son : une seule piste, pas de mixage**, et sur la Flam couper une piste en cours fige
l'image (`lua-api.md` §8). Si la demande veut « musique + bruitages » pendant l'action,
annoncer le compromis avant de coder : par défaut, bruitages seuls pendant la partie,
musique aux écrans calmes (titre, fin).

Si la demande ne tient pas dans ces contraintes, proposer une adaptation (tour par tour,
pas discrets, choix gauche/droite) avant de coder.

### 2. Créer le projet

```bash
python <skill>/scripts/new_project.py <dossier> --title "Mon jeu" [--subtitle "Jeu"] [--module monjeu]
```

Le projet contient `flam.json` (titre, `uuid_seed` aléatoire unique, module, version),
`main.lua` (bootstrap correct, rarement à modifier), `script/<module>_<version>.lua`
(démo à remplacer), `img/`, `sounds/`. Garder `uuid_seed` stable pour les rebuilds du
même jeu.

### 3. Écrire le jeu

Lire `references/lua-api.md` d'abord. Partir du module de démo, il montre le squelette
complet. Les règles qui comptent le plus :

- **Le jeu est un module** chargé par `Global.load_module(nom, version)` : le device ne
  rend que le module courant. Un code qui dessine en `lv.*` hors module marche au
  simulateur et donne un écran noir sur la Flam.
- `main.lua` : aucun `require` hors de `setup()` ; `Global` global (pas `local`).
- `create(args)` construit, `clean()` détruit tout (timers, callbacks, groupe, objets),
  sauf à la sortie vers la bibliothèque (voir « Retour » ci-dessous).
- Entrées : conteneur ajouté à `document` + `lv.group.set_editing(document, true)` ;
  `lv.EVENT_KEY` pour la molette (`string.byte(lv.event.get_key_value(e))`) **et**
  `lv.EVENT_CLICKED` pour le clic, un appui compté une fois.
- Audio : `audio.load` seulement quand `audio.get_status() == "stop"` (sinon gel) ;
  aucun `stop`/`pause` pendant l'action (à-coup).
- Retour : partie → écran d'accueil du jeu → bibliothèque. Pour sortir, couper le son
  puis `goto_library()` **sans** rien nettoyer : la relance réaffiche l'état laissé.
  Aucune écriture dans `state` (écran noir à la relance).
- Boucle : `lv.timer.new(tick, 25, nil)` ; déplacer des objets créés une fois ;
  `lv.obj.clear_flag(container, lv.OBJ_FLAG_SCROLLABLE)` si des objets débordent.
- Ordre de création = ordre d'affichage : créer le HUD (score, messages) en dernier.
- Rester dans la liste blanche d'API (`lua-api.md` §12). Pas de `io`/`os`/`debug`.
- Images via `Global.load_image("x.lif")` (relatif à `img/`), audio via
  `Global.requestAudioPlay({path="x.mp3"})` (relatif à `sounds/`).
- Plusieurs écrans : un module par écran, ou un module avec des phases internes.
- Fichiers `.lua` en UTF-8 sans BOM, dans `script/`, à plat.

### 4. Ajouter les assets

PNG dans `img/` (décor plein écran 320×212, sprites petits et non portrait), MP3
44,1 kHz dans `sounds/`, `img/thumbnail.png` pour la vignette. Le build convertit.
Voir `references/assets.md` pour générer des images par code.

### 5. Builder et valider

```bash
python <skill>/scripts/flam_build.py <dossier>
```

Écrit `<dossier>/_out/<slug>.plain.pk` et le dossier `<dossier>/_out/<slug>.plain/`
(slug = titre sans accent, en minuscules, ex. `attrape-etoiles` ; le chemin exact est
affiché sur les lignes `OK ->`), puis lance `flam_check.py`. Dans une console Windows,
les accents peuvent s'afficher en `?` : c'est la console, pas le paquet
(`PYTHONIOENCODING=utf-8` pour un affichage propre). **Zéro `[ERR]` exigé.** Lire chaque `[WARN]` : la plupart signalent un
piège device réel (texte hors police, timer jamais détruit, `audio.seek`…). Corriger
puis rebuilder. Si la syntaxe Lua n'a pas pu être vérifiée, l'installer (`lupa`) ou le
dire explicitement à l'utilisateur.

### 6. Tester au simulateur (si le dépôt flam-player est disponible)

Contrôle automatique (sans fenêtre) : démarrage, absence d'erreur Lua, capture d'écran.

```bash
SDL_RENDER_DRIVER=software SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy \
FLAM_SCREENSHOT_AUTO_MS=3000 timeout 6 <flam-player>/build/flam-player.exe --strict \
  --screenshot capture.bmp <dossier>/_out/<slug>.plain
python -c "from PIL import Image; Image.open('capture.bmp').save('capture.png')"
```

- `--strict` rapproche le simulateur du device (`require` limité à `script/`, image
  absente = erreur Lua).
- Code retour 124 = arrêt par `timeout`, normal. L'arbre d'objets LVGL et les lignes
  `global.lua:151: plot: ...` sont du journal normal ; chercher `error`/`traceback`.
- Regarder `capture.png` : HUD lisible, objets à leur place, rien de masqué.
- Le mode automatique n'envoie **aucune touche** : molette, ENTER, rejouer et retour ne
  se testent qu'en interactif (même commande sans les variables `SDL_*` ni `timeout` ;
  flèches = molette, Entrée = ENTER, Échap = retour). Le dire à l'utilisateur plutôt
  que d'annoncer les contrôles comme testés.

Même chose en PowerShell :

```powershell
$env:SDL_RENDER_DRIVER='software'; $env:SDL_VIDEODRIVER='dummy'; $env:SDL_AUDIODRIVER='dummy'
$env:FLAM_SCREENSHOT_AUTO_MS='3000'
$p = Start-Process <flam-player>\build\flam-player.exe -PassThru -NoNewWindow `
     -ArgumentList '--strict','--screenshot','capture.bmp','<dossier>\_out\<slug>.plain'
Start-Sleep 6; Stop-Process $p -ErrorAction SilentlyContinue
```

Le simulateur ne remplace pas le device (`references/device-pitfalls.md`).

### 7. Installer et tester sur la Flam

Import avec Lunii.QT (fork `scarlaty/Lunii.QT`) : *Import* classique sur firmware 2.x ;
**import carrier** sur firmware 1.x (l'import classique y donne un titre illisible ;
fonction expérimentale, validée sur une seule Flam).
Protocole à donner à l'utilisateur :

1. le titre s'affiche lisiblement dans la bibliothèque ;
2. le jeu se lance, l'écran n'est pas noir, la molette répond ;
3. le clic molette fait son action ; retour → accueil du jeu → bibliothèque ;
4. relancer **deux fois de suite sans redémarrer la Flam** (un redémarrage masque le
   bug d'écran noir à la relance), depuis l'accueil puis depuis une partie ;
5. jouer une partie complète (fin, rejouer) ; l'image ne se fige pas aux bruitages.

Pour une nouvelle version du même jeu : supprimer l'ancienne de la Flam puis importer
(même UUID refusé sinon).

## Rendu à l'utilisateur

Livrer : le chemin du `.plain.pk`, le résultat du check (OK/WARN), ce qui a été vérifié
(simulateur ou non) et ce qui reste à vérifier sur le device (protocole ci-dessus).
Ne jamais présenter un homebrew comme « fonctionnel sur la Flam » sans test device.

## Dépannage rapide

| Symptôme | Cause probable |
|---|---|
| titre illisible après import | Flam fw 1.x importée en classique → import carrier |
| import refusé « already loaded » | UUID déjà présent → supprimer l'ancienne version ou changer `uuid_seed` |
| écran noir au lancement, OK au simulateur | UI hors module, `require` top-level, asset absent, fichier hors `script/` |
| écran noir au 2ᵉ lancement seulement | sortie qui vide l'écran avant `goto_library`, écriture dans `state` (pièges 10, 23, 24) |
| relance : écran affiché, touches mortes | timer supprimé à la sortie (piège 24) |
| le clic molette ne fait rien | clic attendu en `EVENT_KEY` 10 au lieu de `EVENT_CLICKED` (piège 19) |
| image figée un instant à chaque bruitage | `audio.stop`/`pause` d'une piste en cours (piège 20) |
| console gelée, redémarrage obligatoire | `audio.load` pendant une lecture (piège 21) ou `audio.seek` en lecture (piège 9) |
| rectangles à la place de lettres | caractère hors ASCII/Latin-1 |
| la molette ne fait rien | `set_editing(document, true)` ou `lv.group.add_obj` manquant |
| image en mosaïque / tournée | objet plus grand que l'image / PNG portrait > 64 px |

