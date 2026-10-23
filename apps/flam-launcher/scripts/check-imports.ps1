<#
.SYNOPSIS
  Vérifie que les exécutables installés n'importent aucune DLL du runtime Visual C++ sans
  l'avoir à leur côté (revue de #86 : un Windows propre n'a pas le redistribuable).

.DESCRIPTION
  Pour chaque binaire (flam-launcher.exe, flam-player.exe, SDL2.dll, telmi2flam/telmi2flam.exe)
  du dossier d'installation, liste les imports avec `dumpbin /dependents`. Échec si :
    - un import du runtime VC++ (vcruntime*, msvcp*, vcomp*, concrt*, vccorlib*) n'est pas dans
      le dossier du binaire (le dossier System32 d'un poste de développement ou d'un runner
      en contient une copie : il ne compte pas ; Windows ne cherche pas dans le dossier parent,
      d'où la CRT statique de telmi2flam.exe, #136) ;
    - une autre DLL n'est ni dans le dossier du binaire, ni dans System32 (les DLL
      virtuelles `api-ms-win-*` / `ext-ms-win-*` sont des API Windows et sont acceptées).

.PARAMETER InstallDir
  Dossier de l'application installée.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$InstallDir)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = & $vswhere -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -latest -property installationPath
$dumpbin = Get-ChildItem -Path (Join-Path $vs 'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe') |
  Sort-Object FullName | Select-Object -Last 1
if (-not $dumpbin) { throw "dumpbin introuvable sous $vs" }

$system32 = Join-Path $env:SystemRoot 'System32'
$vcRuntime = '^(vcruntime|msvcp|vcomp|concrt|vccorlib)'
$failures = @()
foreach ($name in 'flam-launcher.exe', 'flam-player.exe', 'SDL2.dll', 'telmi2flam\telmi2flam.exe') {
  $binary = Join-Path $InstallDir $name
  if (-not (Test-Path -LiteralPath $binary)) { $failures += "$name absent de $InstallDir"; continue }
  $binaryDir = Split-Path -Parent $binary
  $imports = & $dumpbin.FullName /nologo /dependents $binary |
    ForEach-Object { $_.Trim() } | Where-Object { $_ -match '\.dll$' -and $_ -notmatch '^Dump of' }
  Write-Host "$name : $($imports -join ', ')"
  foreach ($dll in $imports) {
    $local = Test-Path -LiteralPath (Join-Path $binaryDir $dll)
    if ($dll -match $vcRuntime) {
      if (-not $local) { $failures += "$name importe $dll, non livrée dans $binaryDir" }
    } elseif ($dll -notmatch '^(api|ext)-ms-win-' -and -not $local -and
        -not (Test-Path -LiteralPath (Join-Path $system32 $dll))) {
      $failures += "$name importe $dll, introuvable (ni livrée, ni dans System32)"
    }
  }
}
if ($failures) { throw ("imports non satisfaits :`n" + ($failures -join "`n")) }
Write-Host 'imports : OK'
