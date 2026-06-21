# Installing Slug

Slug ships as a small set of native binaries (a few MB — **no AI model is ever
bundled or downloaded**). There are two ways to install: **download a release**
(easiest, no toolchain) or **build from source**.

After installing, Slug runs a local daemon that serves a **dashboard** at
<http://127.0.0.1:7333/dashboard> and speaks MCP at `POST /127.0.0.1:7333/mcp`.

---

## A. Download a release (recommended)

Go to the repository's **Releases** page and download the file for your system:

| System | File | What it is |
|--------|------|------------|
| **macOS (Apple Silicon)** | `Slug-<ver>-macos-arm64.dmg` | Drag-to-install **disk image** (recommended) |
| **macOS (Intel)** | `Slug-<ver>-macos-x86_64.dmg` | Drag-to-install **disk image** |
| **macOS (zip)** | `Slug-<ver>-macos-*.app.zip` | Same **Slug.app**, zipped |
| **macOS (binaries)** | `slug-<ver>-macos-*.tar.gz` | Binaries + `install.sh` |
| **Windows (installer)** | `SlugSetup-<ver>-windows-x86_64.exe` | One-click **installer** (recommended) |
| **Windows (zip)** | `slug-<ver>-windows-x86_64.zip` | Binaries + `install.ps1` |
| **Linux** | `slug-<ver>-linux-x86_64.tar.gz` | Binaries + `install.sh` |

### macOS (.dmg / Slug.app)
1. Open the **`.dmg`** and drag **Slug.app** onto **Applications** (or unzip the
   `.app.zip` and move Slug.app into `/Applications`).
2. Unless the build was signed/notarized, Gatekeeper quarantines it. Clear the
   flag once (Terminal):
   ```sh
   xattr -dr com.apple.quarantine /Applications/Slug.app
   ```
   (Or right-click **Slug.app → Open → Open** the first time.)
3. **Double-click Slug** — it starts the daemon and opens the dashboard.
4. Grant permissions (one-time), then relaunch Slug:
   - **System Settings → Privacy & Security → Accessibility** → add **Slug**, toggle **ON**
   - **System Settings → Privacy & Security → Input Monitoring** → add **Slug**, toggle **ON**

   > Accessibility lets Slug *read* the UI and click controls; Input Monitoring is
   > needed to *type/click synthetically*. Slug never captures the screen, so
   > Screen Recording is **not** required.

### Windows
**Installer (recommended):** double-click **`SlugSetup-<ver>-windows-x86_64.exe`**
and follow the wizard. It installs per-user (no admin), sets up `%USERPROFILE%\.slug`,
registers the logon daemon, and offers to open the dashboard. Uninstall from
*Apps & features* like any program.

**Or from the zip:**
1. Unzip the folder anywhere.
2. Right-click **`install.ps1` → Run with PowerShell** (or:
   `powershell -ExecutionPolicy Bypass -File .\install.ps1`).
   It installs to `%USERPROFILE%\.slug`, starts the daemon, and runs it at logon.
   No special permission is required on Windows.
3. Open <http://127.0.0.1:7333/dashboard>.

### Linux
```sh
tar -xzf slug-*-linux-x86_64.tar.gz && cd slug-*-linux-x86_64
SLUG_AGENT_BIN="$PWD/slug-agent" ./slug-mcp --http 127.0.0.1:7333
```

> macOS binary tarball: clear quarantine first —
> `xattr -dr com.apple.quarantine slug-mcp slug slug-agent`.
Enable toolkit accessibility so apps expose their trees:
```sh
gsettings set org.gnome.desktop.interface toolkit-accessibility true
```
Then open <http://127.0.0.1:7333/dashboard>.

---

## B. Build from source (any OS)

Requires **Rust 1.77.2+** (<https://rustup.rs>).

```sh
git clone <repo-url> && cd Slug
cargo build --workspace --release
# binaries: target/release/{slug-mcp, slug, slug-agent}  (.exe on Windows)
```

Then install/run per OS:

- **macOS:** `./slug-install/install.sh` (builds + installs to `~/.slug`, registers a
  launchd agent, starts the daemon). Build a double-click app with
  `./slug-install/make-macos-app.sh` → `dist/Slug.app`.
- **Windows:** `powershell -ExecutionPolicy Bypass -File .\slug-install\install.ps1`.
- **Linux:** run the daemon directly (see above); a systemd `--user` unit is the
  natural equivalent of the macOS launchd agent.

---

## Connect an AI client (optional)

The dashboard already lets you start tasks. To also drive Slug from **Claude Code**
over MCP (stdio):

```sh
# macOS / Linux
claude mcp add slug -- ~/.slug/bin/slug-mcp --stdio
# Windows (PowerShell)
claude mcp add slug -- $env:USERPROFILE\.slug\bin\slug-mcp.exe --stdio
```

(On macOS, grant the terminal/Claude Code Accessibility permission for the stdio
path, exactly as for Slug.app.)

---

## Choosing the AI provider

Edit `~/.slug/slug.toml` (Windows: `%USERPROFILE%\.slug\slug.toml`):

```toml
[brain]
provider = "claude"   # auto | claude | openai | openrouter | gemini | ollama

[providers.claude]
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-6"
```

API keys are read from the **environment variable named in the config** — they are
never stored in the file. Set it in your shell (macOS/Linux `export …`, Windows
`setx ANTHROPIC_API_KEY "sk-..."`). With `provider = "ollama"`, Slug uses your
local Ollama models (nothing is downloaded by Slug).

---

## Updating

- **Release download:** grab the newer file and re-run the installer (it backs up
  your `slug.toml`).
- **From source:** `git pull` then `cargo build --release` and re-run the installer.

## Uninstalling

- **macOS:** `./slug-install/install.sh uninstall` (or delete `Slug.app` + `~/.slug`).
- **Windows:** uninstall **Slug** from *Apps & features* (installer build), or
  `powershell -File .\slug-install\install.ps1 -Uninstall` (zip build).
- **Linux:** stop the daemon and remove the extracted folder / `~/.slug`.

---

## Signed / notarized macOS builds (maintainers)

The release `.dmg` is unsigned by default (users clear quarantine once, above).
To ship a signed + notarized image, add these **GitHub repository secrets** and
re-run the release — the workflow picks them up automatically:

- `MACOS_SIGN_IDENTITY` — e.g. `Developer ID Application: Your Name (TEAMID)`
  (the certificate must be importable into the runner keychain; add the import
  step with your `.p12` before packaging).
- `MACOS_NOTARY_PROFILE` — a `notarytool` keychain profile name for
  notarization + stapling.

When both are absent, the build stays unsigned and everything still works.
