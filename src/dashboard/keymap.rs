//! Key chord parsing and normalization for the attached-terminal keymap.
//!
//! A `KeyChord` is the lookup-side representation: a normalized triple of
//! modifier flags plus a lowercase key character. The two construction paths
//! — `parse` (from the config string `"ctrl-shift-g"`) and `from_key_event`
//! (from a live crossterm event) — must agree on what an equivalent press
//! looks like, otherwise the keymap will silently miss.
//!
//! Key normalization rule: the key character is always lowercased. An
//! uppercase character in either the config string or the live event is
//! treated as an implicit `shift` modifier — terminals are inconsistent about
//! whether Ctrl+Shift+G arrives as `('G', CONTROL+SHIFT)` or `('G', CONTROL)`,
//! and we normalize both to the same chord.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::{TerminalAction, TerminalConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Always lowercase ASCII for letters.
    pub key: char,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChordParseError {
    Empty,
    NoKey,
    UnknownModifier(String),
    BadKey(String),
}

impl std::fmt::Display for ChordParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "chord string is empty"),
            Self::NoKey => write!(f, "chord has no key after modifiers"),
            Self::UnknownModifier(m) => write!(f, "unknown modifier `{m}`"),
            Self::BadKey(k) => write!(f, "unsupported key `{k}` (only single ASCII letters supported)"),
        }
    }
}

impl std::error::Error for ChordParseError {}

impl KeyChord {
    /// Parse a chord string like `"ctrl-f"`, `"ctrl-shift-g"`, `"alt-x"`.
    /// Modifier order is unrestricted; case-insensitive for modifiers.
    pub fn parse(input: &str) -> Result<Self, ChordParseError> {
        if input.is_empty() {
            return Err(ChordParseError::Empty);
        }
        let parts: Vec<&str> = input.split('-').collect();
        let (key_part, modifier_parts) = parts
            .split_last()
            .ok_or(ChordParseError::Empty)?;
        if key_part.is_empty() {
            return Err(ChordParseError::NoKey);
        }

        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        for raw in modifier_parts {
            match raw.to_ascii_lowercase().as_str() {
                "ctrl" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                other => return Err(ChordParseError::UnknownModifier(other.to_string())),
            }
        }

        let chars: Vec<char> = key_part.chars().collect();
        if chars.len() != 1 || !chars[0].is_ascii_alphabetic() {
            return Err(ChordParseError::BadKey(key_part.to_string()));
        }
        let raw_key = chars[0];
        if raw_key.is_ascii_uppercase() {
            shift = true;
        }
        let key = raw_key.to_ascii_lowercase();

        Ok(Self {
            ctrl,
            shift,
            alt,
            key,
        })
    }

    /// Normalize a live key event into the lookup form. Returns `None` for
    /// events whose `code` is not a single character (we currently only bind
    /// letter keys).
    pub fn from_key_event(event: &KeyEvent) -> Option<Self> {
        let KeyCode::Char(c) = event.code else {
            return None;
        };
        if !c.is_ascii_alphabetic() {
            return None;
        }
        let mods = event.modifiers;
        let mut shift = mods.contains(KeyModifiers::SHIFT);
        if c.is_ascii_uppercase() {
            shift = true;
        }
        Some(Self {
            ctrl: mods.contains(KeyModifiers::CONTROL),
            shift,
            alt: mods.contains(KeyModifiers::ALT),
            key: c.to_ascii_lowercase(),
        })
    }
}

/// Errors raised while resolving a `TerminalConfig` into the runtime keymap.
#[derive(Debug)]
pub enum KeymapError {
    InvalidChord {
        chord: String,
        source: ChordParseError,
    },
    /// The user provided bindings but did not bind `Detach`. Without it the
    /// user has no way back to the dashboard, so we refuse to start.
    MissingDetachBinding,
}

impl std::fmt::Display for KeymapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidChord { chord, source } => {
                write!(f, "invalid key chord `{chord}`: {source}")
            }
            Self::MissingDetachBinding => write!(
                f,
                "terminal.bindings must include an entry for `detach` — \
                 without it there is no way back to the dashboard"
            ),
        }
    }
}

impl std::error::Error for KeymapError {}

/// The built-in keymap. Used when `terminal.bindings` is absent or empty.
pub fn default_keymap() -> HashMap<KeyChord, TerminalAction> {
    [
        ("ctrl-f", TerminalAction::Detach),
        ("ctrl-c", TerminalAction::Sigint),
        ("ctrl-g", TerminalAction::JumpInputNext),
        ("ctrl-shift-g", TerminalAction::JumpInputPrev),
        ("ctrl-j", TerminalAction::SnapToBottom),
        ("ctrl-s", TerminalAction::EnterCopyMode),
        ("ctrl-r", TerminalAction::RefreshParser),
    ]
    .into_iter()
    .map(|(s, a)| {
        (
            KeyChord::parse(s).expect("default chord parses"),
            a,
        )
    })
    .collect()
}

/// Resolve the user's `TerminalConfig` into a runtime keymap.
///
/// Empty `bindings` is interpreted as "I have no opinion" — defaults apply,
/// matching the current behavior. A non-empty `bindings` map fully replaces
/// the defaults; the user is in charge.
pub fn resolve(cfg: &TerminalConfig) -> Result<HashMap<KeyChord, TerminalAction>, KeymapError> {
    if cfg.bindings.is_empty() {
        return Ok(default_keymap());
    }

    let mut map = HashMap::with_capacity(cfg.bindings.len());
    for (chord_str, action) in &cfg.bindings {
        let chord = KeyChord::parse(chord_str).map_err(|source| KeymapError::InvalidChord {
            chord: chord_str.clone(),
            source,
        })?;
        map.insert(chord, *action);
    }

    if !map.values().any(|a| matches!(a, TerminalAction::Detach)) {
        return Err(KeymapError::MissingDetachBinding);
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn ev(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn parse_simple_ctrl_letter() {
        let chord = KeyChord::parse("ctrl-f").unwrap();
        assert_eq!(
            chord,
            KeyChord {
                ctrl: true,
                shift: false,
                alt: false,
                key: 'f',
            }
        );
    }

    #[test]
    fn parse_ctrl_shift_letter() {
        let chord = KeyChord::parse("ctrl-shift-g").unwrap();
        assert_eq!(
            chord,
            KeyChord {
                ctrl: true,
                shift: true,
                alt: false,
                key: 'g',
            }
        );
    }

    #[test]
    fn parse_uppercase_key_implies_shift() {
        // "ctrl-G" should be treated identically to "ctrl-shift-g" so users
        // can write either form in their config.
        assert_eq!(
            KeyChord::parse("ctrl-G").unwrap(),
            KeyChord::parse("ctrl-shift-g").unwrap(),
        );
    }

    #[test]
    fn parse_modifier_order_is_irrelevant() {
        assert_eq!(
            KeyChord::parse("ctrl-shift-g").unwrap(),
            KeyChord::parse("shift-ctrl-g").unwrap(),
        );
    }

    #[test]
    fn parse_rejects_unknown_modifier() {
        let err = KeyChord::parse("super-f").unwrap_err();
        assert!(matches!(err, ChordParseError::UnknownModifier(ref m) if m == "super"));
    }

    #[test]
    fn parse_rejects_empty_and_no_key() {
        assert!(matches!(KeyChord::parse("").unwrap_err(), ChordParseError::Empty));
        // "ctrl-" splits to ["ctrl", ""] — the empty trailing key part is rejected.
        assert!(matches!(
            KeyChord::parse("ctrl-").unwrap_err(),
            ChordParseError::NoKey
        ));
    }

    #[test]
    fn parse_rejects_non_letter_key() {
        // Future work: support digits, F-keys, etc. For now the bind surface
        // is exactly the conflict surface — `Ctrl+<letter>` combos.
        assert!(matches!(
            KeyChord::parse("ctrl-1").unwrap_err(),
            ChordParseError::BadKey(_)
        ));
    }

    #[test]
    fn from_event_lowercases_letter() {
        let chord = KeyChord::from_key_event(&ev(KeyCode::Char('F'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(chord.key, 'f');
        assert!(chord.shift, "uppercase letter should imply shift");
        assert!(chord.ctrl);
    }

    #[test]
    fn from_event_normalizes_shifted_g_two_ways() {
        // Crossterm sometimes reports Ctrl+Shift+G as `('G', CONTROL)` and
        // sometimes as `('g', CONTROL+SHIFT)` depending on the terminal's
        // keyboard-enhancement support. Both must produce the same chord.
        let a =
            KeyChord::from_key_event(&ev(KeyCode::Char('G'), KeyModifiers::CONTROL)).unwrap();
        let b = KeyChord::from_key_event(&ev(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert_eq!(a, b);
        assert_eq!(a, KeyChord::parse("ctrl-shift-g").unwrap());
    }

    #[test]
    fn from_event_returns_none_for_non_letter() {
        assert!(KeyChord::from_key_event(&ev(KeyCode::Esc, KeyModifiers::NONE)).is_none());
        assert!(
            KeyChord::from_key_event(&ev(KeyCode::Char('1'), KeyModifiers::CONTROL)).is_none(),
            "digits not yet bindable"
        );
    }

    fn chord(s: &str) -> KeyChord {
        KeyChord::parse(s).unwrap()
    }

    #[test]
    fn default_keymap_contains_all_seven_actions() {
        let km = default_keymap();
        let actions: std::collections::HashSet<_> = km.values().copied().collect();
        for expected in [
            TerminalAction::Detach,
            TerminalAction::Sigint,
            TerminalAction::JumpInputNext,
            TerminalAction::JumpInputPrev,
            TerminalAction::SnapToBottom,
            TerminalAction::EnterCopyMode,
            TerminalAction::RefreshParser,
        ] {
            assert!(
                actions.contains(&expected),
                "default keymap missing action {expected:?}"
            );
        }
    }

    #[test]
    fn resolve_returns_defaults_when_bindings_empty() {
        let resolved = resolve(&TerminalConfig::default()).unwrap();
        assert_eq!(resolved, default_keymap());
    }

    #[test]
    fn resolve_uses_user_bindings_when_provided() {
        let mut bindings = HashMap::new();
        bindings.insert("ctrl-f".to_string(), TerminalAction::Detach);
        bindings.insert("ctrl-c".to_string(), TerminalAction::Sigint);
        let resolved = resolve(&TerminalConfig { bindings }).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(
            resolved.get(&chord("ctrl-f")),
            Some(&TerminalAction::Detach)
        );
        assert_eq!(
            resolved.get(&chord("ctrl-c")),
            Some(&TerminalAction::Sigint)
        );
        assert!(
            !resolved.contains_key(&chord("ctrl-r")),
            "user did not bind ctrl-r so it should pass through"
        );
    }

    #[test]
    fn resolve_can_remap_action_to_a_different_key() {
        let mut bindings = HashMap::new();
        bindings.insert("ctrl-f".to_string(), TerminalAction::Detach);
        bindings.insert("alt-r".to_string(), TerminalAction::RefreshParser);
        let resolved = resolve(&TerminalConfig { bindings }).unwrap();
        assert_eq!(
            resolved.get(&chord("alt-r")),
            Some(&TerminalAction::RefreshParser),
            "user remapped refresh-parser onto alt-r"
        );
        assert!(
            !resolved.contains_key(&chord("ctrl-r")),
            "default ctrl-r binding should be gone"
        );
    }

    #[test]
    fn resolve_rejects_user_config_without_detach() {
        let mut bindings = HashMap::new();
        bindings.insert("ctrl-c".to_string(), TerminalAction::Sigint);
        let err = resolve(&TerminalConfig { bindings }).unwrap_err();
        assert!(
            matches!(err, KeymapError::MissingDetachBinding),
            "got {err:?}"
        );
    }

    #[test]
    fn resolve_rejects_unparseable_chord() {
        let mut bindings = HashMap::new();
        bindings.insert("ctrl-f".to_string(), TerminalAction::Detach);
        bindings.insert("super-x".to_string(), TerminalAction::Sigint);
        let err = resolve(&TerminalConfig { bindings }).unwrap_err();
        match err {
            KeymapError::InvalidChord { chord, .. } => assert_eq!(chord, "super-x"),
            other => panic!("expected InvalidChord, got {other:?}"),
        }
    }
}
