use super::*;
use crate::diagram::{Diagram, parse as parse_diagram, width};

fn graph(src: &str) -> Graph {
    match parse_diagram(src) {
        Some(Diagram::Graph(g)) => g,
        _ => panic!("didn't parse: {src}"),
    }
}

fn rows_at(src: &str, avail: usize, ascii: bool) -> Vec<String> {
    let g = graph(src);
    let d = draw(&g, avail, ascii).expect("draws");
    (0..d.canvas.height())
        .map(|r| d.canvas.row_text(r))
        .collect()
}

fn drawn(src: &str) -> String {
    rows_at(src, 200, false).join("\n")
}

#[test]
fn a_chain_runs_straight_down() {
    insta::assert_snapshot!(drawn("flowchart TD\nA[Start] --> B[Middle] --> C[End]"));
}

#[test]
fn shapes_and_lines() {
    let src = "flowchart TD\nA[rect] --> B(round)\nB -.-> C([stadium])\nC ==> D{diamond}\nD --- E((circle))\nD --o F[\"quoted [x]\"]\nD --x G";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn branches_with_labels() {
    let src = "flowchart TD\nA{ok?} -->|yes| B[go]\nA -->|no| C[stop]\nB --> D[done]\nC --> D";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn left_to_right() {
    let src = "flowchart LR\nPR[Pull request] --> Lint{clippy ok?}\nLint -->|no| Fail[Report]\n\
               Lint -->|yes| Test[cargo test]\nTest --> Fuzz[fuzz smoke]\nTest --> Bench[B2 bench]\n\
               Fuzz --> Merge((merge))\nBench --> Merge";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn bottom_to_top_and_right_to_left() {
    insta::assert_snapshot!("bt", drawn("graph BT\na --> b --> c\na --> c"));
    insta::assert_snapshot!("rl", drawn("graph RL\na --> b --> c\na --> c"));
}

#[test]
fn subgraphs_are_framed() {
    let src = "flowchart TD\n    subgraph parse[Parse]\n        A[read file] --> B{markdown?}\n    end\n\
               B -->|yes| C[pulldown-cmark]\n    B -->|no| D[syntect]\n    C --> E[layout]\n    D --> E\n\
               E --> F((paint))\n    E -.-> G[images worker]";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn subgraphs_side_by_side_and_edges_to_them() {
    let src = "flowchart LR\nsubgraph one\na1 --> a2\nend\nsubgraph two\nb1 --> b2\nend\nc1 --> a2\none --> two\na1 --> b2";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn cycles_point_back() {
    insta::assert_snapshot!(drawn("flowchart TD\nA --> B --> C --> A\nC --> D"));
}

#[test]
fn state_diagram() {
    let src = "stateDiagram-v2\n    [*] --> Idle\n    Idle --> Loading: open\n    Loading --> Ready: parsed\n\
               Loading --> Failed: error\n    Failed --> Idle: retry\n    Ready --> Loading: file changed\n\
               Ready --> [*]: quit";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn composite_states_forks_notes_and_loops() {
    let src = "stateDiagram\n[*] --> Active\nstate Active {\n[*] --> Working\nWorking --> Working : tick\nWorking --> [*]\n}\n\
               Active --> F\nstate F <<fork>>\nF --> X\nF --> Y\nnote right of X : a note\nX --> [*]\nY --> [*]";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn ascii_glyphs() {
    let rows = rows_at("flowchart TD\nA --> B{q}\nB -.-> C", 80, true).join("\n");
    assert!(rows.is_ascii(), "{rows}");
    insta::assert_snapshot!(rows);
}

#[test]
fn labels_wrap_to_fit() {
    let src = "flowchart LR\nA[a rather long label for a node] --> B[another quite long label here] --> C[and a third one that is long]";
    let wide = rows_at(src, 400, false);
    let narrow = rows_at(src, 80, false);
    let cols = |rows: &[String]| rows.iter().map(|r| width(r)).max().unwrap_or(0);
    assert!(cols(&narrow) <= 80, "{}", narrow.join("\n"));
    assert!(cols(&wide) > cols(&narrow));
}

#[test]
fn chains_and_ampersands() {
    let g = graph("flowchart LR\nA & B --> C & D --> E");
    assert_eq!(g.edges.len(), 6);
    assert_eq!(g.nodes.len(), 5);
}

#[test]
fn subgraph_membership_and_edges_to_subgraphs() {
    let g = graph("flowchart TB\nc1-->a2\nsubgraph one\na1-->a2\nend\nc1 --> one");
    let a2 = g.nodes.iter().find(|n| n.id == "a2").unwrap();
    assert_eq!(
        a2.parent,
        Some(0),
        "a mention inside a subgraph moves the node there"
    );
    assert_eq!(g.edges[2].to, End::Cluster(0));
    assert!(g.nodes.iter().all(|n| n.id != "one"));
}

#[test]
fn what_doesnt_parse_is_left_as_source() {
    for src in [
        "flowchart XY\nA-->B",
        "flowchart TD\nA[unclosed",
        "flowchart TD\nend",
        "flowchart TD\nA -> B",
        "stateDiagram\nA --> B\n}",
    ] {
        assert!(parse_diagram(src).is_none(), "{src}");
    }
    assert!(parse_diagram("flowchart TD").is_none(), "nothing in it");
}

#[test]
fn big_graphs_stay_bounded() {
    let n = 120;
    let mut src = String::from("graph TD\n");
    for i in 1..=n {
        src.push_str(&format!("n{i}[node {i}] --> n{}\n", i % n + 1));
        src.push_str(&format!("n{i} --> n{}\n", (i * 13) % n + 1));
    }
    let g = graph(&src);
    let start = std::time::Instant::now();
    let d = draw(&g, 120, false).expect("draws");
    assert!(d.canvas.height() > 0);
    // generous: the bound is the point, not the speed
    assert!(start.elapsed().as_secs() < 5);
}

/// Random statements, parsed and drawn directly, outside the guard that would hide a panic.
#[test]
fn random_graphs_draw_without_panicking() {
    const FLOW: &[&str] = &[
        "A[Start] --> B{ok?}",
        "B -->|yes| C(round)",
        "B -- no --> D([stadium])",
        "C -.-> A",
        "D ==> E((circle)) --- A",
        "E --o F[\"quoted [x]\"] --x A",
        "A & B --> G & H",
        "G <--> G",
        "G --> G",
        "H ---> I[日本語のラベル that is long enough to wrap]",
        "subgraph one [One]",
        "subgraph two",
        "end",
        "one --> two",
        "two --> A",
        "A ~~~ H",
        "I --> one",
    ];
    const STATE: &[&str] = &[
        "[*] --> Idle",
        "Idle --> Busy : work",
        "Busy --> Idle",
        "Busy --> Busy : again",
        "Busy --> [*]",
        "state Busy {",
        "state \"A long name\" as L",
        "state F <<fork>>",
        "state C <<choice>>",
        "L --> F",
        "F --> C",
        "C --> Idle",
        "--",
        "}",
        "note right of Idle : a note",
        "note left of L : another",
        "Idle : waiting",
    ];
    let mut seed = 0x00ad_a5ee_d000_0004u64;
    let mut below = |n: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % n
    };
    let headers = [
        "flowchart TD",
        "graph LR",
        "graph BT",
        "flowchart RL",
        "stateDiagram-v2",
    ];
    for i in 0..500 {
        let header = headers[i % headers.len()];
        let list = if header.starts_with("state") {
            STATE
        } else {
            FLOW
        };
        let mut src = format!("{header}\n");
        for _ in 0..below(30) {
            src.push_str(list[below(list.len())]);
            src.push('\n');
        }
        let Some(Diagram::Graph(g)) = parse_diagram(&src) else {
            continue;
        };
        for avail in [1, 20, 60, 200] {
            for ascii in [false, true] {
                let d = draw(&g, avail, ascii).unwrap_or_else(|| panic!("{src}"));
                for r in 0..d.canvas.height() {
                    assert!(width(&d.canvas.row_text(r)) <= d.canvas.width(), "{src}");
                }
            }
        }
    }
}

#[test]
fn nested_subgraphs() {
    let src = "flowchart LR\n  subgraph outer [Outer box]\n    subgraph inner [Inner]\n      x[deep node] --> y[another]\n    end\n\
               z[outer node] --> x\n  end\n  start((go)) --> z\n  y --> finish[done]";
    insta::assert_snapshot!(drawn(src));
}

#[test]
fn concurrent_regions() {
    let src = "stateDiagram-v2\n[*] --> Active\nstate Active {\n[*] --> Off\nOff --> On : press\nOn --> Off : press\n--\n\
               [*] --> Idle\nIdle --> Busy : job\n}";
    insta::assert_snapshot!(drawn(src));
}

/// The probe's flowchart series: `n` nodes in a chain, each with an edge further on.
fn series(n: usize) -> String {
    let mut s = String::from("graph TD\n");
    for i in 1..=n {
        s.push_str(&format!("n{i}[node {i}] --> n{}\n", i + 1));
        s.push_str(&format!("n{i} --> n{}\n", (i * 13) % n + 1));
    }
    s
}

/// A sideways drawing turned to run down, glyphs and all.
fn transposed(rows: &[String]) -> Vec<String> {
    let grid: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    (0..cols)
        .map(|c| {
            grid.iter()
                .map(|row| match row.get(c).copied().unwrap_or(' ') {
                    '─' => '│',
                    '│' => '─',
                    '┐' => '└',
                    '└' => '┐',
                    '├' => '┬',
                    '┬' => '├',
                    '┤' => '┴',
                    '┴' => '┤',
                    '▶' => '▼',
                    '◀' => '▲',
                    '▼' => '▶',
                    '▲' => '◀',
                    ch => ch,
                })
                .collect()
        })
        .collect()
}

/// A line network's sources and targets, as box indices.
type Network = (Vec<usize>, Vec<usize>);

/// A drawing's line networks, read back from its glyphs alone (`TD` or `BT`; `LR` and `RL`
/// transposed first): boxes found by their borders, each line cell joined to the neighbors its
/// arms point at (a `┼` keeps its two lines apart), a `┬` on a bottom border or `┴` on a top
/// border a network's source, a head against a border its target. Each network as
/// `(sources, targets)`, box indices, and each box's text. A network with two or more of each
/// is ambiguous: which source goes to which target is lost.
fn networks(rows: &[String]) -> (Vec<Network>, Vec<String>) {
    let grid: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
    let height = grid.len();
    let width = grid.iter().map(Vec::len).max().unwrap_or(0);
    let at = |r: usize, c: usize| {
        grid.get(r)
            .and_then(|row| row.get(c))
            .copied()
            .unwrap_or(' ')
    };
    // up, down, left, right
    let arms = |ch: char| match ch {
        '│' => [true, true, false, false],
        '─' => [false, false, true, true],
        '┌' => [false, true, false, true],
        '┐' => [false, true, true, false],
        '└' => [true, false, false, true],
        '┘' => [true, false, true, false],
        '├' => [true, true, false, true],
        '┤' => [true, true, true, false],
        '┬' => [false, true, true, true],
        '┴' => [true, false, true, true],
        '┼' => [true, true, true, true],
        '▼' => [true, false, false, false],
        '▲' => [false, true, false, false],
        _ => [false; 4],
    };
    // boxes: (top, left, bottom, right), borders included
    let mut boxes: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut texts = Vec::new();
    let mut owner = vec![None; height * width];
    for r in 0..height {
        for c in 0..width {
            if at(r, c) != '┌' || owner[r * width + c].is_some() {
                continue;
            }
            let mut c2 = c + 1;
            while matches!(at(r, c2), '─' | '┬' | '┴' | '┼') {
                c2 += 1;
            }
            if at(r, c2) != '┐' || c2 == c + 1 {
                continue;
            }
            let mut r2 = r + 1;
            while matches!(at(r2, c), '│' | '├' | '┤' | '┼')
                && matches!(at(r2, c2), '│' | '├' | '┤' | '┼')
            {
                r2 += 1;
            }
            if r2 == r + 1 || at(r2, c) != '└' || at(r2, c2) != '┘' {
                continue;
            }
            let text: Vec<String> = (r + 1..r2)
                .map(|y| {
                    (c + 1..c2)
                        .map(|x| at(y, x))
                        .collect::<String>()
                        .trim()
                        .to_string()
                })
                .collect();
            for y in r..=r2 {
                for x in c..=c2 {
                    owner[y * width + x] = Some(boxes.len());
                }
            }
            texts.push(text.join(" "));
            boxes.push((r, c, r2, c2));
        }
    }
    let line = |r: usize, c: usize| {
        r < height && c < width && owner[r * width + c].is_none() && arms(at(r, c)) != [false; 4]
    };
    // two parts per cell, vertical and horizontal
    let id = |r: usize, c: usize, horizontal: bool| (r * width + c) * 2 + usize::from(horizontal);
    let mut parent: Vec<usize> = (0..height * width * 2).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let join = |p: &mut [usize], a: usize, b: usize| {
        let (x, y) = (find(p, a), find(p, b));
        p[x] = y;
    };
    for r in 0..height {
        for c in 0..width {
            if !line(r, c) {
                continue;
            }
            let [_, down, _, right] = arms(at(r, c));
            if at(r, c) != '┼' {
                join(&mut parent, id(r, c, false), id(r, c, true));
            }
            if down && line(r + 1, c) && arms(at(r + 1, c))[0] {
                join(&mut parent, id(r, c, false), id(r + 1, c, false));
            }
            if right && line(r, c + 1) && arms(at(r, c + 1))[2] {
                join(&mut parent, id(r, c, true), id(r, c + 1, true));
            }
        }
    }
    let mut nets: HashMap<usize, Network> = HashMap::new();
    for (k, &(t, l, b, rt)) in boxes.iter().enumerate() {
        for c in l..=rt {
            // the line beyond each border: leaving through a tee, or arriving in a head
            for (border, out, tee, head) in [(b, b + 1, '┬', '▲'), (t, t.wrapping_sub(1), '┴', '▼')]
            {
                if !line(out, c) {
                    continue;
                }
                let net = nets
                    .entry(find(&mut parent, id(out, c, false)))
                    .or_default();
                if matches!(at(border, c), '┼') || at(border, c) == tee {
                    net.0.push(k);
                } else if at(out, c) == head {
                    net.1.push(k);
                }
            }
        }
    }
    let mut out: Vec<Network> = nets.into_values().collect();
    for (s, t) in &mut out {
        s.sort_unstable();
        s.dedup();
        t.sort_unstable();
        t.dedup();
    }
    (out, texts)
}

/// DG-8: no two edges run in one lane, so each line network is one box's fan-out or a fan-in
/// to one box, and together they carry exactly the graph's edges.
#[test]
fn edges_keep_to_their_own_lanes() {
    for n in [20, 40, 80, 120] {
        let mut want: Vec<(usize, usize)> = (1..=n)
            .flat_map(|i| [(i, i + 1), (i, i * 13 % n + 1)])
            .collect();
        want.sort_unstable();
        want.dedup();
        for avail in [60, 100, 200] {
            let rows = rows_at(&series(n), avail, false);
            let (nets, texts) = networks(&rows);
            let node = |k: usize| {
                let t = &texts[k];
                t.strip_prefix("node ")
                    .or_else(|| t.strip_prefix('n'))
                    .and_then(|i| i.parse::<usize>().ok())
                    .unwrap_or_else(|| panic!("a box reading {t:?}"))
            };
            let mut drawn = Vec::new();
            for (s, t) in &nets {
                let names = |ks: &[usize]| ks.iter().map(|&k| node(k)).collect::<Vec<_>>();
                assert!(
                    s.len() < 2 || t.len() < 2,
                    "{n} nodes at {avail}: {:?} and {:?} share a lane\n{}",
                    names(s),
                    names(t),
                    rows.join("\n")
                );
                drawn.extend(s.iter().flat_map(|&a| t.iter().map(move |&b| (a, b))));
            }
            let mut drawn: Vec<(usize, usize)> =
                drawn.into_iter().map(|(a, b)| (node(a), node(b))).collect();
            drawn.sort_unstable();
            drawn.dedup();
            assert_eq!(drawn, want, "{n} nodes at {avail}");
        }
    }
}

/// DG-8 in the probe set's graphs (vault3 had a shared lane): no network the checker reads (it
/// knows square boxes only) is ambiguous, and vault3's, all square, carry its 24 edges.
#[test]
fn probe_graphs_keep_edges_to_their_own_lanes() {
    let probes = [
        ("vault1", include_str!("../../../bench/diagrams/vault1.mmd")),
        ("vault2", include_str!("../../../bench/diagrams/vault2.mmd")),
        ("vault3", include_str!("../../../bench/diagrams/vault3.mmd")),
        ("vault4", include_str!("../../../bench/diagrams/vault4.mmd")),
        (
            "flow_sub",
            include_str!("../../../bench/diagrams/flow_sub.mmd"),
        ),
        ("ci", include_str!("../../../bench/diagrams/ci.mmd")),
        (
            "lr_wide",
            include_str!("../../../bench/diagrams/lr_wide.mmd"),
        ),
    ];
    for (name, src) in probes {
        let sideways = matches!(graph(src).dir, Dir::Right | Dir::Left);
        for avail in [60, 100, 200] {
            let rows = rows_at(src, avail, false);
            let rows = if sideways { transposed(&rows) } else { rows };
            let (nets, texts) = networks(&rows);
            for (s, t) in &nets {
                let names = |ks: &[usize]| ks.iter().map(|&k| texts[k].clone()).collect::<Vec<_>>();
                assert!(
                    s.len() < 2 || t.len() < 2,
                    "{name} at {avail}: {:?} and {:?} share a lane\n{}",
                    names(s),
                    names(t),
                    rows.join("\n")
                );
            }
            if name == "vault3" {
                let edges: usize = nets.iter().map(|(s, t)| s.len() * t.len()).sum();
                assert_eq!(edges, 24, "vault3 at {avail}");
            }
        }
    }
}
