@echo off
setlocal enabledelayedexpansion
REM ============================================================
REM  test_run.bat - Lance la suite de tests Lua (flam-test)
REM  Decouvre automatiquement tests\lua\*.lua.
REM  Construire d'abord : do_build.bat tests
REM ============================================================
set "BUILD=%~dp0build"
if not exist "%BUILD%\flam-test.exe" (
    echo [ERREUR] flam-test.exe introuvable. Lance d'abord : do_build.bat tests
    exit /b 1
)
set "LUADIR=%~dp0tests\lua"
set "ARGS="
for %%f in ("%LUADIR%\*.lua") do set "ARGS=!ARGS! "%%f""
if not defined ARGS (
    echo [ERREUR] Aucun test .lua trouve dans "%LUADIR%".
    exit /b 1
)
cd /d "%BUILD%"
"%BUILD%\flam-test.exe"!ARGS!
echo EXIT_CODE=%ERRORLEVEL%
endlocal
