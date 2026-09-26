@echo off
setlocal enabledelayedexpansion
REM ============================================================
REM  test_run.bat - Lance toutes les suites de tests
REM  1. Suite    : tests\lua\test_*.lua (hors test_helpers.lua), flam-test,
REM               un seul processus.
REM  2. Moteur   : tests\engine\test_*.lua (moteur story.lua/main.lua et
REM               modules runtime telmi2flam), flam-test.
REM  3. Audio    : tests\audio\test_*.lua puis reopen_1/reopen_2 (meme
REM               processus : fermeture/reouverture du lua_State, issue #1),
REM               flam-test-audio = vrai sdl_audio.c, pilote SDL dummy.
REM  4. Isoles   : tests\lua\regress_*.lua, poc_*.lua, fuzz_*.lua, un
REM               processus par fichier (un crash ne masque pas les autres).
REM  5. Convert. : tools\telmi2flam\tests (python unittest)   [facultatif]
REM  6. E2E      : tests\e2e\e2e_telmi.py : paquet TELMI -> telmi2flam ->
REM               flam-player (pilotes SDL dummy)             [facultatif]
REM  Facultatif : saute si python (ou Pillow) ou flam-player.exe absent.
REM  Code de sortie : 0 si les etapes 1, 2, 3, 5, 6 passent. Avec "strict",
REM  les echecs des tests isoles comptent aussi, et un flam-test-audio.exe
REM  absent est un echec (sinon : saute).
REM  Usage : test_run.bat [strict]
REM  Dossier de build : build\ ou %%FLAM_BUILD_DIR%% s'il est defini.
REM  Python : %%FLAM_PYTHON%% sinon python du PATH. Lua 5.4 autonome
REM  (verifs nodes.lua du convertisseur) : %%FLAM_LUA%% si defini.
REM  Construire d'abord : do_build.bat puis do_build.bat tests
REM  (flam-test-audio : cmake --build build --target flam-test-audio)
REM ============================================================
set "ROOT=%~dp0"
set "BUILD=%~dp0build"
if defined FLAM_BUILD_DIR set "BUILD=%FLAM_BUILD_DIR%"
set "STRICT=0"
if /i "%~1"=="strict" set "STRICT=1"
REM Pas de capture auto du player (sinon screenshot.bmp du depot est reecrit
REM par les etapes qui durent plus de 12 s)
if not defined FLAM_SCREENSHOT_AUTO_MS set "FLAM_SCREENSHOT_AUTO_MS=0"
if not exist "%BUILD%\flam-test.exe" (
    echo [ERREUR] flam-test.exe introuvable dans "%BUILD%". Lance d'abord : do_build.bat tests
    exit /b 1
)
set "LUADIR=%ROOT%tests\lua"
set "ARGS="
for %%f in ("%LUADIR%\test_*.lua") do (
    if /i not "%%~nf"=="test_helpers" set "ARGS=!ARGS! "%%f""
)
if not defined ARGS (
    echo [ERREUR] Aucun test_*.lua trouve dans "%LUADIR%".
    exit /b 1
)
cd /d "%BUILD%"

REM ---------------- 1. Suite ----------------
echo ############ 1. Suite test_*.lua ############
"%BUILD%\flam-test.exe" --timeout 60!ARGS!
set "SUITE_RC=!ERRORLEVEL!"
echo EXIT_CODE=!SUITE_RC!

REM ---------------- 2. Moteur / runtime telmi2flam ----------------
echo.
echo ############ 2. Moteur et runtime telmi2flam : tests\engine ############
set "ENG_ARGS="
for %%f in ("%ROOT%tests\engine\test_*.lua") do set "ENG_ARGS=!ENG_ARGS! "%%f""
if defined ENG_ARGS (
    "%BUILD%\flam-test.exe" --timeout 60!ENG_ARGS!
    set "ENGINE_RC=!ERRORLEVEL!"
) else (
    echo [ERREUR] Aucun test dans tests\engine
    set "ENGINE_RC=1"
)
echo EXIT_CODE=!ENGINE_RC!

REM ---------------- 3. Audio reel ----------------
echo.
echo ############ 3. Audio reel : flam-test-audio, tests\audio ############
set "AUDIO_RC=SKIP"
if exist "%BUILD%\flam-test-audio.exe" (
    set "AUD_ARGS="
    for %%f in ("%ROOT%tests\audio\test_*.lua") do set "AUD_ARGS=!AUD_ARGS! "%%f""
    REM ordre impose : reopen_1 laisse un etat ouvert, reopen_2 le verifie
    set "AUD_ARGS=!AUD_ARGS! "%ROOT%tests\audio\reopen_1_open.lua" "%ROOT%tests\audio\reopen_2_reuse.lua""
    if not defined SDL_AUDIODRIVER set "SDL_AUDIODRIVER=dummy"
    "%BUILD%\flam-test-audio.exe" --timeout 120!AUD_ARGS!
    set "AUDIO_RC=!ERRORLEVEL!"
    echo EXIT_CODE=!AUDIO_RC!
) else (
    echo [ATTENTION] flam-test-audio.exe introuvable dans "%BUILD%" : etape sautee.
    echo             Construire : cmake --build "%BUILD%" --target flam-test-audio
    if "!STRICT!"=="1" set "AUDIO_RC=ABSENT"
)

REM ---------------- 4. Tests isoles ----------------
echo.
echo ############ 4. Tests isoles : regress_, poc_, fuzz_ ############
set "ISO_FAIL=0"
set "ISO_SUMMARY="
for %%p in (regress poc fuzz) do (
    for %%f in ("%LUADIR%\%%p_*.lua") do (
        "%BUILD%\flam-test.exe" --timeout 30 "%%f"
        set "RC=!ERRORLEVEL!"
        if "!RC!"=="0" (
            set "ISO_SUMMARY=!ISO_SUMMARY! [OK]%%~nf"
        ) else (
            set /a ISO_FAIL+=1
            set "ISO_SUMMARY=!ISO_SUMMARY! [ECHEC:!RC!]%%~nf"
        )
    )
)

REM ---------------- Python (etapes 5 et 6) ----------------
set "PY=python"
if defined FLAM_PYTHON set "PY=%FLAM_PYTHON%"
set "HAVE_PY=0"
"%PY%" -c "import PIL" >nul 2>nul
if "!ERRORLEVEL!"=="0" set "HAVE_PY=1"

REM ---------------- 5. Convertisseur ----------------
echo.
echo ############ 5. Convertisseur : tools\telmi2flam\tests ############
set "CONV_RC=SKIP"
if "!HAVE_PY!"=="1" (
    "%PY%" -m unittest discover -s "%ROOT%tools\telmi2flam\tests"
    set "CONV_RC=!ERRORLEVEL!"
    echo EXIT_CODE=!CONV_RC!
) else (
    echo [SKIP] python avec Pillow introuvable : definir FLAM_PYTHON
)

REM ---------------- 6. Bout en bout ----------------
echo.
echo ############ 6. Bout en bout : TELMI -^> telmi2flam -^> flam-player ############
set "E2E_RC=SKIP"
if "!HAVE_PY!"=="1" (
    "%PY%" "%ROOT%tests\e2e\e2e_telmi.py" --build "%BUILD%"
    set "E2E_RC=!ERRORLEVEL!"
    if "!E2E_RC!"=="77" set "E2E_RC=SKIP"
    echo EXIT_CODE=!E2E_RC!
) else (
    echo [SKIP] python avec Pillow introuvable : definir FLAM_PYTHON
)

REM ---------------- Resume ----------------
echo.
echo ========================================
echo 1. Suite test_*.lua      : !SUITE_RC!
echo 2. Moteur/runtime        : !ENGINE_RC!
echo 3. Audio reel            : !AUDIO_RC!
echo 4. Tests isoles          : !ISO_FAIL! en echec
for %%s in (!ISO_SUMMARY!) do echo      %%s
echo 5. Convertisseur         : !CONV_RC!
echo 6. Bout en bout          : !E2E_RC!
echo   (0 = OK ; SKIP = saute ; 1 = assertion/erreur, 3 = timeout, autre = crash)
echo ========================================

set "FINAL_RC=0"
for %%r in (!SUITE_RC! !ENGINE_RC! !AUDIO_RC! !CONV_RC! !E2E_RC!) do (
    if not "%%r"=="0" if not "%%r"=="SKIP" set "FINAL_RC=1"
)
if "!STRICT!"=="1" if not "!ISO_FAIL!"=="0" set "FINAL_RC=1"
if "!FINAL_RC!"=="0" (echo RESULTAT : OK) else (echo RESULTAT : ECHEC)
endlocal & exit /b %FINAL_RC%
