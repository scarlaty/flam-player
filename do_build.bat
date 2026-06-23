@echo off
setlocal enabledelayedexpansion
REM ============================================================
REM  do_build.bat - Build flam-player (auto-detection complete)
REM  Localise Visual Studio, charge l'env C++ x64, trouve cmake
REM  et ninja, configure si besoin, puis compile.
REM  Aucune configuration manuelle requise.
REM ============================================================

REM --- 1. Localiser Visual Studio via vswhere ---
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
    echo [ERREUR] vswhere introuvable. Visual Studio est-il installe ?
    exit /b 1
)
set "VSINSTALL="
for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%i"
if not defined VSINSTALL (
    echo [ERREUR] Aucun VS avec les outils C++ x64.
    echo          Installe le composant "Microsoft.VisualStudio.Component.VC.Tools.x86.x64".
    exit /b 1
)

REM --- 2. Charger l'environnement compilateur x64 ---
call "%VSINSTALL%\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>nul
if errorlevel 1 (
    echo [ERREUR] vcvarsall.bat a echoue.
    exit /b 1
)

REM --- 3. Localiser cmake et ninja (PATH, puis VS, puis pip/Python) ---
set "CMAKE="
set "NINJA="
for %%C in (cmake.exe) do if not defined CMAKE set "CMAKE=%%~$PATH:C"
for %%C in (ninja.exe) do if not defined NINJA set "NINJA=%%~$PATH:C"
if not defined CMAKE if exist "%VSINSTALL%\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe" set "CMAKE=%VSINSTALL%\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
if not defined NINJA if exist "%VSINSTALL%\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe" set "NINJA=%VSINSTALL%\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe"
if not defined CMAKE call :find_python
if not defined NINJA call :find_python

if not defined CMAKE (
    echo [ERREUR] cmake introuvable ^(PATH, VS, ou Python/pip^).
    exit /b 1
)
if not defined NINJA (
    echo [ERREUR] ninja introuvable ^(PATH, VS, ou Python/pip^).
    exit /b 1
)

REM --- 4. Chemins (relatifs a l'emplacement de ce script) ---
set "SRC=%~dp0"
if "%SRC:~-1%"=="\" set "SRC=%SRC:~0,-1%"
set "BUILD=%SRC%\build"

REM --- 5. Mode : "do_build.bat"  ou  "do_build.bat tests" ---
if /i "%~1"=="tests" goto :build_tests

REM --- 5a. Build normal (flam-player) ---
if not exist "%BUILD%\build.ninja" (
    echo [INFO] Configuration ^(premier build^)...
    "%CMAKE%" -S "%SRC%" -B "%BUILD%" -G Ninja -DCMAKE_BUILD_TYPE=Debug -DCMAKE_MAKE_PROGRAM="%NINJA%"
    if errorlevel 1 (
        echo [ERREUR] Configuration cmake echouee.
        exit /b 1
    )
)
"%CMAKE%" --build "%BUILD%"
if errorlevel 1 (
    echo [ERREUR] Build echoue.
    exit /b 1
)
echo [OK] Build termine : %BUILD%\flam-player.exe
endlocal
exit /b 0

REM --- 5b. Build des tests (flam-test) ---
:build_tests
echo [INFO] Configuration des tests ^(BUILD_TESTS=ON^)...
"%CMAKE%" -S "%SRC%" -B "%BUILD%" -G Ninja -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTS=ON -DCMAKE_MAKE_PROGRAM="%NINJA%"
if errorlevel 1 (
    echo [ERREUR] Configuration cmake echouee.
    exit /b 1
)
"%CMAKE%" --build "%BUILD%" --target flam-test
if errorlevel 1 (
    echo [ERREUR] Build des tests echoue.
    exit /b 1
)
echo [OK] Tests construits : %BUILD%\flam-test.exe
endlocal
exit /b 0

REM ============================================================
REM  Sous-routines : recherche cmake/ninja dans les Python locaux
REM  ('for /d' expand les wildcards de repertoires, contrairement
REM   a 'dir' sur des composants intermediaires)
REM ============================================================
:find_python
REM Python du Store (WinGet/MSIX)
for /d %%d in ("%LOCALAPPDATA%\Packages\PythonSoftwareFoundation.Python*") do (
    if exist "%%d\LocalCache\local-packages" (
        for /d %%e in ("%%d\LocalCache\local-packages\Python*") do call :probe "%%e"
    )
)
REM Python "classique" (installeur python.org)
for /d %%d in ("%LOCALAPPDATA%\Programs\Python\Python*" "%ProgramFiles%\Python*") do call :probe "%%d"
exit /b

:probe
if not defined CMAKE if exist "%~1\site-packages\cmake\data\bin\cmake.exe"     set "CMAKE=%~1\site-packages\cmake\data\bin\cmake.exe"
if not defined CMAKE if exist "%~1\Lib\site-packages\cmake\data\bin\cmake.exe" set "CMAKE=%~1\Lib\site-packages\cmake\data\bin\cmake.exe"
if not defined NINJA if exist "%~1\Scripts\ninja.exe"                          set "NINJA=%~1\Scripts\ninja.exe"
exit /b
