@echo off
setlocal
REM ============================================================
REM  run.bat - Lance flam-player
REM  Usage :
REM    run.bat                  -> scanne le dossier build (ex: Enquete.plain)
REM    run.bat "D:\mes\histoires" -> scanne le dossier indique (.plain / .pk)
REM  Logs : build\stdout.log et build\stderr.log
REM ============================================================
set "BUILD=%~dp0build"
if not exist "%BUILD%\flam-player.exe" (
    echo [ERREUR] flam-player.exe introuvable. Lance d'abord do_build.bat.
    exit /b 1
)
cd /d "%BUILD%"
if "%~1"=="" (
    "%BUILD%\flam-player.exe" >stdout.log 2>stderr.log
) else (
    "%BUILD%\flam-player.exe" --scan-dir "%~1" >stdout.log 2>stderr.log
)
echo EXIT_CODE=%ERRORLEVEL%
endlocal
