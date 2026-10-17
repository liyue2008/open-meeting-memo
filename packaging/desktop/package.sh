#!/bin/bash
# Explicit locked arm64 package; never launches hardware, models, notifications or services.
set -euo pipefail
umask 077
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
cd "$ROOT"
[[ $(uname -m) == arm64 && $(uname -s) == Darwin ]] || { echo 'macOS Apple Silicon required' >&2; exit 3; }
[[ -z $(git status --porcelain) ]] || { echo 'package requires committed clean source' >&2; exit 3; }
REV=$(git rev-parse HEAD)
OUT="$ROOT/target/desktop-package/$REV"
DEPS="$ROOT/target/desktop-dependencies"
mkdir -p "$OUT" "$DEPS"
# This project's pinned audio build redistributes source archives and LGPL rebuilding instructions.
[[ -f target/audio-toolchain/manifest.json ]] || bash packaging/audio/build-dev.sh
bash packaging/capture/build-dev.sh
python3 packaging/desktop/resources.py dependencies "$DEPS"
npm ci --prefix apps/desktop
npm run typecheck --prefix apps/desktop
npm run build --prefix apps/desktop
OMM_SOURCE_REVISION="$REV" MACOSX_DEPLOYMENT_TARGET=15.5 cargo build --locked --release --target aarch64-apple-darwin -p open-meeting-memo --bins
OMM_SOURCE_REVISION="$REV" MACOSX_DEPLOYMENT_TARGET=15.5 cargo build --locked --release --target aarch64-apple-darwin -p omm-desktop --features custom-protocol
APP="$OUT/Open Meeting Memo.app"
[[ ! -e "$APP" ]] || { echo 'immutable package already exists; choose a clean output after inspecting it' >&2; exit 3; }
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin" "$APP/Contents/Resources/sherpa" "$APP/Contents/Resources/licenses"
cp target/aarch64-apple-darwin/release/omm-desktop "$APP/Contents/MacOS/omm-desktop"
cp target/aarch64-apple-darwin/release/{omm,omm-asr-worker} "$APP/Contents/Resources/bin/"
ditto target/capture/OMMCapture.app "$APP/Contents/Resources/OMMCapture.app"
python3 packaging/desktop/resources.py stage "$APP/Contents/Resources" "$DEPS"
xcrun swift packaging/desktop/build-icon.swift "$OUT/OMM.iconset"
iconutil -c icns "$OUT/OMM.iconset" -o "$APP/Contents/Resources/OMM.icns"
python3 - "$APP" <<'PY'
import plistlib,sys,pathlib
app=pathlib.Path(sys.argv[1])
version=__import__('json').load(open('apps/desktop/src-tauri/tauri.conf.json'))['version']
with open(app/'Contents/Info.plist','wb') as f:plistlib.dump({'CFBundleIdentifier':'org.openmeetingmemo.desktop','CFBundleName':'Open Meeting Memo','CFBundleDisplayName':'Open Meeting Memo','CFBundleExecutable':'omm-desktop','CFBundleIconFile':'OMM.icns','CFBundlePackageType':'APPL','CFBundleShortVersionString':version,'CFBundleVersion':version,'LSMinimumSystemVersion':'15.5','NSHighResolutionCapable':True},f)
PY
# Sign inside-out; preserve the capture helper's own bundle ID and entitlements from its build.
for executable in "$APP/Contents/Resources/bin/omm" "$APP/Contents/Resources/bin/omm-asr-worker" "$APP/Contents/MacOS/omm-desktop" "$APP/Contents/Resources/sherpa/"*.dylib; do
  file "$executable" | grep -q 'Mach-O 64-bit.*arm64'
  codesign --force --sign - "$executable"
  codesign --verify --strict "$executable"
done
python3 packaging/desktop/resources.py manifest "$APP/Contents/Resources" "$REV"
codesign --force --sign - "$APP"
codesign --verify --deep --strict "$APP"
python3 packaging/desktop/resources.py verify "$APP"
DMG="$OUT/Open-Meeting-Memo-$REV.dmg"
hdiutil create -quiet -format UDZO -volname 'Open Meeting Memo' -srcfolder "$APP" "$DMG"
shasum -a 256 "$DMG" >"$OUT/SHA256SUMS"
printf '%s\n' "$APP"
