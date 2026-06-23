//! Optional visual click feedback: briefly flash a small red dot on screen where
//! Slug just clicked.
//!
//! Off by default; enable with `SLUG_CLICK_FLASH=1`. The dot is drawn by a
//! **separate, detached subprocess** (osascript on macOS, PowerShell on Windows)
//! so it can never block the click or crash the daemon — if it fails, nothing
//! happens. The click itself has already been delivered before this is called.

use std::process::{Command, Stdio};

const DOT_PX: i64 = 26;
const SECONDS: f64 = 2.0;

/// Whether the click flash is enabled (`SLUG_CLICK_FLASH` truthy).
fn enabled() -> bool {
    std::env::var("SLUG_CLICK_FLASH")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false)
}

/// Flash a red dot at absolute screen `(x, y)` for ~2s. No-op unless enabled and
/// on a supported OS. Best-effort and fully detached.
pub fn flash(x: f64, y: f64) {
    if !enabled() {
        return;
    }
    let (xi, yi) = (x.round() as i64, y.round() as i64);
    let _ = spawn(xi, yi);
}

#[cfg(target_os = "macos")]
fn spawn(x: i64, y: i64) -> std::io::Result<()> {
    detach(Command::new("osascript").args(["-l", "JavaScript", "-e", &macos_script(x, y)]))
}

#[cfg(target_os = "windows")]
fn spawn(x: i64, y: i64) -> std::io::Result<()> {
    detach(Command::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &windows_script(x, y),
    ]))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn spawn(_x: i64, _y: i64) -> std::io::Result<()> {
    Ok(())
}

#[allow(dead_code)]
fn detach(cmd: &mut Command) -> std::io::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// macOS overlay via JXA: a borderless, click-through, top-level red circle that
/// orders front and lives for `SECONDS` on its own run loop, then exits.
#[allow(dead_code)]
fn macos_script(x: i64, y: i64) -> String {
    format!(
        "ObjC.import('Cocoa');\
         var x={x},y={y},s={s};\
         var H=$.NSScreen.mainScreen.frame.size.height;\
         var w=$.NSWindow.alloc.initWithContentRectStyleMaskBackingDefer($.NSMakeRect(x-s/2,H-y-s/2,s,s),0,2,false);\
         w.setOpaque(false);w.setBackgroundColor($.NSColor.clearColor);w.setLevel(1000);\
         w.setIgnoresMouseEvents(true);w.setHasShadow(false);\
         var cv=w.contentView;cv.setWantsLayer(true);\
         cv.layer.setBackgroundColor($.NSColor.colorWithCalibratedRedGreenBlueAlpha(1,0,0,0.9).CGColor);\
         cv.layer.setCornerRadius(s/2);\
         w.orderFrontRegardless;\
         $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow({secs}));",
        x = x,
        y = y,
        s = DOT_PX,
        secs = SECONDS,
    )
}

/// Windows overlay via WinForms: a topmost, transparent-keyed borderless form
/// painting a red ellipse at `(x, y)`, shown for `SECONDS` then closed.
#[allow(dead_code)]
fn windows_script(x: i64, y: i64) -> String {
    let ms = (SECONDS * 1000.0) as i64;
    format!(
        "Add-Type -AssemblyName System.Windows.Forms;Add-Type -AssemblyName System.Drawing;\
         $x={x};$y={y};$s={s};\
         $f=New-Object System.Windows.Forms.Form;\
         $f.FormBorderStyle='None';$f.StartPosition='Manual';$f.TopMost=$true;$f.ShowInTaskbar=$false;\
         $f.BackColor=[System.Drawing.Color]::Lime;$f.TransparencyKey=[System.Drawing.Color]::Lime;\
         $f.Bounds=New-Object System.Drawing.Rectangle([int]($x-$s/2),[int]($y-$s/2),$s,$s);\
         $f.Add_Paint({{ $args[1].Graphics.FillEllipse([System.Drawing.Brushes]::Red,0,0,$s,$s) }});\
         $f.Add_Shown({{ $f.Refresh();Start-Sleep -Milliseconds {ms};$f.Close() }});\
         [System.Windows.Forms.Application]::Run($f)",
        x = x,
        y = y,
        s = DOT_PX,
        ms = ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        std::env::remove_var("SLUG_CLICK_FLASH");
        assert!(!enabled());
        std::env::set_var("SLUG_CLICK_FLASH", "1");
        assert!(enabled());
        std::env::set_var("SLUG_CLICK_FLASH", "off");
        assert!(!enabled());
        std::env::remove_var("SLUG_CLICK_FLASH");
    }

    #[test]
    fn scripts_embed_coordinates() {
        let m = macos_script(640, 360);
        assert!(m.contains("var x=640,y=360"), "macOS script missing coords: {m}");
        assert!(m.contains("NSColor") && m.contains("runUntilDate"));
        let w = windows_script(640, 360);
        assert!(w.contains("$x=640;$y=360"), "windows script missing coords: {w}");
        assert!(w.contains("FillEllipse") && w.contains("TopMost"));
    }
}
