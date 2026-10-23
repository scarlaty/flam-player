<#
.SYNOPSIS
  Construit le convertisseur telmi2flam autonome (Python embarqué officiel + Pillow + scripts)
  livré avec l'installateur Windows de flam-launcher (issue #71).

.DESCRIPTION
  Produit <OutDir> (par défaut apps/flam-launcher/build/telmi2flam, ignoré par Git) :

    telmi2flam/
      telmi2flam.exe                                        port Rust (crate crates/telmi2flam, #136)
      telmi2flam.py, lif.py, mp3map.py, engine/, runtime/   copiés depuis tools/telmi2flam
      python/                                               Python embarqué (python.org)
        python.exe, python3XX.dll, python3XX.zip, *.pyd, LICENSE.txt
        python3XX._pth                                      chemins d'import (voir plus bas)
        Lib/site-packages/PIL, pillow-*.dist-info           Pillow (roue PyPI)
      LICENSES.md                                           composants redistribués et licences
      BUILD.txt                                             versions, empreintes, commit source

  Le dossier est déposé tel quel à côté de l'exécutable par `tauri build --config
  src-tauri/tauri.bundle.conf.json` (ressource Tauri `telmi2flam/`). La surcouche y cherche
  `telmi2flam/python/python.exe` et `telmi2flam/telmi2flam.py` (moteur Python, par défaut), et
  `telmi2flam/telmi2flam.exe` (moteur Rust, réglage caché `converter_engine`, #136).

  Le binaire Rust est construit par `cargo build --release --locked -p telmi2flam` pour la cible
  explicite x86_64-pc-windows-msvc, avec la CRT Microsoft liée statiquement
  (`-C target-feature=+crt-static`) : il ne dépend d'aucune DLL VC++ (le dossier `telmi2flam/`
  n'en a pas à son niveau, et Windows ne cherche pas les DLL dans le dossier parent). La cible
  explicite isole ces options : sortie dans `<target>/x86_64-pc-windows-msvc/release`, sans
  toucher aux artefacts du launcher ni aux scripts de build (RUSTFLAGS ne s'applique pas à
  l'hôte avec `--target`). Exige `cargo` (toolchain MSVC) dans le PATH.

  Reproductible : versions épinglées, téléchargements depuis python.org et PyPI seulement,
  empreintes SHA-256 vérifiées (celles publiées par python.org et PyPI). Aucun Python n'est
  requis sur la machine de build : PowerShell 5.1 ou 7 suffit.

  Allègement : modules inutiles au convertisseur retirés (sqlite3, SSL/OpenSSL, hashlib
  OpenSSL : `hashlib.md5` retombe sur l'implémentation intégrée à python3XX.dll ; AVIF,
  FreeType, LittleCMS et le module Tk de Pillow). Chaque fichier retiré doit exister : changer de version
  oblige à relire la liste.

.PARAMETER OutDir
  Dossier produit (supprimé puis recréé).

.PARAMETER DownloadDir
  Cache des téléchargements (réutilisé si l'empreinte correspond).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-converter.ps1
#>
[CmdletBinding()]
param(
  [string]$OutDir,
  [string]$DownloadDir
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# --- Versions épinglées -------------------------------------------------------------------
# Python : https://www.python.org/downloads/release/ (« Windows embeddable package (64-bit) »),
# empreinte SHA-256 publiée par python.org (API /api/v2/downloads/release_file/).
$PythonVersion = '3.14.8'
$PythonTag = '314'
$PythonUrl = "https://www.python.org/ftp/python/$PythonVersion/python-$PythonVersion-embed-amd64.zip"
$PythonSha256 = 'a93abe456ab01bd96d7a085b3cdb6566b3063f4241360d114142fbdb07f0a310'

# Pillow : https://pypi.org/project/pillow/#files, empreinte SHA-256 publiée par PyPI.
$PillowVersion = '12.3.0'
$PillowWheel = "pillow-$PillowVersion-cp$PythonTag-cp$PythonTag-win_amd64.whl"
$PillowUrl = "https://files.pythonhosted.org/packages/f1/e0/492879f69d94f91f60fc8cd05ba03650e9520afebb2fb7aa12777d7c7f38/$PillowWheel"
$PillowSha256 = 'fdafc9cce40277e0f7a0feabce0ee50dd2fa1800f3b38015e51296b5e814048d'

# Fichiers retirés (inutiles au convertisseur).
$TrimPython = @(
  '_sqlite3.pyd', 'sqlite3.dll',
  '_ssl.pyd', 'libssl-3.dll',
  '_hashlib.pyd', 'libcrypto-3.dll'
)
$TrimPillow = @(
  "_avif.cp$PythonTag-win_amd64.pyd",
  "_imagingft.cp$PythonTag-win_amd64.pyd",
  "_imagingcms.cp$PythonTag-win_amd64.pyd",
  "_imagingtk.cp$PythonTag-win_amd64.pyd"
)

# Fichiers de tools/telmi2flam nécessaires à la conversion.
$ConverterFiles = @('telmi2flam.py', 'lif.py', 'mp3map.py')
$ConverterDirs = @('engine', 'runtime')

# --- Chemins ------------------------------------------------------------------------------
$LauncherDir = Split-Path -Parent $PSScriptRoot
$RepoRoot = Split-Path -Parent (Split-Path -Parent $LauncherDir)
$SourceDir = Join-Path $RepoRoot 'tools\telmi2flam'
if (-not $OutDir) { $OutDir = Join-Path $LauncherDir 'build\telmi2flam' }
if (-not $DownloadDir) { $DownloadDir = Join-Path $LauncherDir 'build\downloads' }
$OutDir = [IO.Path]::GetFullPath($OutDir)
$DownloadDir = [IO.Path]::GetFullPath($DownloadDir)
$PythonDir = Join-Path $OutDir 'python'
$SitePackages = Join-Path $PythonDir 'Lib\site-packages'

Add-Type -AssemblyName System.IO.Compression.FileSystem
# TLS 1.2, plus TLS 1.3 quand le .NET de la machine le connaît (absent de PowerShell 5.1 sur
# les Windows anciens : `Tls13` n'existe alors pas dans l'énumération).
$tls = [Net.SecurityProtocolType]::Tls12
if ([Enum]::GetNames([Net.SecurityProtocolType]) -contains 'Tls13') {
  $tls = $tls -bor [Net.SecurityProtocolType]::Tls13
}
[Net.ServicePointManager]::SecurityProtocol = $tls

function Get-Sha256([string]$Path) {
  (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

# Télécharge $Url dans le cache (sauf si déjà présent avec la bonne empreinte), puis vérifie.
function Get-Pinned([string]$Url, [string]$Sha256) {
  $name = [IO.Path]::GetFileName(([Uri]$Url).AbsolutePath)
  $path = Join-Path $DownloadDir $name
  if ((Test-Path -LiteralPath $path) -and (Get-Sha256 $path) -eq $Sha256) {
    Write-Host "cache : $name"
    return $path
  }
  Write-Host "téléchargement : $Url"
  $part = "$path.part"
  try {
    Invoke-WebRequest -Uri $Url -OutFile $part -UseBasicParsing
    $actual = Get-Sha256 $part
    if ($actual -ne $Sha256) {
      throw "empreinte inattendue pour $name : $actual (attendu $Sha256)"
    }
    Move-Item -LiteralPath $part -Destination $path -Force
  } finally {
    # Échec réseau, empreinte inattendue ou interruption : aucun `.part` ne reste dans le cache.
    if (Test-Path -LiteralPath $part) { Remove-Item -LiteralPath $part -Force -ErrorAction SilentlyContinue }
  }
  return $path
}

function Remove-Listed([string]$Dir, [string[]]$Names) {
  foreach ($name in $Names) {
    $path = Join-Path $Dir $name
    if (-not (Test-Path -LiteralPath $path)) {
      throw "fichier à retirer absent : $path (liste à revoir pour cette version)"
    }
    Remove-Item -LiteralPath $path -Force
  }
}

# Lance l'interpréteur embarqué avec un PATH réduit au dossier système (Python masqué).
function Invoke-Embedded([string[]]$Arguments) {
  $saved = $env:PATH
  try {
    $env:PATH = Join-Path $env:SystemRoot 'System32'
    & (Join-Path $PythonDir 'python.exe') @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "échec (code $LASTEXITCODE) : python $($Arguments -join ' ')" }
  } finally {
    $env:PATH = $saved
  }
}

# --- Construction -------------------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $DownloadDir | Out-Null
$pythonZip = Get-Pinned $PythonUrl $PythonSha256
$pillowWhl = Get-Pinned $PillowUrl $PillowSha256

if (Test-Path -LiteralPath $OutDir) { Remove-Item -LiteralPath $OutDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $SitePackages | Out-Null

[IO.Compression.ZipFile]::ExtractToDirectory($pythonZip, $PythonDir)
[IO.Compression.ZipFile]::ExtractToDirectory($pillowWhl, $SitePackages)
Remove-Listed $PythonDir $TrimPython
Remove-Listed (Join-Path $SitePackages 'PIL') $TrimPillow

# Chemins d'import. Avec un fichier `._pth`, Python ignore les variables PYTHON*, n'importe
# pas `site` et n'ajoute PAS le dossier du script à sys.path : `..` (le dossier telmi2flam/)
# le remplace, pour `import lif` / `import mp3map`. Chemins relatifs au dossier python/.
$pth = Join-Path $PythonDir "python$PythonTag._pth"
if (-not (Test-Path -LiteralPath $pth)) { throw "fichier attendu absent : $pth" }
[IO.File]::WriteAllText($pth, "python$PythonTag.zip`r`n.`r`nLib\site-packages`r`n..`r`n")

foreach ($file in $ConverterFiles) {
  Copy-Item -LiteralPath (Join-Path $SourceDir $file) -Destination $OutDir
}

# Binaire Rust (#136). `cargo metadata` donne le dossier cible effectif (CARGO_TARGET_DIR compris).
$RustTarget = 'x86_64-pc-windows-msvc'
$manifest = Join-Path $RepoRoot 'Cargo.toml'
# Sortie UTF-8 de cargo : décodée en UTF-8 (sinon page OEM, chemin cible non ASCII déformé).
$savedEncoding = [Console]::OutputEncoding
try {
  [Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
  $metadata = & cargo metadata --format-version 1 --no-deps --locked --manifest-path $manifest
} finally {
  [Console]::OutputEncoding = $savedEncoding
}
if ($LASTEXITCODE -ne 0) { throw "cargo metadata : échec (code $LASTEXITCODE)" }
$cargoTarget = (($metadata -join "`n") | ConvertFrom-Json).target_directory
$savedFlags = $env:RUSTFLAGS
$savedEncoded = $env:CARGO_ENCODED_RUSTFLAGS
try {
  $env:RUSTFLAGS = '-C target-feature=+crt-static'
  # Prioritaire sur RUSTFLAGS s'il est défini : retiré le temps du build.
  Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
  & cargo build --manifest-path $manifest --release --locked -p telmi2flam --bin telmi2flam --target $RustTarget | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "cargo build telmi2flam : échec (code $LASTEXITCODE)" }
} finally {
  $env:RUSTFLAGS = $savedFlags
  if ($null -ne $savedEncoded) { $env:CARGO_ENCODED_RUSTFLAGS = $savedEncoded }
}
$rustExe = Join-Path $cargoTarget "$RustTarget\release\telmi2flam.exe"
if (-not (Test-Path -LiteralPath $rustExe)) { throw "binaire attendu absent : $rustExe" }
Copy-Item -LiteralPath $rustExe -Destination (Join-Path $OutDir 'telmi2flam.exe')
foreach ($dir in $ConverterDirs) {
  Copy-Item -LiteralPath (Join-Path $SourceDir $dir) -Destination $OutDir -Recurse
}

$pillowLicense = Get-ChildItem -LiteralPath $SitePackages -Recurse -File |
  Where-Object { $_.Directory.Name -in @('licenses', "pillow-$PillowVersion.dist-info") -and $_.Name -like 'LICENSE*' } |
  Select-Object -First 1
if (-not $pillowLicense) { throw "licence de Pillow introuvable dans la roue" }
$pillowLicenseRel = $pillowLicense.FullName.Substring($OutDir.Length + 1).Replace('\', '/')

$licenses = @"
# Composants redistribués avec le convertisseur telmi2flam

Ce dossier (``telmi2flam/``) est livré avec l'installateur de flam-launcher. Les textes de
licence complets des composants tiers sont dans les fichiers cités ci-dessous (le texte du
fichier fait foi, pas ce tableau). Le fichier ``LICENSES.md`` à la racine de l'installation
couvre le reste de l'application (flam-launcher, flam-player, SDL2, Lua, LVGL, minimp3).

| Composant | Version | Licence | Texte |
| --- | --- | --- | --- |
| telmi2flam (``telmi2flam.py``, ``lif.py``, ``mp3map.py``, moteur Lua ``engine/``) | dépôt flam-player | GPL-3.0-or-later | ``LICENSE`` du dépôt (``licenses/flam-launcher-LICENSE.txt`` à la racine de l'installation) |
| ``telmi2flam.exe`` (port Rust, crate ``crates/telmi2flam``, moteur et runtime inclus) | dépôt flam-player | GPL-3.0-or-later, et HPND pour le code dérivé de Pillow (``resample.rs``, ``blur.rs``) | ``licenses/flam-launcher-LICENSE.txt`` et ``THIRD-PARTY-NOTICES.txt`` (crates Rust et texte de Pillow) à la racine de l'installation |
| Runtime C Microsoft (CRT statique) compilé dans ``telmi2flam.exe`` | Visual Studio | Microsoft Visual C++ : bibliothèques redistribuables selon les conditions de licence de Microsoft | |
| Runtime ``runtime/`` (bibliothèque Lua ``script/``, icônes LIF ``img/``) | dépôt flam-player | GPL-3.0-or-later | ``LICENSE`` du dépôt (``licenses/flam-launcher-LICENSE.txt`` à la racine de l'installation) |
| Python (python.org, paquet embarquable) | $PythonVersion | PSF-2.0 | ``python/LICENSE.txt`` |
| libffi (``python/libffi-8.dll``) | livré avec Python | MIT | ``python/LICENSE.txt`` |
| libtommath (``python/libtommath.dll``) | livré avec Python | domaine public (Unlicense) | |
| bzip2, expat, mpdecimal, xz/liblzma, zstd, zlib (compilés dans les modules ``*.pyd`` et ``python3XX.dll``) | livrés avec Python | licences permissives (BSD, MIT, zlib, domaine public) | ``python/LICENSE.txt`` |
| ``python.cat`` | livré avec Python | catalogue de signature (Python Software Foundation) livré tel quel avec le paquet officiel | |
| ``vcruntime140.dll``, ``vcruntime140_1.dll`` | livrés avec Python | Microsoft Visual C++ Redistributable : fichiers redistribuables selon les conditions de licence de Microsoft | |
| Pillow (PyPI) | $PillowVersion | MIT-CMU (HPND) | ``$pillowLicenseRel`` |
| Bibliothèques intégrées à la roue de Pillow (libjpeg-turbo, libpng, zlib, libtiff, libwebp, brotli, harfbuzz, FreeType, LittleCMS…) | livrées avec Pillow | licences de chaque bibliothèque | ``$pillowLicenseRel`` (ce fichier regroupe tous les textes de la roue, y compris de bibliothèques retirées ci-dessous) |

Fichiers retirés du paquet officiel (inutiles à la conversion) : $($TrimPython -join ', '), PIL/$($TrimPillow -join ', PIL/').
"@
[IO.File]::WriteAllText((Join-Path $OutDir 'LICENSES.md'), $licenses, (New-Object Text.UTF8Encoding $false))

$commit = 'inconnu'
try {
  $rev = & git -C $RepoRoot rev-parse HEAD 2>$null
  if ($LASTEXITCODE -eq 0 -and $rev) { $commit = $rev.Trim() }
} catch { }
$build = @"
python $PythonVersion sha256:$PythonSha256 $PythonUrl
pillow $PillowVersion sha256:$PillowSha256 $PillowUrl
telmi2flam.py sha256:$(Get-Sha256 (Join-Path $OutDir 'telmi2flam.py'))
telmi2flam.exe sha256:$(Get-Sha256 (Join-Path $OutDir 'telmi2flam.exe')) cargo build --release --locked --target $RustTarget (crt-static)
commit $commit
"@
[IO.File]::WriteAllText((Join-Path $OutDir 'BUILD.txt'), $build, (New-Object Text.UTF8Encoding $false))

# --- Contrôles ----------------------------------------------------------------------------
# Lancement tel que la surcouche le fait (convert.rs), sans Python dans le PATH.
$script = Join-Path $OutDir 'telmi2flam.py'
Invoke-Embedded @('-E', '-s', '-B', '-u', '-X', 'utf8', $script, '--version')
# Modules utilisés par la conversion (dont `lif`/`mp3map` via le `._pth`, et `hashlib.md5`
# sans OpenSSL).
Invoke-Embedded @('-E', '-s', '-B', '-X', 'utf8', '-c',
  'import hashlib, unicodedata, zipfile, zlib, lif, mp3map; from PIL import Image, ImageChops, PngImagePlugin; hashlib.md5(bytes())')
# Assistant Ctrl+Break de convert.rs : `-I` et ctypes.
Invoke-Embedded @('-I', '-B', '-c', 'import ctypes, sys; ctypes.windll.kernel32')
# Binaire Rust lancé avec un PATH réduit au dossier système (aucune DLL VC++ requise) :
# `--version`, avec l'option `--cancel-on-stdin-eof` passée par la surcouche.
$saved = $env:PATH
try {
  $env:PATH = Join-Path $env:SystemRoot 'System32'
  $version = & (Join-Path $OutDir 'telmi2flam.exe') --cancel-on-stdin-eof --version
  if ($LASTEXITCODE -ne 0) { throw "telmi2flam.exe --version : échec (code $LASTEXITCODE)" }
  Write-Host "telmi2flam.exe : $version"
} finally {
  $env:PATH = $saved
}

$bytes = (Get-ChildItem -LiteralPath $OutDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
Write-Host ("convertisseur prêt : {0} ({1:N1} Mo)" -f $OutDir, ($bytes / 1MB))
