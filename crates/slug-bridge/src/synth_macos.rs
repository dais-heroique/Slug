//! macOS synthetic input via Quartz `CGEvent`.
//!
//! This is the "drive any app" escape hatch: it posts keyboard events to the
//! OS event stream, which the **currently focused application** receives — even
//! if that app exposes no accessibility tree. It captures **no pixels** and
//! costs **zero model tokens**; it is pure event injection.
//!
//! Two operations, both reached through [`crate::action::Action`]:
//! * [`Action::Key`] — a chord like `cmd+s`, `shift+tab`, `return`, `escape`,
//!   `up`. Modifiers (`cmd`/`ctrl`/`alt`/`opt`/`shift`) + one key.
//! * [`Action::TypeText`] — literal unicode text, typed via
//!   `CGEvent::set_string` (no per-character keycode needed).
//!
//! Permission: posting CGEvents requires the host process to hold **Accessibility**
//! (and, depending on macOS version, **Input Monitoring**) permission — the same
//! TCC grant the AX backend already requires.

use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGKeyCode, CGMouseButton,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use crate::action::Action;
use crate::error::{BridgeError, Result};

/// Perform a synthetic-input [`Action`].
pub fn perform_synth(action: &Action) -> Result<()> {
    match action {
        Action::Key(spec) => key_chord(spec),
        Action::TypeText(text) => type_text(text),
        Action::MouseClick { x, y } => mouse_click(*x, *y),
        Action::Scroll { x, y, dx, dy } => scroll(*x, *y, *dx, *dy),
        other => Err(BridgeError::InvalidArgs {
            action: other.id(),
            detail: "not a synthetic-input action".into(),
        }),
    }
}

/// Scroll at a screen point by `(dx, dy)` wheel lines (negative dy = down).
fn scroll(x: f64, y: f64, dx: f64, dy: f64) -> Result<()> {
    let src = source()?;
    // Move the cursor over the target so the scroll lands on the right view.
    if let Ok(mv) =
        CGEvent::new_mouse_event(src.clone(), CGEventType::MouseMoved, CGPoint::new(x, y), CGMouseButton::Left)
    {
        mv.post(CGEventTapLocation::HID);
    }
    let ev = CGEvent::new_scroll_event(src, ScrollEventUnit::LINE, 2, dy as i32, dx as i32, 0)
        .map_err(|_| BridgeError::Backend("CGEvent scroll failed".into()))?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// Left-click at absolute screen coordinates.
pub(crate) fn mouse_click(x: f64, y: f64) -> Result<()> {
    let src = source()?;
    let pt = CGPoint::new(x, y);
    let down = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseDown, pt, CGMouseButton::Left)
        .map_err(|_| BridgeError::Backend("CGEvent mouse (down) failed".into()))?;
    down.post(CGEventTapLocation::HID);
    let up = CGEvent::new_mouse_event(src, CGEventType::LeftMouseUp, pt, CGMouseButton::Left)
        .map_err(|_| BridgeError::Backend("CGEvent mouse (up) failed".into()))?;
    up.post(CGEventTapLocation::HID);
    Ok(())
}

fn source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| BridgeError::Backend("CGEventSource::new failed".into()))
}

/// Type literal unicode text into the focused app (one keyboard event carrying
/// the whole string — robust across layouts).
fn type_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let src = source()?;
    // keycode 0 is a placeholder; the unicode payload is delivered on the key-down.
    // Setting the string on key-up too can double-insert in some apps, so we don't.
    let down = CGEvent::new_keyboard_event(src.clone(), 0, true)
        .map_err(|_| BridgeError::Backend("CGEvent keyboard (down) failed".into()))?;
    down.set_string(text);
    down.post(CGEventTapLocation::HID);
    let up = CGEvent::new_keyboard_event(src, 0, false)
        .map_err(|_| BridgeError::Backend("CGEvent keyboard (up) failed".into()))?;
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// Send a key chord: zero or more modifiers plus exactly one key, e.g.
/// `cmd+shift+z`, `return`, `tab`, `down`.
fn key_chord(spec: &str) -> Result<()> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(BridgeError::InvalidArgs {
            action: "key".into(),
            detail: "empty key spec".into(),
        });
    }
    let mut flags = CGEventFlags::CGEventFlagNonCoalesced;
    let mut key: Option<CGKeyCode> = None;
    for part in spec.split('+') {
        let p = part.trim().to_ascii_lowercase();
        match p.as_str() {
            "cmd" | "command" | "meta" | "super" | "win" => flags |= CGEventFlags::CGEventFlagCommand,
            "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
            "alt" | "opt" | "option" => flags |= CGEventFlags::CGEventFlagAlternate,
            "shift" => flags |= CGEventFlags::CGEventFlagShift,
            "fn" => flags |= CGEventFlags::CGEventFlagSecondaryFn,
            other => {
                key = Some(keycode(other).ok_or_else(|| BridgeError::InvalidArgs {
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

    let src = source()?;
    let down = CGEvent::new_keyboard_event(src.clone(), key, true)
        .map_err(|_| BridgeError::Backend("CGEvent keyboard (down) failed".into()))?;
    down.set_flags(flags);
    down.post(CGEventTapLocation::HID);
    let up = CGEvent::new_keyboard_event(src, key, false)
        .map_err(|_| BridgeError::Backend("CGEvent keyboard (up) failed".into()))?;
    up.set_flags(flags);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// Map a key name to a macOS ANSI virtual keycode. Covers letters, digits, and
/// the common navigation/editing keys an agent needs.
fn keycode(name: &str) -> Option<CGKeyCode> {
    let c = match name {
        // letters
        "a" => 0, "s" => 1, "d" => 2, "f" => 3, "h" => 4, "g" => 5, "z" => 6, "x" => 7,
        "c" => 8, "v" => 9, "b" => 11, "q" => 12, "w" => 13, "e" => 14, "r" => 15, "y" => 16,
        "t" => 17, "o" => 31, "u" => 32, "i" => 34, "p" => 35, "l" => 37, "j" => 38, "k" => 40,
        "n" => 45, "m" => 46,
        // digits (top row)
        "1" => 18, "2" => 19, "3" => 20, "4" => 21, "5" => 23, "6" => 22, "7" => 26, "8" => 28,
        "9" => 25, "0" => 29,
        // punctuation
        "=" | "equal" => 24, "-" | "minus" => 27, "]" => 30, "[" => 33, "'" | "quote" => 39,
        ";" | "semicolon" => 41, "\\" | "backslash" => 42, "," | "comma" => 43,
        "/" | "slash" => 44, "." | "period" => 47, "`" | "grave" => 50,
        // named keys
        "return" | "enter" => 36, "tab" => 48, "space" | "spacebar" => 49,
        "delete" | "backspace" => 51, "escape" | "esc" => 53,
        "left" => 123, "right" => 124, "down" => 125, "up" => 126,
        "home" => 115, "end" => 119, "pageup" | "pgup" => 116, "pagedown" | "pgdn" => 121,
        "forwarddelete" | "fwddelete" => 117,
        "f1" => 122, "f2" => 120, "f3" => 99, "f4" => 118, "f5" => 96, "f6" => 97,
        "f7" => 98, "f8" => 100, "f9" => 101, "f10" => 109, "f11" => 103, "f12" => 111,
        _ => return None,
    };
    Some(c as CGKeyCode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_resolve() {
        assert!(keycode("a").is_some());
        assert!(keycode("return").is_some());
        assert!(keycode("up").is_some());
        assert!(keycode("f5").is_some());
        assert!(keycode("nope").is_none());
    }
}
