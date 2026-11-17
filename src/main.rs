use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;

use mta_graphing::network::Network;
use mta_graphing::render::{self, span, theme, ColorMode, Glyphs, Line, Style};
use mta_graphing::route::{find_route, Mode, RouteError};
use mta_graphing::search::{resolve, search, Resolve};
use mta_graphing::time::{parse_time, Weekday, When};

mod tui;

const HELP: &str = "\
mta: NYC subway route finder (Rust port of mta-graphing)

USAGE
  mta                          open the terminal UI
  mta route <FROM> <TO>        print the best route
  mta stations [QUERY]         list or search stations
  mta classic                  the original program's prompts and output
  mta check                    show how each station's service notes were parsed

OPTIONS
  -d, --day <DAY>      monday..sunday or mon..sun     (default: now in NYC)
  -t, --time <HHMM>    military time, e.g. 0830, 2300 (default: now in NYC)
  -m, --mode <MODE>    fastest | shortest | fewest    (default: fastest)
  -g, --glyphs <G>     unicode | nerd | ascii         (bullet style; or set MTA_GLYPHS)
      --no-color       plain text output
      --data <CSV>     use another stations CSV instead of the built-in one
  -h, --help           show this help

EXAMPLES
  mta route \"times sq\" \"atlantic av\" --day sat --time 2300
  mta route \"pelham bay park\" \"grand central\" -d tue -t 0800 -m shortest
  mta stations 86
";

struct Opts {
    cmd: String,
    positional: Vec<String>,
    day: Option<Weekday>,
    time: Option<u16>,
    mode: Mode,
    glyphs: Glyphs,
    color: ColorMode,
    data: Option<String>,
}

fn parse_args() -> Result<Opts, String> {
    let mut args = std::env::args().skip(1).peekable();
    let mut o = Opts {
        cmd: String::new(),
        positional: vec![],
        day: None,
        time: None,
        mode: Mode::Fastest,
        glyphs: std::env::var("MTA_GLYPHS")
            .ok()
            .and_then(|g| Glyphs::parse(&g))
            .unwrap_or(Glyphs::Unicode),
        color: ColorMode::detect(),
        data: None,
    };
    if !io::stdout().is_terminal() {
        o.color = ColorMode::None;
    }
    while let Some(a) = args.next() {
        let mut val = |name: &str| args.next().ok_or(format!("{} needs a value", name));
        match a.as_str() {
            "-h" | "--help" | "help" => o.cmd = "help".into(),
            "-d" | "--day" => o.day = Some(Weekday::parse(&val(&a)?).map_err(|e| e.to_string())?),
            "-t" | "--time" => o.time = Some(parse_time(&val(&a)?).map_err(|e| e.to_string())?),
            "-m" | "--mode" => {
                let v = val(&a)?;
                o.mode = Mode::parse(&v).ok_or(format!("unknown mode '{}'", v))?
            }
            "-g" | "--glyphs" => {
                let v = val(&a)?;
                o.glyphs = Glyphs::parse(&v).ok_or(format!("unknown glyph style '{}'", v))?
            }
            "--ascii" => o.glyphs = Glyphs::Ascii,
            "--no-color" => o.color = ColorMode::None,
            "--data" => o.data = Some(val(&a)?),
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(format!("unknown option '{}'", s))
            }
            _ if o.cmd.is_empty() => o.cmd = a,
            _ => o.positional.push(a),
        }
    }
    Ok(o)
}

fn load(o: &Opts) -> Result<Network, String> {
    match &o.data {
        None => Ok(Network::embedded()),
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {}", p, e))?;
            Network::from_csv(&text)
        }
    }
}

fn when_from(o: &Opts) -> When {
    let now = When::now_nyc();
    When::new(o.day.unwrap_or(now.day), o.time.unwrap_or(now.minutes))
}

fn print_lines(lines: &[Line], o: &Opts) {
    print!("{}", render::to_ansi(lines, o.color));
}

fn station_line(net: &Network, cid: usize, when: When, o: &Opts) -> Line {
    let mut l: Line = vec![span("  ", Style::PLAIN)];
    l.extend(render::bullets(&net.services_at(cid, when), o.glyphs));

    let w = render::line_width(&l);
    l.push(span(
        " ".repeat(30usize.saturating_sub(w).max(2)),
        Style::PLAIN,
    ));
    l.push(span(net.complex(cid).label.clone(), Style::fg(theme::TEXT)));
    l
}

fn pick(net: &Network, q: &str, when: When, o: &Opts) -> Result<usize, String> {
    match resolve(net, q) {
        Resolve::Found(c) => Ok(c),
        Resolve::NotFound => Err(format!("no station matches '{}'", q)),
        Resolve::Ambiguous(v) => {
            let mut lines: Vec<Line> = vec![vec![span(
                format!(
                    "'{}' matches several stations; use the full name in quotes:",
                    q
                ),
                Style::fg(theme::AMBER),
            )]];
            for c in v {
                lines.push(station_line(net, c, when, o));
            }
            print_lines(&lines, o);
            Err("ambiguous station".into())
        }
    }
}

fn cmd_route(o: &Opts) -> Result<(), String> {
    if o.positional.len() != 2 {
        return Err("usage: mta route <FROM> <TO> [--day D] [--time HHMM] [--mode M]".into());
    }
    let net = load(o)?;
    let when = when_from(o);
    let from = pick(&net, &o.positional[0], when, o)?;
    let to = pick(&net, &o.positional[1], when, o)?;
    let g = net.graph_at(when);

    let head: Line = vec![
        span(" mta ", Style::fg(theme::BG).on(theme::AMBER).bold()),
        span(
            format!(
                " {} {} · {} · {}",
                when.day.short(),
                when.clock(),
                when.period().label(),
                o.mode.flag()
            ),
            Style::fg(theme::DIM),
        ),
    ];
    let mut lines = vec![head, vec![]];

    match find_route(&g, from, to, o.mode) {
        Ok(r) => {
            lines.extend(render::route_lines(&net, &r, o.glyphs));
            lines.push(vec![]);
            lines.push(vec![span(
                format!(" {}", render::summary(&r)),
                Style::fg(theme::TEXT).bold(),
            )]);
            print_lines(&lines, o);
            Ok(())
        }
        Err(e) => {
            print_lines(&lines, o);
            Err(route_error(&net, e, when))
        }
    }
}

fn route_error(net: &Network, e: RouteError, when: When) -> String {
    match e {
        RouteError::SameStation => "origin and destination are the same station".into(),
        RouteError::NoService(c) => format!(
            "no trains stop at {} on {} at {} ({})",
            net.complex(c).label,
            when.day.long(),
            when.clock(),
            when.period().label()
        ),
        RouteError::Unreachable => format!(
            "no route at {} {}; some lines don't run at this time",
            when.day.short(),
            when.clock()
        ),
    }
}

fn cmd_stations(o: &Opts) -> Result<(), String> {
    let net = load(o)?;
    let when = when_from(o);
    let q = o.positional.join(" ");
    let hits = search(&net, &q);

    if hits.is_empty() {
        return Err(format!("no station matches '{}'", q));
    }
    let lines: Vec<Line> = hits
        .iter()
        .map(|&c| station_line(&net, c, when, o))
        .collect();

    print_lines(&lines, o);
    Ok(())
}

/// The interface
fn cmd_classic(o: &Opts) -> Result<(), String> {
    let net = load(o)?;
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut ask = |prompt: &str| -> Result<String, String> {
        print!("{}", prompt);
        io::stdout().flush().ok();

        match lines.next() {
            Some(Ok(l)) => Ok(l.trim().to_string()),
            _ => Err("no input".into()),
        }
    };
    let time = parse_time(&ask("Input a time: ")?).map_err(|e| e.to_string())?;
    let day = Weekday::parse(&ask("\nInput a day: ")?).map_err(|e| e.to_string())?;
    let when = When::new(day, time);
    let g = net.graph_at(when);
    let resolve_one = |q: &str| -> Result<usize, String> {
        match resolve(&net, q) {
            Resolve::Found(c) => Ok(c),
            Resolve::Ambiguous(v) => Err(format!(
                "Ambiguous station '{}'. Did you mean: {}",
                q,
                v.iter()
                    .map(|&c| net.complex(c).label.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            Resolve::NotFound => Err("Invalid origin or destination station.".into()),
        }
    };
    let origin = resolve_one(&ask("\nInput an origin: ")?)?;
    let dest = resolve_one(&ask("\nInput a destination: ")?)?;

    for (title, mode) in [
        ("Shortest Path", Mode::Shortest),
        ("Shortest Walk Path", Mode::FewestTransfers),
    ] {
        match find_route(&g, origin, dest, mode) {
            Ok(r) => println!("{}: {}", title, r.to_classic_string(&net)),
            Err(e) => println!("{}: {}", title, route_error(&net, e, when)),
        }
    }
    Ok(())
}

fn cmd_check(o: &Opts) -> Result<(), String> {
    let net = load(o)?;

    for w in &net.warnings {
        println!("warning: {}", w);
    }
    let when = when_from(o);

    for c in &net.complexes {
        let mut l: Line = vec![span(format!("{:<44}", c.label), Style::fg(theme::TEXT))];
        l.extend(render::bullets(&net.services_at(c.id, when), o.glyphs));
        
        print_lines(&[l], o);

        for &r in &c.rows {
            let row = &net.rows[r];
            println!("    [{}] {}", row.line_col, row.notes);
        }
    }
    println!(
        "\n{} platforms, {} station complexes, {} warnings. Bullets show service at {} {}.",
        net.rows.len(),
        net.complexes.len(),
        net.warnings.len(),
        when.day.short(),
        when.clock()
    );
    Ok(())
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("mta: {}\n\n{}", e, HELP);
            return ExitCode::from(2);
        }
    };
    let result = match o.cmd.as_str() {
        "help" => {
            print!("{}", HELP);
            Ok(())
        }
        "route" | "r" => cmd_route(&o),
        "stations" | "s" | "ls" => cmd_stations(&o),
        "classic" => cmd_classic(&o),
        "check" => cmd_check(&o),
        "" | "ui" => {
            if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
                Err("the UI needs an interactive terminal; try `mta route <FROM> <TO>` or `mta --help`".into())
            } else {
                load(&o).and_then(|net| {
                    tui::run(
                        net,
                        tui::Config {
                            glyphs: o.glyphs,
                            color: if o.color == ColorMode::None {
                                ColorMode::detect()
                            } else {
                                o.color
                            },
                            day: o.day,
                            time: o.time,
                            mode: o.mode,
                        },
                    )
                    .map_err(|e| format!("terminal error: {}", e))
                })
            }
        }
        other => Err(format!("unknown command '{}'. Try `mta --help`.", other)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mta: {}", e);
            ExitCode::FAILURE
        }
    }
}
