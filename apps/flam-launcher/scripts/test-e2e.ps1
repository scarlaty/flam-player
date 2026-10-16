<#
.SYNOPSIS
  Lance le test de bout en bout du backend (src-tauri/tests/e2e.rs) sur l'application construite.

.DESCRIPTION
  Utilise FLAM_E2E_INSTALL_DIR s'il est défini (par exemple le dossier d'une installation),
  sinon src-tauri/target/release, où `tauri build --config src-tauri/tauri.bundle.conf.json`
  dépose flam-player.exe et telmi2flam/ à côté de l'exécutable. Préalable :
  `npm run build:converter`, `npm run build:player`, puis ce build Tauri (voir le README).
  Le test échoue (au lieu de se sauter) si un prérequis manque.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

$LauncherDir = Split-Path -Parent $PSScriptRoot
if (-not $env:FLAM_E2E_INSTALL_DIR) {
  $env:FLAM_E2E_INSTALL_DIR = Join-Path $LauncherDir 'src-tauri\target\release'
}
$env:FLAM_E2E_REQUIRE = '1'
Write-Host "application testée : $env:FLAM_E2E_INSTALL_DIR"
Push-Location (Join-Path $LauncherDir 'src-tauri')
try {
  & cargo test --test e2e -- --nocapture
  if ($LASTEXITCODE -ne 0) { throw "échec du test e2e (code $LASTEXITCODE)" }
} finally {
  Pop-Location
}
