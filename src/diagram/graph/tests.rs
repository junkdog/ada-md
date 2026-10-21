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
