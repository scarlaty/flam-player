# Licences des composants redistribués

Ce fichier est installé à la racine de Flam Launcher. Il liste les composants livrés par
l'installateur et renvoie vers leurs textes de licence complets, présents dans l'installation
(`licenses/` et `telmi2flam/LICENSES.md`). En cas de divergence, le texte de licence fait foi.

| Composant                                                                                                    | Licence                                                                                                     | Texte dans l'installation                                                 |
| ------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Flam Launcher (`flam-launcher.exe`)                                                                          | GPL-3.0-or-later                                                                                            | `licenses/flam-launcher-LICENSE.txt`                                      |
| flam-player (`flam-player.exe`)                                                                              | GPL-3.0-or-later                                                                                            | `licenses/flam-launcher-LICENSE.txt`                                      |
| SDL2 (`SDL2.dll`)                                                                                            | zlib                                                                                                        | `licenses/SDL2-LICENSE.txt`                                               |
| Lua 5.4 (lié dans `flam-player.exe`)                                                                         | MIT                                                                                                         | `licenses/Lua-LICENSE.txt`                                                |
| LVGL 8.3 (lié dans `flam-player.exe`)                                                                        | MIT                                                                                                         | `licenses/LVGL-LICENCE.txt`                                               |
| minimp3 (lié dans `flam-player.exe`)                                                                         | CC0 1.0 (domaine public)                                                                                    | `licenses/minimp3-LICENSE.txt`                                            |
| Police Nunito, Copyright 2014 The Nunito Project Authors (glyphes convertis en bitmap, `flam-player.exe`)    | SIL Open Font License 1.1                                                                                   | `licenses/OFL-Nunito.txt`                                                 |
| Police Montserrat 14, Copyright 2024 The Montserrat.Git Project Authors (intégrée à LVGL, `flam-player.exe`) | SIL Open Font License 1.1                                                                                   | `licenses/OFL-Montserrat.txt`                                             |
| `vcruntime140.dll` (à côté de `flam-player.exe` et dans `telmi2flam/python/`)                                | Microsoft Visual C++ Redistributable : fichier redistribuable de Visual Studio (REDIST.TXT), livré tel quel | `telmi2flam/LICENSES.md`                                                  |
| Convertisseur `telmi2flam/` : scripts et moteur Lua                                                          | GPL-3.0-or-later                                                                                            | `licenses/flam-launcher-LICENSE.txt`                                      |
| Runtime `telmi2flam/runtime/` (bibliothèque Lua, icônes)                                                     | GPL-3.0-or-later (dépôt flam-player)                                                                        | `licenses/flam-launcher-LICENSE.txt`                                      |
| Python embarqué (`telmi2flam/python/`, dont libffi, libtommath, `python.cat`)                                | PSF-2.0 et licences des composants                                                                          | `telmi2flam/python/LICENSE.txt`                                           |
| `vcruntime140.dll`, `vcruntime140_1.dll` (livrées avec Python)                                               | Microsoft Visual C++ Redistributable (fichiers redistribuables)                                             | `telmi2flam/LICENSES.md`                                                  |
| Pillow et ses bibliothèques intégrées (libjpeg-turbo, libpng, zlib, libtiff, libwebp…)                       | MIT-CMU et licences des bibliothèques                                                                       | `telmi2flam/python/Lib/site-packages/pillow-*.dist-info/licenses/LICENSE` |

Les dépendances de l'application elle-même (bibliothèques Rust compilées dans
`flam-launcher.exe`, paquets npm compilés dans l'interface) sont sous licences permissives
(MIT, Apache-2.0, ISC, BSD, MPL-2.0 pour certaines). Leurs licences et textes de licence sont
dans `THIRD-PARTY-NOTICES.txt`, installé à la racine de l'application ; ce fichier est généré à
chaque build par `npm run build:notices` (`scripts/build-notices.mjs`, depuis `cargo metadata` et
`package-lock.json`).
