<#
.SYNOPSIS
  Prépare le ffmpeg LGPL livré avec l'installateur Windows de flam-launcher (issue #209,
  épopée podcasts #207) : artefact épinglé, empreinte SHA-256 vérifiée, licence et offre de sources.

.DESCRIPTION
  Produit <OutDir> (par défaut apps/flam-launcher/build/ffmpeg, ignoré par Git) :

    ffmpeg/
      ffmpeg.exe, ffprobe.exe                       programmes lancés comme processus externes
      avcodec-*.dll, avformat-*.dll, avutil-*.dll,  bibliothèques dynamiques du build « shared »
      avfilter-*.dll, avdevice-*.dll, swresample-*.dll, swscale-*.dll
      LICENSE                                       texte LGPL-3.0 fourni avec le build
      BUILD.txt                                     version, sources, empreinte, configuration

  Source : build « win64-lgpl-shared » de BtbN/FFmpeg-Builds, release datée (jamais `latest`).
  Il est compilé sans `--enable-gpl` ni `--enable-nonfree` (avec `--enable-version3` : le
  résultat est sous LGPL-3.0-or-later, compatible avec la GPL-3.0-or-later du projet), embarque
  libmp3lame (LAME, LGPL) et décode AAC, MP3, Opus, Vorbis, FLAC, WAV. La variante « shared »
  garde les bibliothèques FFmpeg en DLL à côté des exécutables (relien possible, exigence LGPL)
  et pèse la moitié de la variante statique. Retirés : ffplay.exe, include/, lib/, doc/, presets/.

  Le launcher n'est jamais lié à ffmpeg : il le lance comme programme externe (#214).

  Reproductible : empreinte SHA-256 épinglée (celle publiée par GitHub pour l'artefact),
  vérifiée avant toute extraction. Aucun outil à installer : PowerShell 5.1 ou 7 suffit.

  Contrôles (le script échoue sinon) : la configuration n'a ni `--enable-gpl` ni
  `--enable-nonfree`, `-L` annonce la LGPL, `-encoders` contient libmp3lame, les décodeurs
  attendus sont présents, et un WAV synthétique (`-f lavfi -i sine`) est transcodé en MP3
  44,1 kHz relu par ffprobe.

  Changer de version : choisir une release de BtbN/FFmpeg-Builds (les releases du dernier jour
  de chaque mois sont conservées durablement, les autres sont supprimées au bout de quelques
  semaines), mettre à jour les constantes ci-dessous et la ligne ffmpeg de LICENSES.md.

.PARAMETER OutDir
  Dossier produit (supprimé puis recréé).

.PARAMETER DownloadDir
  Cache des téléchargements (réutilisé si l'empreinte correspond).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-ffmpeg.ps1
#>
[CmdletBinding()]
param(
  [string]$OutDir,
  [string]$DownloadDir
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# --- Version épinglée ---------------------------------------------------------------------
# https://github.com/BtbN/FFmpeg-Builds/releases (release datée, fin de mois : conservée).
$ReleaseTag = 'autobuild-2026-09-30-13-08'
$ZipName = 'ffmpeg-n8.1.3-9-g29e619e767-win64-lgpl-shared-8.1.zip'
$ZipUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$ReleaseTag/$ZipName"
$ZipSha256 = '3e47bda1607740550141e37c0e49d1e5182b34699f15adfd137ee266d346811a'
$FfmpegVersion = 'n8.1.3-9-g29e619e767'
# Sources correspondantes (offre de sources LGPL) : tag FFmpeg n8.1.3 + 9 commits de la branche
# release/8.1, commit exact 29e619e767cde9045a75c29bc9a8278ae7b3a98b ; scripts de build (versions
# et sources de chaque bibliothèque tierce) au commit 6c9aec5fc9a72ec3abedd1fa84db141fa18cf52b.
$SourceUrl = 'https://github.com/FFmpeg/FFmpeg/tree/29e619e767cde9045a75c29bc9a8278ae7b3a98b'
$SourceBaseTagUrl = 'https://github.com/FFmpeg/FFmpeg/tree/n8.1.3'
$BuildRepoUrl = 'https://github.com/BtbN/FFmpeg-Builds/tree/6c9aec5fc9a72ec3abedd1fa84db141fa18cf52b'
$ReleaseUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/tag/$ReleaseTag"

# Contenu conservé de bin/ (ffplay.exe et le reste de l'archive sont écartés).
$Keep = @(
  'ffmpeg.exe', 'ffprobe.exe',
  'avcodec-62.dll', 'avdevice-62.dll', 'avfilter-11.dll', 'avformat-62.dll',
  'avutil-60.dll', 'swresample-6.dll', 'swscale-9.dll'
)
$WantedDecoders = @('aac', 'mp3', 'opus', 'vorbis', 'flac', 'pcm_s16le')

# --- Chemins ------------------------------------------------------------------------------
$LauncherDir = Split-Path -Parent $PSScriptRoot
$RepoRoot = Split-Path -Parent (Split-Path -Parent $LauncherDir)
if (-not $OutDir) { $OutDir = Join-Path $LauncherDir 'build\ffmpeg' }
if (-not $DownloadDir) { $DownloadDir = Join-Path $LauncherDir 'build\downloads' }
$OutDir = [IO.Path]::GetFullPath($OutDir)
$DownloadDir = [IO.Path]::GetFullPath($DownloadDir)
$Staging = $OutDir + '.extract'

Add-Type -AssemblyName System.IO.Compression.FileSystem
$tls = [Net.SecurityProtocolType]::Tls12
if ([Enum]::GetNames([Net.SecurityProtocolType]) -contains 'Tls13') {
  $tls = $tls -bor [Net.SecurityProtocolType]::Tls13
}
[Net.ServicePointManager]::SecurityProtocol = $tls

function Get-Sha256([string]$Path) {
  (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

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
    if (Test-Path -LiteralPath $part) { Remove-Item -LiteralPath $part -Force -ErrorAction SilentlyContinue }
  }
  return $path
}

# Lance un exécutable (sans shell), PATH réduit au dossier système, et renvoie stdout + stderr
# (ffmpeg écrit ses bannières sur stderr). Échoue si le code de sortie n'est pas 0.
function Invoke-Tool([string]$Exe, [string[]]$Arguments) {
  $savedPath = $env:PATH
  $savedPref = $ErrorActionPreference
  try {
    $env:PATH = Join-Path $env:SystemRoot 'System32'
    $ErrorActionPreference = 'Continue'
    $text = (& $Exe @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
    $code = $LASTEXITCODE
  } finally {
    $env:PATH = $savedPath
    $ErrorActionPreference = $savedPref
  }
  if ($code -ne 0) { throw "échec (code $code) : $Exe $($Arguments -join ' ')`n$text" }
  return $text
}

# --- Construction -------------------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $DownloadDir | Out-Null
$zip = Get-Pinned $ZipUrl $ZipSha256

foreach ($dir in $OutDir, $Staging) {
  if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
}
try {
  [IO.Compression.ZipFile]::ExtractToDirectory($zip, $Staging)
  $root = @(Get-ChildItem -LiteralPath $Staging -Directory)
  if ($root.Count -ne 1) { throw "archive inattendue : un seul dossier racine attendu dans $ZipName" }
  $bin = Join-Path $root[0].FullName 'bin'
  New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
  foreach ($name in $Keep) {
    $src = Join-Path $bin $name
    if (-not (Test-Path -LiteralPath $src)) { throw "fichier attendu absent de l'archive : $name (liste à revoir pour cette version)" }
    Copy-Item -LiteralPath $src -Destination $OutDir
  }
  $license = Join-Path $root[0].FullName 'LICENSE.txt'
  if (-not (Test-Path -LiteralPath $license)) { throw 'LICENSE.txt absent de l''archive' }
  Copy-Item -LiteralPath $license -Destination (Join-Path $OutDir 'LICENSE')
} finally {
  if (Test-Path -LiteralPath $Staging) { Remove-Item -LiteralPath $Staging -Recurse -Force -ErrorAction SilentlyContinue }
}
if (-not (Select-String -LiteralPath (Join-Path $OutDir 'LICENSE') -Pattern 'GNU LESSER GENERAL PUBLIC LICENSE' -Quiet)) {
  throw 'ffmpeg/LICENSE : texte LGPL attendu'
}

# --- Contrôles ----------------------------------------------------------------------------
$ffmpeg = Join-Path $OutDir 'ffmpeg.exe'
$ffprobe = Join-Path $OutDir 'ffprobe.exe'

$version = Invoke-Tool $ffmpeg @('-hide_banner', '-version')
if ($version -notmatch [regex]::Escape($FfmpegVersion)) { throw "version inattendue (attendu $FfmpegVersion) :`n$version" }
$buildconf = (Invoke-Tool $ffmpeg @('-hide_banner', '-buildconf')).Trim()
foreach ($text in $version, $buildconf) {
  if ($text -match '--enable-(gpl|nonfree)(\s|$)') { throw 'configuration GPL ou nonfree : refusée' }
}
if ($buildconf -notmatch '--enable-libmp3lame') { throw 'configuration sans --enable-libmp3lame' }
$licenseText = Invoke-Tool $ffmpeg @('-hide_banner', '-L')
if ($licenseText -notmatch 'GNU Lesser General Public License') { throw "ffmpeg -L n'annonce pas la LGPL :`n$licenseText" }

$encoders = Invoke-Tool $ffmpeg @('-hide_banner', '-encoders')
if ($encoders -notmatch '\blibmp3lame\b') { throw 'encodeur libmp3lame absent' }
$decoders = Invoke-Tool $ffmpeg @('-hide_banner', '-decoders')
foreach ($d in $WantedDecoders) {
  if ($decoders -notmatch "(?m)^\s*[A-Z.]{6}\s+$([regex]::Escape($d))\s") { throw "décodeur absent : $d" }
}

# Bout en bout : WAV synthétique (22,05 kHz mono, généré par ffmpeg) -> MP3 44,1 kHz.
$work = Join-Path ([IO.Path]::GetTempPath()) ("flam-ffmpeg-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
  $wav = Join-Path $work 'sine.wav'
  $mp3 = Join-Path $work 'sine.mp3'
  Invoke-Tool $ffmpeg @('-hide_banner', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1:sample_rate=22050',
    '-ac', '1', '-c:a', 'pcm_s16le', $wav) | Out-Null
  Invoke-Tool $ffmpeg @('-hide_banner', '-nostdin', '-v', 'error', '-i', $wav, '-ar', '44100', '-c:a', 'libmp3lame', '-b:a', '64k', $mp3) | Out-Null
  $probe = Invoke-Tool $ffprobe @('-v', 'error', '-select_streams', 'a:0', '-show_entries', 'stream=codec_name,sample_rate',
    '-of', 'default=noprint_wrappers=1', $mp3)
  if ($probe -notmatch 'codec_name=mp3' -or $probe -notmatch 'sample_rate=44100') { throw "MP3 inattendu :`n$probe" }
} finally {
  Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

# --- BUILD.txt ----------------------------------------------------------------------------
$buildTxt = @"
FFmpeg $FfmpegVersion (build Windows x64 « lgpl-shared », LGPL-3.0-or-later)

Ce dossier est livré avec Flam Launcher. FFmpeg est lancé comme programme externe : il n'est
lié ni au launcher ni à flam-player. Ce build n'utilise ni --enable-gpl ni --enable-nonfree.
Texte de licence : LICENSE (dans ce dossier). Ses bibliothèques (DLL) sont fournies à côté des
exécutables : elles peuvent être remplacées par des versions modifiées compatibles.

Origine du binaire
  release : $ReleaseUrl
  archive : $ZipUrl
  sha256  : $ZipSha256

Sources correspondantes (offre de sources)
  FFmpeg  : $SourceUrl
            (tag $SourceBaseTagUrl, puis 9 commits de la branche release/8.1)
  build   : $BuildRepoUrl
            (scripts qui téléchargent, configurent et compilent FFmpeg et chaque bibliothèque
            tierce, aux versions exactes de ce build)
  Les sources complètes correspondant à ce binaire peuvent aussi être demandées aux mainteneurs
  de Flam Launcher (https://github.com/scarlaty/flam-player/issues) ; elles sont fournies sur demande.

Encodage MP3 : LAME (libmp3lame, LGPL), compilé dans avcodec.

Configuration (ffmpeg -buildconf)
$buildconf
"@
[IO.File]::WriteAllText((Join-Path $OutDir 'BUILD.txt'), $buildTxt.Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding $false))

$bytes = (Get-ChildItem -LiteralPath $OutDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
Write-Host ("ffmpeg prêt : {0} ({1:N1} Mo)" -f $OutDir, ($bytes / 1MB))
