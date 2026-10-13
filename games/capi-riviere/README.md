# Capi et la rivière (jeu FLAM)

Course-esquive à la molette pour le lecteur FLAM (enfant de 7 ans), fabriquée avec le
skill [`skills/flam-homebrew`](../../skills/flam-homebrew/SKILL.md). Capi le capybara,
une mandarine sur la tête, descend une rivière de la jungle sur sa bouée.

## Règles et contrôles
- Vue de dessus, la rivière défile et accélère peu à peu (3 paliers, « Plus vite ! » avec un petit jingle).
- Rochers, troncs, nénuphars : plouf ! une bouée en moins (3 bouées), Capi clignote
  2 s sans risque. Plus de bouée : écran « Bravo ! » avec le score, jamais de « perdu ».
- Mandarines : +10 ; enchaînées en moins de 4 s : combo x2, x3… (jusqu'à x5).
- **Molette ← / →** : diriger la bouée (24 px par cran, glissement doux).
- **ENTER** : saut (passe au-dessus des obstacles). Comme le geste ENTER du device
  n'est pas documenté, **3 crans très rapides** font aussi sauter (c'est ce que
  l'écran titre explique).
- Titre : ENTER ou un cran (après 0,5 s) pour partir. Fin : ENTER ou un cran après
  1,5 s pour rejouer.
- **Retour** : pendant la partie ou sur « Bravo ! » → écran titre ; sur le titre →
  bibliothèque, par le même chemin que le casse-briques (`back_callback = goto_library`,
  sans nettoyage du module), après avoir coupé le son et la boucle.
- Record : gardé pour la session seulement. Le jeu n'écrit **rien** dans `state` : une
  sauvegarde donnait un écran noir au 2ᵉ lancement sur la Flam (corrigé en effaçant la
  progression), comme les histoires converties (`tools/telmi2flam/DEVICE_VS_SIM.md` §10).

## Son
La Flam ne joue qu'une piste à la fois, et (constaté sur l'appareil avec un
paquet de test, voir `skills/flam-homebrew/references/lua-api.md` §8) arrêter une piste en cours fige l'image un instant, tandis que
charger un son sans arrêter le précédent gèle la console. Donc :
- **pendant la partie, pas de musique** : glop, saut, plouf, combo et « Plus vite ! »
  sont des bruitages qui ne coupent jamais rien. Un bruitage attend la fin du précédent
  (au plus 0,4 s, sinon il est oublié, sauf le plouf) ;
- les bruitages de mandarine et de saut sont plus aigus et plus courts à chaque palier ;
- **musique douce au titre et sur « Bravo ! »** (après la fanfare), en 4 segments
  enchaînés (`title_1..4.mp3`) ; l'arrêt au départ de la partie ne se voit pas.

## Build
```
python3 make_images.py .                                   # img/*.png (Pillow)
python3 make_sounds.py .                                   # sounds/*.mp3 (numpy + ffmpeg)
python3 ../../skills/flam-homebrew/scripts/flam_build.py .  # -> _out/capi-et-la-riviere.plain.pk
```

## Test
- **Simulateur** : `build/flam-player.exe --strict "games/capi-riviere/_out/capi-et-la-riviere.plain"`
  (flèches = molette, Entrée = ENTER, Échap = retour).
- **Device** : import Lunii.QT (classique en fw 2.x, carrier en fw 1.x), puis le
  protocole de l'étape 7 du skill (relancer deux fois sans redémarrer).

## État
- Device (2026-10-13) : jeu, clic molette (ENTER, reçu en `EVENT_CLICKED`), retour,
  musique et défilement OK. À revalider : relance x2 sans reboot (sortie titre corrigée)
  et la partie sans musique (plus d'à-coups attendus).
