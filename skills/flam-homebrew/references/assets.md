# Assets : images LIF et audio MP3

`flam_build.py` convertit tout automatiquement. Ce document explique les formats,
les tailles conseillées et comment générer les fichiers sans le script.

## Images

Déposer des **PNG** (RGBA ou non) dans `<projet>/img/` ; le build écrit
`img/<nom>.lif` (sous-dossiers conservés). Un `.lif` déjà prêt est copié tel quel.
`img/thumbnail.png` devient la vignette de bibliothèque `img/thumbnail.lif` (**104×136**, portrait, comme les histoires officielles).

Tailles conseillées :
- décor plein écran : **320×212** exactement (la zone sous le bandeau) ;
- sprites : petits (≤ 64 px), paysage ou carrés ;
- vignette : dessiner en **104×136** (portrait). Un autre PNG est ajusté : option
  `flam_build.py --thumb-fit` `flou` (défaut : image entière centrée, fond = la même image
  agrandie et floutée), `rogner` (cover centré) ou `bandes` (bandes transparentes).

Limites : ≤ 2047 px par côté (réduit sinon) ; une image plus haute que large et de
plus de 64 px est posée sur un canevas carré transparent (sinon le décodeur la
transpose) : préférer des sprites paysage/carrés pour garder la taille.

Générer des images sans fichier source : dessiner avec Pillow puis sauver en PNG dans
`img/`, ou directement :

```python
import sys; sys.path.insert(0, "<skill>/scripts")
import lif
from PIL import Image
im = Image.new("RGBA", (32, 32), (255, 200, 0, 255))
open("img/coin.lif", "wb").write(lif.encode(im.tobytes(), 32, 32))
```

Format LIF (référence, encodeur complet : `scripts/lif.py`) :
`"liff"` + largeur u32 BE + hauteur u32 BE + `0xA2` + flux d'opcodes + fin
`00 00 00 00 00 00 00 01`. Couleur interne RGB565 + alpha 8 bits (dégradés fins →
léger banding). Opcodes : `0xFE` couleur RGB565, `0xFF` couleur + alpha, `00xxxxxx`
index cache (hash `(r*7+g*5+b*3) & 63`), `01bbggrr` petit delta, `10gggggg` + octet
delta vert étendu, `11rrrrrr` répétition (`run+1`, max 62). Le 1er pixel est toujours
émis en `0xFF`.

Dans le jeu : `Global.load_image("coin.lif")` (chemin relatif à `img/`), voir
`lua-api.md` §7.

## Audio

Déposer des **MP3** dans `<projet>/sounds/` ; le build les copie et génère
`sounds/<nom>.mp3map`. Tout autre format (WAV, OGG) est refusé.

- Encodage conseillé : **MPEG-1 Layer III, 44 100 Hz**, mono ou stéréo, 64–128 kbps
  (`ffmpeg -i in.wav -ar 44100 -codec:a libmp3lame -b:a 96k out.mp3`). Une autre
  fréquence est signalée : le firmware est calibré pour 44,1 kHz.
- `sounds/silent.mp3` (+ map) est toujours ajouté : remplissage pour les modules qui
  exigent un audio.
- Une seule piste à la fois ; latence de ~500 ms via `Global.requestAudioPlay`.

Format `.mp3map` (référence : `scripts/mp3map.py`) : en-tête 12 octets little-endian
(`total_units` = échantillons × 88200 / fréquence, offset de la 1re trame après ID3v2,
0), puis un enregistrement de 8 octets par seconde (`offset` absolu de trame,
`unit_pos`). Sans `.mp3map`, le seek et la durée sont faux.
