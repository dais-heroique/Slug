//! Windows synthetic input via `SendInput`.
//!
//! The "drive any app" escape hatch on Windows: it posts keyboard events to the
//! OS input queue, delivered to the **focused** application — even one that
//! exposes no UI Automation tree. No pixels, no screenshot, no model tokens.
//!
//! * [`Action::Key`] — a chord like `ctrl+s`, `alt+tab`, `enter`, `up`.
//!   Modifiers (`ctrl`/`alt`/`shift`/`win`) + one key.
//! * [`Action::TypeText`] — literal unicode text, sent via `KEYEVENTF_UNICODE`
//!   (no per-character virtual-key mapping needed).

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10,
    VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_LEFT,
    VK_LWIN, VK_MENU as VK_ALT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP, VK_CONTROL,
};

use crate::action::Action;
use crate::error::{BridgeError, Result};

/// Perform a synthetic-input [`Action`] (`Key` or `TypeText`).
pub fn perform_synth(action: &Action) -> Result<()> {
    match action {
        Action::Key(spec) => key_chord(spec),
        Action::TypeText(text) => type_text(text),
        other => Err(BridgeError::InvalidArgs {
            action: other.id(),
            detail: "not a synthetic-input action".into(),
        }),
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(BridgeError::Backend(format!("SendInput sent {sent}/{} events", inputs.len())))
    }
}

fn key_event(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Type literal unicode text into the focused app via `KEYEVENTF_UNICODE`.
fn type_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let mut inputs: Vec<INPUT> = Vec::new();
    for unit in text.encode_utf16() {
        inputs.push(key_event(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
        inputs.push(key_event(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    send(&inputs)
}

/// Send a key chord: zero or more modifiers plus exactly one key.
fn key_chord(spec: &str) -> Result<()> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(BridgeError::InvalidArgs { action: "key".into(), detail: "empty key spec".into() });
    }
    let mut mods: Vec<VIRTUAL_KEY> = Vec::new();
    let mut key: Option<VIRTUAL_KEY> = None;
    for part in spec.split('+') {
        let p = part.trim().to_ascii_lowercase();
        match p.as_str() {
            "ctrl" | "control" => mods.push(VK_CONTROL),
            "alt" | "opt" | "option" => mods.push(VK_ALT),
            "shift" => mods.push(VK_SHIFT),
            "cmd" | "command" | "meta" | "super" | "win" => mods.push(VK_LWIN),
            other => {
                key = Some(vk_for(other).ok_or_else(|| BridgeError::InvalidArgs {
                    action: "key".into(),
                    detail: format!("unknown key '{other}' in chord '{spec}'"),
                })?);
            }
        }
    }
    let key = key.ok_or_else(|| BridgeError::InvalidArgs {
        action: "key".into(),
        detail: format!("chord '{spec}' has modifiers but no key"),
    })?;

    let mut inputs: Vec<INPUT> = Vec::new();
    for m in &mods {
        inputs.push(key_event(*m, 0, KEYBD_EVENT_FLAGS(0)));
    }
    inputs.push(key_event(key, 0, KEYBD_EVENT_FLAGS(0)));
    inputs.push(key_event(key, 0, KEYEVENTF_KEYUP));
    for m in mods.iter().rev() {
        inputs.push(key_event(*m, 0, KEYEVENTF_KEYUP));
    }
    send(&inputs)
}

/// Map a key name to a Windows virtual-key code. Letters/digits use their ASCII
/// codes; common navigation/editing/function keys are named.
fn vk_for(name: &str) -> Option<VIRTUAL_KEY> {
    if name.len() == 1 {
        let c = name.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return Some(VIRTUAL_KEY(c.to_ascii_uppercase() as u16));
        }
        if c.is_ascii_digit() {
            return Some(VIRTUAL_KEY(c as u16));
        }
    }
    let vk = match name {
        "return" | "enter" => VK_RETURN,
        "tab" => VK_TAB,
        "space" | "spacebar" => VK_SPACE,
        "delete" | "backspace" => VK_BACK,
        "forwarddelete" | "fwddelete" | "del" => VK_DELETE,
        "escape" | "esc" => VK_ESCAPE,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "f1" => VK_F1,
        "f2" => VK_F2,
        "f3" => VK_F3,
        "f4" => VK_F4,
        "f5" => VK_F5,
        "f6" => VK_F6,
        "f7" => VK_F7,
        "f8" => VK_F8,
        "f9" => VK_F9,
        "f10" => VK_F10,
        "f11" => VK_F11,
        "f12" => VK_F12,
        _ => return None,
    };
    Some(vk)
}
