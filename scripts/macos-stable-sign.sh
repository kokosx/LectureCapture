#!/usr/bin/env bash
# Re-sign the macOS bundle ad hoc with a designated requirement based on the bundle
# identifier instead of the binary hash, and rebuild the DMG.
#
# Without this, the ad hoc signature's designated requirement is `cdhash H"…"`, which
# changes with every build: macOS then treats each update as a different app, the old
# "Screen & System Audio Recording" / "Microphone" toggles no longer apply and the app
# keeps asking for permissions. With `identifier "app.lecturecapture"` a permission
# granted once survives updates.
#
# usage: scripts/macos-stable-sign.sh <path/to/LectureCapture.app> [out.dmg]
set -euo pipefail

APP="$1"
DMG="${2:-}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ID="app.lecturecapture"

codesign --force --deep --sign - --options runtime \
  --entitlements "$ROOT/src-tauri/Entitlements.plist" \
  -r="designated => identifier \"$ID\"" \
  "$APP"
codesign --verify --strict --verbose=2 "$APP"
codesign -d -r- "$APP"

if [[ -n "$DMG" ]]; then
  STAGE="$(mktemp -d)"
  trap 'rm -rf "$STAGE"' EXIT
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  rm -f "$DMG"
  hdiutil create -volname "LectureCapture" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
fi
