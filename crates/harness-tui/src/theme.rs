//! The cockpit's colours (docs/COCKPIT-WRAPPER-DESIGN.md §9.2): one palette
//! of named roles, four themes and the person's own overrides. Colour
//! delineates the parts of the screen (sections, the status line, notices,
//! secondary labels) and says states; it never decorates.
//!
//! - `text`: the terminal's own foreground.
//! - `muted`: secondary text that must still be read (code comments, a
//!   greyed item's reason): never dim, never dark grey.
//! - `heading`: bold, no colour.
//! - `section`: pane titles and in-pane section headings, in a hue of their
//!   own, bold.
//! - `status`: the bottom status line ("Ready…", "Last: …", a running
//!   command's progress).
//! - `notice`: informational lines that are neither a state nor the next
//!   step (a file written, the project changed, the chat waiting).
//! - `label`: secondary labels ("(a tool, by its folder)", a program's path,
//!   "c for details").
//! - `accent`: one colour, only for the Next step's label and the menu's
//!   recommended item.
//! - `good`, `bad`, `attention`: the three state colours, only where a state
//!   is shown and always beside a mark or a word.
//! - `selected`: reverse video. `disabled`: a greyed menu item, still
//!   readable. `border`: an unfocused pane's frame.
//! - `backdrop`: every pane behind an open dialog, menu or help, in one
//!   grey, so the dialog holds every bright thing; `panel`: the background
//!   of a dialog's, a menu's and the help's cells (a background, not a
//!   foreground; none in the terminal and mono themes)
//!   (docs/COCKPIT-WRAPPER-DESIGN.md §9.4).
//!
//! The themes: **terminal** (the default: the terminal's own blue, cyan,
//! magenta, green, red and yellow, so the person's terminal theme decides
//! the look),
//! **dark** and **light** (designed 256-colour values, tested for contrast)
//! and **mono** (no colour: bold, underline and reverse carry everything;
//! the default when `NO_COLOR` is set). The file `cockpit.toml`
//! ([`config_path`]) may name the theme and set any role's colour.
//!
//! The view draws through [`style`], which reads the palette of the drawing
//! thread ([`set`]); `view::draw` sets it from `App::palette` each frame.

use ratatui::style::{Color, Modifier, Style};
use std::cell::Cell;
use std::path::{Path, PathBuf};

/// One of the four themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// The terminal's own colours (the default).
    #[default]
    Terminal,
    /// Designed for a dark background.
    Dark,
    /// Designed for a light (white) background.
    Light,
    /// No colour: bold, underline and reverse only.
    Mono,
}

/// A role a piece of the screen plays; its look comes from the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Ordinary text: the terminal's foreground.
    Text,
    /// Secondary text, still readable.
    Muted,
    /// A heading: bold, no colour.
    Heading,
    /// The one accent: the Next step's label, the recommended menu item.
    Accent,
    /// A pane title or an in-pane section heading.
    Section,
    /// The bottom status line, and a running command's progress.
    Status,
    /// An informational line: neither a state nor the next step.
    Notice,
    /// A secondary label beside or under what it describes.
    Label,
    /// A state that is fine (beside `✓`, "passed", …).
    Good,
    /// A state that failed (beside `✗`, "failed", …).
    Bad,
    /// A state that wants a look (beside `⚠`, "needs attention", …).
    Attention,
    /// The selected row: reverse video.
    Selected,
    /// A greyed menu item (its reason beside it, muted).
    Disabled,
    /// An unfocused pane's frame.
    Border,
    /// Every pane behind an open dialog, menu or help: one grey, no bold,
    /// no state colours, no reverse.
    Backdrop,
    /// The background of a dialog's, a menu's and the help's cells.
    Panel,
}

impl Role {
    /// Every role, for the tests, the file and the docs.
    pub const ALL: [Role; 16] = [
        Role::Text,
        Role::Muted,
        Role::Heading,
        Role::Accent,
        Role::Section,
        Role::Status,
        Role::Notice,
        Role::Label,
        Role::Good,
        Role::Bad,
        Role::Attention,
        Role::Selected,
        Role::Disabled,
        Role::Border,
        Role::Backdrop,
        Role::Panel,
    ];

    /// The role's name, as the docs and the file say it.
    pub fn name(self) -> &'static str {
        match self {
            Role::Text => "text",
            Role::Muted => "muted",
            Role::Heading => "heading",
            Role::Accent => "accent",
            Role::Section => "section",
            Role::Status => "status",
            Role::Notice => "notice",
            Role::Label => "label",
            Role::Good => "good",
            Role::Bad => "bad",
            Role::Attention => "attention",
            Role::Selected => "selected",
            Role::Disabled => "disabled",
            Role::Border => "border",
            Role::Backdrop => "backdrop",
            Role::Panel => "panel",
        }
    }

    /// Words are read in this role (4.5:1 contrast); otherwise it carries
    /// frames (3:1), or it is a background (the panel). The backdrop's words
    /// are not read while the box is open: it keeps the page's shape (3:1).
    pub fn is_text(self) -> bool {
        !matches!(self, Role::Border | Role::Panel | Role::Backdrop)
    }

    /// The roles drawn in words on a panel (a dialog, a menu, the help):
    /// each readable on it (the contrast test).
    pub const ON_PANEL: [Role; 10] = [
        Role::Muted,
        Role::Accent,
        Role::Section,
        Role::Notice,
        Role::Label,
        Role::Good,
        Role::Bad,
        Role::Attention,
        Role::Disabled,
        Role::Status,
    ];

    fn index(self) -> usize {
        Role::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// The themes' names, as a refusal lists them.
const THEMES: &str = "terminal, dark, light and mono";

impl Theme {
    /// Every theme.
    pub const ALL: [Theme; 4] = [Theme::Terminal, Theme::Dark, Theme::Light, Theme::Mono];

    /// The theme a `--theme` value, `RUHARNESS_THEME` or the file names.
    pub fn parse(name: &str) -> Option<Theme> {
        match name.trim().to_ascii_lowercase().as_str() {
            "terminal" => Some(Theme::Terminal),
            "dark" => Some(Theme::Dark),
            "light" => Some(Theme::Light),
            "mono" => Some(Theme::Mono),
            _ => None,
        }
    }

    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            Theme::Terminal => "terminal",
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Mono => "mono",
        }
    }

    /// The theme to start with: `--theme`, else `RUHARNESS_THEME`, else the
    /// file's `[theme] name`, else mono when `NO_COLOR` is set (to anything
    /// but empty), else terminal. A bad flag or variable is an error in
    /// words.
    pub fn choose(
        flag: Option<&str>,
        file: Option<Theme>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Theme, String> {
        let named = |from: &str, value: &str| {
            Theme::parse(value).ok_or_else(|| format!("{from} {value:?}: the themes are {THEMES}"))
        };
        if let Some(value) = flag {
            return named("--theme", value);
        }
        if let Some(value) = env("RUHARNESS_THEME").filter(|v| !v.is_empty()) {
            return named("RUHARNESS_THEME", &value);
        }
        if let Some(theme) = file {
            return Ok(theme);
        }
        if env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return Ok(Theme::Mono);
        }
        Ok(Theme::Terminal)
    }

    /// The backgrounds a designed theme's colours were chosen for (the
    /// contrast test's): dark against #1e1e1e and #000000, light against
    /// #ffffff. The terminal theme's colours are the terminal's own, and
    /// cannot be tested.
    pub fn backgrounds(self) -> &'static [[u8; 3]] {
        match self {
            Theme::Dark => &[[0x1e, 0x1e, 0x1e], [0, 0, 0]],
            Theme::Light => &[[0xff, 0xff, 0xff]],
            Theme::Terminal | Theme::Mono => &[],
        }
    }

    /// The 256-colour index a designed theme gives a role, if any.
    pub fn colour(self, role: Role) -> Option<u8> {
        use Role as R;
        match (self, role) {
            (Theme::Terminal | Theme::Mono, _) | (_, R::Text | R::Heading | R::Selected) => None,
            (Theme::Dark, R::Muted) => Some(250),
            (Theme::Dark, R::Accent) => Some(81),
            (Theme::Dark, R::Section) => Some(75),
            (Theme::Dark, R::Status) => Some(250),
            (Theme::Dark, R::Notice) => Some(176),
            (Theme::Dark, R::Label) => Some(248),
            (Theme::Dark, R::Good) => Some(114),
            // 203 was 3.8:1 on the panel (237): 210 (fix pass, item 15).
            (Theme::Dark, R::Bad) => Some(210),
            (Theme::Dark, R::Attention) => Some(214),
            // 245, then 246, were under 4.5:1 on the panel (237): 249.
            (Theme::Dark, R::Disabled) => Some(249),
            (Theme::Dark, R::Border) => Some(244),
            // Darker than every word on the page, so the box stands out
            // (fix pass, item 15).
            (Theme::Dark, R::Backdrop) => Some(243),
            // A visible step from a #1e1e1e page.
            (Theme::Dark, R::Panel) => Some(237),
            (Theme::Light, R::Muted) => Some(240),
            // 31, the first choice, is 4.1:1 on white: too faint.
            (Theme::Light, R::Accent) => Some(23),
            (Theme::Light, R::Section) => Some(25),
            (Theme::Light, R::Status) => Some(240),
            (Theme::Light, R::Notice) => Some(127),
            // Darker than the first choices (242, 28, 160, 130, 243), which
            // were under 4.5:1 on the panel (254): words in a dialog are
            // read there (§9.4).
            // 238: the dialog's command line, apart from the backdrop's 242
            // (fix pass, item 15).
            (Theme::Light, R::Label) => Some(238),
            (Theme::Light, R::Good) => Some(22),
            (Theme::Light, R::Bad) => Some(124),
            (Theme::Light, R::Attention) => Some(94),
            (Theme::Light, R::Disabled) => Some(240),
            (Theme::Light, R::Border) => Some(244),
            (Theme::Light, R::Backdrop) => Some(242),
            (Theme::Light, R::Panel) => Some(254),
        }
    }

    /// A role's foreground in this theme: the terminal theme's are the
    /// terminal's own named colours; muted, status, label, disabled and
    /// border stay the terminal's foreground there (its grey can be
    /// unreadable).
    pub fn fg(self, role: Role) -> Option<Color> {
        match self {
            Theme::Terminal => match role {
                Role::Good => Some(Color::Green),
                Role::Bad => Some(Color::Red),
                Role::Attention => Some(Color::Yellow),
                Role::Accent => Some(Color::Cyan),
                // Bright blue: the plain blue is navy on a black terminal
                // (the person's screen, 2026-10-20).
                Role::Section => Some(Color::LightBlue),
                Role::Notice => Some(Color::Magenta),
                // The terminal's own "bright black" (§9.4).
                Role::Backdrop => Some(Color::DarkGray),
                _ => None,
            },
            // The panel is a background (see [`Theme::bg`]).
            _ if role == Role::Panel => None,
            _ => self.colour(role).map(Color::Indexed),
        }
    }

    /// A role's background in this theme: only the panel has one, in the
    /// designed themes.
    pub fn bg(self, role: Role) -> Option<Color> {
        match (self, role) {
            (Theme::Dark | Theme::Light, Role::Panel) => self.colour(role).map(Color::Indexed),
            _ => None,
        }
    }

    /// A role's modifiers in this theme.
    pub fn modifiers(self, role: Role) -> Modifier {
        match (self, role) {
            (_, Role::Heading | Role::Section) => Modifier::BOLD,
            (_, Role::Selected) => Modifier::REVERSED,
            (Theme::Mono, Role::Accent) => Modifier::BOLD | Modifier::UNDERLINED,
            (_, Role::Accent) => Modifier::BOLD,
            // Mono: the marks and words carry the state; a failure and a
            // state that wants a look are also bold.
            (Theme::Mono, Role::Bad | Role::Attention) => Modifier::BOLD,
            // Mono has no grey: the backdrop is dim there (§9.4).
            (Theme::Mono, Role::Backdrop) => Modifier::DIM,
            _ => Modifier::empty(),
        }
    }

    /// A role's whole style in this theme, with no override.
    pub fn style(self, role: Role) -> Style {
        Palette::of(self).style(role)
    }
}

/// A theme and the person's per-role colours (`[colors]` in the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Palette {
    /// The theme under the overrides.
    pub theme: Theme,
    overrides: [Option<Color>; Role::ALL.len()],
}

impl Palette {
    /// The theme as designed.
    pub fn of(theme: Theme) -> Palette {
        Palette {
            theme,
            overrides: [None; Role::ALL.len()],
        }
    }

    /// `role` drawn in `colour` instead.
    pub fn with(mut self, role: Role, colour: Color) -> Palette {
        self.overrides[role.index()] = Some(colour);
        self
    }

    /// A role's foreground: the person's, else the theme's (the panel has
    /// none: its colour is a background).
    pub fn fg(&self, role: Role) -> Option<Color> {
        if role == Role::Panel {
            return None;
        }
        self.overrides[role.index()].or_else(|| self.theme.fg(role))
    }

    /// A role's background: the panel's, the person's or the theme's.
    pub fn bg(&self, role: Role) -> Option<Color> {
        if role != Role::Panel {
            return None;
        }
        self.overrides[role.index()].or_else(|| self.theme.bg(role))
    }

    /// A role's whole style.
    pub fn style(&self, role: Role) -> Style {
        let mut s = Style::default().add_modifier(self.theme.modifiers(role));
        if let Some(c) = self.fg(role) {
            s = s.fg(c);
        }
        if let Some(c) = self.bg(role) {
            s = s.bg(c);
        }
        s
    }
}

/// What `cockpit.toml` says: its theme, its colours, and what in it was
/// refused (each in one sentence; the default is used instead).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileSettings {
    /// `[theme] name`.
    pub theme: Option<Theme>,
    /// `[colors]`: role → colour.
    pub colours: Vec<(Role, Color)>,
    /// `files_width = N` (a top-level key): the Files / Project pane's
    /// width, fixed (docs/COCKPIT-WRAPPER-DESIGN.md §9.4); the view keeps it
    /// within 24 and half the screen.
    pub files_width: Option<u16>,
    /// What was refused, in words.
    pub refused: Vec<String>,
}

impl FileSettings {
    /// `palette` with the file's colours over it.
    pub fn apply(&self, theme: Theme) -> Palette {
        self.colours
            .iter()
            .fold(Palette::of(theme), |p, (role, c)| p.with(*role, *c))
    }
}

/// Where the cockpit's settings file is: `$RUHARNESS_COCKPIT_CONFIG`, else
/// `~/Library/Application Support/ruharness/cockpit.toml` on macOS,
/// `$XDG_CONFIG_HOME/ruharness/cockpit.toml` (default `~/.config/…`)
/// elsewhere.
pub fn config_path(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(p) = env("RUHARNESS_COCKPIT_CONFIG").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let home = env("HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home?.join("Library/Application Support")
    } else {
        match env("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(x) => PathBuf::from(x),
            None => home?.join(".config"),
        }
    };
    Some(base.join("ruharness/cockpit.toml"))
}

/// Reads the settings file at `path`; a missing file is no settings.
pub fn read_settings(path: &Path) -> FileSettings {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_settings(&text, &path.display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileSettings::default(),
        Err(e) => FileSettings {
            refused: vec![format!(
                "{}: cannot be read ({e}); the default colours are used",
                path.display()
            )],
            ..FileSettings::default()
        },
    }
}

/// The settings in `text` (the file `name`): `[theme] name = "…"` and
/// `[colors] <role> = "<colour>"`. Anything else is refused by name.
pub fn parse_settings(text: &str, name: &str) -> FileSettings {
    let mut out = FileSettings::default();
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            let why = e.to_string();
            let first = why.lines().next().unwrap_or_default().to_string();
            out.refused.push(format!(
                "{name} is not valid TOML ({first}); the default colours are used"
            ));
            return out;
        }
    };
    let roles = Role::ALL.map(Role::name).join(", ");
    for (key, value) in &table {
        match (key.as_str(), value) {
            ("theme", toml::Value::Table(t)) => {
                for (k, v) in t {
                    match (k.as_str(), v.as_str().and_then(Theme::parse)) {
                        ("name", Some(theme)) => out.theme = Some(theme),
                        ("name", None) => out.refused.push(format!(
                            "{name}: [theme] name {v} is not a theme (the themes are {THEMES}); \
                             the default is used"
                        )),
                        _ => out.refused.push(format!(
                            "{name}: [theme] {k} is not a setting (only name is); it is ignored"
                        )),
                    }
                }
            }
            ("colors", toml::Value::Table(t)) => {
                for (k, v) in t {
                    let Some(role) = Role::ALL.into_iter().find(|r| r.name() == k) else {
                        out.refused.push(format!(
                            "{name}: [colors] {k} is not a role (the roles are {roles}); \
                             it is ignored"
                        ));
                        continue;
                    };
                    let colour = match v {
                        toml::Value::String(s) => parse_colour(s),
                        toml::Value::Integer(i) => u8::try_from(*i).ok().map(Color::Indexed),
                        _ => None,
                    };
                    match colour {
                        Some(c) => out.colours.push((role, c)),
                        None => out.refused.push(format!(
                            "{name}: [colors] {k} = {v} is not a colour (a name such as red or \
                             bright-blue, a number 0–255, or #rrggbb); the theme's {k} is used"
                        )),
                    }
                }
            }
            ("files_width", v) => match v.as_integer().and_then(|i| u16::try_from(i).ok()) {
                Some(n) if n > 0 => out.files_width = Some(n),
                _ => out.refused.push(format!(
                    "{name}: files_width = {v} is not a number of columns; the Files pane's \
                     width follows its rows"
                )),
            },
            _ => out.refused.push(format!(
                "{name}: [{key}] is not a section (only [theme], [colors] and files_width are); \
                 it is ignored"
            )),
        }
    }
    out
}

/// A colour as the file writes it: a name (`red`, `bright-blue`,
/// `default`), a 256-colour index (`"214"`), or `#rrggbb`.
pub fn parse_colour(text: &str) -> Option<Color> {
    let t = text.trim().to_ascii_lowercase();
    if let Some(hex) = t.strip_prefix('#') {
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?));
    }
    if let Ok(i) = t.parse::<u8>() {
        return Some(Color::Indexed(i));
    }
    Some(match t.replace('_', "-").as_str() {
        "default" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::Gray,
        "bright-black" | "grey" | "gray" => Color::DarkGray,
        "bright-red" => Color::LightRed,
        "bright-green" => Color::LightGreen,
        "bright-yellow" => Color::LightYellow,
        "bright-blue" => Color::LightBlue,
        "bright-magenta" => Color::LightMagenta,
        "bright-cyan" => Color::LightCyan,
        "bright-white" => Color::White,
        _ => return None,
    })
}

thread_local! {
    static CURRENT: Cell<Palette> = Cell::new(Palette::of(Theme::Terminal));
}

/// Draw this thread's frames in `palette`.
pub fn set(palette: Palette) {
    CURRENT.with(|c| c.set(palette));
}

/// The palette this thread draws in.
pub fn current() -> Palette {
    CURRENT.with(Cell::get)
}

/// A role's style in the current palette.
pub fn style(role: Role) -> Style {
    current().style(role)
}

/// The sRGB colour of a 256-colour index (16–255: the cube and the grey
/// ramp; 0–15 are the terminal's to choose and have none).
pub fn rgb(index: u8) -> Option<[u8; 3]> {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => None,
        16..=231 => {
            let i = index - 16;
            Some([
                LEVELS[usize::from(i / 36)],
                LEVELS[usize::from(i / 6 % 6)],
                LEVELS[usize::from(i % 6)],
            ])
        }
        _ => {
            let g = 8 + 10 * (index - 232);
            Some([g, g, g])
        }
    }
}

/// WCAG 2 contrast ratio of two sRGB colours.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    fn luminance([r, g, b]: [u8; 3]) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn the_flag_then_the_variable_then_the_file_then_no_color() {
        let choose = |flag, file, vars| Theme::choose(flag, file, env(vars));
        assert_eq!(choose(None, None, &[]), Ok(Theme::Terminal));
        assert_eq!(choose(Some("light"), None, &[]), Ok(Theme::Light));
        assert_eq!(
            choose(
                Some("mono"),
                Some(Theme::Dark),
                &[("RUHARNESS_THEME", "light")]
            ),
            Ok(Theme::Mono)
        );
        assert_eq!(
            choose(
                None,
                Some(Theme::Dark),
                &[("RUHARNESS_THEME", "Light"), ("NO_COLOR", "1")]
            ),
            Ok(Theme::Light)
        );
        assert_eq!(
            choose(None, Some(Theme::Dark), &[("NO_COLOR", "1")]),
            Ok(Theme::Dark)
        );
        assert_eq!(choose(None, None, &[("NO_COLOR", "1")]), Ok(Theme::Mono));
        assert_eq!(choose(None, None, &[("NO_COLOR", "")]), Ok(Theme::Terminal));
        // Nothing is guessed from the terminal's variables.
        assert_eq!(
            choose(None, None, &[("COLORFGBG", "0;15")]),
            Ok(Theme::Terminal)
        );
        let bad = choose(Some("solarized"), None, &[]).unwrap_err();
        assert!(bad.contains("terminal, dark, light and mono"), "{bad}");
        assert!(choose(None, None, &[("RUHARNESS_THEME", "x")]).is_err());
    }

    #[test]
    fn the_file_lives_where_the_platform_keeps_settings() {
        let p = config_path(env(&[
            ("RUHARNESS_COCKPIT_CONFIG", "/x/c.toml"),
            ("HOME", "/h"),
        ]));
        assert_eq!(p, Some(PathBuf::from("/x/c.toml")));
        let p = config_path(env(&[("HOME", "/h")])).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(
                p,
                Path::new("/h/Library/Application Support/ruharness/cockpit.toml")
            );
        } else {
            assert_eq!(p, Path::new("/h/.config/ruharness/cockpit.toml"));
        }
        assert_eq!(config_path(env(&[])), None);
    }

    #[test]
    fn the_file_sets_the_theme_and_each_role_and_refuses_by_name() {
        let s = parse_settings(
            "[theme]\nname = \"dark\"\n[colors]\nattention = \"bright-yellow\"\n\
             muted = 250\naccent = \"#5fd7ff\"\ngood = \"34\"\n",
            "cockpit.toml",
        );
        assert_eq!(s.theme, Some(Theme::Dark));
        assert!(s.refused.is_empty(), "{:?}", s.refused);
        let p = s.apply(Theme::Terminal);
        assert_eq!(p.fg(Role::Attention), Some(Color::LightYellow));
        assert_eq!(p.fg(Role::Muted), Some(Color::Indexed(250)));
        assert_eq!(p.fg(Role::Accent), Some(Color::Rgb(0x5f, 0xd7, 0xff)));
        assert_eq!(p.fg(Role::Good), Some(Color::Indexed(34)));
        assert_eq!(
            p.fg(Role::Bad),
            Some(Color::Red),
            "untouched roles keep the theme's"
        );
        assert!(p.style(Role::Accent).add_modifier.contains(Modifier::BOLD));

        let s = parse_settings(
            "[theme]\nname = \"pink\"\n[colors]\ngoood = \"red\"\nbad = \"crimson\"\n\
             good = 300\n[keys]\nx = 1\n",
            "cockpit.toml",
        );
        assert_eq!(s.theme, None);
        assert!(s.colours.is_empty());
        // files_width: a number of columns, at the top of the file.
        let s = parse_settings("files_width = 40\n[theme]\nname = \"dark\"\n", "c.toml");
        assert_eq!((s.files_width, s.refused.len()), (Some(40), 0));
        let s = parse_settings("files_width = \"wide\"\n", "c.toml");
        assert_eq!(s.files_width, None);
        assert!(s.refused[0].contains("files_width = \"wide\" is not a number of columns"));
        let s = parse_settings(
            "[theme]\nname = \"pink\"\n[colors]\ngoood = \"red\"\nbad = \"crimson\"\n\
             good = 300\n[keys]\nx = 1\n",
            "cockpit.toml",
        );
        let all = s.refused.join("\n");
        for needle in [
            "[theme] name \"pink\" is not a theme",
            "[colors] goood is not a role",
            "[colors] bad = \"crimson\" is not a colour",
            "[colors] good = 300 is not a colour",
            "[keys] is not a section",
        ] {
            assert!(all.contains(needle), "{needle} in\n{all}");
        }
        assert!(s.refused.iter().all(|r| r.lines().count() == 1));
        let s = parse_settings("[theme\n", "c.toml");
        assert!(
            s.refused[0].starts_with("c.toml is not valid TOML"),
            "{:?}",
            s.refused
        );
        assert_eq!(
            read_settings(Path::new("/nonexistent/ruharness/cockpit.toml")),
            FileSettings::default()
        );
    }

    /// The example file in docs/topics/cockpit.md is one the cockpit takes
    /// whole.
    #[test]
    fn the_documented_example_is_accepted() {
        let doc = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/topics/cockpit.md"),
        )
        .unwrap();
        let example = doc
            .split("```toml\n")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("a toml example in docs/topics/cockpit.md");
        let s = parse_settings(example, "the example");
        assert!(s.refused.is_empty(), "{:?}", s.refused);
        assert_eq!(s.theme, Some(Theme::Terminal));
        assert_eq!(s.colours, [(Role::Attention, Color::Rgb(0xff, 0xaf, 0))]);
        // Every role is in it, by name.
        for role in Role::ALL {
            assert!(
                example.lines().any(|l| {
                    let mut words = l.trim_start_matches("# ").split_whitespace();
                    words.next() == Some(role.name()) && words.next() == Some("=")
                }),
                "{} is not in the example",
                role.name()
            );
        }
        // Every commented line names a role and a colour the file takes.
        for line in example
            .lines()
            .filter(|l| l.starts_with("# ") && l.contains(" = "))
        {
            let uncommented = format!("[colors]\n{}", &line[2..]);
            let s = parse_settings(&uncommented, "line");
            assert!(
                s.refused.is_empty() && s.colours.len() == 1,
                "{line}: {:?}",
                s.refused
            );
        }
    }

    #[test]
    fn colours_by_name_number_and_hex() {
        assert_eq!(parse_colour("red"), Some(Color::Red));
        assert_eq!(parse_colour("Bright-Blue"), Some(Color::LightBlue));
        assert_eq!(parse_colour("bright_white"), Some(Color::White));
        assert_eq!(parse_colour("214"), Some(Color::Indexed(214)));
        assert_eq!(parse_colour("#FFaf00"), Some(Color::Rgb(255, 175, 0)));
        assert_eq!(parse_colour("default"), Some(Color::Reset));
        for bad in ["", "256", "#fff", "#gggggg", "crimson"] {
            assert_eq!(parse_colour(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_cube_and_the_greys_are_xterms() {
        assert_eq!(rgb(214), Some([255, 175, 0]));
        assert_eq!(rgb(114), Some([135, 215, 135]));
        assert_eq!(rgb(203), Some([255, 95, 95]));
        assert_eq!(rgb(81), Some([95, 215, 255]));
        assert_eq!(rgb(75), Some([95, 175, 255]));
        assert_eq!(rgb(176), Some([215, 135, 215]));
        assert_eq!(rgb(23), Some([0, 95, 95]));
        assert_eq!(rgb(250), Some([188, 188, 188]));
        assert_eq!(rgb(232), Some([8, 8, 8]));
        assert_eq!(rgb(255), Some([238, 238, 238]));
        assert_eq!(rgb(9), None);
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
    }

    /// The designed themes: every role's colour against the background its
    /// theme assumes, at least 4.5:1 for what is read and 3:1 for frames;
    /// fixed 256-colour values, never one of the terminal's named sixteen.
    #[test]
    fn every_designed_role_is_readable_on_its_background() {
        for theme in [Theme::Dark, Theme::Light] {
            for role in Role::ALL {
                let Some(i) = theme.colour(role).filter(|_| role != Role::Panel) else {
                    continue;
                };
                assert_eq!(theme.fg(role), Some(Color::Indexed(i)));
                let fg = rgb(i).expect("a fixed colour, never a named one");
                let floor = if role.is_text() { 4.5 } else { 3.0 };
                for &bg in theme.backgrounds() {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= floor,
                        "{} {} (colour {i}) on {bg:?}: {ratio:.2}:1 < {floor}:1",
                        theme.name(),
                        role.name()
                    );
                }
            }
        }
    }

    /// The panel (§9.4): one shade off the page in the designed themes, and
    /// every role a dialog, a menu or the help draws in words is readable on
    /// it (4.5:1); the backdrop is the label grey, readable on the page.
    #[test]
    fn words_on_the_panel_and_the_backdrop_are_readable() {
        assert_eq!(Theme::Dark.bg(Role::Panel), Some(Color::Indexed(237)));
        assert_eq!(Theme::Light.bg(Role::Panel), Some(Color::Indexed(254)));
        assert_eq!(Theme::Terminal.bg(Role::Panel), None);
        assert_eq!(Theme::Mono.bg(Role::Panel), None);
        assert_eq!(Theme::Dark.colour(Role::Backdrop), Some(243));
        assert_eq!(Theme::Light.colour(Role::Backdrop), Some(242));
        assert_eq!(Theme::Terminal.fg(Role::Backdrop), Some(Color::DarkGray));
        assert!(Theme::Mono
            .style(Role::Backdrop)
            .add_modifier
            .contains(Modifier::DIM));
        for theme in [Theme::Dark, Theme::Light] {
            let panel = rgb(theme.colour(Role::Panel).unwrap()).unwrap();
            for role in Role::ON_PANEL {
                let fg = rgb(theme.colour(role).unwrap()).unwrap();
                let ratio = contrast(fg, panel);
                assert!(
                    ratio >= 4.5,
                    "{} {} on the panel: {ratio:.2}:1 < 4.5:1",
                    theme.name(),
                    role.name()
                );
            }
        }
        // The person's colour for the panel is a background.
        let p = Palette::of(Theme::Terminal).with(Role::Panel, Color::Indexed(236));
        assert_eq!(p.style(Role::Panel).bg, Some(Color::Indexed(236)));
        assert_eq!(p.fg(Role::Panel), None);
    }

    /// Every theme: nothing dims (but mono's backdrop, which has no grey),
    /// blinks or sets a background (but the panel); text is never
    /// recoloured; no dark grey (but the terminal theme's backdrop, its own
    /// "bright black"); the terminal theme colours only the three states,
    /// the accent, sections, notices and the backdrop; mono has no colour.
    #[test]
    fn every_theme_keeps_the_rules() {
        for theme in Theme::ALL {
            for role in Role::ALL {
                let style = theme.style(role);
                let banned = if (theme, role) == (Theme::Mono, Role::Backdrop) {
                    Modifier::SLOW_BLINK | Modifier::RAPID_BLINK
                } else {
                    Modifier::DIM | Modifier::SLOW_BLINK | Modifier::RAPID_BLINK
                };
                let what = format!("{} {}", theme.name(), role.name());
                assert!(
                    !style.add_modifier.intersects(banned),
                    "{what}: dims or blinks"
                );
                if role != Role::Panel {
                    assert_eq!(style.bg, None, "{what}");
                }
                if (theme, role) != (Theme::Terminal, Role::Backdrop) {
                    assert_ne!(style.fg, Some(Color::DarkGray), "{what}");
                }
                let coloured = matches!(
                    role,
                    Role::Good
                        | Role::Bad
                        | Role::Attention
                        | Role::Accent
                        | Role::Section
                        | Role::Notice
                        | Role::Backdrop
                );
                match theme {
                    Theme::Mono => assert_eq!(style.fg, None, "{what}"),
                    Theme::Terminal => assert_eq!(style.fg.is_some(), coloured, "{what}"),
                    _ => {}
                }
            }
            assert_eq!(theme.fg(Role::Text), None, "text is never recoloured");
            assert_eq!(
                theme.style(Role::Heading),
                Style::default().add_modifier(Modifier::BOLD)
            );
            assert!(theme
                .style(Role::Section)
                .add_modifier
                .contains(Modifier::BOLD));
        }
    }

    /// The roles that delineate the screen each have a hue of their own in
    /// every coloured theme, so a section, a notice, the accent and the
    /// three states never look alike.
    #[test]
    fn the_delineating_roles_differ_in_every_coloured_theme() {
        let roles = [
            Role::Section,
            Role::Notice,
            Role::Accent,
            Role::Good,
            Role::Bad,
            Role::Attention,
        ];
        for theme in [Theme::Terminal, Theme::Dark, Theme::Light] {
            let colours: Vec<_> = roles.iter().map(|r| theme.fg(*r)).collect();
            for (i, c) in colours.iter().enumerate() {
                assert!(c.is_some(), "{} {}", theme.name(), roles[i].name());
                assert!(
                    !colours[..i].contains(c),
                    "{} {}: shares its colour",
                    theme.name(),
                    roles[i].name()
                );
            }
        }
        assert_eq!(Theme::Dark.colour(Role::Status), Some(250));
        assert_eq!(Theme::Dark.colour(Role::Label), Some(248));
        assert_eq!(Theme::Light.colour(Role::Status), Some(240));
        assert_eq!(Theme::Light.colour(Role::Label), Some(238));
    }
}
