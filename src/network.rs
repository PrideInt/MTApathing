//! Stations, station complexes, and the time-dependent graph. This is the port
//! of `MetroGraph.generateGraph`, `Node`, `Edge` and `Graph`.
//!
//! Each CSV row is a platform with its own coordinates and lines. Rows that
//! form one in-system transfer complex (e.g., the 4 Times Sq rows) are
//! grouped into a `Complex`, and complexes are the nodes of the graph. Like the
//! Java version, an edge joins two complexes when a train serves both of them
//! at the requested time. Its weight is the Haversine distance in km.

use std::collections::HashMap;

use crate::rules::{parse_notes, Heading, StationRules};
use crate::service::Service;
use crate::time::When;

pub const EMBEDDED_CSV: &str = include_str!("../data/mta_stations.csv");

/// Same-name platforms closer than this are treated as one complex.
const SAME_NAME_RADIUS_KM: f64 = 0.16;

/// Free transfers the radius rule can't find: complexes whose platforms have
/// different names, or same-name platforms that are further apart. Most are
/// in-system; Lexington Av 59 St / 63 St is a free out-of-system transfer.
/// Each entry pairs two CSV rows, identified by (NAME, LINE).
/// A CSV row identified by (NAME, LINE).
type RowKey = (&'static str, &'static str);

const TRANSFER_LINKS: &[(RowKey, RowKey)] = &[
    (
        ("Grand Central - 42nd St", "S"),
        ("Grand Central - 42nd St", "4-5-6-6 Express"),
    ),
    (
        ("Times Sq - 42nd St", "1-2-3"),
        ("42nd St - Port Authority Bus Term", "A-C-E"),
    ),
    (
        ("Brooklyn Bridge - City Hall", "4-5-6-6 Express"),
        ("Chambers St", "J-Z"),
    ),
    (("Franklin Ave", "2-3-4-5"), ("Botanic Garden", "S")),
    (("Franklin Ave - Fulton St", "S"), ("Franklin Ave", "A-C")),
    (
        ("42nd St - Bryant Pk", "B-D-F-M"),
        ("5th Ave - Bryant Pk", "7-7 Express"),
    ),
    (("Court Sq - 23rd St", "E-M"), ("Court Sq", "7-7 Express")),
    (
        ("Court Sq", "7-7 Express"),
        ("Long Island City - Court Sq", "G"),
    ),
    (("Lorimer St", "L"), ("Metropolitan Ave", "G")),
    (
        ("W 4th St - Washington Sq (Lower)", "B-D-F-M"),
        ("W 4th St - Washington Sq (Upper)", "A-C-E"),
    ),
    (
        ("51st St", "4-6-6 Express"),
        ("Lexington Ave - 53rd St", "E-M"),
    ),
    (
        ("74th St - Broadway", "7"),
        ("Jackson Hts - Roosevelt Av", "E-F-M-R"),
    ),
    (("62nd St", "D"), ("New Utrecht Ave", "N")),
    (("Chambers St", "A-C"), ("World Trade Center", "E")),
    (("World Trade Center", "E"), ("Park Pl", "2-3")),
    (("Fulton St", "4-5"), ("Cortlandt St", "R-W")),
    (
        ("Broadway - Lafayette St", "B-D-F-M"),
        ("Bleecker St", "4-6-6 Express"),
    ),
    (("14th St", "F-M"), ("6th Ave", "L")),
    (("6th Ave", "L"), ("14th St", "1-2-3")),
    (("8th Ave", "L"), ("14th St", "A-C-E")),
    (("South Ferry", "1"), ("Whitehall St", "R-W")),
    (("Livonia Ave", "L"), ("Junius St", "3-4")),
    (("Court St", "N-R"), ("Borough Hall", "2-3")),
    (
        ("Lexington Ave - 59th St", "4-5-6-6 Express"),
        ("Lexington Ave - 63rd St", "F-Q"),
    ),
];

/// Same-name platforms that are close together but are separate stations.
const NOT_LINKED: &[(RowKey, RowKey)] = &[(("Rector St", "1"), ("Rector St", "R-W"))];

#[derive(Debug, Clone)]
pub struct StationRow {
    pub id: usize,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub line_col: String,
    pub notes: String,
    pub rules: StationRules,
}

impl StationRow {
    pub fn pos(&self) -> (f64, f64) {
        (self.lat, self.lon)
    }
}

#[derive(Debug, Clone)]
pub struct Complex {
    pub id: usize,
    pub name: String,
    pub label: String,
    pub aka: Vec<String>,
    pub rows: Vec<usize>,
    pub lat: f64,
    pub lon: f64,
    pub services: Vec<Service>,
}

pub struct Network {
    pub rows: Vec<StationRow>,
    pub complexes: Vec<Complex>,
    pub row_complex: Vec<usize>,
    pub by_service: HashMap<Service, Vec<usize>>,
    pub warnings: Vec<String>,
}

/// Haversine distance in kilometres between two (lat, lon) points.
/// Port of `Node.distance`.
pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = 0.5 - dlat.cos() / 2.0 + lat1.cos() * lat2.cos() * (1.0 - dlon.cos()) / 2.0;
    2.0 * 6371.0 * h.sqrt().asin()
}

/// Minimal CSV reader since Rust doesn't have a built-in one like Java without a package.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();

    for line in text.lines() {
        let line = line.trim_end_matches('\r');

        if line.trim().is_empty() {
            continue;
        }
        let mut fields = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        let mut chars = line.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    cur.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => fields.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
        fields.push(cur);
        rows.push(fields);
    }
    rows
}

fn parse_point(geom: &str) -> Option<(f64, f64)> {
    let inner = geom.split_once('(')?.1.trim_end_matches(')');
    let mut it = inner.split_whitespace();
    let lon: f64 = it.next()?.parse().ok()?;
    let lat: f64 = it.next()?.parse().ok()?;

    Some((lat, lon))
}

struct UnionFind(Vec<usize>);
impl UnionFind {
    fn find(&mut self, x: usize) -> usize {
        let p = self.0[x];

        if p == x {
            x
        } else {
            let r = self.find(p);
            self.0[x] = r;
            r
        }
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));

        if ra != rb {
            self.0[rb.max(ra)] = ra.min(rb);
        }
    }
}

impl Network {
    pub fn embedded() -> Network {
        Network::from_csv(EMBEDDED_CSV).expect("embedded station data is valid")
    }

    pub fn from_csv(text: &str) -> Result<Network, String> {
        let table = parse_csv(text);
        let header = table.first().ok_or("empty CSV")?;
        let col = |name: &str| {
            header
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))
                .ok_or(format!("CSV is missing the {} column", name))
        };
        let (c_name, c_geom, c_line, c_notes) =
            (col("NAME")?, col("the_geom")?, col("LINE")?, col("NOTES")?);

        let mut rows = Vec::new();
        let mut warnings = Vec::new();

        for (i, rec) in table.iter().enumerate().skip(1) {
            let get = |c: usize| rec.get(c).map(|s| s.trim()).unwrap_or("");
            let name = get(c_name);

            if name.is_empty() {
                continue;
            }
            let (lat, lon) = parse_point(get(c_geom)).ok_or(format!(
                "line {}: bad coordinates '{}'",
                i + 1,
                get(c_geom)
            ))?;
            let rules = parse_notes(get(c_notes), get(c_line), lat, lon);

            for w in &rules.warnings {
                warnings.push(format!("{} [{}]: {}", name, get(c_line), w));
            }
            rows.push(StationRow {
                id: rows.len(),
                name: name.to_string(),
                lat,
                lon,
                line_col: get(c_line).to_string(),
                notes: get(c_notes).to_string(),
                rules,
            });
        }

        // Group platforms into complexes.
        let n = rows.len();
        let mut uf = UnionFind((0..n).collect());

        for i in 0..n {
            for j in (i + 1)..n {
                let ki = (rows[i].name.as_str(), rows[i].line_col.as_str());
                let kj = (rows[j].name.as_str(), rows[j].line_col.as_str());
                let excluded = NOT_LINKED
                    .iter()
                    .any(|&(a, b)| (a == ki && b == kj) || (a == kj && b == ki));

                if rows[i].name == rows[j].name
                    && !excluded
                    && haversine_km(rows[i].pos(), rows[j].pos()) <= SAME_NAME_RADIUS_KM
                {
                    uf.union(i, j);
                }
            }
        }
        let find_row = |name: &str, line: &str| {
            rows.iter()
                .position(|r| r.name == name && r.line_col == line)
        };
        for ((n1, l1), (n2, l2)) in TRANSFER_LINKS {
            match (find_row(n1, l1), find_row(n2, l2)) {
                (Some(a), Some(b)) => uf.union(a, b),
                _ => warnings.push(format!(
                    "transfer link not found in data: {} [{}] <-> {} [{}]",
                    n1, l1, n2, l2
                )),
            }
        }

        let mut root_to_complex: HashMap<usize, usize> = HashMap::new();
        let mut complexes: Vec<Complex> = Vec::new();
        let mut row_complex = vec![0; n];

        for (i, rc) in row_complex.iter_mut().enumerate() {
            let root = uf.find(i);
            let cid = *root_to_complex.entry(root).or_insert_with(|| {
                complexes.push(Complex {
                    id: complexes.len(),
                    name: String::new(),
                    label: String::new(),
                    aka: vec![],
                    rows: vec![],
                    lat: 0.0,
                    lon: 0.0,
                    services: vec![],
                });
                complexes.len() - 1
            });
            complexes[cid].rows.push(i);
            *rc = cid;
        }
        for c in &mut complexes {
            let k = c.rows.len() as f64;
            c.lat = c.rows.iter().map(|&r| rows[r].lat).sum::<f64>() / k;
            c.lon = c.rows.iter().map(|&r| rows[r].lon).sum::<f64>() / k;

            let mut svcs: Vec<Service> = c
                .rows
                .iter()
                .flat_map(|&r| rows[r].rules.services())
                .collect();
            svcs.sort();
            svcs.dedup();
            c.services = svcs;

            let mut names: Vec<(&str, usize)> = Vec::new();

            for &r in &c.rows {
                let cnt = rows[r].rules.rules.len();
                match names.iter_mut().find(|(nm, _)| *nm == rows[r].name) {
                    Some(e) => e.1 += cnt,
                    None => names.push((&rows[r].name, cnt)),
                }
            }
            let best = names
                .iter()
                .max_by_key(|(_, c)| *c)
                .map(|(n, _)| *n)
                .unwrap_or("");
            c.name = best.to_string();
            c.aka = names
                .iter()
                .map(|(n, _)| n.to_string())
                .filter(|n| n != best)
                .collect();
        }
        let mut count: HashMap<String, usize> = HashMap::new();

        for c in &complexes {
            *count.entry(c.name.clone()).or_default() += 1;
        }
        for c in &mut complexes {
            c.label = if count[&c.name] > 1 {
                let mut labels: Vec<&str> = c.services.iter().map(|s| s.label()).collect();
                labels.dedup();
                format!("{} ({})", c.name, labels.join(" "))
            } else {
                c.name.clone()
            };
        }
        let mut by_service: HashMap<Service, Vec<usize>> = HashMap::new();

        for r in &rows {
            for s in r.rules.services() {
                by_service.entry(s).or_default().push(r.id);
            }
        }

        Ok(Network {
            rows,
            complexes,
            row_complex,
            by_service,
            warnings,
        })
    }

    pub fn complex(&self, id: usize) -> &Complex {
        &self.complexes[id]
    }

    /// Services at a complex with whether each one stops there at `when`.
    pub fn services_at(&self, cid: usize, when: When) -> Vec<(Service, bool)> {
        let c = &self.complexes[cid];

        c.services
            .iter()
            .map(|&s| {
                let running = c.rows.iter().any(|&r| {
                    self.rows[r]
                        .rules
                        .rule(s)
                        .is_some_and(|rule| rule.stops_any(when))
                });
                (s, running)
            })
            .collect()
    }

    /// Port of `generateGraph`: build the graph of what's running at `when`.
    pub fn graph_at(&self, when: When) -> Graph<'_> {
        let nc = self.complexes.len();
        let mut pairs: HashMap<(usize, usize), (f64, Vec<Service>)> = HashMap::new();
        let mut services: Vec<&Service> = self.by_service.keys().collect();

        services.sort();

        for &s in services {
            let rows = &self.by_service[&s];

            for &a in rows {
                let ra = &self.rows[a];
                let rule_a = ra.rules.rule(s).expect("indexed");

                for &b in rows {
                    let (ca, cb) = (self.row_complex[a], self.row_complex[b]);

                    if ca == cb {
                        continue;
                    }
                    let rb = &self.rows[b];
                    let h = Heading::between(ra.pos(), rb.pos());

                    if !rule_a.boards(when, h) {
                        continue;
                    }
                    if !rb.rules.rule(s).expect("indexed").stops(when, h) {
                        continue;
                    }
                    let km = haversine_km(ra.pos(), rb.pos());
                    let e = pairs.entry((ca, cb)).or_insert((f64::INFINITY, Vec::new()));
                    e.0 = e.0.min(km);

                    if !e.1.contains(&s) {
                        e.1.push(s);
                    }
                }
            }
        }
        let mut adj: Vec<Vec<Edge>> = (0..nc).map(|_| Vec::new()).collect();
        let mut active = vec![false; nc];
        let mut edge_count = 0;
        let mut keys: Vec<_> = pairs.into_iter().collect();
        keys.sort_by_key(|((a, b), _)| (*a, *b));

        for ((a, b), (km, mut rides)) in keys {
            rides.sort();
            active[a] = true;
            active[b] = true;
            adj[a].push(Edge { to: b, km, rides });
            edge_count += 1;
        }
        Graph {
            net: self,
            when,
            adj,
            active,
            edge_count,
        }
    }
}

/// An edge `from -> to`: some train runs between them at this time. Port of `Edge`.
#[derive(Debug, Clone)]
pub struct Edge {
    pub to: usize,
    pub km: f64,
    pub rides: Vec<Service>,
}

/// The graph G = {V, E} as an adjacency list, for one moment in the week.
pub struct Graph<'a> {
    pub net: &'a Network,
    pub when: When,
    pub adj: Vec<Vec<Edge>>,
    pub active: Vec<bool>,
    pub edge_count: usize,
}

impl Graph<'_> {
    pub fn node_count(&self) -> usize {
        self.active.iter().filter(|a| **a).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Kind, Line};
    use crate::time::Weekday;

    fn net() -> Network {
        Network::embedded()
    }

    fn find(net: &Network, label: &str) -> usize {
        net.complexes
            .iter()
            .position(|c| c.label == label)
            .unwrap_or_else(|| panic!("no complex labelled {label}"))
    }

    #[test]
    fn loads_all_rows_without_parser_warnings() {
        let n = net();

        assert_eq!(n.rows.len(), 473);
        assert!(n.warnings.is_empty(), "{:#?}", n.warnings);
    }

    #[test]
    fn complexes_group_transfers_but_keep_distinct_stations_apart() {
        let n = net();
        let ts = find(&n, "Times Sq - 42nd St");
        let ts_c = n.complex(ts);

        assert_eq!(ts_c.rows.len(), 5);
        assert!(ts_c.aka.iter().any(|a| a.contains("Port Authority")));

        let n86 = n.complexes.iter().filter(|c| c.name == "86th St").count();

        assert_eq!(n86, 5);
        assert!(n.complexes.iter().any(|c| c.label == "86th St (4 5 6)"));

        let chambers: Vec<_> = n
            .complexes
            .iter()
            .filter(|c| c.name == "Chambers St" || c.aka.iter().any(|a| a == "Chambers St"))
            .collect();

        assert!(chambers.len() >= 2);
        assert_eq!(
            n.complexes.iter().filter(|c| c.name == "Rector St").count(),
            2
        );
    }

    #[test]
    fn haversine_is_correct_order_of_magnitude() {
        let d = haversine_km((40.75598, -73.98623), (40.75277, -73.97919));
        assert!((0.6..0.7).contains(&d), "{d}");
    }

    #[test]
    fn graph_depends_on_time() {
        let n = net();
        let day = n.graph_at(When::new(Weekday::Tue, 12 * 60));
        let night = n.graph_at(When::new(Weekday::Tue, 3 * 60));

        assert_ne!(day.edge_count, night.edge_count);

        let ts = find(&n, "Times Sq - 42nd St");
        let gc = find(&n, "Grand Central - 42nd St");
        let s42 = Service::new(Line::S42, Kind::Local);

        assert!(day.adj[ts]
            .iter()
            .any(|e| e.to == gc && e.rides.contains(&s42)));
        assert!(!night.adj[ts]
            .iter()
            .any(|e| e.to == gc && e.rides.contains(&s42)));
    }

    #[test]
    fn shuttles_do_not_connect_to_each_other() {
        let n = net();
        let g = n.graph_at(When::new(Weekday::Sat, 12 * 60));
        let ts = find(&n, "Times Sq - 42nd St");
        let rp = n
            .complexes
            .iter()
            .position(|c| c.name.starts_with("Rockaway Park"))
            .unwrap();

        assert!(!g.adj[ts].iter().any(|e| e.to == rp));
    }
}
