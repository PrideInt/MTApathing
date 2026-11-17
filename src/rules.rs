//! Parses the free-text `NOTES` column ("4 nights, 6-all times, 6 Express-weekdays
//! AM southbound, PM northbound") into structured availability rules.
//!
//! This replaces `Node.build` / `Node.applyConditions` from the Java version.
//! The Java code matched keywords with `String.contains` and dropped the
//! direction information. Here every note becomes an expression tree that is
//! evaluated against a time (`When`) and a direction of travel (`Heading`).
//! Directional service such as the <7> (AM westbound, PM eastbound) is
//! therefore modelled properly.

use crate::service::{Bound, Kind, Line, Service};
use crate::time::When;

/// Direction of travel between two points (sign of latitude/longitude change).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heading {
    pub dlat: f64,
    pub dlon: f64,
}

impl Heading {
    pub fn between(from: (f64, f64), to: (f64, f64)) -> Heading {
        Heading {
            dlat: to.0 - from.0,
            dlon: to.1 - from.1,
        }
    }

    pub fn matches(self, b: Bound) -> bool {
        match b {
            Bound::North => self.dlat > 0.0,
            Bound::South => self.dlat < 0.0,
            Bound::East => self.dlon > 0.0,
            Bound::West => self.dlon < 0.0,
        }
    }

    /// One representative heading per quadrant, used to ask "does this
    /// service stop here at all right now, in any direction?".
    pub const QUADRANTS: [Heading; 4] = [
        Heading {
            dlat: 1.0,
            dlon: 1.0,
        },
        Heading {
            dlat: 1.0,
            dlon: -1.0,
        },
        Heading {
            dlat: -1.0,
            dlon: 1.0,
        },
        Heading {
            dlat: -1.0,
            dlon: -1.0,
        },
    ];
}

/// Which way (if at all) a directional rule applies in the AM or the PM.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot {
    Off,
    Any,
    Toward(Bound),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirSpec {
    pub am: Slot,
    pub pm: Slot,
}

impl DirSpec {
    pub const ANY: DirSpec = DirSpec {
        am: Slot::Any,
        pm: Slot::Any,
    };

    fn allows(&self, w: When, h: Heading) -> bool {
        match if w.is_am() { self.am } else { self.pm } {
            Slot::Off => false,
            Slot::Any => true,
            Slot::Toward(b) => h.matches(b),
        }
    }
}

/// Time-of-week conditions found in the notes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeCond {
    Always,
    Never,
    LateNights,
    Rush,
    Weekdays,
    WeekdaysAndEvenings,
    Evenings,
    Weekends,
}

impl TimeCond {
    fn holds(self, w: When) -> bool {
        match self {
            TimeCond::Always => true,
            TimeCond::Never => false,
            TimeCond::LateNights => w.is_late_night(),
            TimeCond::Rush => w.is_rush(),
            TimeCond::Weekdays => w.is_weekday_daytime(),
            TimeCond::WeekdaysAndEvenings => w.is_weekday_not_night(),
            TimeCond::Evenings => w.is_evening(),
            TimeCond::Weekends => w.is_weekend_daytime(),
        }
    }
}

/// Availability expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Avail {
    Leaf(TimeCond, DirSpec),
    Not(Box<Avail>),
    All(Vec<Avail>),
    Any(Vec<Avail>),
}

impl Avail {
    pub fn always() -> Avail {
        Avail::Leaf(TimeCond::Always, DirSpec::ANY)
    }

    pub fn eval(&self, w: When, h: Heading) -> bool {
        match self {
            Avail::Leaf(c, d) => c.holds(w) && d.allows(w, h),
            Avail::Not(a) => !a.eval(w, h),
            Avail::All(v) => v.iter().all(|a| a.eval(w, h)),
            Avail::Any(v) => v.iter().any(|a| a.eval(w, h)),
        }
    }

    /// Apply a trailing "PM eastbound"-style continuation.
    fn set_slot(&mut self, pm: bool, slot: Slot) -> bool {
        match self {
            Avail::Leaf(_, d) => {
                if pm {
                    d.pm = slot;
                } else {
                    d.am = slot;
                }
                true
            }
            Avail::Not(a) => a.set_slot(pm, slot),
            Avail::All(v) | Avail::Any(v) => v.iter_mut().rev().any(|a| a.set_slot(pm, slot)),
        }
    }
}

/// When one service stops at one station.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceRule {
    pub service: Service,
    pub base: Avail,
    pub skips: Vec<Avail>,
    pub exit_only: Option<Bound>,
}

impl ServiceRule {
    pub fn stops(&self, w: When, h: Heading) -> bool {
        self.base.eval(w, h) && !self.skips.iter().any(|s| s.eval(w, h))
    }
    pub fn boards(&self, w: When, h: Heading) -> bool {
        self.stops(w, h) && !self.exit_only.is_some_and(|b| h.matches(b))
    }
    pub fn stops_any(&self, w: When) -> bool {
        Heading::QUADRANTS.iter().any(|h| self.stops(w, *h))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StationRules {
    pub rules: Vec<ServiceRule>,
    pub closed: bool,
    pub warnings: Vec<String>,
}

impl StationRules {
    pub fn rule(&self, s: Service) -> Option<&ServiceRule> {
        self.rules.iter().find(|r| r.service == s)
    }
    pub fn services(&self) -> impl Iterator<Item = Service> + '_ {
        self.rules.iter().map(|r| r.service)
    }
}

/// Parse a `LINE` column value like "4-6-6 Express" or "A-C-E".
pub fn parse_line_column(col: &str, lat: f64, lon: f64) -> Vec<Service> {
    let mut out = Vec::new();

    for tok in col.split('-') {
        if let Some(s) = parse_train(tok, lat, lon) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// "6 Express" -> <6>, "S to Times Sq" -> 42 St Shuttle, "A" -> (A).
fn parse_train(item: &str, lat: f64, lon: f64) -> Option<Service> {
    let mut words = item.split_whitespace();
    let line = Line::from_token(words.next()?, lat, lon)?;
    let kind = if words.next() == Some("Express") {
        Kind::Express
    } else {
        Kind::Local
    };
    Some(Service::new(line, kind))
}

fn parse_train_list(list: &str, lat: f64, lon: f64) -> Option<Vec<Service>> {
    let v: Option<Vec<Service>> = list.split(',').map(|i| parse_train(i, lat, lon)).collect();
    v.filter(|v| !v.is_empty())
}

/// Parse a condition such as "rush hours AM westbound", "all times exc nights",
/// "weekdays PM", "nights".
pub fn parse_cond(text: &str, warnings: &mut Vec<String>) -> Avail {
    let text = text.trim();

    if let Some((a, b)) = text.split_once(" exc ") {
        return Avail::All(vec![
            parse_cond(a, warnings),
            Avail::Not(Box::new(parse_cond(b, warnings))),
        ]);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let (mut am, mut pm, mut bare) = (None, None, None);
    let mut base: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let w = words[i];

        if w == "AM" || w == "PM" {
            let slot = match words.get(i + 1).and_then(|n| Bound::parse_word(n)) {
                Some(b) => {
                    i += 1;
                    Slot::Toward(b)
                }
                None => Slot::Any,
            };
            if w == "AM" {
                am = Some(slot);
            } else {
                pm = Some(slot);
            }
        } else if let Some(b) = Bound::parse_word(w) {
            bare = Some(b);
        } else {
            base.push(w);
        }
        i += 1;
    }
    let dir = match (am, pm, bare) {
        (None, None, Some(b)) => DirSpec {
            am: Slot::Toward(b),
            pm: Slot::Toward(b),
        },
        (None, None, None) => DirSpec::ANY,
        (am, pm, _) => DirSpec {
            am: am.unwrap_or(Slot::Off),
            pm: pm.unwrap_or(Slot::Off),
        },
    };
    let base = base.join(" ");
    let cond = match base.as_str() {
        "" | "all times" => TimeCond::Always,
        "nights" | "late nights" => TimeCond::LateNights,
        "rush hours" => TimeCond::Rush,
        "weekdays" => TimeCond::Weekdays,
        "weekdays and evenings" | "part time" => TimeCond::WeekdaysAndEvenings,
        "evenings" => TimeCond::Evenings,
        "weekends" => TimeCond::Weekends,
        "racing days only" | "to Manhattan only" => TimeCond::Never,
        other => {
            warnings.push(format!(
                "unrecognised condition '{}', assuming all times",
                other
            ));
            TimeCond::Always
        }
    };
    Avail::Leaf(cond, dir)
}

/// What the default availability is for a service listed in the `LINE` column
/// but never mentioned in `NOTES` (e.g., the W at Whitehall St).
fn default_for(line: Line) -> Avail {
    let c = match line {
        Line::W | Line::B => TimeCond::WeekdaysAndEvenings,
        Line::Z => TimeCond::Rush,
        _ => TimeCond::Always,
    };
    Avail::Leaf(c, DirSpec::ANY)
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Nothing,
    Base,
    Skip,
}

/// Parse one station's `NOTES` given its `LINE` column and location.
pub fn parse_notes(notes: &str, line_col: &str, lat: f64, lon: f64) -> StationRules {
    let mut out = StationRules::default();
    let listed = parse_line_column(line_col, lat, lon);

    // Rule indices from the last segment, the continuation target, and the
    // rules seen since the last "all other times" boundary.
    let mut group: std::ops::Range<usize> = 0..0;
    let mut target = Target::Nothing;
    let mut since_boundary: Vec<usize> = Vec::new();

    for raw in notes.split(", ") {
        let seg = raw.trim();

        if seg.is_empty() {
            continue;
        }
        let lower = seg.to_ascii_lowercase();

        if lower == "temporarily closed" {
            out.closed = true;
            continue;
        }
        if lower.starts_with("closed nights") {
            for r in &mut out.rules[group.clone()] {
                r.skips.push(Avail::Leaf(TimeCond::LateNights, DirSpec::ANY));
            }
            target = Target::Skip;
            continue;
        }
        if let Some(rest) = seg.strip_prefix("skips ") {
            let cond = parse_cond(rest, &mut out.warnings);

            for r in &mut out.rules[group.clone()] {
                r.skips.push(cond.clone());
            }
            target = Target::Skip;
            continue;
        }
        if let Some(rest) = seg.strip_prefix("exit only") {
            let b = rest.split_whitespace().find_map(Bound::parse_word);

            for r in &mut out.rules[group.clone()] {
                r.exit_only = b;
            }
            continue;
        }

        let mut parsed: Option<(Vec<Service>, &str)> = None;

        if let Some(idx) = seg.rfind('-') {
            if let Some(trains) = parse_train_list(&seg[..idx], lat, lon) {
                parsed = Some((trains, &seg[idx + 1..]));
            }
        }
        if parsed.is_none() {
            let mut it = seg.splitn(2, ' ');
            let first = it.next().unwrap_or("");
            let rest = it.next().unwrap_or("");

            if let (Some(line), false) = (Line::from_token(first, lat, lon), rest.is_empty()) {
                let (kind, rest) = match rest.strip_prefix("Express ") {
                    Some(r) => (Kind::Express, r),
                    None => (Kind::Local, rest),
                };
                parsed = Some((vec![Service::new(line, kind)], rest));
            }
        }

        match parsed {
            Some((trains, cond_text)) => {
                let cond_text = cond_text.trim();
                let base = if cond_text == "all other times" {
                    let prior: Vec<Avail> = since_boundary
                        .iter()
                        .map(|&i| out.rules[i].base.clone())
                        .collect();
                    since_boundary.clear();
                    Avail::Not(Box::new(Avail::Any(prior)))
                } else {
                    parse_cond(cond_text, &mut out.warnings)
                };
                let start = out.rules.len();

                for s in trains {
                    out.rules.push(ServiceRule {
                        service: s,
                        base: base.clone(),
                        skips: Vec::new(),
                        exit_only: None,
                    });
                }
                group = start..out.rules.len();

                if cond_text != "all other times" {
                    since_boundary.extend(group.clone());
                }
                target = Target::Base;
            }
            None => {
                let words: Vec<&str> = seg.split_whitespace().collect();
                let is_dir = matches!(words.first(), Some(&"AM") | Some(&"PM"));

                if is_dir && target != Target::Nothing {
                    let pm = words[0] == "PM";
                    let slot = words
                        .get(1)
                        .and_then(|w| Bound::parse_word(w))
                        .map(Slot::Toward)
                        .unwrap_or(Slot::Any);
                    for r in &mut out.rules[group.clone()] {
                        let a = match target {
                            Target::Skip => r.skips.last_mut(),
                            _ => Some(&mut r.base),
                        };
                        if let Some(a) = a {
                            a.set_slot(pm, slot);
                        }
                    }
                } else if target == Target::Base && !group.is_empty() {
                    // e.g., "E-evenings, weekends, nights": widen the previous condition.
                    let extra = parse_cond(seg, &mut out.warnings);

                    for r in &mut out.rules[group.clone()] {
                        let old = std::mem::replace(&mut r.base, Avail::always());
                        r.base = Avail::Any(vec![old, extra.clone()]);
                    }
                } else {
                    out.warnings.push(format!("ignored note segment '{}'", seg));
                }
            }
        }
    }

    if out.closed {
        out.rules.clear();
        return out;
    }

    // Merge duplicate mentions of the same service (OR their availability).
    let mut merged: Vec<ServiceRule> = Vec::new();

    for r in out.rules.drain(..) {
        if let Some(m) = merged.iter_mut().find(|m| m.service == r.service) {
            fn fold(x: ServiceRule) -> Avail {
                if x.skips.is_empty() {
                    x.base
                } else {
                    Avail::All(vec![x.base, Avail::Not(Box::new(Avail::Any(x.skips)))])
                }
            }
            let exit_only = m.exit_only.or(r.exit_only);
            let old = std::mem::replace(
                m,
                ServiceRule {
                    service: r.service,
                    base: Avail::always(),
                    skips: vec![],
                    exit_only,
                },
            );
            m.base = Avail::Any(vec![fold(old), fold(r)]);
        } else {
            merged.push(r);
        }
    }
    out.rules = merged;

    for s in listed {
        if out.rule(s).is_none() {
            out.rules.push(ServiceRule {
                service: s,
                base: default_for(s.line),
                skips: vec![],
                exit_only: None,
            });
        }
    }
    out.rules.sort_by_key(|r| r.service);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Weekday::*;

    const MANHATTAN: (f64, f64) = (40.75, -73.98);

    fn at(day: crate::time::Weekday, h: u16, m: u16) -> When {
        When::new(day, h * 60 + m)
    }
    fn svc(l: Line) -> Service {
        Service::local(l)
    }
    fn exp(l: Line) -> Service {
        Service::new(l, Kind::Express)
    }
    const N: Heading = Heading {
        dlat: 1.0,
        dlon: 0.0,
    };
    const S: Heading = Heading {
        dlat: -1.0,
        dlon: 0.0,
    };
    const E: Heading = Heading {
        dlat: 0.0,
        dlon: 1.0,
    };
    const W: Heading = Heading {
        dlat: 0.0,
        dlon: -1.0,
    };

    fn parse(notes: &str, lines: &str) -> StationRules {
        let r = parse_notes(notes, lines, MANHATTAN.0, MANHATTAN.1);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        r
    }

    #[test]
    fn astor_place_example_from_readme() {
        // "4 nights, 6-all times, 6 Express-weekdays AM southbound, PM northbound"
        let r = parse(
            "4 nights, 6-all times, 6 Express-weekdays AM southbound, PM northbound",
            "4-6-6 Express",
        );
        let four = r.rule(svc(Line::L4)).unwrap();
        let six = r.rule(svc(Line::L6)).unwrap();
        let six_x = r.rule(exp(Line::L6)).unwrap();

        // Saturday 23:00 (the README's example): 6 local only; the 4 is not yet
        // running local because late nights begin at midnight.
        assert!(six.stops_any(at(Sat, 23, 0)));
        assert!(!six_x.stops_any(at(Sat, 23, 0)));
        assert!(!four.stops_any(at(Sat, 23, 0)));
        // Saturday 02:00: the 4 runs local overnight.
        assert!(four.stops_any(at(Sat, 2, 0)));

        // Weekday AM: <6> southbound only; weekday PM: northbound only.
        assert!(six_x.stops(at(Tue, 8, 0), S));
        assert!(!six_x.stops(at(Tue, 8, 0), N));
        assert!(six_x.stops(at(Tue, 17, 0), N));
        assert!(!six_x.stops(at(Tue, 17, 0), S));
    }

    #[test]
    fn all_other_times_is_complement_of_earlier_segments() {
        let r = parse("A-nights, C-all other times", "A-C");
        let a = r.rule(svc(Line::A)).unwrap();
        let c = r.rule(svc(Line::C)).unwrap();

        assert!(a.stops_any(at(Mon, 3, 0)) && !c.stops_any(at(Mon, 3, 0)));
        assert!(!a.stops_any(at(Mon, 12, 0)) && c.stops_any(at(Mon, 12, 0)));

        // Boundary: R's complement only covers "N-nights", not the B.
        let r = parse(
            "B-weekdays and evenings, D-all other times, N-nights, R-all other times, Q-all times",
            "B-D-N-Q-R",
        );
        let d = r.rule(svc(Line::D)).unwrap();
        let rr = r.rule(svc(Line::R)).unwrap();

        assert!(!d.stops_any(at(Tue, 12, 0)) && d.stops_any(at(Sat, 12, 0)));
        assert!(rr.stops_any(at(Tue, 12, 0)) && !rr.stops_any(at(Tue, 3, 0)));
    }

    #[test]
    fn directional_complement_for_skip_stop() {
        // J skips these stations in the peak direction when the Z runs.
        let r = parse(
            "Z-rush hours AM westbound, PM eastbound, J-all other times",
            "J-Z",
        );
        let j = r.rule(svc(Line::J)).unwrap();
        let z = r.rule(svc(Line::Z)).unwrap();

        assert!(z.stops(at(Wed, 8, 0), W) && !z.stops(at(Wed, 8, 0), E));
        assert!(!j.stops(at(Wed, 8, 0), W) && j.stops(at(Wed, 8, 0), E));
        assert!(j.stops(at(Wed, 12, 0), W));
        assert!(z.stops(at(Wed, 17, 0), E) && !j.stops(at(Wed, 17, 0), E));
    }

    #[test]
    fn skips_and_exc() {
        let r = parse(
            "2-all times, 5-all times exc nights, skips rush hours AM southbound, PM northbound",
            "2-5",
        );
        let five = r.rule(svc(Line::L5)).unwrap();
        
        assert!(!five.stops_any(at(Mon, 3, 0)));
        assert!(five.stops(at(Mon, 12, 0), S));
        assert!(!five.stops(at(Mon, 8, 0), S) && five.stops(at(Mon, 8, 0), N));
        assert!(!five.stops(at(Mon, 17, 0), N) && five.stops(at(Mon, 17, 0), S));
    }

    #[test]
    fn train_lists_continuations_and_defaults() {
        let r = parse("F,G-all times", "F-G");
        assert_eq!(r.rules.len(), 2);

        let r = parse("E-evenings, weekends, nights, F-all times", "E-F");
        let e = r.rule(svc(Line::E)).unwrap();
        assert!(!e.stops_any(at(Tue, 12, 0)));
        assert!(
            e.stops_any(at(Tue, 21, 0))
                && e.stops_any(at(Sun, 12, 0))
                && e.stops_any(at(Tue, 2, 0))
        );

        // W listed in LINE but not in NOTES -> weekdays & evenings.
        let r = parse("N-nights, R-all other times", "R-W");
        let w = r.rule(svc(Line::W)).unwrap();
        assert!(w.stops_any(at(Thu, 12, 0)) && !w.stops_any(at(Sat, 12, 0)));

        let r = parse("N-all times, W part time", "N-W");
        let w = r.rule(svc(Line::W)).unwrap();
        assert!(w.stops_any(at(Thu, 22, 0)) && !w.stops_any(at(Thu, 2, 0)));
    }

    #[test]
    fn exit_only_and_closed() {
        let r = parse("1-all times, exit only northbound", "1");
        let one = r.rule(svc(Line::L1)).unwrap();
        assert!(one.stops(at(Mon, 12, 0), N) && !one.boards(at(Mon, 12, 0), N));
        assert!(one.boards(at(Mon, 12, 0), S));

        let r = parse("Temporarily closed", "1");
        assert!(r.closed && r.rules.is_empty());

        let r = parse("A-to Manhattan only, racing days only", "A");
        assert!(!r.rule(svc(Line::A)).unwrap().stops_any(at(Sat, 12, 0)));
    }

    #[test]
    fn shuttle_descriptions() {
        let r = parse(
            "S to Times Sq-all times exc nights, closed nights-use 7",
            "S",
        );
        let s = r.rule(svc(Line::S42)).unwrap();
        assert!(s.stops_any(at(Mon, 12, 0)) && !s.stops_any(at(Mon, 2, 0)));

        // Lefferts branch station
        let r = parse_notes(
            "A-all times, S Euclid Av to Ozone Park-Lefferts Blvd-nights",
            "A-S",
            40.684,
            -73.832,
        );
        assert!(r.warnings.is_empty());
        assert!(r
            .rule(svc(Line::SLefferts))
            .unwrap()
            .stops_any(at(Mon, 2, 0)));
    }
}
