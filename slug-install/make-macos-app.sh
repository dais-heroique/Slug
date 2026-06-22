#!/usr/bin/env bash
# Build a double-clickable Slug.app from already-built release binaries.
#
#   ./slug-install/make-macos-app.sh [BIN_DIR] [OUT_DIR]
#
# BIN_DIR defaults to ./target/release, OUT_DIR to ./dist. Produces
# OUT_DIR/Slug.app — a minimal bundle that starts the slug-mcp daemon and opens
# the dashboard in the browser. Accessibility / Input Monitoring permission is
# granted once to Slug.app (System Settings → Privacy & Security).
set -euo pipefail

BIN_DIR="${1:-target/release}"
OUT_DIR="${2:-dist}"
APP="$OUT_DIR/Slug.app"
HTTP_ADDR="127.0.0.1:7333"

for b in slug-mcp slug slug-agent; do
  [ -f "$BIN_DIR/$b" ] || { echo "missing binary: $BIN_DIR/$b (build with cargo build --release)"; exit 1; }
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Bundled binaries.
cp "$BIN_DIR/slug-mcp" "$BIN_DIR/slug" "$BIN_DIR/slug-agent" "$APP/Contents/MacOS/"

# Launcher: start the daemon (pointing the agent controller at the bundled
# slug-agent) and open the dashboard.
cat > "$APP/Contents/MacOS/Slug" <<'LAUNCHER'
#!/bin/bash
# Slug.app launcher: start the background service (if not already up) and open the
# dashboard. Designed for a normal double-click — the service is detached so it
# keeps running after this launcher exits.
DIR="$(cd "$(dirname "$0")" && pwd)"
export SLUG_AGENT_BIN="$DIR/slug-agent"
export RUST_LOG="${RUST_LOG:-slug_mcp=info,slug_brain=info,slug_bridge=info}"
# Gate destructive actions behind dashboard approval by default.
export SLUG_DESTRUCTIVE="${SLUG_DESTRUCTIVE:-ask}"
# Honour a user config if one exists (e.g. from a prior install or hand-edit).
[ -f "$HOME/.slug/slug.toml" ] && export SLUG_CONFIG="$HOME/.slug/slug.toml"
LOG_DIR="$HOME/Library/Logs/slug"; mkdir -p "$LOG_DIR"
# If a service is already up, just open the dashboard.
if ! curl -s "http://127.0.0.1:7333/healthz" >/dev/null 2>&1; then
  nohup "$DIR/slug-mcp" --http 127.0.0.1:7333 \
    >>"$LOG_DIR/slug-mcp.out.log" 2>>"$LOG_DIR/slug-mcp.err.log" &
  disown 2>/dev/null || true
  # Wait briefly so the dashboard isn't opened before the server is ready.
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    curl -s "http://127.0.0.1:7333/healthz" >/dev/null 2>&1 && break
    sleep 0.3
  done
fi
# Open the dashboard as its OWN app window (Chrome/Edge/Brave --app), so it looks
# like a native app rather than a browser tab on localhost. Fall back to the
# default browser if none is installed.
URL="http://127.0.0.1:7333/dashboard"
open_app_window() {
  for b in "Google Chrome" "Microsoft Edge" "Brave Browser" "Chromium"; do
    APP="/Applications/$b.app/Contents/MacOS/${b}"
    if [ -x "$APP" ]; then
      "$APP" --app="$URL" --window-size=1280,860 >/dev/null 2>&1 &
      return 0
    fi
  done
  return 1
}
open_app_window || open "$URL"
LAUNCHER
chmod +x "$APP/Contents/MacOS/Slug" "$APP/Contents/MacOS/slug-mcp" \
  "$APP/Contents/MacOS/slug" "$APP/Contents/MacOS/slug-agent"

# Info.plist
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Slug</string>
  <key>CFBundleDisplayName</key><string>Slug</string>
  <key>CFBundleIdentifier</key><string>org.slug.app</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>Slug</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>NSAccessibilityUsageDescription</key>
  <string>Slug reads the accessibility tree and drives apps on your behalf.</string>
</dict>
</plist>
PLIST

echo "built $APP"
