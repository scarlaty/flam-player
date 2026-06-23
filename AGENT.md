# AGENT.md — Flam Player

Emulateur du dispositif FLAM (lecteur d'histoires interactives pour enfants, type Lunii).
Reverse-engineering du firmware : LVGL 8.3 + Lua 5.4 + SDL2 sur desktop Windows.

## Compilation

**Prerequis** : Visual Studio 2022 Community (MSVC x64), CMake + Ninja (fournis par VS).

**Build rapide (depuis un terminal normal)** :
```bat
do_build.bat
```
Cela appelle `vcvars64.bat` puis `cmake --build .` dans `build/`.

**Build depuis un shell bash/Claude Code** :
```bash
powershell.exe -Command "& { cmd /c '\"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat\" x64 && cd /d C:\temp\flam-player\build && ninja' }"
```
`vcvarsall.bat` est obligatoire pour que MSVC trouve `stddef.h` et les headers Windows SDK.
Sans cet appel, la compilation echoue avec `fatal error C1083: stddef.h: No such file or directory`.

**Generateur** : Ninja (pas MSBuild).
**Compilateur** : `cl.exe` (MSVC 14.34, VS2022).
**Build dir** : `build/` (deja configure, pas besoin de re-run cmake sauf si on ajoute des fichiers).
**Re-configurer cmake** (si ajout de sources) :
```bash
powershell.exe -Command "& { cmd /c '\"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat\" x64 && cd /d C:\temp\flam-player\build && cmake .. -G Ninja' }"
```

**Build des tests** :
```bat
do_build_tests.bat
```
Ou manuellement : `cmake .. -G Ninja -DBUILD_TESTS=ON && cmake --build . --target flam-test`

## Execution

```bash
# Lancer une histoire .plain (dossier)
./build/flam-player.exe "stories/mon-histoire.plain"

# Lancer une archive .plain.pk (ZIP store)
./build/flam-player.exe "stories/mon-histoire.plain.pk"

# Lancer un script Lua directement
./build/flam-player.exe "test.lua"

# Browser d'histoires (scan le dossier courant)
./build/flam-player.exe

# Scanner un dossier specifique
./build/flam-player.exe --scan-dir "C:\chemin\vers\histoires"

# Lancer les tests unitaires
./build/flam-test.exe tests/lua/test_obj.lua tests/lua/test_style.lua ...
```

**Timeout en CI/test** : utiliser `timeout 8 ./build/flam-player.exe ...` car le player ouvre une fenetre SDL et attend indefiniment.

## Architecture

```
src/
  main.c                  — Point d'entree, story browser, chargement .plain/.pk
  bindings/
    lua_lv.h              — Core des bindings Lua↔LVGL, metatables, push/check helpers
    lua_lv.c              — Registration des modules lv.*, gestion images/timers
    lua_lv_obj.c           — Bindings lv.obj.*, lv.label.*, lv.btn.*, lv.img.*, etc.
    lua_lv_style.c         — Bindings lv.style.*
    lua_lv_event.c         — Bindings lv.obj.add_event_cb, lv.group.*
  formats/
    lif_decoder.c/h        — Decodeur d'images .lif (format proprietaire FLAM)
    pk_reader.c/h          — Lecteur d'archives .plain.pk (ZIP store, pas de compression)
    mp3map_parser.c/h      — Parser de .mp3map (mapping audio)
  platform/
    sdl_driver.c/h         — Driver SDL2 pour LVGL (affichage + input encodeur)
    sdl_audio.c/h          — Audio SDL2 + minimp3, API Lua audio.*
  firmware/
    fw_globals.c/h         — Emulation des globales firmware (state, save, context_menu)
  fonts/
    nunito_*.c             — Polices LVGL compilees (Nunito Bold/ExtraBold 12-20px)
libs/
  lvgl/                   — LVGL 8.3.x (sources completes)
  lua/src/                — Lua 5.4 (sources completes)
  SDL2/                   — SDL2 (headers + .lib/.dll x64 pre-compiles)
  minimp3/                — Decodeur MP3 header-only
  lv_conf.h               — Configuration LVGL (320x240, 32-bit, 256KB heap)
tests/
  test_main.c             — Harnais de test headless (260+ tests)
  lua/test_*.lua           — Tests unitaires Lua (obj, style, label, btn, event, etc.)
  lua/fuzz_lvgl.lua        — Fuzzer LVGL (buffer overflow, UAF, integer overflow, heap)
  lua/poc_*.lua            — PoCs de securite (use-after-free, memory leak, struct dump)
stories/
  *.plain/                — Histoires (dossier avec info.plain + main.lua + img/ + sounds/)
  *.plain.pk              — Archives d'histoires (ZIP store)
```

## Format d'histoire .plain

Un dossier `nom.plain/` contenant :
- `info.plain` — premiere ligne = titre de l'histoire
- `main.lua` — script Lua principal (point d'entree)
- `script/` — modules Lua additionnels (chargeables via `require`)
- `img/` — images .lif (format proprietaire) et `thumbnail.lif`
- `sounds/` — fichiers audio .mp3

Le format `.plain.pk` est un ZIP store (compression=0) contenant les memes fichiers.
Le player extrait automatiquement le `.pk` dans un dossier `.plain` adjacent.

### Creer un .plain.pk depuis un dossier .plain

Utiliser Python avec `zipfile.ZIP_STORED` (compression=0). **Ne pas utiliser** `Compress-Archive` de PowerShell
car il compresse les fichiers meme avec `-CompressionLevel Optimal`, ce qui produit un ZIP invalide pour le player.

```bash
python -c "
import zipfile, pathlib, sys
src = pathlib.Path(sys.argv[1])          # ex: stories/mon-histoire.plain
dst = str(src) + '.pk'                   # => stories/mon-histoire.plain.pk
with zipfile.ZipFile(dst, 'w', compression=zipfile.ZIP_STORED) as zf:
    for f in sorted(src.rglob('*')):
        if f.is_file():
            zf.write(f, f.relative_to(src))
print(f'Created {dst}')
" "stories/mon-histoire.plain"
```

**Points importants** :
- Les chemins dans le ZIP sont relatifs a la racine du dossier `.plain` (pas de prefixe de dossier parent)
- Compression = `ZIP_STORED` (0) obligatoire — le reader `pk_reader.c` lit les fichiers directement sans decompression
- Verifier avec `python -m zipfile -l archive.plain.pk` que `CompressedLength == Length` pour chaque fichier

## API Lua disponible

Le script Lua a acces a :
- `window` — conteneur LVGL principal (320x240, sous la header bar de 28px)
- `document` — focus group principal (navigation encodeur)
- `lv.*` — bindings LVGL complets (obj, label, btn, img, slider, arc, style, color, anim, timer, group)
- `audio.*` — lecture audio (play, stop, pause, resume, set_volume, on_end)
- `state.*` — persistence (get, set, sauvegarde automatique dans saves/)
- `context_menu.*` — menu contextuel (comme le firmware)
- Bibliotheques standard Lua completes (`io`, `os`, `debug`, `package`) — PAS de sandbox dans l'emulateur

**Sur le vrai dispositif FLAM** : la sandbox Lua ne laisse que `lv.*`, pas de `io`/`os`/`debug`/`package`.

## Ecran FLAM

- Resolution : 320x240 pixels
- Facteur d'upscale desktop : x3 (fenetre 960x720)
- Header bar : 28px en haut (titre de l'histoire)
- Zone de contenu (`window`) : 320x212 pixels
- Couleur : 32-bit ARGB
- Heap LVGL : 256 KB (`LV_MEM_SIZE` dans `lv_conf.h`)

## Vulnerabilites connues (etude de securite)

### Use-After-Free (CWE-416)
`lv_obj_del()` libere la memoire C mais ne nullifie pas le userdata Lua.
- Fichier cle : `src/bindings/lua_lv.h:94-97` — `lua_lv_check_obj()` retourne le pointeur meme apres free
- Fichier cle : `src/bindings/lua_lv_obj.c:21-25` — `l_obj_del()` ne fait pas `*ud = NULL`
- Exploitable : oui, lecture de RAM via dangling pointers (demontre dans les PoCs)
- Limitation : `pcall` ne catch pas les SIGSEGV C — si le bloc est reutilise par un non-label, `lv_label_get_text` crash
- Getters safe sur objets freed : `get_text`, `get_state`, `get_child_cnt`, `get_scroll_y`
- Getters qui crashent : `get_width`, `get_height`, `get_x` (appellent `lv_obj_update_layout` qui traverse l'arbre parent)

### Integer Overflow dans lv_txt.c
`libs/lvgl/src/misc/lv_txt.c:111` — overflow dans le calcul de hauteur de texte.
Declenchable via des labels avec texte > 65KB.

## Notes techniques

- L'allocateur LVGL (`lv_mem`) est LIFO best-fit : un bloc libere est reutilise par la prochaine allocation de meme taille
- `lv_label_t` etend `lv_obj_t` avec un champ `char *text` — c'est un pointeur vers un buffer separe
- Pour le heap spray UAF : creer des labels (pas des obj generiques) pour garantir que le bloc freed est reutilise par un autre label
- Les styles (`lv.style.new()`) ne sont pas des `lv_obj_t` — ils ont leur propre metatable et taille d'allocation
- L'encodeur (molette FLAM) est emule via les fleches clavier haut/bas + Enter

## Fichiers batch utiles

| Fichier | Usage |
|---------|-------|
| `do_build.bat` | Compile le player (vcvars64 + cmake --build) |
| `do_build_tests.bat` | Compile les tests (-DBUILD_TESTS=ON) |
| `build_run.bat` | Compile puis lance (vcvars64 + ninja) |
| `run.bat` | Lance le player avec --scan-dir vers les histoires Lunii-Qt |
| `test_run.bat` | Lance tous les tests unitaires Lua |
