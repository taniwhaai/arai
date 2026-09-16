//! CLI adapter identities and declared capabilities. Internal, not a Kete API.
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Platform {
    Claude,
    Grok,
    Codex,
    Cursor,
}

impl Platform {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Grok, Self::Codex, Self::Cursor];
    pub const LEGACY_DEFAULTS: [Self; 3] = [Self::Claude, Self::Grok, Self::Codex];

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Grok => "grok",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }

    pub fn config_path(self) -> &'static str {
        match self {
            Self::Claude => ".claude/settings.json",
            Self::Grok => ".grok/hooks/arai.json",
            Self::Codex => ".codex/hooks.json",
            Self::Cursor => ".cursor/hooks.json",
        }
    }

    pub fn pre_event(self) -> &'static str {
        if self == Self::Cursor {
            "preToolUse"
        } else {
            "PreToolUse"
        }
    }

    pub fn post_event(self) -> &'static str {
        if self == Self::Cursor {
            "postToolUse"
        } else {
            "PostToolUse"
        }
    }

    /// Whether Arai emits allow-side context. Grok delivery is best-effort;
    /// this flag is not a claim that the host has displayed it to the model.
    pub fn pre_advisory_context(self) -> bool {
        self != Self::Cursor
    }

    pub fn prompt_context(self) -> bool {
        self != Self::Cursor
    }
}
