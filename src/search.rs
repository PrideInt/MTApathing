//! Station search. The Java version required the exact station name. This one
//! accepts "times sq", "atlantic", "86 st 6" (a line label narrows the
//! results), and ordinals typed either way ("42nd" / "42").

use crate::network::Network;

/// Lowercase, strip punctuation, and normalise common abbreviations.
pub fn normalize(s: &str) -> Vec<String> {
    let cleaned: String = s
        .chars()
        .filter(|c| *c != '\'' && *c != '.')
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    cleaned
        .split_whitespace()
        .map(|w| {
            let w = match w {
                "street" => "st",
                "avenue" | "ave" => "av",
                "avs" | "aves" | "avenues" => "avs",
                "square" => "sq",
                "road" => "rd",
                "boulevard" => "blvd",
                "parkway" => "pkwy",
                "place" => "pl",
                "heights" => "hts",
                "center" | "centre" => "ctr",
                "junction" => "jct",
                other => other,
            };
            let digits = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());

            if !digits.is_empty()
                && digits.chars().all(|c| c.is_ascii_digit())
                && matches!(&w[digits.len()..], "st" | "nd" | "rd" | "th")
            {
                digits.to_string()
            } else {
                w.to_string()
            }
        })
        .collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Score how well `query` matches complex `cid`. None = no match.
fn score(net: &Network, cid: usize, q: &[String], qjoined: &str) -> Option<f64> {
    let c = net.complex(cid);
    let line_labels: Vec<String> = c
        .services
        .iter()
        .map(|s| s.label().to_ascii_lowercase())
        .collect();
    let mut best: Option<f64> = None;

    for name in std::iter::once(&c.name).chain(c.aka.iter()) {
        let words = normalize(name);
        let joined = words.join(" ");
        let mut total = 0.0;
        let mut ok = true;

        for qw in q {
            let s = words
                .iter()
                .map(|w| {
                    if w == qw {
                        3.0
                    } else if w.starts_with(qw.as_str()) {
                        2.0
                    } else if w.contains(qw.as_str()) {
                        1.0
                    } else {
                        0.0
                    }
                })
                .fold(0.0, f64::max);
            if s > 0.0 {
                total += s;
            } else if line_labels.iter().any(|l| l == qw) {
                total += 1.5;
            } else {
                ok = false;
                break;
            }
        }
        if !ok {
            if qjoined.len() >= 3
                && is_subsequence(&qjoined.replace(' ', ""), &joined.replace(' ', ""))
            {
                total = 0.5;
            } else {
                continue;
            }
        }
        if joined == qjoined {
            total += 10.0;
        } else if joined.starts_with(qjoined) {
            total += 5.0;
        }
        if name != &c.name {
            total -= 0.25;
        }
        best = Some(best.map_or(total, |b: f64| b.max(total)));
    }
    best
}

/// Complexes matching `query`, best first. An empty query lists every
/// station alphabetically.
pub fn search(net: &Network, query: &str) -> Vec<usize> {
    let q = normalize(query);
    let qjoined = q.join(" ");
    let mut hits: Vec<(f64, usize)> = (0..net.complexes.len())
        .filter_map(|cid| {
            if q.is_empty() {
                Some((0.0, cid))
            } else {
                score(net, cid, &q, &qjoined).map(|s| (s, cid))
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        b.0.total_cmp(&a.0).then_with(|| {
            let (ca, cb) = (net.complex(a.1), net.complex(b.1));

            cb.services
                .len()
                .cmp(&ca.services.len())
                .then(ca.label.cmp(&cb.label))
        })
    });
    if q.is_empty() {
        hits.sort_by(|a, b| net.complex(a.1).label.cmp(&net.complex(b.1).label));
    }
    hits.into_iter().map(|(_, c)| c).collect()
}

#[derive(Debug)]
pub enum Resolve {
    Found(usize),
    Ambiguous(Vec<usize>),
    NotFound,
}

/// Resolve user input to one station: exact label, then exact name, then a
/// search result that clearly beats the next one.
pub fn resolve(net: &Network, query: &str) -> Resolve {
    let q = query.trim();

    if let Some(c) = net
        .complexes
        .iter()
        .find(|c| c.label.eq_ignore_ascii_case(q))
    {
        return Resolve::Found(c.id);
    }
    let exact: Vec<usize> = net
        .complexes
        .iter()
        .filter(|c| {
            c.name.eq_ignore_ascii_case(q) || c.aka.iter().any(|a| a.eq_ignore_ascii_case(q))
        })
        .map(|c| c.id)
        .collect();
    match exact.len() {
        1 => return Resolve::Found(exact[0]),
        n if n > 1 => return Resolve::Ambiguous(exact),
        _ => {}
    }
    let qn = normalize(q);
    let qj = qn.join(" ");
    let mut scored: Vec<(f64, usize)> = (0..net.complexes.len())
        .filter_map(|c| score(net, c, &qn, &qj).map(|s| (s, c)))
        .collect();

    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    match scored.as_slice() {
        [] => Resolve::NotFound,
        [(_, only)] => Resolve::Found(*only),
        [(s0, top), (s1, _), ..] if s0 - s1 >= 1.0 => Resolve::Found(*top),
        _ => {
            let top = scored[0].0;

            Resolve::Ambiguous(
                scored
                    .iter()
                    .take_while(|(s, _)| top - s < 1.0)
                    .take(12)
                    .map(|(_, c)| *c)
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation() {
        assert_eq!(
            normalize("42nd St - Bryant Pk"),
            ["42", "st", "bryant", "pk"]
        );
        assert_eq!(
            normalize("Atlantic Av - Barclay's Center"),
            ["atlantic", "av", "barclays", "ctr"]
        );
        assert_eq!(
            normalize("Jackson Hts - Roosevelt Ave"),
            ["jackson", "hts", "roosevelt", "av"]
        );
    }

    #[test]
    fn finds_stations_loosely() {
        let net = Network::embedded();
        let top = |q: &str| net.complex(search(&net, q)[0]).label.clone();

        assert_eq!(top("times sq"), "Times Sq - 42nd St");
        assert_eq!(top("port authority"), "Times Sq - 42nd St");
        assert!(top("atlantic").starts_with("Atlantic Av"));
        assert_eq!(top("86 6"), "86th St (4 5 6)");
        assert_eq!(top("flushing main"), "Flushing - Main St");
    }

    #[test]
    fn resolves_or_reports_ambiguity() {
        let net = Network::embedded();
        assert!(matches!(
            resolve(&net, "Times Sq - 42nd St"),
            Resolve::Found(_)
        ));
        assert!(matches!(
            resolve(&net, "86th St (4 5 6)"),
            Resolve::Found(_)
        ));
        match resolve(&net, "86th St") {
            Resolve::Ambiguous(v) => assert_eq!(v.len(), 5),
            other => panic!("{other:?}"),
        }
        assert!(matches!(resolve(&net, "zzzzqqq"), Resolve::NotFound));
    }
}
