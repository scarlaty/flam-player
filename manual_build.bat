@echo off
REM ============================================================
REM  manual_build.bat - Alias conserve pour compatibilite.
REM  Delegue a do_build.bat (chemins relatifs a ce script,
REM  detection VS/cmake/ninja automatique).
REM  Usage : manual_build.bat [tests]
REM ============================================================
call "%~dp0do_build.bat" %*
exit /b %ERRORLEVEL%
