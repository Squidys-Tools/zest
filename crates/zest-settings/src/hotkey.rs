//! Turning a recorded keystroke into a hotkey string.
//!
//! `global-hotkey` parses `"Ctrl+Shift+K"`, with modifiers first and one main
//! key last. This is the only place that knows gpui's key names, so it is pure
//! and unit-tested rather than buried in a key handler.

use gpui::Keystroke;

/// The recorded shortcut, or `None` while the user is still mid-combination
/// (a bare modifier is not a shortcut).
pub fn hotkey_from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let key = key_token(&keystroke.key)?;
    let mut parts: Vec<&str> = Vec::with_capacity(4);
    if keystroke.modifiers.control {
        parts.push("Ctrl");
    }
    if keystroke.modifiers.alt {
        parts.push("Alt");
    }
    if keystroke.modifiers.platform {
        parts.push("Super");
    }
    if keystroke.modifiers.shift {
        parts.push("Shift");
    }
    parts.push(&key);
    Some(parts.join("+"))
}

/// A shortcut with no modifier would swallow ordinary typing, and the global
/// listener is armed system-wide, so require one.
pub fn has_modifier(keystroke: &Keystroke) -> bool {
    let m = &keystroke.modifiers;
    m.control || m.alt || m.platform || m.shift || m.function
}

/// gpui lower-cases every key name on Windows (`space`, `pageup`, `f7`); those
/// all parse as-is. Anything unrecognised is rejected so we never write a
/// shortcut that cannot be registered.
fn key_token(key: &str) -> Option<String> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    if key.chars().count() == 1 {
        return Some(key.to_uppercase());
    }
    const NAMED: &[&str] = &[
        "space", "backspace", "enter", "tab", "up", "down", "left", "right", "home", "end", "pageup",
        "pagedown", "escape", "insert", "delete", "printscreen", "scrolllock", "capslock", "numlock",
        "menu", "plus", "minus", "backquote", "backslash", "bracketleft", "bracketright", "comma",
        "dot", "equal", "quote", "semicolon", "slash",
    ];
    let lower = key.to_lowercase();
    if NAMED.contains(&lower.as_str()) || is_function_key(&lower) {
        return Some(lower.to_uppercase());
    }
    None
}

fn is_function_key(key: &str) -> bool {
    let Some(number) = key.strip_prefix('f') else {
        return false;
    };
    !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    use gpui::Modifiers;

    fn stroke(modifiers: Modifiers, key: &str) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        }
    }

    fn mods(control: bool, alt: bool, platform: bool, shift: bool) -> gpui::Modifiers {
        gpui::Modifiers {
            control,
            alt,
            shift,
            platform,
            function: false,
        }
    }

    #[test]
    fn records_a_plain_letter_with_a_modifier() {
        let stroke = stroke(mods(true, false, false, true), "k");
        assert_eq!(hotkey_from_keystroke(&stroke).as_deref(), Some("Ctrl+Shift+K"));
    }

    #[test]
    fn modifiers_come_out_in_parse_order() {
        let stroke = stroke(mods(true, true, true, true), "a");
        assert_eq!(
            hotkey_from_keystroke(&stroke).as_deref(),
            Some("Ctrl+Alt+Super+Shift+A")
        );
    }

    #[test]
    fn named_keys_survive_the_round_trip() {
        for (gpui_name, expected) in [
            ("space", "Ctrl+Shift+SPACE"),
            ("pageup", "Ctrl+Shift+PAGEUP"),
            ("up", "Ctrl+Shift+UP"),
            ("escape", "Ctrl+Shift+ESCAPE"),
            ("f7", "Ctrl+Shift+F7"),
        ] {
            let stroke = stroke(mods(true, false, false, true), gpui_name);
            let recorded = hotkey_from_keystroke(&stroke).expect("maps");
            assert_eq!(recorded, expected);
            assert!(
                recorded.parse::<global_hotkey::hotkey::HotKey>().is_ok(),
                "{recorded} must be registerable"
            );
        }
    }

    #[test]
    fn digits_and_punctuation_are_accepted() {
        for key in ["1", "0", ",", "9"] {
            let stroke = stroke(mods(true, false, false, false), key);
            let recorded = hotkey_from_keystroke(&stroke).expect("maps");
            assert!(recorded.parse::<global_hotkey::hotkey::HotKey>().is_ok(), "{recorded}");
        }
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_saved() {
        let unknown = stroke(mods(true, false, false, false), "browserback");
        assert!(hotkey_from_keystroke(&unknown).is_none());
        let modifier_only = stroke(mods(true, false, false, false), "");
        assert!(hotkey_from_keystroke(&modifier_only).is_none());
    }

    #[test]
    fn a_bare_key_is_not_a_usable_global_shortcut() {
        let bare = stroke(mods(false, false, false, false), "k");
        assert!(!has_modifier(&bare));
        let with_shift = stroke(mods(false, false, false, true), "k");
        assert!(has_modifier(&with_shift));
    }
}
