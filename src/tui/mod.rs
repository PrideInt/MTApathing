//! Interactive terminal UI.

mod screen;
mod term;

use std::io::{self, Write};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use mta_graphing::network::{Graph, Network};
use mta_graphing::render::{self, span, theme, ColorMode, Glyphs, Line, Style};
use mta_graphing::route::{find_route, Mode, Route, RouteError};
use mta_graphing::search::search;
use mta_graphing::service::Service;
use mta_graphing::time::{parse_time, Weekday, When};

use screen::Screen;
use term::Key;

pub struct Config {
    pub glyphs: Glyphs,
    pub color: ColorMode,
    pub day: Option<Weekday>,
    pub time: Option<u16>,
    pub mode: Mode,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    From,
    To,
    When,
    Mode,
}

impl Field {
    fn next(self) -> Field {
        match self {
            Field::From => Field::To,
            Field::To => Field::When,
            Field::When => Field::Mode,
            Field::Mode => Field::From,
        }
    }
    fn prev(self) -> Field {
        self.next().next().next()
    }
}

#[derive(Default)]
struct StationField {
    query: String,
    selected: Option<usize>,
    editing: bool,
    cursor: usize,
    matches: Vec<usize>,
}

struct Results {
    by_mode: Vec<Result<Route, RouteError>>,
    ms: f64,
}

struct App<'a> {
    net: &'a Network,
    graph: Graph<'a>,
    graph_ms: f64,
    when: When,
    live: bool,
    mode: Mode,
    glyphs: Glyphs,
    focus: Field,
    from: StationField,
    to: StationField,
    time_buf: String,
    results: Option<Results>,
    scroll: usize,
    blink: bool,
    status: Option<(String, bool)>,
}

const LABEL_X: usize = 2;
const VALUE_X: usize = 10;

impl<'a> App<'a> {
    fn new(net: &'a Network, cfg: &Config) -> App<'a> {
        let now = When::now_nyc();
        let live = cfg.day.is_none() && cfg.time.is_none();
        let when = When::new(cfg.day.unwrap_or(now.day), cfg.time.unwrap_or(now.minutes));
        let t = Instant::now();
        let graph = net.graph_at(when);

        App {
            net,
            graph,
            graph_ms: t.elapsed().as_secs_f64() * 1000.0,
            when,
            live,
            mode: cfg.mode,
            glyphs: cfg.glyphs,
            focus: Field::From,
            from: StationField::default(),
            to: StationField::default(),
            time_buf: String::new(),
            results: None,
            scroll: 0,
            blink: true,
            status: None,
        }
    }

    fn set_when(&mut self, w: When) {
        if w == self.when {
            return;
        }
        self.when = w;

        let t = Instant::now();

        self.graph = self.net.graph_at(w);
        self.graph_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.recompute();
    }

    fn recompute(&mut self) {
        self.results = match (self.from.selected, self.to.selected) {
            (Some(a), Some(b)) => {
                let t = Instant::now();
                let by_mode = Mode::ALL
                    .iter()
                    .map(|&m| find_route(&self.graph, a, b, m))
                    .collect();
                Some(Results {
                    by_mode,
                    ms: t.elapsed().as_secs_f64() * 1000.0,
                })
            }
            _ => None,
        };
        self.scroll = 0;
    }

    fn field_mut(&mut self, f: Field) -> &mut StationField {
        if f == Field::To {
            &mut self.to
        } else {
            &mut self.from
        }
    }

    fn update_matches(&mut self, f: Field) {
        let net = self.net;
        let sf = self.field_mut(f);

        sf.matches = search(net, &sf.query);
        sf.cursor = 0;
    }

    fn focus_to(&mut self, f: Field) {
        for sf in [&mut self.from, &mut self.to] {
            sf.editing = false;
            sf.query.clear();
        }
        self.time_buf.clear();
        self.focus = f;
    }

    fn select_station(&mut self, f: Field, cid: usize) {
        let sf = self.field_mut(f);

        sf.selected = Some(cid);
        sf.editing = false;
        sf.query.clear();
        
        self.recompute();

        let next = match f {
            Field::From if self.to.selected.is_none() => Field::To,
            Field::To if self.from.selected.is_none() => Field::From,
            _ => Field::Mode,
        };
        self.focus_to(next);
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.from.selected, &mut self.to.selected);
        self.recompute();
        self.status = Some(("swapped origin and destination".into(), false));
    }

    fn nudge_time(&mut self, delta: i32) {
        self.live = false;
        let base = self.when.minutes as i32;
        let m = if delta.abs() == 15 {
            let q = base.div_euclid(15) * 15;

            if delta > 0 {
                q + 15
            } else if q == base {
                q - 15
            } else {
                q
            }
        } else {
            base + delta
        };
        let mut day = self.when.day;
        let m = if m < 0 {
            day = day.prev();
            m + 1440
        } else if m >= 1440 {
            day = day.next();
            m - 1440
        } else {
            m
        };
        self.set_when(When::new(day, m as u16));
    }

    /// Returns false to quit.
    fn on_key(&mut self, k: Key) -> bool {
        self.status = None;
        match k {
            Key::Ctrl('c') | Key::Ctrl('q') | Key::Ctrl('d') => return false,
            Key::Ctrl('r') => {
                self.swap();
                return true;
            }
            Key::Ctrl('g') => {
                self.glyphs = self.glyphs.next();
                self.status = Some((format!("bullet style: {}", self.glyphs.name()), false));
                return true;
            }
            Key::PageUp => {
                self.scroll = self.scroll.saturating_sub(5);
                return true;
            }
            Key::PageDown => {
                self.scroll += 5;
                return true;
            }
            Key::Tab => {
                self.focus_to(self.focus.next());
                return true;
            }
            Key::BackTab => {
                self.focus_to(self.focus.prev());
                return true;
            }
            _ => {}
        }
        match self.focus {
            Field::From | Field::To => self.station_key(k),
            Field::When => self.when_key(k),
            Field::Mode => self.mode_key(k),
        }
    }

    fn station_key(&mut self, k: Key) -> bool {
        let f = self.focus;
        let net = self.net;
        let editing = self.field_mut(f).editing;

        match k {
            Key::Char(c) if !c.is_control() => {
                let sf = self.field_mut(f);

                if !sf.editing {
                    sf.editing = true;
                    sf.query.clear();
                }
                sf.query.push(c);
                self.update_matches(f);
            }
            Key::Backspace => {
                let sf = self.field_mut(f);
                if sf.editing {
                    sf.query.pop();
                } else if let Some(c) = sf.selected {
                    sf.editing = true;
                    sf.query = net.complex(c).label.clone();
                    sf.query.pop();
                }
                self.update_matches(f);
            }
            Key::Ctrl('u') | Key::Ctrl('w') => {
                let sf = self.field_mut(f);
                sf.editing = true;
                sf.query.clear();
                self.update_matches(f);
            }
            Key::Up if editing => {
                let sf = self.field_mut(f);
                sf.cursor = sf.cursor.saturating_sub(1);
            }
            Key::Down if editing => {
                let sf = self.field_mut(f);

                if sf.cursor + 1 < sf.matches.len() {
                    sf.cursor += 1;
                }
            }
            Key::Home if editing => self.field_mut(f).cursor = 0,
            Key::End if editing => {
                let sf = self.field_mut(f);
                sf.cursor = sf.matches.len().saturating_sub(1);
            }
            Key::Enter if editing => {
                let sf = self.field_mut(f);

                if let Some(&c) = sf.matches.get(sf.cursor) {
                    self.select_station(f, c);
                } else {
                    self.status = Some(("no matching station".into(), true));
                }
            }
            Key::Esc if editing => {
                let sf = self.field_mut(f);
                sf.editing = false;
                sf.query.clear();
            }
            Key::Up => self.focus_to(f.prev()),
            Key::Down | Key::Enter => self.focus_to(f.next()),
            _ => {}
        }
        true
    }

    fn when_key(&mut self, k: Key) -> bool {
        match k {
            Key::Left => self.nudge_time(-15),
            Key::Right => self.nudge_time(15),
            Key::Char('+') | Key::Char('=') => self.nudge_time(60),
            Key::Char('-') | Key::Char('_') => self.nudge_time(-60),
            Key::Char('[') | Key::Char(',') => {
                self.live = false;
                self.set_when(When::new(self.when.day.prev(), self.when.minutes));
            }
            Key::Char(']') | Key::Char('.') => {
                self.live = false;
                self.set_when(When::new(self.when.day.next(), self.when.minutes));
            }
            Key::Char('n') | Key::Char('N') => {
                self.live = true;
                self.time_buf.clear();
                self.set_when(When::now_nyc());
                self.status = Some(("following the live NYC clock".into(), false));
            }
            Key::Char(c) if c.is_ascii_digit() => {
                self.time_buf.push(c);

                if self.time_buf.len() == 4 {
                    self.apply_time_buf();
                }
            }
            Key::Char(c) if c.is_ascii_alphabetic() && c != 'q' => {
                self.time_buf.retain(|ch| ch.is_ascii_alphabetic());
                self.time_buf.push(c.to_ascii_lowercase());

                if self.time_buf.len() >= 2 {
                    let prefix = self.time_buf.clone();
                    let names = ["mo", "tu", "we", "th", "fr", "sa", "su"];

                    if let Some(i) = names.iter().position(|n| prefix.starts_with(n)) {
                        self.live = false;
                        self.set_when(When::new(Weekday::ALL[i], self.when.minutes));
                    }
                    self.time_buf.clear();
                }
            }
            Key::Backspace => {
                self.time_buf.pop();
            }
            Key::Esc => self.time_buf.clear(),
            Key::Enter => {
                if self.time_buf.len() == 3 {
                    self.apply_time_buf();
                } else {
                    self.focus_to(Field::Mode);
                }
            }
            Key::Up => self.focus_to(Field::To),
            Key::Down => self.focus_to(Field::Mode),
            Key::Char('q') => return false,
            _ => {}
        }
        true
    }

    fn apply_time_buf(&mut self) {
        match parse_time(&self.time_buf) {
            Ok(m) => {
                self.live = false;
                self.set_when(When::new(self.when.day, m));
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
        self.time_buf.clear();
    }

    fn mode_key(&mut self, k: Key) -> bool {
        match k {
            Key::Left => self.mode = self.mode.prev(),
            Key::Right | Key::Char(' ') => self.mode = self.mode.next(),
            Key::Char('1') => self.mode = Mode::Fastest,
            Key::Char('2') => self.mode = Mode::Shortest,
            Key::Char('3') => self.mode = Mode::FewestTransfers,
            Key::Up => self.focus_to(Field::When),
            Key::Down | Key::Enter => self.focus_to(Field::From),
            Key::Char('q') => return false,
            _ => {}
        }
        self.scroll = 0;
        true
    }

    fn draw(&self, s: &mut Screen) {
        s.clear();
        let (w, h) = (s.w, s.h);

        if w < 60 || h < 16 {
            let msg = format!("terminal too small ({}×{}); need at least 60×16", w, h);
            s.put(1, 0, &msg, Style::fg(theme::AMBER));
            return;
        }
        self.draw_title(s);
        self.draw_fields(s);

        let top = 8;
        let bottom = h - 3; // inclusive last content row
        let (title, body) = self.body(w);

        self.draw_rule(s, top - 1, &title);

        let rows = bottom + 1 - top;
        let max_scroll = body.len().saturating_sub(rows);
        let scroll = self.scroll.min(max_scroll);

        for (i, l) in body.iter().skip(scroll).take(rows).enumerate() {
            s.put_line(1, top + i, l, w - 1);
        }
        if max_scroll > 0 {
            let more = format!(
                " {}/{} pgup/pgdn ",
                scroll + rows.min(body.len()),
                body.len()
            );
            s.put(
                w.saturating_sub(more.len() + 2),
                bottom,
                &more,
                Style::fg(theme::DIM).on(theme::PANEL),
            );
        }
        self.draw_footer(s);
    }

    fn draw_title(&self, s: &mut Screen) {
        let w = s.w;
        s.fill(0, 0, w, 1, Style::fg(theme::TEXT).on(theme::PANEL));

        let mut x = s.put(0, 0, " ▌", Style::fg(theme::AMBER).bold());
        x = s.put(x, 0, " mta-graphing ", Style::fg(theme::BRIGHT).bold());
        s.put(x, 0, " rust · nyc subway router", Style::fg(theme::DIM));

        let clock = format!("{} {} NYC ", self.when.day.short(), self.when.clock());
        let (badge, bstyle) = if self.live {
            (" ● LIVE ", Style::fg(theme::OK).bold())
        } else {
            (" ◌ SET ", Style::fg(theme::AMBER).bold())
        };
        let total = clock.chars().count() + badge.chars().count() + 1;
        let x = w.saturating_sub(total);
        let x = s.put(x, 0, &clock, Style::fg(theme::BRIGHT).bold());
        s.put(x, 0, badge, bstyle);

        // Trunk-line color band.
        let colors = [
            (0xEE, 0x35, 0x2E),
            (0x00, 0x93, 0x3C),
            (0xB9, 0x33, 0xAD),
            (0x00, 0x39, 0xA6),
            (0xFF, 0x63, 0x19),
            (0x6C, 0xBE, 0x45),
            (0x99, 0x66, 0x33),
            (0xA7, 0xA9, 0xAC),
            (0xFC, 0xCC, 0x0A),
            (0x80, 0x81, 0x83),
        ];
        let ch = if self.glyphs == Glyphs::Ascii {
            "="
        } else {
            "▀"
        };
        for x in 0..w {
            let (r, g, b) = colors[(x * colors.len() / w).min(colors.len() - 1)];
            s.put(x, 1, ch, Style::fg(mta_graphing::service::Rgb(r, g, b)));
        }
    }

    fn label(&self, s: &mut Screen, y: usize, name: &str, f: Field) {
        let focused = self.focus == f;
        let st = if focused {
            Style::fg(theme::AMBER).bold()
        } else {
            Style::fg(theme::DIM)
        };
        s.put(LABEL_X, y, name, st);

        let prompt = if self.glyphs == Glyphs::Ascii {
            ">"
        } else {
            "❯"
        };
        s.put(
            LABEL_X + 5,
            y,
            prompt,
            if focused {
                Style::fg(theme::AMBER).bold()
            } else {
                Style::fg(theme::FAINT)
            },
        );
    }

    fn cursor(&self, s: &mut Screen, x: usize, y: usize) -> usize {
        if self.blink {
            s.put(x, y, "█", Style::fg(theme::AMBER))
        } else {
            x + 1
        }
    }

    fn draw_station_field(&self, s: &mut Screen, y: usize, f: Field) {
        let sf = if f == Field::From {
            &self.from
        } else {
            &self.to
        };
        let name = if f == Field::From { "FROM" } else { "TO" };
        self.label(s, y, name, f);

        let focused = self.focus == f;
        let mut x = VALUE_X;

        if sf.editing {
            x = s.put(x, y, &sf.query, Style::fg(theme::BRIGHT));
            x = self.cursor(s, x, y);
            let n = sf.matches.len();
            let hint = match n {
                0 => "  no match".to_string(),
                1 => "  1 match · enter to choose".to_string(),
                _ => format!("  {} matches · ↑↓ enter", n),
            };
            s.put(
                x,
                y,
                &hint,
                Style::fg(if n == 0 { theme::WARN } else { theme::DIM }),
            );
            return;
        }
        match sf.selected {
            Some(c) => {
                let label = &self.net.complex(c).label;
                let st = if focused {
                    Style::fg(theme::BRIGHT).bold().underline()
                } else {
                    Style::fg(theme::BRIGHT).bold()
                };
                x = s.put(x, y, label, st);
                let mut b: Line = render::bullets(&self.net.services_at(c, self.when), self.glyphs);
                let bw = render::line_width(&b);
                let right = s.w - 2;

                if x + 3 + bw <= right {
                    s.put_line(right - bw, y, &b, right);
                } else if x + 6 < right {
                    let room = right - x - 3;
                    let mut acc = 0;
                    let mut cut: Line = Vec::new();

                    for chunk in b.chunks(4) {
                        let cw: usize = chunk.iter().map(|sp| sp.text.chars().count()).sum();
                        if acc + cw + 1 > room {
                            break;
                        }
                        acc += cw;
                        cut.extend_from_slice(chunk);
                    }
                    b = cut;
                    let bw = render::line_width(&b);
                    s.put_line(right - bw, y, &b, right);
                }
            }
            None => {
                if focused {
                    x = self.cursor(s, x, y);
                }
                let ph = if f == Field::From {
                    " type an origin station"
                } else {
                    " type a destination"
                };
                s.put(x, y, ph, Style::fg(theme::FAINT));
            }
        }
    }

    fn draw_fields(&self, s: &mut Screen) {
        self.draw_station_field(s, 3, Field::From);
        self.draw_station_field(s, 4, Field::To);

        let y = 5;
        self.label(s, y, "WHEN", Field::When);

        let focused = self.focus == Field::When;
        let mut x = s.put(
            VALUE_X,
            y,
            self.when.day.short(),
            Style::fg(theme::BRIGHT).bold(),
        );
        x = s.put(x, y, "  ", Style::PLAIN);

        let digits: String = self
            .time_buf
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();

        if !digits.is_empty() {
            let mut t: String = digits.clone();

            while t.len() < 4 {
                t.push('_');
            }
            x = s.put(
                x,
                y,
                &format!("{}:{}", &t[..2], &t[2..]),
                Style::fg(theme::AMBER).bold(),
            );
        } else {
            x = s.put(x, y, &self.when.clock(), Style::fg(theme::BRIGHT).bold());
        }
        x = s.put(
            x,
            y,
            &format!("  {}", self.when.period().label()),
            Style::fg(theme::TEXT),
        );
        if self.live {
            x = s.put(x, y, "  ● live", Style::fg(theme::OK));
        } else {
            x = s.put(x, y, "  ◌ set", Style::fg(theme::AMBER));
        }
        if focused {
            let hint = "   ←→ ±15m  +/- ±1h  [ ] day  type 2300 or sat  n now";
            s.put_clip(x, y, hint, Style::fg(theme::DIM), s.w - 1);
        }

        // MODE
        let y = 6;
        self.label(s, y, "MODE", Field::Mode);

        let mut x = VALUE_X;

        for m in Mode::ALL {
            let st = if m == self.mode {
                if self.focus == Field::Mode {
                    Style::fg(theme::BG).on(theme::AMBER).bold()
                } else {
                    Style::fg(theme::BRIGHT).on(theme::RULE).bold()
                }
            } else {
                Style::fg(theme::DIM)
            };
            x = s.put(x, y, &format!(" {} ", m.label()), st);
            x = s.put(x, y, " ", Style::PLAIN);
        }
        if self.focus == Field::Mode {
            s.put_clip(x, y, "  ←→ change", Style::fg(theme::DIM), s.w - 1);
        }
    }

    fn draw_rule(&self, s: &mut Screen, y: usize, title: &str) {
        let w = s.w;
        let rule = if self.glyphs == Glyphs::Ascii {
            "-"
        } else {
            "─"
        };
        for x in 0..w {
            s.put(x, y, rule, Style::fg(theme::RULE));
        }
        s.put(1, y, rule, Style::fg(theme::RULE));

        let x = s.put(
            3,
            y,
            &format!(" {} ", title),
            Style::fg(theme::AMBER).bold(),
        );
        let mut stats = format!(
            " graph {} nodes · {} edges · {:.1} ms ",
            self.graph.node_count(),
            thousands(self.graph.edge_count),
            self.graph_ms
        );
        if let Some(r) = &self.results {
            stats = format!("{}· dijkstra {:.2} ms ", stats, r.ms);
        }
        let sx = w.saturating_sub(stats.chars().count() + 2);

        if sx > x + 2 {
            s.put(sx, y, &stats, Style::fg(theme::DIM));
        }
    }

    fn draw_footer(&self, s: &mut Screen) {
        let (w, h) = (s.w, s.h);
        let y = h - 2;

        match &self.status {
            Some((msg, err)) => {
                let st = if *err {
                    Style::fg(theme::WARN).bold()
                } else {
                    Style::fg(theme::AMBER)
                };
                let x = s.put(1, y, if *err { "! " } else { "» " }, st);
                s.put_clip(x, y, msg, st, w - 1);
            }
            None => {
                let x = s.put(1, y, "$ ", Style::fg(theme::AMBER).bold());
                s.put_line(
                    x,
                    y,
                    &vec![span(self.command_line(), Style::fg(theme::DIM))],
                    w - 1,
                );
            }
        }
        s.fill(0, h - 1, w, 1, Style::fg(theme::TEXT).on(theme::PANEL));
        let keys: &[(&str, &str)] = match self.focus {
            Field::From | Field::To => &[
                ("type", "search"),
                ("↑↓", "pick"),
                ("enter", "choose"),
                ("tab", "next"),
                ("^R", "swap"),
                ("^G", "glyphs"),
                ("^C", "quit"),
            ],
            Field::When => &[
                ("←→", "time"),
                ("[ ]", "day"),
                ("n", "now"),
                ("tab", "next"),
                ("^R", "swap"),
                ("q", "quit"),
            ],
            Field::Mode => &[
                ("←→", "mode"),
                ("1 2 3", "pick"),
                ("tab", "next"),
                ("^R", "swap"),
                ("q", "quit"),
            ],
        };
        let mut x = 1;

        for (k, d) in keys {
            if x + k.len() + d.len() + 3 >= w {
                break;
            }
            x = s.put(x, h - 1, k, Style::fg(theme::BRIGHT).bold());
            x = s.put(x, h - 1, &format!(" {}   ", d), Style::fg(theme::DIM));
        }
    }

    fn command_line(&self) -> String {
        let q = |c: usize| format!("\"{}\"", self.net.complex(c).label);

        match (self.from.selected, self.to.selected) {
            (Some(a), Some(b)) => format!(
                "mta route {} {} -d {} -t {} -m {}",
                q(a),
                q(b),
                self.when.day.short().to_ascii_lowercase(),
                self.when.military(),
                self.mode.flag()
            ),
            _ => "mta   # pick two stations".into(),
        }
    }

    /// The content area: (title, lines).
    fn body(&self, w: usize) -> (String, Vec<Line>) {
        let sf = match self.focus {
            Field::From => Some(&self.from),
            Field::To => Some(&self.to),
            _ => None,
        };
        if let Some(sf) = sf.filter(|sf| sf.editing) {
            return ("MATCHES".into(), self.match_lines(sf, w));
        }
        if let Some(r) = &self.results {
            return ("ROUTE".into(), self.route_body(r));
        }
        ("READY".into(), self.idle_lines(w))
    }

    fn match_lines(&self, sf: &StationField, w: usize) -> Vec<Line> {
        let rows = 64usize;
        let start = sf.cursor.saturating_sub(4);
        let mut out = Vec::new();

        for (i, &c) in sf.matches.iter().enumerate().skip(start).take(rows) {
            let cx = self.net.complex(c);
            let sel = i == sf.cursor;
            let mut l: Line = vec![if sel {
                span(" ▸ ", Style::fg(theme::AMBER).bold())
            } else {
                span("   ", Style::PLAIN)
            }];
            let name_st = if sel {
                Style::fg(theme::BRIGHT).bold()
            } else {
                Style::fg(theme::TEXT)
            };
            l.push(span(cx.label.clone(), name_st));

            if !cx.aka.is_empty() {
                l.push(span(
                    format!(" · {}", cx.aka.join(" · ")),
                    Style::fg(theme::DIM),
                ));
            }
            let b = render::bullets(&self.net.services_at(c, self.when), self.glyphs);
            let bw = render::line_width(&b);
            let lw = render::line_width(&l);
            let avail = w.saturating_sub(4);

            if lw + bw + 2 > avail {
                let keep = avail.saturating_sub(bw + 2);
                let mut acc = 0;
                let mut trimmed: Line = Vec::new();

                for sp in l {
                    let n = sp.text.chars().count();

                    if acc + n <= keep {
                        acc += n;
                        trimmed.push(sp);
                    } else {
                        let t: String = sp.text.chars().take(keep.saturating_sub(acc)).collect();
                        trimmed.push(span(t, sp.style));
                        break;
                    }
                }
                l = trimmed;
            }
            let pad = avail.saturating_sub(render::line_width(&l) + bw);

            l.push(span(" ".repeat(pad.max(2)), Style::PLAIN));
            l.extend(b);
            out.push(l);
        }
        if out.is_empty() {
            out.push(vec![span(
                "   no station matches. Try fewer letters.",
                Style::fg(theme::WARN),
            )]);
        }
        out
    }

    fn route_body(&self, r: &Results) -> Vec<Line> {
        let idx = Mode::ALL.iter().position(|m| *m == self.mode).unwrap_or(0);
        let mut out: Vec<Line> = Vec::new();

        match &r.by_mode[idx] {
            Ok(route) => {
                out.push(vec![
                    span(
                        format!(" {} ", self.mode.label()),
                        Style::fg(theme::BG).on(theme::AMBER).bold(),
                    ),
                    span(
                        format!("  {}", render::summary(route)),
                        Style::fg(theme::BRIGHT).bold(),
                    ),
                ]);
                let mut cmp: Line = vec![span("  ", Style::PLAIN)];
                let mut any = false;

                for (i, m) in Mode::ALL.iter().enumerate() {
                    if i == idx {
                        continue;
                    }
                    if let Ok(o) = &r.by_mode[i] {
                        let same = o.stations() == route.stations();
                        let txt = if same {
                            format!("{} same route", m.flag())
                        } else {
                            format!(
                                "{} ≈{} · {:.1} km · {} transfer{}",
                                m.flag(),
                                render::fmt_min(o.minutes()),
                                o.km(),
                                o.transfers(),
                                if o.transfers() == 1 { "" } else { "s" }
                            )
                        };
                        if any {
                            cmp.push(span("   │   ", Style::fg(theme::FAINT)));
                        }
                        cmp.push(span(
                            txt,
                            Style::fg(if same { theme::FAINT } else { theme::DIM }),
                        ));
                        any = true;
                    }
                }
                out.push(cmp);
                out.push(vec![]);
                out.extend(render::route_lines(self.net, route, self.glyphs));
                out.push(vec![]);
                out.push(vec![span(
                    "  times are estimates: straight-line km at 0.35 km/min + 5 min per transfer",
                    Style::fg(theme::FAINT),
                )]);
            }
            Err(e) => {
                let msg = match e {
                    RouteError::SameStation => {
                        "origin and destination are the same station".to_string()
                    }
                    RouteError::NoService(c) => format!(
                        "no trains stop at {} on {} at {} ({})",
                        self.net.complex(*c).label,
                        self.when.day.long(),
                        self.when.clock(),
                        self.when.period().label()
                    ),
                    RouteError::Unreachable => "no route at this time".to_string(),
                };
                out.push(vec![span(
                    format!(" ✕ {}", msg),
                    Style::fg(theme::WARN).bold(),
                )]);
                out.push(vec![span(
                    "   try another time (WHEN field): some lines and stations don't run late nights or weekends",
                    Style::fg(theme::DIM),
                )]);
            }
        }
        out
    }

    fn idle_lines(&self, w: usize) -> Vec<Line> {
        let ok = |t: String| -> Line {
            vec![
                span(" [", Style::fg(theme::FAINT)),
                span(" OK ", Style::fg(theme::OK).bold()),
                span("] ", Style::fg(theme::FAINT)),
                span(t, Style::fg(theme::TEXT)),
            ]
        };
        let n_services = {
            let mut v: Vec<(&str, bool)> = self
                .net
                .by_service
                .keys()
                .map(|s| (s.label(), s.is_express()))
                .collect();
            v.sort();
            v.dedup();
            v.len()
        };
        let mut out = vec![
            ok(format!(
                "loaded {} platforms → {} station complexes",
                self.net.rows.len(),
                self.net.complexes.len()
            )),
            ok(format!(
                "parsed service notes for {} services ({} warnings)",
                n_services,
                self.net.warnings.len()
            )),
            ok(format!(
                "graph @ {} {} ({}): {} nodes · {} edges",
                self.when.day.short(),
                self.when.clock(),
                self.when.period().label(),
                self.graph.node_count(),
                thousands(self.graph.edge_count)
            )),
            vec![],
        ];

        // Live legend: which services run right now.
        let mut seen: Vec<(&str, bool)> = Vec::new();
        let mut legend: Vec<(Service, bool)> = Vec::new();
        let mut services: Vec<&Service> = self.net.by_service.keys().collect();
        services.sort();

        for s in services {
            let key = (s.label(), s.is_express());
            let running = self
                .net
                .by_service
                .iter()
                .filter(|(k, _)| (k.label(), k.is_express()) == key)
                .any(|(k, rows)| {
                    rows.iter().any(|&r| {
                        self.net.rows[r]
                            .rules
                            .rule(*k)
                            .is_some_and(|rule| rule.stops_any(self.when))
                    })
                });
            if let Some(pos) = seen.iter().position(|k| *k == key) {
                legend[pos].1 |= running;
            } else {
                seen.push(key);
                legend.push((*s, running));
            }
        }
        let prefix = "  running now  ";
        let mut line: Line = vec![span(prefix, Style::fg(theme::DIM))];
        let mut last_color = None;

        for (s, on) in legend {
            let gap = if last_color.is_some() && last_color != Some(s.color()) {
                "  "
            } else {
                " "
            };
            last_color = Some(s.color());

            if render::line_width(&line) + 5 > w.saturating_sub(4) {
                out.push(std::mem::replace(
                    &mut line,
                    vec![span(" ".repeat(prefix.len()), Style::PLAIN)],
                ));
            } else if line.len() > 1 {
                line.push(span(gap, Style::PLAIN));
            }
            line.extend(render::bullet(s, self.glyphs, on));
        }
        out.push(line);

        let demo_local = render::bullet(
            Service::local(mta_graphing::service::Line::L6),
            self.glyphs,
            true,
        );
        let demo_express = render::bullet(
            Service::new(
                mta_graphing::service::Line::L6,
                mta_graphing::service::Kind::Express,
            ),
            self.glyphs,
            true,
        );
        let mut shapes: Line = vec![span("  shapes       ", Style::fg(theme::DIM))];
        shapes.extend(demo_local);
        shapes.push(span(" local    ", Style::fg(theme::TEXT)));
        shapes.extend(demo_express);
        shapes.push(span(" express    ", Style::fg(theme::TEXT)));
        shapes.extend(render::bullet(
            Service::local(mta_graphing::service::Line::L6),
            self.glyphs,
            false,
        ));
        shapes.push(span(" not running at this time", Style::fg(theme::TEXT)));
        out.push(shapes);
        out.push(vec![]);

        let mut prompt: Line = vec![
            span("  > ", Style::fg(theme::AMBER).bold()),
            span("type a station name to begin", Style::fg(theme::TEXT)),
        ];
        if self.blink {
            prompt.push(span("_", Style::fg(theme::AMBER)));
        }
        out.push(prompt);
        out
    }
}

fn thousands(n: usize) -> String {
    let digits: Vec<char> = n.to_string().chars().collect();
    let groups: Vec<String> = digits
        .rchunks(3)
        .rev()
        .map(|g| g.iter().collect())
        .collect();
    groups.join(",")
}

pub fn run(net: Network, cfg: Config) -> io::Result<()> {
    let mut app = App::new(&net, &cfg);
    let _term = term::Terminal::enter()?;
    let rx = term::spawn_input();
    let (w, h) = term::size();
    let mut screen = Screen::new(w as usize, h as usize);
    let mut out = io::stdout();
    let mut buf: Vec<u8> = Vec::new();
    let mut last_blink = Instant::now();

    loop {
        let (w, h) = term::size();

        if (w as usize, h as usize) != (screen.w, screen.h) {
            screen.resize(w as usize, h as usize);
        }
        if app.live {
            let now = When::now_nyc();
            if now != app.when {
                app.set_when(now);
            }
        }
        if last_blink.elapsed() >= Duration::from_millis(530) {
            app.blink = !app.blink;
            last_blink = Instant::now();
        }
        app.draw(&mut screen);
        screen.flush(&mut out, cfg.color)?;

        match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(bytes) => {
                buf.extend(bytes);

                if buf.last() == Some(&0x1b) {
                    if let Ok(more) = rx.recv_timeout(Duration::from_millis(30)) {
                        buf.extend(more);
                    }
                }
                while let Ok(more) = rx.try_recv() {
                    buf.extend(more);
                }
                let keys = term::parse_keys(&mut buf, true);
                app.blink = true;
                last_blink = Instant::now();

                for k in keys {
                    if !app.on_key(k) {
                        return Ok(());
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(net: &Network) -> App<'_> {
        App::new(
            net,
            &Config {
                glyphs: Glyphs::Unicode,
                color: ColorMode::TrueColor,
                day: Some(Weekday::Tue),
                time: Some(16 * 60 + 5),
                mode: Mode::Fastest,
            },
        )
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            app.on_key(Key::Char(c));
        }
    }

    fn screen_text(app: &App, w: usize, h: usize) -> String {
        let mut s = Screen::new(w, h);
        app.draw(&mut s);

        let mut v = Vec::new();
        s.flush(&mut v, ColorMode::None).unwrap();
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn full_flow_picks_stations_and_routes() {
        let net = Network::embedded();
        let mut app = app_with(&net);

        type_str(&mut app, "times sq");
        assert!(app.from.editing && !app.from.matches.is_empty());
        app.on_key(Key::Enter);
        assert_eq!(
            net.complex(app.from.selected.unwrap()).label,
            "Times Sq - 42nd St"
        );
        assert_eq!(app.focus, Field::To);
        type_str(&mut app, "flushing");
        app.on_key(Key::Enter);
        assert!(app
            .results
            .as_ref()
            .unwrap()
            .by_mode
            .iter()
            .all(|r| r.is_ok()));

        let text = screen_text(&app, 100, 30);

        assert!(text.contains("ROUTE"));
        assert!(text.contains("Flushing - Main St"));
        assert!(text.contains("◖7◗"));
        assert!(text.contains(
            "mta route \"Times Sq - 42nd St\" \"Flushing - Main St\" -d tue -t 1605 -m fastest"
        ));

        app.focus_to(Field::When);
        type_str(&mut app, "0300");
        assert_eq!(app.when.clock(), "03:00");
        assert!(!app.live);

        app.on_key(Key::Ctrl('r'));
        assert_eq!(
            net.complex(app.to.selected.unwrap()).label,
            "Times Sq - 42nd St"
        );

        app.focus_to(Field::Mode);
        app.on_key(Key::Right);
        assert_eq!(app.mode, Mode::Shortest);
        assert!(!app.on_key(Key::Ctrl('c')));
    }

    #[test]
    fn draws_at_small_and_large_sizes_without_panicking() {
        let net = Network::embedded();
        let mut app = app_with(&net);

        for (w, h) in [(40, 10), (60, 16), (80, 24), (200, 60)] {
            screen_text(&app, w, h);
        }
        type_str(&mut app, "86");
        
        for (w, h) in [(60, 16), (80, 24), (120, 40)] {
            screen_text(&app, w, h);
        }
    }
}
