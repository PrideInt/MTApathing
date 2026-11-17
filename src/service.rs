//! Train services (the original `Train` record), their official MTA colors, and
//! bullet shapes: circles for local service and diamonds for express.

use std::fmt;

/// An RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Subway routes in the dataset. The four shuttles all carry the letter "S" on
/// the map but are separate lines, so each gets its own variant. The Java
/// version treated them as one line, which joined Times Sq to Rockaway Park.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Line {
    L1,
    L2,
    L3,
    L4,
    L5,
    L6,
    L7,
    A,
    C,
    E,
    B,
    D,
    F,
    M,
    G,
    J,
    Z,
    L,
    N,
    Q,
    R,
    W,
    /// 42 St Shuttle (Times Sq – Grand Central)
    S42,
    /// Franklin Av Shuttle
    SFranklin,
    /// Rockaway Park Shuttle
    SRockaway,
    /// Late-night Lefferts Blvd shuttle (A branch)
    SLefferts,
}

/// Port of `Train.TrainType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Local,
    Express,
}

/// Port of `Train.Bound`. A heading on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bound {
    North,
    South,
    East,
    West,
}

impl Bound {
    pub fn parse_word(w: &str) -> Option<Bound> {
        match w.to_ascii_lowercase().as_str() {
            "northbound" => Some(Bound::North),
            "southbound" => Some(Bound::South),
            "eastbound" => Some(Bound::East),
            "westbound" => Some(Bound::West),
            _ => None,
        }
    }
}

/// A train service: a line plus local/express (e.g. the 6 local vs. the <6>).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Service {
    pub line: Line,
    pub kind: Kind,
}

impl Line {
    /// Resolve a token from the CSV ("4", "A", "S", …). Shuttles are told apart
    /// by the station's location.
    pub fn from_token(tok: &str, lat: f64, lon: f64) -> Option<Line> {
        Some(match tok.trim() {
            "1" => Line::L1,
            "2" => Line::L2,
            "3" => Line::L3,
            "4" => Line::L4,
            "5" => Line::L5,
            "6" => Line::L6,
            "7" => Line::L7,
            "A" => Line::A,
            "B" => Line::B,
            "C" => Line::C,
            "D" => Line::D,
            "E" => Line::E,
            "F" => Line::F,
            "G" => Line::G,
            "J" => Line::J,
            "L" => Line::L,
            "M" => Line::M,
            "N" => Line::N,
            "Q" => Line::Q,
            "R" => Line::R,
            "W" => Line::W,
            "Z" => Line::Z,
            "S" => {
                if lat < 40.63 {
                    Line::SRockaway
                } else if lat > 40.74 {
                    Line::S42
                } else if lon < -73.92 {
                    Line::SFranklin
                } else {
                    Line::SLefferts
                }
            }
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Line::L1 => "1",
            Line::L2 => "2",
            Line::L3 => "3",
            Line::L4 => "4",
            Line::L5 => "5",
            Line::L6 => "6",
            Line::L7 => "7",
            Line::A => "A",
            Line::B => "B",
            Line::C => "C",
            Line::D => "D",
            Line::E => "E",
            Line::F => "F",
            Line::G => "G",
            Line::J => "J",
            Line::L => "L",
            Line::M => "M",
            Line::N => "N",
            Line::Q => "Q",
            Line::R => "R",
            Line::W => "W",
            Line::Z => "Z",
            Line::S42 | Line::SFranklin | Line::SRockaway | Line::SLefferts => "S",
        }
    }

    pub fn is_shuttle(self) -> bool {
        matches!(
            self,
            Line::S42 | Line::SFranklin | Line::SRockaway | Line::SLefferts
        )
    }

    /// Long name used in descriptions.
    pub fn name(self) -> &'static str {
        match self {
            Line::S42 => "42 St Shuttle",
            Line::SFranklin => "Franklin Av Shuttle",
            Line::SRockaway => "Rockaway Park Shuttle",
            Line::SLefferts => "Lefferts Blvd Shuttle",
            _ => "",
        }
    }

    /// Official MTA trunk-line colors.
    pub fn color(self) -> Rgb {
        match self {
            Line::L1 | Line::L2 | Line::L3 => Rgb(0xEE, 0x35, 0x2E), // 7th Av: red
            Line::L4 | Line::L5 | Line::L6 => Rgb(0x00, 0x93, 0x3C), // Lexington Av: green
            Line::L7 => Rgb(0xB9, 0x33, 0xAD),                       // Flushing: purple
            Line::A | Line::C | Line::E => Rgb(0x00, 0x39, 0xA6),    // 8th Av: blue
            Line::B | Line::D | Line::F | Line::M => Rgb(0xFF, 0x63, 0x19), // 6th Av: orange
            Line::G => Rgb(0x6C, 0xBE, 0x45),                        // Crosstown: lime
            Line::J | Line::Z => Rgb(0x99, 0x66, 0x33),              // Nassau St: brown
            Line::L => Rgb(0xA7, 0xA9, 0xAC),                        // Canarsie: light grey
            Line::N | Line::Q | Line::R | Line::W => Rgb(0xFC, 0xCC, 0x0A), // Broadway: yellow
            _ => Rgb(0x80, 0x81, 0x83),                              // Shuttles: dark grey
        }
    }

    /// Letter color on the bullet: black on yellow, white everywhere else.
    pub fn text_color(self) -> Rgb {
        match self {
            Line::N | Line::Q | Line::R | Line::W => Rgb(0x00, 0x00, 0x00),
            _ => Rgb(0xFF, 0xFF, 0xFF),
        }
    }
}

impl Service {
    pub fn new(line: Line, kind: Kind) -> Service {
        Service { line, kind }
    }
    pub fn local(line: Line) -> Service {
        Service::new(line, Kind::Local)
    }
    pub fn is_express(self) -> bool {
        self.kind == Kind::Express
    }
    pub fn label(self) -> &'static str {
        self.line.label()
    }
    pub fn color(self) -> Rgb {
        self.line.color()
    }
    pub fn text_color(self) -> Rgb {
        self.line.text_color()
    }

    /// Text form from the Java `toString`: "(4)" for local, "<6>" for express.
    pub fn ascii_bullet(self) -> String {
        match self.kind {
            Kind::Local => format!("({})", self.label()),
            Kind::Express => format!("<{}>", self.label()),
        }
    }

    /// Long description, e.g. "6 Express", "42 St Shuttle", "A".
    pub fn describe(self) -> String {
        if self.line.is_shuttle() {
            return self.line.name().to_string();
        }
        match self.kind {
            Kind::Local => self.label().to_string(),
            Kind::Express => format!("{} Express", self.label()),
        }
    }
}

impl fmt::Display for Service {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.ascii_bullet())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuttles_are_distinguished_by_location() {
        // Times Sq
        assert_eq!(Line::from_token("S", 40.756, -73.986), Some(Line::S42));
        // Botanic Garden
        assert_eq!(
            Line::from_token("S", 40.670, -73.959),
            Some(Line::SFranklin)
        );
        // Beach 90th St
        assert_eq!(
            Line::from_token("S", 40.588, -73.814),
            Some(Line::SRockaway)
        );
        // 111th St (Liberty Av)
        assert_eq!(
            Line::from_token("S", 40.684, -73.832),
            Some(Line::SLefferts)
        );
    }

    #[test]
    fn ordering_follows_map_legend() {
        let mut v = [
            Service::local(Line::Q),
            Service::local(Line::A),
            Service::new(Line::L6, Kind::Express),
            Service::local(Line::L6),
            Service::local(Line::L1),
        ];
        v.sort();

        let labels: Vec<String> = v.iter().map(|s| s.ascii_bullet()).collect();
        assert_eq!(labels, ["(1)", "(6)", "<6>", "(A)", "(Q)"]);
    }
}
