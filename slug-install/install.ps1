<#
.SYNOPSIS
  Lightweight Slug installer for Windows.

.DESCRIPTION
  Installs the Slug binaries under %USERPROFILE%\.slug, writes a starter
  slug.toml, and registers a logon Scheduled Task that runs the slug-mcp daemon
  (dashboard + agent controller) at sign-in. Run from a release download (uses
  the bundled .exe files) or from a source checkout (builds with cargo).

  No special OS permission is required on Windows (unlike macOS).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\slug-install\install.ps1
  powershell -ExecutionPolicy Bypass -File .\slug-install\install.ps1 -Uninstall
#>
param(
  [switch]$Uninstall,
  [string]$HttpAddr = "127.0.0.1:7333"
)

$ErrorActionPreference = "Stop"
$Prefix  = Join-Path $env:USERPROFILE ".slug"
$BinDir  = Join-Path $Prefix "bin"
$Config  = Join-Path $Prefix "slug.toml"
$LogDir  = Join-Path $Prefix "logs"
$TaskName = "SlugDaemon"

function Say($m) { Write-Host "[slug] $m" -ForegroundColor Cyan }

if ($Uninstall) {
  Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  if (Test-Path $Prefix) { Remove-Item -Recurse -Force $Prefix }
  Say "uninstalled (removed task + $Prefix)"
  return
}

New-Item -ItemType Directory -Force -Path $BinDir, $LogDir | Out-Null

# --- Locate or build the binaries -------------------------------------------
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$bins = @("slug-mcp.exe", "slug.exe", "slug-agent.exe")
$srcDir = $null
if (Test-Path (Join-Path $here "slug-mcp.exe")) {
  $srcDir = $here                                    # release download layout
} elseif (Test-Path (Join-Path $here "..\target\release\slug-mcp.exe")) {
  $srcDir = (Resolve-Path (Join-Path $here "..\target\release")).Path
} else {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "No prebuilt binaries found and cargo is not installed. Install Rust from https://rustup.rs or use a release download."
  }
  Say "building release binaries with cargo..."
  Push-Location (Join-Path $here "..")
  cargo build --release -p slug-mcp -p slug-cli -p slug-brain
  Pop-Location
  $srcDir = (Resolve-Path (Join-Path $here "..\target\release")).Path
}

foreach ($b in $bins) { Copy-Item (Join-Path $srcDir $b) (Join-Path $BinDir $b) -Force }
Say "installed slug-mcp, slug, slug-agent -> $BinDir"

# --- Starter config (Ollama if present, else Claude) ------------------------
if (-not (Test-Path $Config)) {
  $ollama = Get-Command ollama -ErrorAction SilentlyContinue
  if ($ollama) {
    @"
# Slug configuration. API keys are read from the named env vars, never stored here.
[brain]
provider = "ollama"

[providers.ollama]
base_url = "http://127.0.0.1:11434"
model = "qwen3:8b"   # edit to any model from 'ollama list'
"@ | Set-Content -Encoding UTF8 $Config
    Say "wrote $Config (provider = ollama)"
  } else {
    @"
# Slug configuration. API keys are read from the named env vars, never stored here.
[brain]
provider = "claude"

[providers.claude]
api_key_env = "ANTHROPIC_API_KEY"   # set it: setx ANTHROPIC_API_KEY "sk-..."
model = "claude-sonnet-4-6"
"@ | Set-Content -Encoding UTF8 $Config
    Say "wrote $Config (provider = claude) - set ANTHROPIC_API_KEY"
  }
} else {
  Say "existing slug.toml kept"
}

# --- Logon Scheduled Task running the daemon --------------------------------
$mcp = Join-Path $BinDir "slug-mcp.exe"
$action = New-ScheduledTaskAction -Execute $mcp -Argument "--http $HttpAddr"
$trigger = New-ScheduledTaskTrigger -AtLogOn
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable
# Persist env vars for the user so the logon task (and this session) see them.
# The daemon also finds slug-agent next to itself, so this is belt-and-braces.
setx SLUG_AGENT_BIN (Join-Path $BinDir "slug-agent.exe") | Out-Null
setx SLUG_CONFIG $Config | Out-Null
# Destructive actions from external clients: ask (approve in dashboard) | deny | allow
if (-not $env:SLUG_DESTRUCTIVE) { setx SLUG_DESTRUCTIVE "ask" | Out-Null }
$env:SLUG_AGENT_BIN = Join-Path $BinDir "slug-agent.exe"
$env:SLUG_CONFIG = $Config
if (-not $env:SLUG_DESTRUCTIVE) { $env:SLUG_DESTRUCTIVE = "ask" }
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Description "Slug semantic-bus daemon" | Out-Null
Say "registered logon task '$TaskName'"

# Start it now too.
Start-Process -FilePath $mcp -ArgumentList "--http $HttpAddr" -WindowStyle Hidden
Say "Done. Dashboard: http://$HttpAddr/dashboard"
Say "Config: $Config   Logs: $LogDir"
