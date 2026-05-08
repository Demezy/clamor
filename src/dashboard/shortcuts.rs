/// The compact footer strip at the bottom of the dashboard. When a shortcut
/// sets `footer: Some(..)` it is included; `None` keeps it out. Required on
/// every `Shortcut` so adding a new binding forces an explicit decision.
#[derive(Debug, Clone, Copy)]
pub struct FooterEntry {
    /// Bracketed key label, e.g. `"r"` renders as `[r]`.
    pub key: &'static str,
    /// Text printed immediately after `]`. Include a leading space for the
    /// normal `[x] kill` spacing, or omit it for the run-on `[c]reate` style.
    pub label: &'static str,
}

/// A keyboard shortcut for display in help popups, documentation, and the
/// dashboard footer strip.
pub struct Shortcut {
    pub keys: &'static str,
    pub description: &'static str,
    /// `Some(..)` → visible in the dashboard footer; `None` → help-popup only.
    pub footer: Option<FooterEntry>,
}

const SECTIONS: &[(&str, &[Shortcut])] = &[
    ("Dashboard", DASHBOARD_SHORTCUTS),
    ("Spawn Prompt", SPAWN_PROMPT_SHORTCUTS),
    ("Terminal", TERMINAL_SHORTCUTS),
    ("Copy Mode", COPY_MODE_SHORTCUTS),
];

/// Whether a shortcut matches a filter query (case-insensitive).
fn matches_filter(shortcut: &Shortcut, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let f = filter.to_ascii_lowercase();
    shortcut.keys.to_ascii_lowercase().contains(&f)
        || shortcut.description.to_ascii_lowercase().contains(&f)
}

/// Count total display lines for the help popup (with optional filter).
pub fn help_line_count(filter: &str) -> usize {
    let mut count = 0;
    for (_, items) in SECTIONS.iter() {
        let filtered: Vec<_> = items.iter().filter(|s| matches_filter(s, filter)).collect();
        if filtered.is_empty() {
            continue;
        }
        if count > 0 {
            count += 1; // blank line between sections
        }
        count += 2 + filtered.len(); // header + separator + items
    }
    count
}

/// Return the sections (for use by render).
pub fn sections() -> &'static [(&'static str, &'static [Shortcut])] {
    SECTIONS
}

pub const DASHBOARD_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "J / K / ↑↓",
        description: "navigate",
        footer: Some(FooterEntry {
            key: "J/K",
            label: " select",
        }),
    },
    Shortcut {
        keys: "Enter",
        description: "attach",
        footer: None,
    },
    Shortcut {
        keys: "gg / G",
        description: "first / last",
        footer: None,
    },
    Shortcut {
        keys: "^G / ^\u{21E7}G",
        description: "jump to next / prev agent waiting for input",
        footer: Some(FooterEntry {
            key: "^G",
            label: " input",
        }),
    },
    Shortcut {
        keys: "v",
        description: "toggle select",
        footer: Some(FooterEntry {
            key: "v",
            label: " mark",
        }),
    },
    Shortcut {
        keys: "V",
        description: "select all / none",
        footer: None,
    },
    Shortcut {
        keys: "/",
        description: "filter",
        footer: Some(FooterEntry {
            key: "/",
            label: " filter",
        }),
    },
    Shortcut {
        keys: "c",
        description: "create",
        footer: Some(FooterEntry {
            key: "c",
            label: "reate",
        }),
    },
    Shortcut {
        keys: "C",
        description: "create ($EDITOR)",
        footer: None,
    },
    Shortcut {
        keys: "e + key",
        description: "edit description",
        footer: None,
    },
    Shortcut {
        keys: "x + key",
        description: "kill (batch if selected)",
        footer: Some(FooterEntry {
            key: "x",
            label: " kill",
        }),
    },
    Shortcut {
        keys: "r + key",
        description: "reload session",
        footer: Some(FooterEntry {
            key: "r",
            label: "eload",
        }),
    },
    Shortcut {
        keys: "R",
        description: "adopt session",
        footer: None,
    },
    Shortcut {
        keys: "b",
        description: "rebind all agent keys (ergonomic order)",
        footer: Some(FooterEntry {
            key: "b",
            label: " rebind",
        }),
    },
    Shortcut {
        keys: "?",
        description: "help",
        footer: Some(FooterEntry {
            key: "?",
            label: " help",
        }),
    },
    Shortcut {
        keys: "Esc",
        description: "clear selection",
        footer: None,
    },
    Shortcut {
        keys: "q",
        description: "quit",
        footer: None,
    },
];

/// Defaults shown for the attached terminal. All entries except `^F` (detach)
/// can be omitted or remapped via the `terminal.bindings` section in
/// `~/.config/clamor/config.yaml`. Combos absent from the resolved keymap are
/// forwarded to the agent's PTY, so wrapped tools like Claude Code receive
/// their own Ctrl+R / Ctrl+S / Ctrl+J as expected.
pub const TERMINAL_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "^F",
        description: "detach (back to dashboard) — required, cannot be unbound",
        footer: None,
    },
    Shortcut {
        keys: "^C",
        description: "send SIGINT to agent (configurable)",
        footer: None,
    },
    Shortcut {
        keys: "^J",
        description: "snap to bottom (configurable)",
        footer: None,
    },
    Shortcut {
        keys: "^G / ^\u{21E7}G",
        description: "jump to next / prev agent waiting for input (configurable)",
        footer: None,
    },
    Shortcut {
        keys: "^R",
        description: "refresh terminal (configurable)",
        footer: None,
    },
    Shortcut {
        keys: "^S",
        description: "enter copy mode (configurable)",
        footer: None,
    },
    Shortcut {
        keys: "scroll up",
        description: "freeze + scroll",
        footer: None,
    },
];

pub const COPY_MODE_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "h/j/k/l",
        description: "move cursor",
        footer: None,
    },
    Shortcut {
        keys: "v",
        description: "toggle selection",
        footer: None,
    },
    Shortcut {
        keys: "V",
        description: "toggle line selection",
        footer: None,
    },
    Shortcut {
        keys: "y",
        description: "yank to clipboard + exit",
        footer: None,
    },
    Shortcut {
        keys: "0 / $",
        description: "start / end of line",
        footer: None,
    },
    Shortcut {
        keys: "^U / ^D",
        description: "half page up / down",
        footer: None,
    },
    Shortcut {
        keys: "gg / G",
        description: "top / bottom of scrollback",
        footer: None,
    },
    Shortcut {
        keys: "q / Esc",
        description: "exit copy mode",
        footer: None,
    },
];

pub const SPAWN_PROMPT_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "Tab / Shift+Tab",
        description: "cycle fields (title \u{2192} description \u{2192} backend)",
        footer: None,
    },
    Shortcut {
        keys: "\u{2190} / \u{2192}",
        description: "select backend (when backend field active)",
        footer: None,
    },
    Shortcut {
        keys: "Shift+Enter",
        description: "new line in description",
        footer: None,
    },
    Shortcut {
        keys: "\u{2191} / \u{2193}",
        description: "prompt history",
        footer: None,
    },
    Shortcut {
        keys: "^W / Alt+Bksp",
        description: "delete word",
        footer: None,
    },
    Shortcut {
        keys: "^U",
        description: "delete line",
        footer: None,
    },
];
