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

    /// Whether the protocol accepts allow-side context. This does not prove
    /// host activation or delivery; Grok delivers it after the tool completes.
    pub fn pre_advisory_context(self) -> bool {
        self != Self::Cursor
    }

    pub fn prompt_context(self) -> bool {
        matches!(self, Self::Claude | Self::Codex)
    }

    pub fn pre_context_timing(self) -> &'static str {
        match self {
            Self::Claude | Self::Codex => "before-tool",
            Self::Grok => "after-tool",
            Self::Cursor => "unsupported",
        }
    }

    /// Startup/subagent model context, independent of a visible user notice.
    pub fn lifecycle_context(self) -> bool {
        matches!(self, Self::Claude | Self::Codex)
    }

    /// A user-facing lifecycle notice is supported by the current host contract.
    /// This is a capability, not evidence that a particular installation ran it.
    pub fn lifecycle_status(self) -> bool {
        self != Self::Cursor
    }
}
