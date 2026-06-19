#!/usr/bin/env bash
# Lightweight Slug installer (macOS-verified).
#
# Builds the Rust binaries (a few MB total — no models are ever downloaded),
# installs them under ~/.slug, writes a starter slug.toml, and registers a
# launchd user agent that runs the slug-mcp daemon (which hosts the MCP dashboard
# and supervises slug-brain on demand) at login.
#
# Usage:
#   ./install.sh            # build + install + load the launchd agent
#   ./install.sh uninstall  # unload the agent and remove ~/.slug
#
# Windows: not yet automated — see README.md (manual setup path).
set -euo pipefail

PREFIX="${SLUG_PREFIX:-$HOME/.slug}"
BIN_DIR="$PREFIX/bin"
CONFIG="$PREFIX/slug.toml"
LABEL="org.slug.daemon"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG_DIR="$HOME/Library/Logs/slug"
HTTP_ADDR="${SLUG_HTTP_ADDR:-127.0.0.1:7333}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

say()  { printf '\033[1;34m[slug]\033[0m %s\n' "$1"; }
warn() { printf '\033[1;33m[slug]\033[0m %s\n' "$1"; }

uninstall() {
  if [ -f "$PLIST" ]; then
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || launchctl unload "$PLIST" 2>/dev/null || true
    rm -f "$PLIST"
    say "unloaded and removed $PLIST"
  fi
  rm -rf "$PREFIX"
  say "removed $PREFIX (logs in $LOG_DIR were kept)"
  exit 0
}

[ "${1:-}" = "uninstall" ] && uninstall

# --- Platform gate -----------------------------------------------------------
OS="$(uname -s)"
if [ "$OS" != "Darwin" ]; then
  warn "Automated install is macOS-only for now (detected: $OS)."
  warn "Build with 'cargo build --release' and run 'slug-mcp --http $HTTP_ADDR'."
  warn "See slug-install/README.md for the manual Windows/Linux path."
  exit 0
fi

# --- Build (binaries only; no models) ----------------------------------------
command -v cargo >/dev/null 2>&1 || { warn "cargo not found — install Rust from https://rustup.rs"; exit 1; }
say "building release binaries (this is the entire footprint — a few MB)…"
( cd "$REPO_ROOT" && cargo build --release -p slug-mcp -p slug-cli -p slug-brain )

mkdir -p "$BIN_DIR" "$LOG_DIR"
for b in slug-mcp slug slug-agent; do
  install -m 0755 "$REPO_ROOT/target/release/$b" "$BIN_DIR/$b"
done
say "installed slug-mcp, slug, slug-agent → $BIN_DIR"
du -sh "$BIN_DIR" 2>/dev/null | awk '{print "[slug] installed size: "$1}'

# --- Provider: detect Ollama, else default to Claude -------------------------
write_config() {
  if [ -f "$CONFIG" ]; then
    cp "$CONFIG" "$CONFIG.bak.$(date +%s)"
    warn "existing slug.toml backed up; not overwriting your settings"
    return
  fi
  if command -v ollama >/dev/null 2>&1 && ollama list >/dev/null 2>&1; then
    say "Ollama detected — local models available:"
    ollama list | sed 's/^/[slug]   /'
    local first
    first="$(ollama list | awk 'NR==2 {print $1}')"
    cat > "$CONFIG" <<EOF
# Slug configuration. API keys are read from the named env vars, never stored here.
[brain]
provider = "ollama"   # Ollama detected; pick a model below from 'ollama list'

[providers.ollama]
base_url = "http://127.0.0.1:11434"
model = "${first:-qwen3:8b}"   # ← edit to any model shown by 'ollama list'
EOF
    say "wrote $CONFIG (provider = ollama, model = ${first:-qwen3:8b})"
  else
    warn "Ollama not detected — defaulting to the Claude API."
    cat > "$CONFIG" <<'EOF'
# Slug configuration. API keys are read from the named env vars, never stored here.
[brain]
provider = "claude"

[providers.claude]
api_key_env = "ANTHROPIC_API_KEY"   # ← export this in your shell / launchd env
model = "claude-sonnet-4-6"

# To use another provider instead, set provider above to one of:
#   openai | openrouter | gemini | ollama   and fill its block, e.g.:
# [providers.openai]
# api_key_env = "OPENAI_API_KEY"
# base_url = "https://api.openai.com/v1"
# model = "gpt-4o"
EOF
    say "wrote $CONFIG (provider = claude)"
    warn "Set your key:  export ANTHROPIC_API_KEY=sk-...  (and add it to the launchd env if you want the daemon to use it)."
  fi
}
write_config

# --- launchd user agent ------------------------------------------------------
mkdir -p "$(dirname "$PLIST")"
cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN_DIR/slug-mcp</string>
    <string>--http</string>
    <string>$HTTP_ADDR</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>SLUG_AGENT_BIN</key><string>$BIN_DIR/slug-agent</string>
    <key>SLUG_CONFIG</key><string>$CONFIG</string>
    <key>RUST_LOG</key><string>slug_mcp=info,slug_brain=info,slug_bridge=info</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$LOG_DIR/slug-mcp.out.log</string>
  <key>StandardErrorPath</key><string>$LOG_DIR/slug-mcp.err.log</string>
</dict>
</plist>
EOF
say "wrote launchd agent → $PLIST"

# (Re)load it.
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
if launchctl bootstrap "gui/$(id -u)" "$PLIST" 2>/dev/null; then :; else launchctl load "$PLIST"; fi
say "loaded $LABEL (starts at login; logs in $LOG_DIR)"

cat <<EOF

[slug] Done.
  • Dashboard:   http://$HTTP_ADDR/dashboard
  • MCP (HTTP):  POST http://$HTTP_ADDR/mcp
  • Config:      $CONFIG
  • Logs:        $LOG_DIR
  • slug-brain runs on demand, supervised by slug-mcp (start a task from the dashboard).

macOS reminder: grant Accessibility permission to the slug-mcp binary in
System Settings → Privacy & Security → Accessibility, then:
  launchctl kickstart -k gui/$(id -u)/$LABEL
EOF
