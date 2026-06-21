//! Best-effort synthetic input on Linux.
//!
//! Unlike macOS (CGEvent) and Windows (SendInput), there is no in-process API to
//! inject input on Linux: **Wayland blocks an app from synthesising input into
//! another by design**. We therefore shell out to a system input tool when one is
//! installed:
//!
//! * **`xdotool`** (X11 / XWayland) — full support: key chords, text, click, scroll.
//! * **`ydotool`** (Wayland, via the uinput kernel device + its daemon) — text and
//!   click (its key API needs raw keycodes, so chords/scroll are left to xdotool).
//!
//! If neither tool is present we return a clear, actionable error rather than
//! failing silently — the agent should then fall back to the semantic path
//! (`slug_invoke` on a node ref), which always works on Linux.

use std::process::{Command, Stdio};

use crate::action::Action;
use crate::error::{BridgeError, Result};

/// Which external input tool to drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Xdotool,
    Ydotool,
}

impl Tool {
    fn binary(self) -> &'static str {
        match self {
            Tool::Xdotool => "xdotool",
            Tool::Ydotool => "ydotool",
        }
    }
}

/// Inject one synthetic-input action. Only [`Action::Key`], [`Action::TypeText`],
/// [`Action::MouseClick`] and [`Action::Scroll`] are valid here.
pub fn synth(action: &Action) -> Result<()> {
    // Prefer xdotool (broadest coverage); fall back to ydotool for what it can do.
    let tool = pick_tool(action).ok_or_else(|| {
        BridgeError::Unsupported(
            "synthetic input on Linux needs 'xdotool' (X11/XWayland) or 'ydotool' \
             (Wayland) installed and on PATH — neither was found. Install one, or use \
             the semantic path (slug_invoke on a node ref), which needs no injection."
                .to_string(),
        )
    })?;
    for argv in build_plan(tool, action)? {
        run(&argv)?;
    }
    Ok(())
}

/// Choose a tool that is installed *and* can perform `action`.
fn pick_tool(action: &Action) -> Option<Tool> {
    let xdo = on_path("xdotool");
    let ydo = on_path("ydotool");
    // xdotool can do everything; ydotool only text + click.
    let ydo_capable = matches!(action, Action::TypeText(_) | Action::MouseClick { .. });
    if xdo {
        Some(Tool::Xdotool)
    } else if ydo && ydo_capable {
        Some(Tool::Ydotool)
    } else {
        None
    }
}

/// Build the sequence of commands (program + args) to run for `action`. Returned
/// as a list so a tool that needs two steps (move then click) is uniform.
fn build_plan(tool: Tool, action: &Action) -> Result<Vec<Vec<String>>> {
    let bin = tool.binary().to_string();
    let plan = match (tool, action) {
        // --- xdotool: full coverage -----------------------------------------
        (Tool::Xdotool, Action::Key(chord)) => {
            vec![vec![bin, "key".into(), translate_chord_x11(chord)]]
        }
        (Tool::Xdotool, Action::TypeText(text)) => {
            vec![vec![bin, "type".into(), "--".into(), text.clone()]]
        }
        (Tool::Xdotool, Action::MouseClick { x, y }) => {
            vec![vec![
                bin,
                "mousemove".into(),
                round(*x),
                round(*y),
                "click".into(),
                "1".into(),
            ]]
        }
        (Tool::Xdotool, Action::Scroll { x, y, dx, dy }) => {
            let mut cmd = vec![bin, "mousemove".into(), round(*x), round(*y)];
            // X11 wheel buttons: 4=up, 5=down, 6=left, 7=right. Our convention:
            // negative dy scrolls down.
            if *dy != 0.0 {
                let button = if *dy < 0.0 { "5" } else { "4" };
                cmd.extend(["click".into(), "--repeat".into(), repeat(*dy), button.into()]);
            }
            if *dx != 0.0 {
                let button = if *dx < 0.0 { "6" } else { "7" };
                cmd.extend(["click".into(), "--repeat".into(), repeat(*dx), button.into()]);
            }
            vec![cmd]
        }

        // --- ydotool: text + click only -------------------------------------
        (Tool::Ydotool, Action::TypeText(text)) => {
            vec![vec![bin, "type".into(), "--".into(), text.clone()]]
        }
        (Tool::Ydotool, Action::MouseClick { x, y }) => {
            vec![
                vec![bin.clone(), "mousemove".into(), "-a".into(), round(*x), round(*y)],
                // 0xC0 = left button down+up in ydotool's click encoding.
                vec![bin, "click".into(), "0xC0".into()],
            ]
        }

        // Anything else on a given tool is unsupported.
        (_, Action::Key(_) | Action::Scroll { .. }) => {
            return Err(BridgeError::Unsupported(
                "key chords and scrolling on Wayland need 'xdotool' (install it; \
                 'ydotool' cannot do these without raw keycodes)"
                    .to_string(),
            ));
        }
        _ => {
            return Err(BridgeError::Unsupported(
                "only key/type_text/click/scroll are valid synthetic-input actions".to_string(),
            ))
        }
    };
    Ok(plan)
}

/// Translate a portable chord (`cmd+s`, `shift+tab`, `return`, `up`) into the
/// `xdotool key` keysym form (`ctrl+s`, `shift+Tab`, `Return`, `Up`). On Linux
/// the Mac `cmd` modifier maps to `ctrl` (what app shortcuts actually use).
fn translate_chord_x11(chord: &str) -> String {
    chord
        .split('+')
        .map(|tok| {
            let t = tok.trim().to_ascii_lowercase();
            match t.as_str() {
                "cmd" | "command" | "meta" => "ctrl".to_string(),
                "super" | "win" => "super".to_string(),
                "ctrl" | "control" => "ctrl".to_string(),
                "alt" | "option" => "alt".to_string(),
                "shift" => "shift".to_string(),
                "enter" | "return" => "Return".to_string(),
                "esc" | "escape" => "Escape".to_string(),
                "tab" => "Tab".to_string(),
                "space" => "space".to_string(),
                "backspace" => "BackSpace".to_string(),
                "delete" | "del" => "Delete".to_string(),
                "home" => "Home".to_string(),
                "end" => "End".to_string(),
                "pageup" | "pgup" => "Prior".to_string(),
                "pagedown" | "pgdn" => "Next".to_string(),
                "up" => "Up".to_string(),
                "down" => "Down".to_string(),
                "left" => "Left".to_string(),
                "right" => "Right".to_string(),
                // function keys f1..f12 → F1..F12
                f if f.starts_with('f') && f[1..].parse::<u8>().is_ok() => f.to_uppercase(),
                other => other.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn round(v: f64) -> String {
    (v.round() as i64).to_string()
}

/// Number of wheel "clicks" for a scroll amount (at least 1).
fn repeat(v: f64) -> String {
    (v.abs().round() as i64).max(1).to_string()
}

/// Whether `name` is an executable on `PATH`.
fn on_path(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

/// Spawn one command, mapping failure to a backend error.
fn run(argv: &[String]) -> Result<()> {
    let (prog, args) = argv.split_first().expect("non-empty argv");
    let status = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| BridgeError::Backend(format!("failed to run {prog}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(BridgeError::Backend(format!("{prog} exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_translation_maps_mac_and_named_keys() {
        assert_eq!(translate_chord_x11("cmd+s"), "ctrl+s");
        assert_eq!(translate_chord_x11("shift+tab"), "shift+Tab");
        assert_eq!(translate_chord_x11("return"), "Return");
        assert_eq!(translate_chord_x11("ctrl+shift+z"), "ctrl+shift+z");
        assert_eq!(translate_chord_x11("up"), "Up");
        assert_eq!(translate_chord_x11("f5"), "F5");
        assert_eq!(translate_chord_x11("super+l"), "super+l");
    }

    #[test]
    fn xdotool_plan_for_each_action() {
        let key = build_plan(Tool::Xdotool, &Action::Key("cmd+s".into())).unwrap();
        assert_eq!(key, vec![vec!["xdotool", "key", "ctrl+s"]]);

        let text = build_plan(Tool::Xdotool, &Action::TypeText("hi".into())).unwrap();
        assert_eq!(text, vec![vec!["xdotool", "type", "--", "hi"]]);

        let click = build_plan(Tool::Xdotool, &Action::MouseClick { x: 12.4, y: 30.6 }).unwrap();
        assert_eq!(click, vec![vec!["xdotool", "mousemove", "12", "31", "click", "1"]]);

        // Negative dy → scroll down (button 5), repeated abs(dy) times.
        let scroll =
            build_plan(Tool::Xdotool, &Action::Scroll { x: 5.0, y: 6.0, dx: 0.0, dy: -3.0 }).unwrap();
        assert_eq!(
            scroll,
            vec![vec!["xdotool", "mousemove", "5", "6", "click", "--repeat", "3", "5"]]
        );
    }

    #[test]
    fn ydotool_plan_text_and_click_only() {
        let text = build_plan(Tool::Ydotool, &Action::TypeText("hello".into())).unwrap();
        assert_eq!(text, vec![vec!["ydotool", "type", "--", "hello"]]);

        let click = build_plan(Tool::Ydotool, &Action::MouseClick { x: 1.0, y: 2.0 }).unwrap();
        assert_eq!(
            click,
            vec![
                vec!["ydotool", "mousemove", "-a", "1", "2"],
                vec!["ydotool", "click", "0xC0"],
            ]
        );

        // ydotool can't do chords/scroll → explicit error pointing at xdotool.
        assert!(build_plan(Tool::Ydotool, &Action::Key("cmd+s".into())).is_err());
        assert!(build_plan(
            Tool::Ydotool,
            &Action::Scroll { x: 0.0, y: 0.0, dx: 0.0, dy: -1.0 }
        )
        .is_err());
    }
}
