//! Path finding: Dijkstra's algorithm over the time-dependent graph. This is
//! the port of `Graph.dijkstras`, `Graph.shortestWalkPath` and `Path`.
//!
//! The graph links every pair of stations a single train serves, so each edge
//! is a *ride* (board here, get off there), and a path is a list of rides with
//! transfers in between, as in the Java version. The Java `shortestWalkPath`
//! first ran Dijkstra by distance and then pruned stations from the result.
//! Here "fewest transfers" is a cost function of its own, so Dijkstra returns
//! the optimal path for it directly.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::network::{haversine_km, Graph, Network};
use crate::service::Service;
use crate::time::When;

/// The Java version's travel-time estimate: 0.35 km of straight-line
/// distance per minute.
pub const KM_PER_MIN: f64 = 0.35;
/// Estimated time to change trains (walk + wait). Used by `Fastest` and in totals.
pub const TRANSFER_MIN: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Fastest,
    Shortest,
    FewestTransfers,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Fastest, Mode::Shortest, Mode::FewestTransfers];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Fastest => "FASTEST",
            Mode::Shortest => "SHORTEST",
            Mode::FewestTransfers => "FEWEST TRANSFERS",
        }
    }
    pub fn flag(self) -> &'static str {
        match self {
            Mode::Fastest => "fastest",
            Mode::Shortest => "shortest",
            Mode::FewestTransfers => "fewest",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "fastest" | "fast" | "time" => Some(Mode::Fastest),
            "shortest" | "short" | "distance" | "dist" => Some(Mode::Shortest),
            "fewest" | "fewest-transfers" | "transfers" | "walk" => Some(Mode::FewestTransfers),
            _ => None,
        }
    }
    pub fn next(self) -> Mode {
        match self {
            Mode::Fastest => Mode::Shortest,
            Mode::Shortest => Mode::FewestTransfers,
            Mode::FewestTransfers => Mode::Fastest,
        }
    }
    pub fn prev(self) -> Mode {
        self.next().next()
    }

    fn edge_cost(self, km: f64) -> f64 {
        match self {
            Mode::Fastest => km / KM_PER_MIN + TRANSFER_MIN,
            Mode::Shortest => km + 1e-6,
            Mode::FewestTransfers => 1_000.0 + km,
        }
    }
}

/// One ride: board at `from`, stay on to `to`.
#[derive(Debug, Clone)]
pub struct Leg {
    pub from: usize,
    pub to: usize,
    pub km: f64,
    pub rides: Vec<Service>,
}

impl Leg {
    pub fn minutes(&self) -> f64 {
        self.km / KM_PER_MIN
    }
}

/// Port of `Path`: the stations of the route, linked by rides.
#[derive(Debug, Clone)]
pub struct Route {
    pub mode: Mode,
    pub when: When,
    pub legs: Vec<Leg>,
}

impl Route {
    pub fn stations(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.legs.iter().map(|l| l.from).collect();
        if let Some(l) = self.legs.last() {
            v.push(l.to);
        }
        v
    }
    pub fn transfers(&self) -> usize {
        self.legs.len().saturating_sub(1)
    }
    pub fn km(&self) -> f64 {
        self.legs.iter().map(|l| l.km).sum()
    }
    /// Ride time plus transfer time, in minutes.
    pub fn minutes(&self) -> f64 {
        self.legs.iter().map(|l| l.minutes()).sum::<f64>() + self.transfers() as f64 * TRANSFER_MIN
    }

    /// Clock times: (depart, arrive) for each leg.
    pub fn timeline(&self) -> Vec<(When, When)> {
        let mut t = 0.0;
        let mut out = Vec::new();

        for (i, l) in self.legs.iter().enumerate() {
            if i > 0 {
                t += TRANSFER_MIN;
            }
            let dep = self.when.plus_minutes(t.round() as u32);
            t += l.minutes();

            let arr = self.when.plus_minutes(t.round() as u32);
            out.push((dep, arr));
        }
        out
    }

    /// Port of `Path.toString`:
    /// `Astor Pl (6) [7.00 mins] -> Grand Central - 42nd St`.
    pub fn to_classic_string(&self, net: &Network) -> String {
        let mut s = String::new();

        for l in &self.legs {
            s.push_str(&net.complex(l.from).label);

            for r in &l.rides {
                s.push(' ');
                s.push_str(&r.ascii_bullet());
            }
            s.push_str(&format!(" [{:.2} mins] -> ", l.minutes().round()));
        }
        if let Some(l) = self.legs.last() {
            s.push_str(&net.complex(l.to).label);
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouteError {
    SameStation,
    NoService(usize),
    Unreachable,
}

#[derive(Copy, Clone, PartialEq)]
struct State {
    cost: f64,
    node: usize,
}
impl Eq for State {}
impl Ord for State {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost
            .total_cmp(&self.cost)
            .then_with(|| o.node.cmp(&self.node))
    }
}
impl PartialOrd for State {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Port of `Graph.dijkstras`. Unlike the Java version, the priority queue is
/// ordered by tentative cost, and a node is settled only once.
pub fn find_route(g: &Graph, origin: usize, dest: usize, mode: Mode) -> Result<Route, RouteError> {
    if origin == dest {
        return Err(RouteError::SameStation);
    }
    for c in [origin, dest] {
        if !g.active[c] {
            return Err(RouteError::NoService(c));
        }
    }
    let n = g.adj.len();
    let mut dist = vec![f64::INFINITY; n];
    let mut prev: Vec<Option<(usize, usize)>> = vec![None; n]; // (from node, edge index)
    let mut done = vec![false; n];
    let mut heap = BinaryHeap::new();

    dist[origin] = 0.0;

    heap.push(State {
        cost: 0.0,
        node: origin,
    });

    while let Some(State { cost, node }) = heap.pop() {
        if done[node] {
            continue;
        }
        done[node] = true;

        if node == dest {
            break;
        }
        for (ei, e) in g.adj[node].iter().enumerate() {
            if done[e.to] {
                continue;
            }
            let nd = cost + mode.edge_cost(e.km);

            if nd < dist[e.to] {
                dist[e.to] = nd;
                prev[e.to] = Some((node, ei));
                heap.push(State {
                    cost: nd,
                    node: e.to,
                });
            }
        }
    }

    if !done[dest] {
        return Err(RouteError::Unreachable);
    }
    let mut legs = Vec::new();
    let mut cur = dest;

    while let Some((p, ei)) = prev[cur] {
        let e = &g.adj[p][ei];

        legs.push(Leg {
            from: p,
            to: cur,
            km: e.km,
            rides: e.rides.clone(),
        });
        cur = p;
    }
    legs.reverse();
    Ok(Route {
        mode,
        when: g.when,
        legs,
    })
}

/// Straight-line distance between two complexes, for display.
pub fn crow_km(net: &Network, a: usize, b: usize) -> f64 {
    let (ca, cb) = (net.complex(a), net.complex(b));
    haversine_km((ca.lat, ca.lon), (cb.lat, cb.lon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Kind, Line};
    use crate::time::Weekday::{self, *};

    fn id(net: &Network, label: &str) -> usize {
        net.complexes
            .iter()
            .position(|c| c.label == label)
            .unwrap_or_else(|| panic!("no complex {label}"))
    }

    fn route(from: &str, to: &str, day: Weekday, hhmm: u16, mode: Mode) -> (Network, Route) {
        let net = Network::embedded();
        let g = net.graph_at(When::new(day, hhmm / 100 * 60 + hhmm % 100));
        let r = find_route(&g, id(&net, from), id(&net, to), mode).expect("route");

        drop(g);
        (net, r)
    }

    fn uses(r: &Route, line: Line) -> bool {
        r.legs
            .iter()
            .any(|l| l.rides.iter().any(|s| s.line == line))
    }

    #[test]
    fn single_seat_ride_on_one_line() {
        let (_, r) = route(
            "Times Sq - 42nd St",
            "Flushing - Main St",
            Tue,
            1200,
            Mode::Fastest,
        );
        assert_eq!(r.legs.len(), 1);
        assert!(uses(&r, Line::L7));
    }

    #[test]
    fn transfer_needed_between_unconnected_lines() {
        let (net, r) = route(
            "Greenpoint Ave",
            "Flushing - Main St",
            Tue,
            1200,
            Mode::FewestTransfers,
        );
        assert_eq!(r.legs.len(), 2, "{}", r.to_classic_string(&net));
    }

    #[test]
    fn express_6_runs_peak_direction_only() {
        let six_x = Service::new(Line::L6, Kind::Express);
        let (_, am_south) = route("Pelham Bay Park", "Parkchester", Tue, 800, Mode::Shortest);
        assert!(am_south.legs[0].rides.contains(&six_x));

        let (_, am_north) = route("Parkchester", "Pelham Bay Park", Tue, 800, Mode::Shortest);
        assert!(!am_north.legs[0].rides.contains(&six_x));

        let (_, weekend) = route("Pelham Bay Park", "Parkchester", Sat, 800, Mode::Shortest);
        assert!(!weekend.legs[0].rides.contains(&six_x));
    }

    #[test]
    fn late_night_service_differs() {
        let (_, day) = route("Spring St (A C E)", "High St", Tue, 1200, Mode::Shortest);
        assert!(uses(&day, Line::C));

        let (_, night) = route("Spring St (A C E)", "High St", Tue, 300, Mode::Shortest);
        assert!(!uses(&night, Line::C) && uses(&night, Line::A));
    }

    #[test]
    fn modes_are_consistent() {
        let net = Network::embedded();
        let g = net.graph_at(When::new(Wed, 13 * 60));
        let (a, b) = (
            id(&net, "Coney Island - Stillwell Av"),
            id(&net, "Pelham Bay Park"),
        );
        let short = find_route(&g, a, b, Mode::Shortest).unwrap();
        let fewest = find_route(&g, a, b, Mode::FewestTransfers).unwrap();
        let fast = find_route(&g, a, b, Mode::Fastest).unwrap();

        assert!(short.km() <= fewest.km() + 1e-9);
        assert!(short.km() <= fast.km() + 1e-9);
        assert!(fewest.transfers() <= short.transfers());
        assert!(fewest.transfers() <= fast.transfers());
        assert!(fast.minutes() <= short.minutes() + 1e-9);
        assert!(fast.minutes() <= fewest.minutes() + 1e-9);

        for w in short.legs.windows(2) {
            assert_eq!(w[0].to, w[1].from);
        }
    }

    #[test]
    fn closed_station_has_no_service() {
        let net = Network::embedded();
        let g = net.graph_at(When::new(Tue, 12 * 60));
        let cortlandt_1 = net
            .complexes
            .iter()
            .position(|c| c.rows.iter().all(|&r| net.rows[r].rules.closed))
            .expect("Cortlandt St (1) is closed in this dataset");
        let ts = id(&net, "Times Sq - 42nd St");

        assert_eq!(
            find_route(&g, cortlandt_1, ts, Mode::Fastest).unwrap_err(),
            RouteError::NoService(cortlandt_1)
        );
    }

    #[test]
    fn classic_format_matches_java_shape() {
        let (net, r) = route(
            "Times Sq - 42nd St",
            "Flushing - Main St",
            Tue,
            1200,
            Mode::Shortest,
        );
        let s = r.to_classic_string(&net);
        
        assert!(s.starts_with("Times Sq - 42nd St (7)"), "{s}");
        assert!(s.contains(" mins] -> Flushing - Main St"), "{s}");
    }
}
