# Attrape-étoiles (jeu FLAM)

Petit jeu molette pour le lecteur FLAM, fabriqué avec le skill
[`skills/flam-homebrew`](../../skills/flam-homebrew/SKILL.md) (sert aussi de test de ce skill).
100 % contenu Lua, packagé comme une histoire (`.plain.pk`).

## Règles et contrôles
- Un panier en bas, des étoiles qui tombent : +1 point par étoile attrapée.
- 3 étoiles ratées = perdu. La vitesse augmente avec le score.
- **Molette ← / →** : déplace le panier (20 px par cran).
- **ENTER** : rejouer après « Perdu ». Un cran de molette relance aussi, après 1 s
  (le device n'a pas de bouton central dédié).
- **Retour** : bibliothèque.

## Build
```
python make_images.py .                                   # (re)génère img/*.png (Pillow)
python ../../skills/flam-homebrew/scripts/flam_build.py .  # -> _out/attrape-etoiles.plain.pk + .plain
```
`flam_build.py` embarque le runtime du skill, convertit les PNG en `.lif` (dont
`img/thumbnail.png` -> vignette) et lance `flam_check.py`.

## Test
- **Simulateur** : `build/flam-player.exe --strict "games/attrape-etoiles/_out/attrape-etoiles.plain"`
- **Device** : import Lunii.QT (classique en fw 2.x, carrier en fw 1.x), puis le protocole
  de l'étape 7 du skill (relancer deux fois sans redémarrer).

## État
- Simulateur `--strict` : affichage et fin de partie vérifiés (capture sans touches) ;
  contrôles à confirmer en interactif.
- Device : **pas encore testé**.
