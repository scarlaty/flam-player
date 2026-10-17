#!/usr/bin/env bash
# Construit un ffmpeg Windows x64 minimal (audio seulement, LGPL, statique) par
# cross-compilation MinGW-w64 sous Linux : ffmpeg.exe, ffprobe.exe, licences (#209).
#
# Usage : bash tools/ffmpeg/build.sh <dossier de sortie>      (INSTALL_DEPS=1 : apt-get install)
# Sources : tarballs épinglés dans pins.env, SHA-256 vérifiés avant toute extraction.
# Aucun --enable-gpl / --enable-nonfree ; LAME (libmp3lame, LGPL) compilé depuis ses sources.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$(mkdir -p "${1:?dossier de sortie}" && cd "$1" && pwd)"
# shellcheck source=pins.env
. "$here/pins.env"

if [ "${INSTALL_DEPS:-0}" = 1 ]; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends \
    gcc libc6-dev gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 make nasm pkg-config curl xz-utils ca-certificates >/dev/null
fi

host=x86_64-w64-mingw32
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
prefix="$work/prefix"
mkdir -p "$prefix"

fetch() { # url sha256 fichier
  curl -fsSL --retry 3 --retry-delay 5 -o "$3" "$1"
  echo "$2  $3" | sha256sum -c - >/dev/null || { echo "empreinte inattendue : $1" >&2; exit 1; }
}

fetch "$LAME_URL" "$LAME_SHA256" "$work/lame.tar.gz"
fetch "$FFMPEG_URL" "$FFMPEG_SHA256" "$work/ffmpeg.tar.xz"
tar -C "$work" -xzf "$work/lame.tar.gz"
tar -C "$work" -xJf "$work/ffmpeg.tar.xz"

# --- LAME : bibliothèque statique seule (sans frontal ni décodeur mpglib) ---------------
(
  cd "$work/lame-$LAME_VERSION"
  ./configure --host=$host --prefix="$prefix" --enable-static --disable-shared \
    --disable-frontend --disable-decoder --disable-gtktest >/dev/null
  make -j"$(nproc)" >/dev/null
  make install >/dev/null
)

# --- FFmpeg : tout désactivé, puis le strict nécessaire ---------------------------------
decoders=mp3float,mp3,aac,opus,vorbis,flac,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,pcm_u8
demuxers=mp3,mov,ogg,flac,wav,aac,matroska
parsers=mpegaudio,aac,aac_latm,opus,vorbis,flac
filters=abuffer,abuffersink,aresample,aformat,anull,anullsrc,volume,loudnorm,sine
conf=(
  --arch=x86_64 --target-os=mingw32 --cross-prefix=$host- --pkg-config=pkg-config
  --prefix="$prefix/ff"
  --disable-everything --disable-autodetect --disable-network --disable-doc
  --disable-ffplay --disable-debug --disable-shared --enable-static
  --disable-swscale --enable-ffmpeg --enable-ffprobe
  --enable-libmp3lame --enable-encoder=libmp3lame,pcm_s16le
  --enable-decoder=$decoders --enable-demuxer=$demuxers --enable-parser=$parsers
  --enable-muxer=mp3,wav,null --enable-filter=$filters
  --enable-indev=lavfi --enable-protocol=file
  --extra-cflags="-I$prefix/include" --extra-ldflags="-L$prefix/lib -static -static-libgcc"
)
cd "$work/ffmpeg-$FFMPEG_VERSION"
PKG_CONFIG_PATH="$prefix/lib/pkgconfig" ./configure "${conf[@]}"
make -j"$(nproc)" ffmpeg.exe ffprobe.exe >"$work/ffmpeg.log" 2>&1 || { tail -40 "$work/ffmpeg.log" >&2; exit 1; }

rm -f "$out"/*.exe "$out"/LICENSE*
$host-strip -o "$out/ffmpeg.exe" ffmpeg.exe
$host-strip -o "$out/ffprobe.exe" ffprobe.exe
cp COPYING.LGPLv2.1 "$out/LICENSE"
cp "$work/lame-$LAME_VERSION/COPYING" "$out/LICENSE-LAME"
# Avis propre à LAME (le fichier LICENSE de LAME précise les conditions de la LGPL).
cp "$work/lame-$LAME_VERSION/LICENSE" "$out/LICENSE-LAME-NOTICE" 2>/dev/null || true
printf '%s\n' "${conf[@]}" >"$out/configure-args.txt"
echo "ffmpeg minimal construit : $out"
