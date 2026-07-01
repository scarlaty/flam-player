@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64 > nul 2>&1
if not exist "C:\repo\flam-player\build\build.ninja" (
    cmake -S "C:\repo\flam-player" -B "C:\repo\flam-player\build" -G Ninja -DCMAKE_BUILD_TYPE=Debug
    if errorlevel 1 exit /b 1
)
cmake --build "C:\repo\flam-player\build"
