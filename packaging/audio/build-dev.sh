#!/bin/bash
# Explicit local build only. No system install or user service changes.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
OUT="$ROOT/target/audio-toolchain"
mkdir -p "$OUT/sources" "$OUT/source" "$OUT/licenses"
fetch() {
  local file="$OUT/sources/$1"
  test -f "$file" || curl --fail --location --silent --show-error "$2" -o "$file"
  test "$(shasum -a 256 "$file" | awk '{print $1}')" = "$3" || { echo 'source checksum mismatch' >&2; exit 1; }
  tar -xf "$file" -C "$OUT/source"
}
fetch ffmpeg-8.1.2.tar.xz https://ffmpeg.org/releases/ffmpeg-8.1.2.tar.xz 464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c
fetch opus-1.5.2.tar.gz https://downloads.xiph.org/releases/opus/opus-1.5.2.tar.gz 65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1
PREFIX="$OUT/opus-static"
(
 cd "$OUT/source/opus-1.5.2"
 CFLAGS='-arch arm64 -mmacosx-version-min=13.0' LDFLAGS='-arch arm64 -mmacosx-version-min=13.0' ./configure --prefix="$PREFIX" --disable-shared --enable-static --disable-extra-programs --disable-doc
 make -j 4; make install
 cp COPYING "$OUT/licenses/opus.txt"
)
(
 cd "$OUT/source/ffmpeg-8.1.2"
 PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" ./configure --arch=arm64 --target-os=darwin --cc=clang --disable-autodetect --disable-debug --disable-doc --disable-ffplay --disable-network --disable-gpl --disable-version3 --disable-everything --enable-ffmpeg --enable-ffprobe --enable-protocol=file --enable-demuxer=wav,ogg --enable-muxer=ogg,wav --enable-decoder=pcm_s16le,opus --enable-encoder=libopus,pcm_s16le --enable-filter=pan,aresample,aformat,anull --enable-libopus --enable-static --disable-shared --pkg-config-flags=--static --extra-cflags="-I$PREFIX/include -mmacosx-version-min=13.0" --extra-ldflags="-L$PREFIX/lib -mmacosx-version-min=13.0"
 make -j 4 ffmpeg ffprobe
 cp ffmpeg ffprobe "$OUT/"
 cp COPYING.LGPLv2.1 "$OUT/licenses/ffmpeg.txt"
)
for name in ffmpeg ffprobe; do
 codesign --force --sign - "$OUT/$name"
 codesign --verify --strict "$OUT/$name"
done
"$OUT/ffmpeg" -hide_banner -encoders 2>/dev/null | grep libopus
cp "$ROOT/packaging/audio/sources.json" "$OUT/sources.json"
# Full verified source and build script accompany redistribution for LGPL rebuilding.
cp "$0" "$OUT/build-dev.sh"
printf '{"schemaVersion":1,"ffmpegVersion":"8.1.2","opusVersion":"1.5.2","license":"LGPL-2.1-or-later","gplEnabled":false,"networkEnabled":false,"sourceManifestSha256":"%s","ffmpegSha256":"%s","ffprobeSha256":"%s","signing":"ad-hoc-development"}\n' "$(shasum -a 256 "$OUT/sources.json" | awk '{print $1}')" "$(shasum -a 256 "$OUT/ffmpeg" | awk '{print $1}')" "$(shasum -a 256 "$OUT/ffprobe" | awk '{print $1}')" > "$OUT/manifest.json"
