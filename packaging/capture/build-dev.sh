#!/bin/bash
# Local arm64 development bundle only; no install, permission request, or helper launch.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
OUT="$ROOT/target/capture/OMMCapture.app"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources" "$ROOT/target/capture/swift-cache"
/usr/bin/xcrun swiftc -swift-version 5 -target arm64-apple-macosx13.0 \
  -module-cache-path "$ROOT/target/capture/swift-cache" \
  "$ROOT/native/yulu-capture/audio_daemon.swift" -o "$OUT/Contents/MacOS/omm-capture" \
  -framework Cocoa -framework ScreenCaptureKit -framework AVFoundation \
  -framework CoreMedia -framework CoreAudio -framework AudioToolbox
cp "$ROOT/packaging/capture/Info.plist" "$OUT/Contents/Info.plist"
cp "$ROOT/native/yulu-capture/LICENSE" "$OUT/Contents/Resources/Yulu-MIT-LICENSE"
cp "$ROOT/native/yulu-capture/NOTICE.md" "$OUT/Contents/Resources/NOTICE.md"
/usr/bin/codesign --force --sign - --entitlements "$ROOT/packaging/capture/OMMCapture.entitlements" "$OUT"
/usr/bin/codesign --verify --strict "$OUT"
printf '%s\n' "$OUT"
