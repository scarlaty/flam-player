<#
.SYNOPSIS
  Prépare le ffmpeg LGPL minimal (audio seulement) livré avec l'installateur Windows de
  flam-launcher (issue #209, épopée podcasts #207) : sources épinglées, licences, offre de sources.

.DESCRIPTION
  Produit <OutDir> (par défaut apps/flam-launcher/build/ffmpeg, ignoré par Git) :

    ffmpeg/
      ffmpeg.exe, ffprobe.exe     exécutables statiques (aucune DLL), lancés comme processus externes
      LICENSE                     LGPL-2.1 de FFmpeg
      LICENSE-LAME                licence de LAME (LGPL, GNU Library GPL v2 ; encodeur MP3 libmp3lame), + LICENSE-LAME-NOTICE
      LICENSE-MINGW-RUNTIME       mentions du runtime MinGW-w64 lié statiquement
      BUILD.txt                   versions, sources exactes, empreintes, configuration (-buildconf)

  Le binaire est construit par nous (tools/ffmpeg/build.sh : cross-compilation MinGW-w64 sous
  Linux) depuis les tarballs FFmpeg et LAME épinglés dans tools/ffmpeg/pins.env (SHA-256
  vérifiés). Configuration : `--disable-everything --disable-autodetect --disable-network`, ni
  `--enable-gpl` ni `--enable-nonfree`, `--enable-libmp3lame` ; seuls les décodeurs, démuxeurs,
  filtres et le protocole `file` utiles à la conversion de podcasts sont activés.

  Où trouver le binaire :
    -ArtifactDir <dossier>   sortie de tools/ffmpeg/build.sh déjà construite (la CI la produit
                             dans un job Linux, puis la télécharge ici) ;
    sans paramètre           construit avec Docker (image ubuntu:24.04) si `docker` est disponible,
                             sinon échoue en expliquant quoi faire.

  Contrôles (le script échoue sinon) : la configuration n'a ni `--enable-gpl` ni
  `--enable-nonfree`, `-L` annonce la LGPL, `libmp3lame` est dans `-encoders`, les décodeurs
  attendus sont présents, aucun protocole réseau, et deux transcodages vérifiés par ffprobe :
  WAV synthétique (`-f lavfi -i sine`) vers MP3 44,1 kHz, puis MP3 mono 22 050 Hz vers MP3 44 100 Hz.

  Relien LGPL : le binaire étant statique, la liaison est refaite en reconstruisant avec
  tools/ffmpeg/build.sh (sources et script cités dans BUILD.txt).

.PARAMETER OutDir
  Dossier produit (supprimé puis recréé).

.PARAMETER ArtifactDir
  Sortie de build.sh (ffmpeg.exe, ffprobe.exe, LICENSE, LICENSE-LAME, configure-args.txt).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-ffmpeg.ps1
#>
[CmdletBinding()]
param(
  [string]$OutDir,
  [string]$ArtifactDir
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

# --- Chemins et versions épinglées --------------------------------------------------------
$LauncherDir = Split-Path -Parent $PSScriptRoot
$RepoRoot = Split-Path -Parent (Split-Path -Parent $LauncherDir)
$FfmpegToolsDir = Join-Path $RepoRoot 'tools\ffmpeg'
if (-not $OutDir) { $OutDir = Join-Path $LauncherDir 'build\ffmpeg' }
$OutDir = [IO.Path]::GetFullPath($OutDir)

$pins = @{}
foreach ($line in Get-Content -LiteralPath (Join-Path $FfmpegToolsDir 'pins.env')) {
  if ($line -match '^([A-Z0-9_]+)=(.+)$') { $pins[$Matches[1]] = $Matches[2] }
}
foreach ($key in 'FFMPEG_VERSION', 'FFMPEG_URL', 'FFMPEG_SHA256', 'FFMPEG_SOURCE_PAGE', 'LAME_VERSION', 'LAME_URL', 'LAME_SHA256', 'LAME_SOURCE_PAGE') {
  if (-not $pins.ContainsKey($key)) { throw "tools/ffmpeg/pins.env : $key manquant" }
}

$WantedDecoders = @('aac', 'mp3', 'opus', 'vorbis', 'flac', 'pcm_s16le')
$Shipped = @('ffmpeg.exe', 'ffprobe.exe', 'LICENSE', 'LICENSE-LAME', 'LICENSE-MINGW-RUNTIME')

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

function Get-Stream([string]$Ffprobe, [string]$File) {
  Invoke-Tool $Ffprobe @('-v', 'error', '-select_streams', 'a:0', '-show_entries', 'stream=codec_name,sample_rate,channels',
    '-of', 'default=noprint_wrappers=1', $File)
}

# --- Binaire : artefact fourni, sinon construction par Docker -----------------------------
$builtHere = $null
if (-not $ArtifactDir) {
  if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw 'aucun binaire fourni : passer -ArtifactDir <sortie de tools/ffmpeg/build.sh> (produite par la CI, job « ffmpeg »), ou installer Docker pour le construire ici'
  }
  $builtHere = Join-Path ([IO.Path]::GetTempPath()) ('flam-ffmpeg-build-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $builtHere | Out-Null
  Write-Host 'construction de ffmpeg avec Docker (ubuntu:24.04)...'
  $env:MSYS_NO_PATHCONV = '1'
  & docker run --rm -e INSTALL_DEPS=1 -v "${RepoRoot}:/src:ro" -v "${builtHere}:/out" ubuntu:24.04 bash /src/tools/ffmpeg/build.sh /out
  if ($LASTEXITCODE -ne 0) { throw "échec de la construction Docker (code $LASTEXITCODE)" }
  $ArtifactDir = $builtHere
}
$ArtifactDir = [IO.Path]::GetFullPath($ArtifactDir)

try {
  if (Test-Path -LiteralPath $OutDir) { Remove-Item -LiteralPath $OutDir -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
  try {
    foreach ($name in $Shipped + @('configure-args.txt')) {
      if (-not (Test-Path -LiteralPath (Join-Path $ArtifactDir $name))) { throw "fichier attendu absent de l'artefact : $name" }
    }
    foreach ($name in $Shipped) { Copy-Item -LiteralPath (Join-Path $ArtifactDir $name) -Destination $OutDir }
    $notice = Join-Path $ArtifactDir 'LICENSE-LAME-NOTICE'
    if (Test-Path -LiteralPath $notice) { Copy-Item -LiteralPath $notice -Destination $OutDir }

    foreach ($file in 'LICENSE', 'LICENSE-LAME') {
      if (-not (Select-String -LiteralPath (Join-Path $OutDir $file) -Pattern 'GNU (LESSER|LIBRARY) GENERAL PUBLIC LICENSE' -Quiet)) { throw "$file : texte LGPL attendu" }
    }

    # --- Contrôles ------------------------------------------------------------------------
    $ffmpeg = Join-Path $OutDir 'ffmpeg.exe'
    $ffprobe = Join-Path $OutDir 'ffprobe.exe'

    $version = Invoke-Tool $ffmpeg @('-hide_banner', '-version')
    if ($version -notmatch ('ffmpeg version ' + [regex]::Escape($pins['FFMPEG_VERSION']))) { throw "version inattendue (attendu $($pins['FFMPEG_VERSION'])) :`n$version" }
    $buildconf = (Invoke-Tool $ffmpeg @('-hide_banner', '-buildconf')).Trim()
    foreach ($text in $version, $buildconf) {
      if ($text -match '--enable-(gpl|nonfree)(\s|$)') { throw 'configuration GPL ou nonfree : refusée' }
    }
    if ($buildconf -notmatch '--enable-libmp3lame') { throw 'configuration sans --enable-libmp3lame' }
    if ($buildconf -notmatch '--disable-network') { throw 'configuration sans --disable-network' }
    $licenseText = Invoke-Tool $ffmpeg @('-hide_banner', '-L')
    if ($licenseText -notmatch 'GNU\s+Lesser\s+General\s+Public\s+License') { throw "ffmpeg -L n'annonce pas la LGPL :`n$licenseText" }

    $encoders = Invoke-Tool $ffmpeg @('-hide_banner', '-encoders')
    if ($encoders -notmatch '\blibmp3lame\b') { throw 'encodeur libmp3lame absent' }
    $decoders = Invoke-Tool $ffmpeg @('-hide_banner', '-decoders')
    foreach ($d in $WantedDecoders) {
      if ($decoders -notmatch "(?m)^\s*[A-Z.]{6}\s+$([regex]::Escape($d))\s") { throw "décodeur absent : $d" }
    }
    $protocols = Invoke-Tool $ffmpeg @('-hide_banner', '-protocols')
    if ($protocols -match '\b(http|https|tcp|udp|ftp|rtmp|tls)\b') { throw "protocole réseau présent :`n$protocols" }

    # Bout en bout : WAV synthétique -> MP3 44,1 kHz ; MP3 mono 22 050 Hz -> MP3 44 100 Hz.
    $work = Join-Path ([IO.Path]::GetTempPath()) ('flam-ffmpeg-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
      $wav = Join-Path $work 'sine.wav'
      $mp3 = Join-Path $work 'sine.mp3'
      $mono = Join-Path $work 'mono-22050.mp3'
      $up = Join-Path $work 'mono-44100.mp3'
      $common = @('-hide_banner', '-nostdin', '-v', 'error')
      Invoke-Tool $ffmpeg ($common + @('-f', 'lavfi', '-i', 'sine=frequency=440:duration=1:sample_rate=22050', '-ac', '1', '-c:a', 'pcm_s16le', $wav)) | Out-Null
      Invoke-Tool $ffmpeg ($common + @('-i', $wav, '-ar', '44100', '-c:a', 'libmp3lame', '-b:a', '64k', $mp3)) | Out-Null
      $probe = Get-Stream $ffprobe $mp3
      if ($probe -notmatch 'codec_name=mp3' -or $probe -notmatch 'sample_rate=44100') { throw "MP3 inattendu :`n$probe" }
      Invoke-Tool $ffmpeg ($common + @('-i', $wav, '-ar', '22050', '-ac', '1', '-c:a', 'libmp3lame', '-b:a', '32k', $mono)) | Out-Null
      $probe = Get-Stream $ffprobe $mono
      if ($probe -notmatch 'sample_rate=22050' -or $probe -notmatch 'channels=1') { throw "MP3 mono 22 050 Hz inattendu :`n$probe" }
      Invoke-Tool $ffmpeg ($common + @('-i', $mono, '-vn', '-ar', '44100', '-c:a', 'libmp3lame', '-b:a', '64k', $up)) | Out-Null
      $probe = Get-Stream $ffprobe $up
      if ($probe -notmatch 'codec_name=mp3' -or $probe -notmatch 'sample_rate=44100') { throw "MP3 44 100 Hz inattendu :`n$probe" }
    } finally {
      Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    # --- BUILD.txt ------------------------------------------------------------------------
    $commit = 'inconnu'
    try {
      $rev = & git -C $RepoRoot rev-parse HEAD 2>$null
      if ($LASTEXITCODE -eq 0 -and $rev) { $commit = $rev.Trim() }
    } catch { }
    $configureArgs = (Get-Content -LiteralPath (Join-Path $ArtifactDir 'configure-args.txt')) -join "`n"
    $buildTxt = @"
FFmpeg $($pins['FFMPEG_VERSION']) minimal, audio seulement (Windows x64, statique, LGPL-2.1-or-later)
avec LAME $($pins['LAME_VERSION']) (libmp3lame, LGPL : GNU Library GPL v2)

Ce dossier est livré avec Flam Launcher. FFmpeg est lancé comme programme externe : il n'est
lié ni au launcher ni à flam-player. Ce build n'utilise ni --enable-gpl ni --enable-nonfree.
Textes de licence : LICENSE (FFmpeg), LICENSE-LAME (LAME) et LICENSE-MINGW-RUNTIME (runtime
MinGW-w64 : libgcc, winpthreads, runtime C, liés statiquement), dans ce dossier.

Sources exactes (offre de sources LGPL)
  FFmpeg $($pins['FFMPEG_VERSION']) : $($pins['FFMPEG_URL'])
    sha256 $($pins['FFMPEG_SHA256'])  (tag : $($pins['FFMPEG_SOURCE_PAGE']))
  LAME $($pins['LAME_VERSION']) : $($pins['LAME_URL'])
    sha256 $($pins['LAME_SHA256'])  ($($pins['LAME_SOURCE_PAGE']))
  Script de construction (cross-compilation MinGW-w64) : tools/ffmpeg/build.sh et
    tools/ffmpeg/pins.env dans https://github.com/scarlaty/flam-player, commit $commit.
  Les exécutables étant statiques, la liaison avec une version modifiée de FFmpeg ou de LAME se
  refait en modifiant ces sources puis en relançant tools/ffmpeg/build.sh. Les mainteneurs de
  Flam Launcher fournissent aussi ces sources sur demande, au prix de la distribution et pendant
  au moins trois ans (https://github.com/scarlaty/flam-player/issues).

Configuration (ffmpeg -buildconf)
$buildconf

Arguments de configure (tools/ffmpeg/build.sh)
$configureArgs
"@
    [IO.File]::WriteAllText((Join-Path $OutDir 'BUILD.txt'), $buildTxt.Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding $false))
  } catch {
    # Un dossier non validé ne doit jamais être embarqué ni retenu par la détection de développement.
    Remove-Item -LiteralPath $OutDir -Recurse -Force -ErrorAction SilentlyContinue
    throw
  }
} finally {
  if ($builtHere -and (Test-Path -LiteralPath $builtHere)) { Remove-Item -LiteralPath $builtHere -Recurse -Force -ErrorAction SilentlyContinue }
}

$bytes = (Get-ChildItem -LiteralPath $OutDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
Write-Host ("ffmpeg prêt : {0} ({1:N1} Mo)" -f $OutDir, ($bytes / 1MB))
