#!/usr/bin/env bash
# Wrap a built Slug.app into a drag-to-install .dmg.
#
#   ./slug-install/make-macos-dmg.sh [APP_PATH] [DMG_PATH]
#
# APP_PATH defaults to dist/Slug.app, DMG_PATH to dist/Slug.dmg. The disk image
# contains Slug.app plus an /Applications symlink so the user drags one onto the
# other — the familiar macOS install gesture. Requires macOS (hdiutil).
#
# Optional code-signing / notarization (skipped automatically if the secrets are
# absent, so unsigned builds still work):
#   SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"  → codesign the app
#   AC_PROFILE="notary-profile"                                   → notarytool + staple
set -euo pipefail

APP="${1:-dist/Slug.app}"
DMG="${2:-dist/Slug.dmg}"
VOLNAME="Slug"

[ -d "$APP" ] || { echo "missing app bundle: $APP (run make-macos-app.sh first)"; exit 1; }
command -v hdiutil >/dev/null || { echo "hdiutil not found — this script requires macOS"; exit 1; }

# Optional: sign the bundle if a Developer ID identity is provided.
if [ -n "${SIGN_IDENTITY:-}" ]; then
  echo "codesigning $APP with: $SIGN_IDENTITY"
  codesign --force --deep --options runtime --sign "$SIGN_IDENTITY" "$APP"
fi

STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

rm -f "$DMG"
mkdir -p "$(dirname "$DMG")"
hdiutil create -volname "$VOLNAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

# Optional: notarize + staple if an App Store Connect profile is configured.
if [ -n "${AC_PROFILE:-}" ]; then
  echo "notarizing $DMG via profile: $AC_PROFILE"
  xcrun notarytool submit "$DMG" --keychain-profile "$AC_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

echo "built $DMG"
