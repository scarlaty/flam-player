<#
.SYNOPSIS
  Construit flam-player.exe (Release) et rassemble ce que l'installateur de flam-launcher
  embarque à son côté (issue #21) : flam-player.exe, SDL2.dll et les textes de licence.

.DESCRIPTION
  Produit <OutDir> (par défaut apps/flam-launcher/build/player, ignoré par Git) :

    player/
      flam-player.exe, SDL2.dll        build CMake Release (Ninja) de la racine du dépôt
      vcruntime140.dll                 runtime Visual C++ redistribuable, à côté du player
      licenses/                        textes de licence de SDL2, Lua, LVGL, minimp3, flam-launcher,
                                       polices Nunito et Montserrat (OFL)

  Compilation dans <racine du dépôt>/build/release (ignoré par Git), sans toucher au dossier
  build/ de développement (Debug) de do_build.bat. Prérequis : sous-modules initialisés
  (`git submodule update --init --recursive`) et Visual Studio avec les outils C++ x64 (la
  version 2022 est préférée : certaines versions plus récentes n'ont pas les bibliothèques C
  attendues). CMake et Ninja sont cherchés dans le PATH, puis dans Visual Studio.

.PARAMETER OutDir
  Dossier produit (supprimé puis recréé).

.PARAMETER BuildDir
  Dossier de compilation CMake (réutilisé d'un appel à l'autre).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-player.ps1
#>
[CmdletBinding()]
param(
  [string]$OutDir,
  [string]$BuildDir
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

$LauncherDir = Split-Path -Parent $PSScriptRoot
$RepoRoot = Split-Path -Parent (Split-Path -Parent $LauncherDir)
if (-not $OutDir) { $OutDir = Join-Path $LauncherDir 'build\player' }
if (-not $BuildDir) { $BuildDir = Join-Path $RepoRoot 'build\release' }
$OutDir = [IO.Path]::GetFullPath($OutDir)
$BuildDir = [IO.Path]::GetFullPath($BuildDir)

function Invoke-Native([string]$What, [scriptblock]$Command) {
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "échec ($What) : code $LASTEXITCODE" }
}

# --- Sous-modules -------------------------------------------------------------------------
foreach ($lib in 'SDL2', 'lua', 'lvgl', 'minimp3') {
  $dir = Join-Path $RepoRoot "libs\$lib"
  if (-not (Test-Path -LiteralPath $dir) -or -not (Get-ChildItem -LiteralPath $dir -Force | Select-Object -First 1)) {
    throw "sous-module libs/$lib vide : git submodule update --init --recursive"
  }
}

# --- Visual Studio ------------------------------------------------------------------------
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'vswhere introuvable : Visual Studio est-il installé ?' }
$vsArgs = @('-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath')
$vs = (& $vswhere @vsArgs -version '[17.0,18.0)' | Select-Object -First 1)
if (-not $vs) { $vs = (& $vswhere @vsArgs -latest | Select-Object -First 1) }
if (-not $vs) { throw 'aucun Visual Studio avec les outils C++ x64 (composant VC.Tools.x86.x64)' }
if ($vs -notmatch '2022') {
  Write-Warning "Visual Studio 2022 introuvable : $vs utilisé (bibliothèques C éventuellement absentes)"
}
Write-Host "Visual Studio : $vs"

# Environnement du compilateur x64 (équivalent de vcvars64.bat) importé dans ce processus.
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path -LiteralPath $vcvars)) { throw "vcvars64.bat introuvable : $vcvars" }
$envDump = & cmd.exe /d /c "`"$vcvars`" >nul 2>&1 && set"
if ($LASTEXITCODE -ne 0) { throw 'vcvars64.bat a échoué' }
foreach ($line in $envDump) {
  if ($line -match '^([^=]+)=(.*)$') { Set-Item -LiteralPath "Env:$($Matches[1])" -Value $Matches[2] }
}

function Find-Tool([string]$Name, [string]$VsRelative) {
  $cmd = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($cmd) { return $cmd.Source }
  $path = Join-Path $vs $VsRelative
  if (Test-Path -LiteralPath $path) { return $path }
  # `pip install cmake ninja` (installation utilisateur, y compris Python du Microsoft Store).
  $pipScripts = @(
    (Join-Path $env:APPDATA 'Python\Python*\Scripts'),
    (Join-Path $env:LOCALAPPDATA 'Packages\PythonSoftwareFoundation.Python.*\LocalCache\local-packages\Python*\Scripts')
  )
  foreach ($pattern in $pipScripts) {
    $found = Get-ChildItem -Path (Join-Path $pattern $Name) -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) { return $found.FullName }
  }
  throw "$Name introuvable (PATH, Visual Studio, pip) : pip install cmake ninja"
}
$cmake = Find-Tool 'cmake.exe' 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
$ninja = Find-Tool 'ninja.exe' 'Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe'

# --- Compilation --------------------------------------------------------------------------
Invoke-Native 'cmake (configuration)' {
  & $cmake -S $RepoRoot -B $BuildDir -G Ninja -DCMAKE_BUILD_TYPE=Release "-DCMAKE_MAKE_PROGRAM=$ninja"
}
Invoke-Native 'cmake (compilation)' { & $cmake --build $BuildDir --target flam-player }

foreach ($file in 'flam-player.exe', 'SDL2.dll') {
  if (-not (Test-Path -LiteralPath (Join-Path $BuildDir $file))) {
    throw "$file absent de $BuildDir après la compilation"
  }
}

# --- Assemblage ---------------------------------------------------------------------------
if (Test-Path -LiteralPath $OutDir) { Remove-Item -LiteralPath $OutDir -Recurse -Force }
$licDir = Join-Path $OutDir 'licenses'
New-Item -ItemType Directory -Force -Path $licDir | Out-Null
Copy-Item -LiteralPath (Join-Path $BuildDir 'flam-player.exe') -Destination $OutDir
Copy-Item -LiteralPath (Join-Path $BuildDir 'SDL2.dll') -Destination $OutDir

# Runtime Visual C++ : flam-player.exe importe VCRUNTIME140.dll. Un Windows propre n'a pas le
# redistribuable : la DLL (fichier redistribuable de Visual Studio, voir REDIST.TXT de VS) est
# livrée à côté du player. `scripts/check-imports.ps1` contrôle les imports des binaires
# installés. Le launcher (Rust, UCRT) et SDL2 n'importent aucune DLL du runtime VC++.
$redist = Get-ChildItem -Path (Join-Path $vs 'VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT\vcruntime140.dll') -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -notmatch '\\(onecore|debug_nonredist)\\' } |
  Sort-Object FullName | Select-Object -Last 1
if (-not $redist) { throw "vcruntime140.dll introuvable dans le dossier Redist de $vs" }
Copy-Item -LiteralPath $redist.FullName -Destination $OutDir
Write-Host "runtime VC++ : $($redist.FullName)"

# Textes de licence repris des sous-modules (jamais recopiés à la main).
$utf8 = New-Object Text.UTF8Encoding $false
Copy-Item -LiteralPath (Join-Path $RepoRoot 'libs\SDL2\LICENSE.txt') -Destination (Join-Path $licDir 'SDL2-LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $RepoRoot 'libs\lvgl\LICENCE.txt') -Destination (Join-Path $licDir 'LVGL-LICENCE.txt')
Copy-Item -LiteralPath (Join-Path $RepoRoot 'libs\minimp3\LICENSE') -Destination (Join-Path $licDir 'minimp3-LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $RepoRoot 'LICENSE') -Destination (Join-Path $licDir 'flam-launcher-LICENSE.txt')
# Polices (SIL OFL 1.1), textes et copyright tirés des dépôts amont : voir third-party/.
foreach ($ofl in 'OFL-Nunito.txt', 'OFL-Montserrat.txt') {
  Copy-Item -LiteralPath (Join-Path $LauncherDir "third-party\$ofl") -Destination $licDir
}
# Lua n'a pas de fichier de licence dans son dépôt : le texte est en fin de lua.h.
$luaHeader = [IO.File]::ReadAllText((Join-Path $RepoRoot 'libs\lua\lua.h'))
$luaBlock = [regex]::Match($luaHeader, '(?s)/\*{10,}\r?\n\* Copyright \(C\) 1994-\d{4} Lua\.org.*?\*{10,}/')
if (-not $luaBlock.Success) { throw 'texte de licence introuvable dans libs/lua/lua.h' }
$luaLines = $luaBlock.Value -split "`r?`n" |
  Where-Object { $_ -notmatch '^[*/]+$' } |
  ForEach-Object { $_ -replace '^\* ?', '' }
[IO.File]::WriteAllText((Join-Path $licDir 'Lua-LICENSE.txt'), (($luaLines -join "`n") + "`n"), $utf8)

$bytes = (Get-ChildItem -LiteralPath $OutDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
Write-Host ('player prêt : {0} ({1:N1} Mo)' -f $OutDir, ($bytes / 1MB))
