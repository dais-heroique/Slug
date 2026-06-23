# slug-install

A lightweight installer for the Slug daemons. It installs **Rust binaries only**
— a few MB total. It never downloads, pulls, or bundles an Ollama model; if you
already run Ollama it simply detects your local models and lets you pick one.

## macOS (verified)

```sh
./slug-install/install.sh
```

What it does:

1. Builds `slug-mcp`, `slug` and `slug-agent` (`cargo build --release`) and
   installs them to `~/.slug/bin`.
2. Writes a starter `~/.slug/slug.toml`:
   - If **Ollama** is detected (`ollama list` succeeds), it lists your local
     models and sets `provider = "ollama"` with the first one — edit to taste.
   - Otherwise it defaults to `provider = "claude"` with `api_key_env =
     "ANTHROPIC_API_KEY"` and instructions to export your key. API keys live in
     environment variables named by the config — **never** in the file.
3. Registers a **launchd** user agent `~/Library/LaunchAgents/org.slug.daemon.plist`
   that runs `slug-mcp --http 127.0.0.1:7333` at login (background, `KeepAlive`),
   logging to `~/Library/Logs/slug/`. The daemon hosts the MCP **dashboard**
   (`http://127.0.0.1:7333/dashboard`) and supervises `slug-brain` on demand
   (`SLUG_AGENT_BIN` points the controller at the installed `slug-agent`).

Then grant Accessibility permission to the `slug-mcp` binary in **System Settings
→ Privacy & Security → Accessibility** and restart the agent:

```sh
launchctl kickstart -k gui/$(id -u)/org.slug.daemon
```

Uninstall:

```sh
./slug-install/install.sh uninstall
```

### Footprint

Only the three Rust binaries are installed (`du -sh ~/.slug/bin` ≈ a few MB).
Any Ollama model you choose is owned and stored by Ollama, not by Slug.

## Windows (manual setup, for now)

Windows is compile-verified in CI but not yet auto-installed. Manual path:

```powershell
cargo build --release -p slug-mcp -p slug-cli -p slug-brain
# Run the daemon (no special accessibility permission needed on Windows):
.\target\release\slug-mcp.exe --http 127.0.0.1:7333
# Optional: register as a logon task with Task Scheduler, setting env vars
#   SLUG_AGENT_BIN = ...\target\release\slug-agent.exe
#   SLUG_CONFIG    = %USERPROFILE%\.slug\slug.toml
```

Dashboard: `http://127.0.0.1:7333/dashboard`.
