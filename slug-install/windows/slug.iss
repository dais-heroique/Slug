; Inno Setup script for the Slug Windows installer (.exe).
;
; Produces SlugSetup.exe. Installs per-user (no admin needed), stages the
; binaries, then runs install.ps1 which copies them to %USERPROFILE%\.slug,
; writes a starter slug.toml, sets env vars (incl. SLUG_DESTRUCTIVE=ask) and
; registers the logon Scheduled Task that serves the dashboard.
;
; Build:  iscc /DMyAppVersion=0.1.0 slug-install\windows\slug.iss
; Expects the three release .exe files in slug-install\windows\payload\ (the CI
; copies them there before compiling).

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif

[Setup]
AppId={{4F6B2C9A-5C2E-4E1B-9C3A-5B1B7A0C7E10}
AppName=Slug
AppVersion={#MyAppVersion}
AppPublisher=Slug
DefaultDirName={localappdata}\Programs\Slug
DefaultGroupName=Slug
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=SlugSetup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Files]
Source: "payload\slug-mcp.exe";   DestDir: "{app}"; Flags: ignoreversion
Source: "payload\slug.exe";       DestDir: "{app}"; Flags: ignoreversion
Source: "payload\slug-agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\install.ps1";         DestDir: "{app}"; Flags: ignoreversion
Source: "open-dashboard.cmd";     DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\INSTALL.md";       DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\..\README.md";        DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Slug Dashboard"; Filename: "{app}\open-dashboard.cmd"
Name: "{group}\Uninstall Slug"; Filename: "{uninstallexe}"

[Run]
; Real setup: ~/.slug, config, env vars, scheduled task, and starts the daemon.
Filename: "powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\install.ps1"""; \
  StatusMsg: "Configuring Slug (daemon + dashboard)..."; \
  Flags: runhidden waituntilterminated
; Offer to open the dashboard at the end.
Filename: "{app}\open-dashboard.cmd"; Description: "Open the Slug dashboard"; \
  Flags: postinstall nowait skipifsilent

[UninstallRun]
Filename: "powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\install.ps1"" -Uninstall"; \
  Flags: runhidden waituntilterminated; RunOnceId: "SlugUninstall"
