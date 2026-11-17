//! Styled text shared by the CLI and the terminal UI: MTA bullets, the route
//! strip map, and ANSI escape output.
//!
//! Bullets keep the map's shapes: a circle for local service and a diamond
//! for express.

use crate::network::Network;
use crate::route::{Route, TRANSFER_MIN};
use crate::service::{Kind, Rgb, Service};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    Ansi256,
    None,
}

impl ColorMode {
    /// Pick a color mode from the environment.
    pub fn detect() -> ColorMode {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        if !env("NO_COLOR").is_empty() {
            return ColorMode::None;
        }
        if cfg!(windows) {
            return ColorMode::TrueColor;
        }
        let ct = env("COLORTERM").to_ascii_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            return ColorMode::TrueColor;
        }
        if env("TERM_PROGRAM") == "Apple_Terminal" {
            return ColorMode::Ansi256;
        }
        if env("TERM") == "dumb" {
            return ColorMode::None;
        }
        ColorMode::Ansi256
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    Unicode,
    Nerd,
    Ascii,
}

impl Glyphs {
    pub fn parse(s: &str) -> Option<Glyphs> {
        match s.to_ascii_lowercase().as_str() {
            "unicode" | "default" => Some(Glyphs::Unicode),
            "nerd" | "powerline" => Some(Glyphs::Nerd),
            "ascii" | "plain" => Some(Glyphs::Ascii),
            _ => None,
        }
    }
    pub fn next(self) -> Glyphs {
        match self {
            Glyphs::Unicode => Glyphs::Nerd,
            Glyphs::Nerd => Glyphs::Ascii,
            Glyphs::Ascii => Glyphs::Unicode,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Glyphs::Unicode => "unicode",
            Glyphs::Nerd => "nerd",
            Glyphs::Ascii => "ascii",
        }
    }
    fn caps(self, kind: Kind) -> (char, char) {
        match (self, kind) {
            (Glyphs::Unicode, Kind::Local) => ('◖', '◗'),
            (Glyphs::Unicode, Kind::Express) => ('◀', '▶'),
            (Glyphs::Nerd, Kind::Local) => ('\u{e0b6}', '\u{e0b4}'),
            (Glyphs::Nerd, Kind::Express) => ('\u{e0b2}', '\u{e0b0}'),
            (Glyphs::Ascii, Kind::Local) => ('(', ')'),
            (Glyphs::Ascii, Kind::Express) => ('<', '>'),
        }
    }
    pub fn rail(self) -> char {
        if self == Glyphs::Ascii {
            '|'
        } else {
            '┃'
        }
    }
}

/// Dark mode shit.
pub mod theme {
    use crate::service::Rgb;
    pub const BG: Rgb = Rgb(0x0B, 0x0D, 0x10);
    pub const PANEL: Rgb = Rgb(0x14, 0x17, 0x1C);
    pub const RULE: Rgb = Rgb(0x2A, 0x30, 0x38);
    pub const TEXT: Rgb = Rgb(0xD8, 0xDD, 0xE3);
    pub const BRIGHT: Rgb = Rgb(0xFF, 0xFF, 0xFF);
    pub const DIM: Rgb = Rgb(0x6E, 0x77, 0x82);
    pub const FAINT: Rgb = Rgb(0x3C, 0x43, 0x4C);
    pub const AMBER: Rgb = Rgb(0xFF, 0xB0, 0x00);
    pub const OK: Rgb = Rgb(0x5F, 0xD7, 0x87);
    pub const WARN: Rgb = Rgb(0xFF, 0x6B, 0x5B);
    pub const OFF_CAP: Rgb = Rgb(0x30, 0x35, 0x3C);
    pub const OFF_TEXT: Rgb = Rgb(0x7A, 0x82, 0x8C);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub underline: bool,
}

impl Style {
    pub const PLAIN: Style = Style {
        fg: None,
        bg: None,
        bold: false,
        underline: false,
    };
    pub fn fg(c: Rgb) -> Style {
        Style {
            fg: Some(c),
            ..Style::PLAIN
        }
    }
    pub fn bold(mut self) -> Style {
        self.bold = true;
        self
    }
    pub fn on(mut self, c: Rgb) -> Style {
        self.bg = Some(c);
        self
    }
    pub fn underline(mut self) -> Style {
        self.underline = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

pub type Line = Vec<Span>;

pub fn span(text: impl Into<String>, style: Style) -> Span {
    Span {
        text: text.into(),
        style,
    }
}

pub fn line_width(l: &Line) -> usize {
    l.iter().map(|s| s.text.chars().count()).sum()
}

/// A train bullet as three cells: left cap, letter, right cap. When `running`
/// is false the bullet is greyed out (the service doesn't stop here right now).
pub fn bullet(s: Service, glyphs: Glyphs, running: bool) -> Vec<Span> {
    let (l, r) = glyphs.caps(s.kind);
    let (cap, txt) = if running {
        (s.color(), s.text_color())
    } else {
        (theme::OFF_CAP, theme::OFF_TEXT)
    };
    vec![
        span(l.to_string(), Style::fg(cap)),
        span(s.label(), Style::fg(txt).on(cap).bold()),
        span(r.to_string(), Style::fg(cap)),
    ]
}

/// Several bullets separated by single spaces.
pub fn bullets(items: &[(Service, bool)], glyphs: Glyphs) -> Vec<Span> {
    let mut out = Vec::new();

    for (i, (s, on)) in items.iter().enumerate() {
        if i > 0 {
            out.push(span(" ", Style::PLAIN));
        }
        out.extend(bullet(*s, glyphs, *on));
    }
    out
}

/// Compass heading between two complexes, as an arrow and a label.
pub fn compass(net: &Network, from: usize, to: usize, glyphs: Glyphs) -> String {
    let (a, b) = (net.complex(from), net.complex(to));
    let dy = b.lat - a.lat;
    let dx = (b.lon - a.lon) * a.lat.to_radians().cos();
    let deg = dx.atan2(dy).to_degrees().rem_euclid(360.0);
    let idx = (((deg + 22.5) / 45.0) as usize) % 8;
    let names = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let arrows = ['↑', '↗', '→', '↘', '↓', '↙', '←', '↖'];

    if glyphs == Glyphs::Ascii {
        names[idx].to_string()
    } else {
        format!("{} {}", arrows[idx], names[idx])
    }
}

pub fn fmt_min(m: f64) -> String {
    let m = m.round() as i64;
    if m >= 60 {
        format!("{}h{:02}", m / 60, m % 60)
    } else {
        format!("{} min", m.max(1))
    }
}

/// One-line summary: "2 rides · 1 transfer · 12.4 km · ≈41 min".
pub fn summary(route: &Route) -> String {
    let rides = route.legs.len();
    let t = route.transfers();
    format!(
        "{} ride{} · {} transfer{} · {:.1} km · ≈{}",
        rides,
        if rides == 1 { "" } else { "s" },
        t,
        if t == 1 { "" } else { "s" },
        route.km(),
        fmt_min(route.minutes())
    )
}

/// The route as a vertical strip map.
///
/// ```text
///  16:05  ●  Greenpoint Ave
///         ┃  ◖G◗           ↑ N · 2.0 km · ≈6 min
///  16:11  ○  Court Sq                 transfer ≈5 min
///         ┃  ◖7◗ ◀7▶       → E · 9.6 km · ≈27 min
///  16:43  ◉  Flushing - Main St
/// ```
pub fn route_lines(net: &Network, route: &Route, glyphs: Glyphs) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let times = route.timeline();
    let rail = glyphs.rail();
    let (start_dot, xfer_dot, end_dot) = if glyphs == Glyphs::Ascii {
        ('*', 'o', '@')
    } else {
        ('●', '○', '◉')
    };
    let dim = Style::fg(theme::DIM);
    let text = Style::fg(theme::TEXT);
    let name = Style::fg(theme::BRIGHT).bold();

    for (i, leg) in route.legs.iter().enumerate() {
        let color = leg.rides.first().map(|s| s.color()).unwrap_or(theme::TEXT);
        let (dep, arr) = times[i];
        let mut l: Line = vec![span(format!(" {}  ", dep.clock()), Style::fg(theme::AMBER))];

        if i == 0 {
            l.push(span(start_dot.to_string(), Style::fg(color).bold()));
        } else {
            l.push(span(xfer_dot.to_string(), Style::fg(theme::BRIGHT).bold()));
        }
        l.push(span("  ", Style::PLAIN));
        l.push(span(net.complex(leg.from).label.clone(), name));

        if i > 0 {
            l.push(span(format!("   transfer ≈{}", fmt_min(TRANSFER_MIN)), dim));
        }
        out.push(l);

        // The ride.
        let mut r: Line = vec![
            span("        ", Style::PLAIN),
            span(rail.to_string(), Style::fg(color).bold()),
        ];
        r.push(span("  ", Style::PLAIN));

        let on: Vec<(Service, bool)> = leg.rides.iter().map(|s| (*s, true)).collect();

        r.extend(bullets(&on, glyphs));

        let w = line_width(&r);
        let pad = 26usize.saturating_sub(w).max(2);

        r.push(span(" ".repeat(pad), Style::PLAIN));
        r.push(span(
            format!(
                "{} · {:.1} km · ≈{}",
                compass(net, leg.from, leg.to, glyphs),
                leg.km,
                fmt_min(leg.minutes())
            ),
            text,
        ));
        out.push(r);
        out.push(vec![
            span("        ", Style::PLAIN),
            span(rail.to_string(), Style::fg(color).bold()),
        ]);

        if i + 1 == route.legs.len() {
            out.push(vec![
                span(format!(" {}  ", arr.clock()), Style::fg(theme::AMBER)),
                span(end_dot.to_string(), Style::fg(color).bold()),
                span("  ", Style::PLAIN),
                span(net.complex(leg.to).label.clone(), name),
            ]);
        }
    }
    out
}

/// Nearest xterm-256 color for an RGB value.
pub fn rgb_to_256(c: Rgb) -> u8 {
    let Rgb(r, g, b) = c;
    let to6 = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    let (r6, g6, b6) = (to6(r), to6(g), to6(b));
    let lvl = |i: u8| if i == 0 { 0 } else { 55 + 40 * i as i32 };
    let cube = 16 + 36 * r6 + 6 * g6 + b6;
    let cd =
        (r as i32 - lvl(r6)).pow(2) + (g as i32 - lvl(g6)).pow(2) + (b as i32 - lvl(b6)).pow(2);
    let avg = (r as i32 + g as i32 + b as i32) / 3;
    let gi = if avg > 238 {
        23
    } else {
        ((avg - 3).max(0) / 10) as u8
    };
    let gv = 8 + 10 * gi as i32;
    let gd = (r as i32 - gv).pow(2) + (g as i32 - gv).pow(2) + (b as i32 - gv).pow(2);

    if gd < cd {
        232 + gi
    } else {
        cube
    }
}

/// SGR escape sequence that sets this style (resetting first).
pub fn sgr(style: Style, mode: ColorMode) -> String {
    if mode == ColorMode::None {
        return String::new();
    }
    let mut s = String::from("\x1b[0");

    if style.bold {
        s.push_str(";1");
    }
    if style.underline {
        s.push_str(";4");
    }
    let mut col = |base: u8, c: Option<Rgb>| {
        if let Some(c) = c {
            match mode {
                ColorMode::TrueColor => s.push_str(&format!(";{};2;{};{};{}", base, c.0, c.1, c.2)),
                ColorMode::Ansi256 => s.push_str(&format!(";{};5;{}", base, rgb_to_256(c))),
                ColorMode::None => {}
            }
        }
    };
    col(38, style.fg);
    col(48, style.bg);
    s.push('m');
    s
}

/// Render lines to a string with escape codes (or plain text for `None`).
pub fn to_ansi(lines: &[Line], mode: ColorMode) -> String {
    let mut out = String::new();

    for l in lines {
        for sp in l {
            out.push_str(&sgr(sp.style, mode));
            out.push_str(&sp.text);
        }
        if mode != ColorMode::None {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Line as L;

    #[test]
    fn bullets_have_shapes() {
        let six = Service::local(L::L6);
        let six_x = Service::new(L::L6, Kind::Express);
        let txt = |v: Vec<Span>| v.into_iter().map(|s| s.text).collect::<String>();

        assert_eq!(txt(bullet(six, Glyphs::Unicode, true)), "◖6◗");
        assert_eq!(txt(bullet(six_x, Glyphs::Unicode, true)), "◀6▶");
        assert_eq!(txt(bullet(six, Glyphs::Ascii, true)), "(6)");
        assert_eq!(txt(bullet(six_x, Glyphs::Ascii, true)), "<6>");
    }

    #[test]
    fn yellow_lines_use_black_letters() {
        let n = bullet(Service::local(L::N), Glyphs::Unicode, true);

        assert_eq!(n[1].style.fg, Some(Rgb(0, 0, 0)));
        assert_eq!(n[1].style.bg, Some(Rgb(0xFC, 0xCC, 0x0A)));
    }

    #[test]
    fn color_256_approximation() {
        assert_eq!(rgb_to_256(Rgb(0, 0, 0)), 16);
        assert_eq!(rgb_to_256(Rgb(255, 0, 0)), 196);
        assert_eq!(rgb_to_256(Rgb(0x80, 0x81, 0x83)), 244);
    }
}
