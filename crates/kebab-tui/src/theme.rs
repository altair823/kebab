//! p9-fb-14: TUI palette + role-style mapping.
//!
//! Every pane (`library`, `search`, `ask`, `inspect`, `error_popup`,
//! the `run::render_root` shell) routes its `ratatui::style::Style`
//! through `Theme::style(role)` instead of inlining
//! `Style::default().fg(...)`. Adding a new role here is the only
//! place a color decision needs to land — accidental drift between
//! panes (`Cyan` for one badge, `LightCyan` for another) becomes a
//! single-file diff.
//!
//! ## Why role-based, not "style table"
//!
//! Earlier sketches keyed a hashmap by role at runtime. A `match`
//! against an enum is faster (no allocation, no hashing), exhaustive
//! at compile time (forgetting a role for `Theme::light` is a
//! compile error if you `match` exhaustively on `Role` in the
//! palette body — and we do), and lets `Theme::style` return
//! `Style` by value without lifetimes.
//!
//! ## Accessibility
//!
//! Color is never the *only* signal — the score badge ships
//! `[score=0.92]` text alongside its color, the mode badge ships
//! `[Hybrid]` text, the refusal renders the literal `(refused)`
//! prefix. The theme just amplifies signals that the text already
//! carries.

use ratatui::style::{Color, Modifier, Style};

/// Role-style enumeration. Adding a variant requires updating both
/// `dark_style` and `light_style` (the compiler enforces it via the
/// exhaustive `match` in each palette).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Role {
    /// Active pane border (the focused one).
    BorderActive,
    /// Inactive pane border.
    BorderInactive,
    /// Document/section title (bold, accent color).
    Title,
    /// Secondary path / subpath (dim).
    Path,
    /// Lexical search-mode badge.
    ModeLexical,
    /// Vector search-mode badge.
    ModeVector,
    /// Hybrid search-mode badge.
    ModeHybrid,
    /// Selected row in any list (search hits, library docs, …).
    Selected,
    /// Dim hint / placeholder text (mode line subtext, "loading…").
    Hint,
    /// Section heading (bold + accent — Inspect uses this).
    Heading,
    /// Warning yellow — refusals, malformed-frontmatter notices.
    Warning,
    /// Error red — error overlays, "spawn failed" lines.
    Error,
    /// Success green — completed ingest, grounded answer.
    Success,
    /// Citation marker (`[1]`, `[2]`) and citation link text.
    CitationMarker,
    /// Bullet glyph in list rendering.
    Bullet,
    /// Default body text (no decoration). Returned as
    /// `Style::default()` in both palettes — kept as a Role so
    /// callers don't sprinkle `Style::default()` directly.
    Body,
}

/// Palette identity. `Theme` carries this so panes can branch on
/// "is dark" if they need a different glyph (rarely needed since
/// roles already abstract the color), but in practice the
/// `Theme::style` dispatcher is the only consumer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Palette {
    Dark,
    Light,
}

#[derive(Clone, Debug)]
pub struct Theme {
    palette: Palette,
}

impl Theme {
    /// Default dark palette — intended for the typical terminal
    /// (white-on-black scheme). Distinct from `Theme::light`.
    pub fn dark() -> Self {
        Self {
            palette: Palette::Dark,
        }
    }

    /// Light palette — intended for users running a light-background
    /// terminal scheme. Hues stay the same; brightness shifts so the
    /// foreground stays readable on white.
    pub fn light() -> Self {
        Self {
            palette: Palette::Light,
        }
    }

    /// Resolve a config string ("dark" / "light", case-insensitive)
    /// to a `Theme`. Unknown values fall back to dark — never errors.
    /// p9-fb-14 spec: "config never errors on a typo, the TUI just
    /// keeps the default theme so the user has a working shell."
    pub fn from_name(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Self::light(),
            _ => Self::dark(),
        }
    }

    /// The underlying palette identity. Mostly a debugging aid.
    pub fn palette(&self) -> Palette {
        self.palette
    }

    /// Resolve a `Role` to a `Style`. Both palettes implement every
    /// role exhaustively (compile error if a variant is added but
    /// the palette body forgets it).
    pub fn style(&self, role: Role) -> Style {
        match self.palette {
            Palette::Dark => dark_style(role),
            Palette::Light => light_style(role),
        }
    }
}

/// `Theme::default() == Theme::dark()` — pinned by
/// `default_palette_is_dark` test. If the default ever flips, both
/// the test and downstream callers (e.g. integration smokes that
/// rely on dark contrast) need a coordinated update.
impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

/// Dark palette — high-contrast on black. The exhaustive match
/// guarantees adding a `Role` variant here forces the same in
/// `light_style`.
fn dark_style(role: Role) -> Style {
    match role {
        Role::BorderActive => Style::default().fg(Color::Cyan),
        Role::BorderInactive => Style::default().fg(Color::DarkGray),
        Role::Title => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        Role::Path => Style::default().fg(Color::DarkGray),
        Role::ModeLexical => Style::default().fg(Color::Yellow),
        Role::ModeVector => Style::default().fg(Color::Magenta),
        Role::ModeHybrid => Style::default().fg(Color::Cyan),
        Role::Selected => Style::default().add_modifier(Modifier::REVERSED),
        Role::Hint => Style::default().add_modifier(Modifier::DIM),
        Role::Heading => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        Role::Warning => Style::default().fg(Color::Yellow),
        Role::Error => Style::default().fg(Color::Red),
        Role::Success => Style::default().fg(Color::Green),
        Role::CitationMarker => Style::default().fg(Color::Cyan),
        Role::Bullet => Style::default().fg(Color::DarkGray),
        Role::Body => Style::default(),
    }
}

/// Light palette — high-contrast on white. Same hues as dark
/// (so user mental-models transfer) but with darker variants where
/// `Color::*` differs in 16-color terminals (e.g., `LightYellow`
/// would wash out on white, so `Yellow` stays).
fn light_style(role: Role) -> Style {
    match role {
        Role::BorderActive => Style::default().fg(Color::Blue),
        Role::BorderInactive => Style::default().fg(Color::Gray),
        Role::Title => Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
        Role::Path => Style::default().fg(Color::Gray),
        Role::ModeLexical => Style::default().fg(Color::Yellow),
        Role::ModeVector => Style::default().fg(Color::Magenta),
        Role::ModeHybrid => Style::default().fg(Color::Blue),
        Role::Selected => Style::default().add_modifier(Modifier::REVERSED),
        Role::Hint => Style::default().add_modifier(Modifier::DIM),
        Role::Heading => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        Role::Warning => Style::default().fg(Color::Yellow),
        Role::Error => Style::default().fg(Color::Red),
        Role::Success => Style::default().fg(Color::Green),
        Role::CitationMarker => Style::default().fg(Color::Blue),
        Role::Bullet => Style::default().fg(Color::Gray),
        Role::Body => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both palettes resolve every `Role` to a `Style` (no panic /
    /// no `unreachable!()` branch). The exhaustive match in
    /// `dark_style` / `light_style` makes this true at compile
    /// time, but we exercise it at runtime so a regression to
    /// `match _ => unreachable!()` would surface in test instead
    /// of in production.
    #[test]
    fn every_role_resolves_in_dark_and_light() {
        let roles = [
            Role::BorderActive,
            Role::BorderInactive,
            Role::Title,
            Role::Path,
            Role::ModeLexical,
            Role::ModeVector,
            Role::ModeHybrid,
            Role::Selected,
            Role::Hint,
            Role::Heading,
            Role::Warning,
            Role::Error,
            Role::Success,
            Role::CitationMarker,
            Role::Bullet,
            Role::Body,
        ];
        for r in roles {
            let _ = Theme::dark().style(r);
            let _ = Theme::light().style(r);
        }
    }

    /// `Theme::from_name` recognizes exactly two palette names; any
    /// other input falls back to dark. Pinned per spec: "config
    /// never errors on a typo".
    #[test]
    fn from_name_recognizes_dark_light_and_falls_back() {
        assert_eq!(Theme::from_name("dark").palette(), Palette::Dark);
        assert_eq!(Theme::from_name("DARK").palette(), Palette::Dark);
        assert_eq!(Theme::from_name(" dark ").palette(), Palette::Dark);
        assert_eq!(Theme::from_name("light").palette(), Palette::Light);
        assert_eq!(Theme::from_name("LIGHT").palette(), Palette::Light);
        assert_eq!(Theme::from_name("solarized").palette(), Palette::Dark);
        assert_eq!(Theme::from_name("").palette(), Palette::Dark);
    }

    /// `Theme::default()` is dark — pinned so the default doesn't
    /// silently flip in a future refactor.
    #[test]
    fn default_palette_is_dark() {
        assert_eq!(Theme::default().palette(), Palette::Dark);
    }

    /// Critical roles emit `Style` with at least one decoration —
    /// catches regressions where someone replaces a styled palette
    /// branch with a bare `Style::default()`. `Body` is excluded
    /// (it intentionally returns the default).
    #[test]
    fn primary_roles_carry_decoration_in_dark() {
        let theme = Theme::dark();
        for r in [
            Role::Title,
            Role::Selected,
            Role::Heading,
            Role::Error,
            Role::Warning,
            Role::Success,
        ] {
            let style = theme.style(r);
            let has_color = style.fg.is_some() || style.bg.is_some();
            let has_modifier = !style.add_modifier.is_empty();
            assert!(
                has_color || has_modifier,
                "role {:?} resolves to bare Style::default() in dark palette",
                r
            );
        }
    }
}
