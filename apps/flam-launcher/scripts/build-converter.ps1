<#
.SYNOPSIS
  Construit le convertisseur telmi2flam autonome (binaire Rust) livré avec l'installateur
  Windows de flam-launcher (issues #71, #136, #138).

.DESCRIPTION
  Produit <OutDir> (par défaut apps/flam-launcher/build/telmi2flam, ignoré par Git) :

    telmi2flam/
      telmi2flam.exe   port Rust (crate crates/telmi2flam) : moteur Lua et runtime du projet
                       sont inclus dans le binaire, rien d'autre n'est à livrer à côté
      LICENSES.md      composants redistribués et licences
      BUILD.txt        empreinte, commande de build, commit source

  Le dossier est déposé tel quel à côté de l'exécutable par `tauri build --config
  src-tauri/tauri.bundle.conf.json` (ressource Tauri `telmi2flam/`). La surcouche y cherche
  `telmi2flam/telmi2flam.exe`. Aucun Python, aucun Pillow, aucun script n'est livré (#138).

  Le binaire Rust est construit par `cargo build --release --locked -p telmi2flam` pour la cible
  explicite x86_64-pc-windows-msvc, avec la CRT Microsoft liée statiquement
  (`-C target-feature=+crt-static`) : il ne dépend d'aucune DLL VC++ (le dossier `telmi2flam/`
  n'en a pas à son niveau, et Windows ne cherche pas les DLL dans le dossier parent). La cible
  explicite isole ces options : sortie dans `<target>/x86_64-pc-windows-msvc/release`, sans
  toucher aux artefacts du launcher ni aux scripts de build (RUSTFLAGS ne s'applique pas à
  l'hôte avec `--target`). Exige `cargo` (toolchain MSVC) dans le PATH ; aucun téléchargement.

.PARAMETER OutDir
  Dossier produit (supprimé puis recréé).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-converter.ps1
#>
[CmdletBinding()]
param(
  [string]$OutDir
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# --- Chemins ------------------------------------------------------------------------------
$LauncherDir = Split-Path -Parent $PSScriptRoot
$RepoRoot = Split-Path -Parent (Split-Path -Parent $LauncherDir)
if (-not $OutDir) { $OutDir = Join-Path $LauncherDir 'build\telmi2flam' }
$OutDir = [IO.Path]::GetFullPath($OutDir)

function Get-Sha256([string]$Path) {
  (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

# --- Construction -------------------------------------------------------------------------
if (Test-Path -LiteralPath $OutDir) { Remove-Item -LiteralPath $OutDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# `cargo metadata` donne le dossier cible effectif (CARGO_TARGET_DIR compris).
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

$licenses = @"
# Composants redistribués avec le convertisseur telmi2flam

Ce dossier (``telmi2flam/``) est livré avec l'installateur de flam-launcher. Le fichier
``LICENSES.md`` à la racine de l'installation couvre le reste de l'application (flam-launcher,
flam-player, SDL2, Lua, LVGL, minimp3) ; ``THIRD-PARTY-NOTICES.txt`` liste les crates Rust et
reprend le texte de licence du code dérivé de Pillow.

| Composant | Version | Licence | Texte |
| --- | --- | --- | --- |
| ``telmi2flam.exe`` (port Rust, crate ``crates/telmi2flam`` ; moteur Lua ``engine/`` et runtime du projet ``runtime/`` (bibliothèque Lua ``script/``, icônes LIF ``img/``) inclus dans le binaire) | dépôt flam-player | GPL-3.0-or-later, et HPND pour le code dérivé de Pillow (``resample.rs``, ``blur.rs``) | ``licenses/flam-launcher-LICENSE.txt`` et ``THIRD-PARTY-NOTICES.txt`` (crates Rust et texte de Pillow) à la racine de l'installation |
| Runtime C Microsoft (CRT statique) compilé dans ``telmi2flam.exe`` ; ``vcruntime140.dll`` livrée à côté de ``flam-player.exe`` | Visual Studio | Microsoft Visual C++ : bibliothèques redistribuables selon les conditions de licence de Microsoft | |

Aucun Python ni aucune bibliothèque Pillow n'est livré : seul un algorithme de Pillow 12.3.0 est
porté en Rust (voir ci-dessus).
"@
[IO.File]::WriteAllText((Join-Path $OutDir 'LICENSES.md'), $licenses, (New-Object Text.UTF8Encoding $false))

$commit = 'inconnu'
try {
  $rev = & git -C $RepoRoot rev-parse HEAD 2>$null
  if ($LASTEXITCODE -eq 0 -and $rev) { $commit = $rev.Trim() }
} catch { }
$build = @"
telmi2flam.exe sha256:$(Get-Sha256 (Join-Path $OutDir 'telmi2flam.exe')) cargo build --release --locked --target $RustTarget (crt-static)
commit $commit
"@
[IO.File]::WriteAllText((Join-Path $OutDir 'BUILD.txt'), $build, (New-Object Text.UTF8Encoding $false))

# --- Contrôles ----------------------------------------------------------------------------
# Binaire Rust lancé avec un PATH réduit au dossier système (aucune DLL VC++, aucun Python) :
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
# Aucun résidu de Python ni de scripts : le dossier ne contient que ce qui est listé plus haut.
$unexpected = @(Get-ChildItem -LiteralPath $OutDir -Force | Where-Object { $_.Name -notin @('telmi2flam.exe', 'LICENSES.md', 'BUILD.txt') })
if ($unexpected.Count -gt 0) { throw "fichier(s) inattendu(s) dans $OutDir : $($unexpected.Name -join ', ')" }

$bytes = (Get-ChildItem -LiteralPath $OutDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
Write-Host ("convertisseur prêt : {0} ({1:N1} Mo)" -f $OutDir, ($bytes / 1MB))
