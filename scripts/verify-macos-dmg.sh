#!/usr/bin/env bash
set -euo pipefail

if [[ $(uname -s) != Darwin || $# != 1 || ! -f "$1" ]]; then
  echo 'Usage on macOS: bash scripts/verify-macos-dmg.sh path/to/Keel.dmg' >&2
  exit 2
fi

image="$1"
if ! xcrun stapler validate "$image"; then
  echo 'Disk image has no valid notarization ticket.' >&2
  exit 1
fi
spctl --assess --type open --context context:primary-signature --verbose "$image"

mount_point=$(mktemp -d "${TMPDIR:-/tmp}/keel-dmg-check.XXXXXXXX")
mounted=false
cleanup() {
  if $mounted; then hdiutil detach "$mount_point" -quiet; fi
  rmdir "$mount_point"
}
trap cleanup EXIT
hdiutil attach "$image" -nobrowse -readonly -mountpoint "$mount_point" -quiet
mounted=true
app="$mount_point/Keel Desktop.app"
codesign --verify --deep --strict --verbose=2 "$app"
codesign --display --verbose=4 "$app" 2>&1 | grep '^Authority=Developer ID Application:'
xcrun stapler validate "$app"
spctl --assess --type execute --verbose "$app"
echo 'PASS: notarized disk image and Developer ID app accepted by Gatekeeper.'
