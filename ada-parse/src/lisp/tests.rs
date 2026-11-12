use crate::tests::marked;
use crate::{Lang, State, lex};

fn lisp(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
    marked(Lang::Lisp, lines)
}

/// The scopes `text` has in `lines`' marked pieces, the first time it is one.
fn scope_of(lines: &[&'static str], text: &str) -> &'static str {
    lisp(lines)
        .into_iter()
        .find(|(t, _)| *t == text)
        .map_or("(none)", |(_, s)| s)
}

#[test]
fn a_defun() {
    let l = ["(defun foo (x &optional (y 2)) \"doc ~a~%\" (car x)) ; done"];
    assert_eq!(scope_of(&l, "defun"), "keyword.declaration.function.lisp");
    assert_eq!(scope_of(&l, "foo"), "entity.name.function.lisp");
    assert_eq!(scope_of(&l, "x"), "variable.parameter.lisp");
    assert_eq!(
        scope_of(&l, "&"),
        "variable.annotation.lisp punctuation.definition.annotation.lisp"
    );
    assert_eq!(scope_of(&l, "y"), "variable.parameter.lisp");
    assert_eq!(scope_of(&l, "car"), "support.function.lisp");
    assert_eq!(
        scope_of(&l, "~a"),
        "(none)",
        "a directive is its `~` and its letter"
    );
    assert_eq!(
        scope_of(&l, "~"),
        "string.quoted.double.lisp constant.other.placeholder.lisp punctuation.definition.placeholder.lisp"
    );
    assert_eq!(scope_of(&l, "; "), "(none)", "the `;` is its own piece");
    assert_eq!(
        scope_of(&l, ";"),
        "comment.line.semicolon.lisp punctuation.definition.comment.lisp"
    );
}

#[test]
fn namespaces_and_escapes() {
    let l = ["(cl-user::foo (pkg:bar) a\\:b::c |weird pkg|::x esc\\ aped:x keyword:y :k)"];
    let m = lisp(&l);
    let ns: Vec<_> = m
        .iter()
        .filter(|(_, s)| s.starts_with("variable.namespace"))
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(
        ns,
        [
            "cl-user",
            "pkg",
            "a\\:b",
            "|",
            "weird pkg",
            "|",
            "esc\\ aped",
            "keyword"
        ]
    );
    // `|…|` alone is a symbol, a namespace before `:`
    assert_eq!(scope_of(&l, "weird pkg"), "variable.namespace.lisp");
    assert_eq!(scope_of(&l, "foo"), "variable.function.lisp");
    assert_eq!(scope_of(&l, "bar"), "variable.function.lisp");
    assert_eq!(scope_of(&l, "y"), "keyword.other.symbol.lisp");
    assert_eq!(scope_of(&l, "k"), "keyword.other.symbol.lisp");
}

#[test]
fn backslash_runs_are_linear() {
    // G-001's shape: a run of backslashes, words and a colon
    for n in [32, 1024, 65536] {
        let line = format!("({} foo bar:baz)", "\\".repeat(n));
        let t = std::time::Instant::now();
        let (mut spans, mut state) = (Vec::new(), State::default());
        lex(Lang::Lisp, &line, &mut state, &mut spans);
        assert!(t.elapsed().as_millis() < 200, "{n}: {:?}", t.elapsed());
    }
}

#[test]
fn state_across_lines() {
    let l = [
        "(defun",
        "  bar",
        "  (a b)",
        "  #| a #| nested |# comment",
        "  still |# \"a string",
        "  goes on\" (loop for x in y",
        "  collect x))",
    ];
    assert_eq!(scope_of(&l, "bar"), "entity.name.function.lisp");
    assert_eq!(scope_of(&l, "a"), "variable.parameter.lisp");
    assert_eq!(scope_of(&l, "  still "), "comment.block.lisp");
    assert_eq!(scope_of(&l, "  goes on"), "string.quoted.double.lisp");
    assert_eq!(scope_of(&l, "for"), "keyword.control.loop.lisp");
    assert_eq!(scope_of(&l, "in"), "keyword.other.loop.lisp");
}

#[test]
fn deep_nesting_keeps_its_bodies() {
    let open = "(".repeat(500);
    let close = ")".repeat(500);
    let lines = [open.as_str(), close.as_str(), ")"];
    let (mut spans, mut state) = (Vec::new(), State::default());
    for l in &lines[..2] {
        lex(Lang::Lisp, l, &mut state, &mut spans);
    }
    assert!(
        spans
            .iter()
            .all(|s| Lang::Lisp.scopes(s.kind).last()
                == Some(&"punctuation.section.parens.end.lisp"))
    );
    lex(Lang::Lisp, lines[2], &mut state, &mut spans);
    assert_eq!(
        Lang::Lisp.scopes(spans[0].kind),
        ["invalid.illegal.stray-bracket-end.lisp"]
    );
}

/// Long lines of shapes that make a scan read ahead stay linear: each shape's line at four times
/// the length takes well under sixteen times as long (the best of three).
#[test]
fn long_lines_stay_linear() {
    let shapes: &[(Lang, &str)] = &[
        (Lang::Lisp, "\\"),
        (Lang::Lisp, "\\\\ a:"),
        (Lang::Lisp, "+(a"),
        (Lang::Lisp, "+a"),
        (Lang::Lisp, "1.1."),
        (Lang::Lisp, "1"),
        (Lang::Lisp, "."),
        (Lang::Lisp, "|a "),
        (Lang::Lisp, "#\\a-"),
        (Lang::Lisp, "a:"),
        (Lang::Lisp, "(make-"),
        (Lang::Lisp, "(a-p "),
        (Lang::Lisp, "(("),
        (Lang::Lisp, "))"),
        (Lang::Lisp, "\"~,'"),
        (Lang::Lisp, "#1"),
        (Lang::Lisp, "#b1"),
        (Lang::Lisp, "(loop a "),
        (Lang::Lisp, "#|"),
        (Lang::Scheme, "#;("),
        (Lang::Scheme, "#;"),
        (Lang::Scheme, "#{"),
        (Lang::Scheme, "[("),
        (Lang::Scheme, "#rx\"\\"),
        (Lang::Scheme, "#:"),
        (Lang::Scheme, "(define (("),
        (Lang::Scheme, "'#"),
        (Lang::Racket, "#{"),
        (Lang::Racket, "#lang "),
        (Lang::Clojure, "\\"),
        (Lang::Clojure, "#\"\\"),
        (Lang::Clojure, "#\"(["),
        (Lang::Clojure, "^"),
        (Lang::Clojure, "'"),
        (Lang::Clojure, "#"),
        (Lang::Clojure, "1"),
        (Lang::Clojure, ":"),
        (Lang::Clojure, "(["),
        (Lang::Clojure, "(def"),
    ];
    let time = |lang: Lang, line: &str| {
        (0..3)
            .map(|_| {
                let (mut spans, mut state) = (Vec::new(), State::default());
                let t = std::time::Instant::now();
                lex(lang, line, &mut state, &mut spans);
                t.elapsed()
            })
            .min()
            .unwrap()
    };
    for &(lang, shape) in shapes {
        let short = shape.repeat(8192 / shape.len());
        let long = shape.repeat(32768 / shape.len());
        let (a, b) = (time(lang, &short), time(lang, &long));
        assert!(
            b.as_secs_f64() < 10.0 * a.as_secs_f64().max(20e-6),
            "{lang:?} {shape:?}: {a:?} for 8 KB, {b:?} for 32 KB"
        );
    }
}

#[test]
fn emacs_lisp_characters() {
    // not Lisp's: Scheme's `(? pred)` and Common Lisp's `?var` are symbols
    let lisp_chars = lisp(&["(? a) (match x (?a ?b))"])
        .into_iter()
        .filter(|(_, s)| s.starts_with("constant.character"))
        .count();
    assert_eq!(lisp_chars, 0);
    let l = ["(list ?a ?\\) ?\\( ?| ?\\C-\\M-x ?\\^I ?\\x41 ?ab '((?c . 1)))"];
    let chars: Vec<_> = marked(Lang::EmacsLisp, &l)
        .into_iter()
        .filter(|(_, s)| s.starts_with("constant.character"))
        .map(|(t, _)| t)
        .collect();
    assert_eq!(
        chars,
        [
            "?",
            "a",
            "?",
            "\\)",
            "?",
            "\\(",
            "?",
            "|",
            "?",
            "\\C-\\M-x",
            "?",
            "\\^I",
            "?",
            "\\x41",
            "?",
            "c"
        ]
    );
    let ab = marked(Lang::EmacsLisp, &l)
        .into_iter()
        .find(|(t, _)| *t == "?ab");
    assert_eq!(ab.map(|(_, s)| s), Some("variable.other.lisp"));
}

fn scheme_scope_of(lang: Lang, lines: &[&'static str], text: &str) -> &'static str {
    marked(lang, lines)
        .into_iter()
        .find(|(t, _)| *t == text)
        .map_or("(none)", |(_, s)| s)
}

/// Scheme's reading (X-006, a design of ada's own): its heads, brackets as lists, its `#` forms.
#[test]
fn scheme_heads_and_forms() {
    let l = [
        "(define (square x) (* x x)) (define pi2 6.28) (define ((curry a) b) a)",
        "(define-syntax swap! (syntax-rules () [(_ a b) (let ([t a]) (set! a b))]))",
        "(let loop ([i 0] [acc '()]) (if (< i 3) (loop (+ i 1) (cons i acc)) acc))",
        "(lambda args (apply + args)) (case-lambda [(a) a] [(a b) b])",
        "(define-record-type point (make-point x y) point? (x point-x)) #t #f #true",
        "(cond [else => car]) #;(ignored (list 1 \")\") #\\() visible #:kw +inf.0 #e1.5",
        "#u8(1 2) #hash((a . 1)) #\"bytes\" #'stx #`q (import (scheme base)) #rx\"a\\\\d+\" t nil",
    ];
    let s = |t: &str| scheme_scope_of(Lang::Scheme, &l, t);
    assert_eq!(s("define"), "keyword.declaration.function.lisp");
    assert_eq!(s("square"), "entity.name.function.lisp");
    assert_eq!(s("x"), "variable.parameter.lisp");
    assert_eq!(s("pi2"), "entity.name.variable.lisp");
    assert_eq!(s("curry"), "entity.name.function.lisp");
    assert_eq!(s("swap!"), "entity.name.macro.lisp");
    assert_eq!(s("syntax-rules"), "keyword.control.lisp");
    assert_eq!(s("t"), "variable.other.lisp", "the binding's name");
    assert_eq!(s("loop"), "entity.name.function.lisp", "a named let's name");
    assert_eq!(s("i"), "variable.other.lisp");
    assert_eq!(s("if"), "keyword.control.conditional.lisp");
    assert_eq!(s("cons"), "support.function.lisp");
    assert_eq!(s("args"), "variable.parameter.lisp");
    assert_eq!(s("point"), "entity.name.struct.lisp");
    assert_eq!(s("make-point"), "variable.function.constructor.lisp");
    assert_eq!(s("#t"), "constant.language.boolean.lisp");
    assert_eq!(s("#true"), "constant.language.boolean.lisp");
    assert_eq!(s("else"), "keyword.control.conditional.lisp");
    assert_eq!(
        s("#;"),
        "comment.block.lisp punctuation.definition.comment.lisp"
    );
    assert_eq!(
        s("visible"),
        "variable.other.lisp",
        "the datum's end found past `\")\"` and `#\\(`"
    );
    assert_eq!(s("kw"), "keyword.other.symbol.lisp");
    assert_eq!(s("inf.0"), "constant.numeric.value.lisp");
    assert_eq!(s("#u8"), "punctuation.definition.vector.lisp");
    assert_eq!(s("#"), "storage.type.string.lisp");
    assert_eq!(s("import"), "keyword.control.import.lisp");
    assert_eq!(s("#rx"), "storage.type.string.lisp");
    assert_eq!(
        s("\\\\d"),
        "string.regexp.lisp constant.other.character-class.escape.backslash.regexp"
    );
    assert_eq!(
        s("nil"),
        "variable.other.lisp",
        "Common Lisp's nil is a symbol in Scheme"
    );
    // the commented datum is a comment, whole
    let m = marked(Lang::Scheme, &l[5..6]);
    let ignored: String = m
        .iter()
        .filter(|(_, s)| *s == "comment.block.lisp")
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(ignored, "(ignored (list 1 \")\") #\\()");
}

#[test]
fn racket_lines() {
    let l = [
        "#lang racket/base",
        "(require racket/list) (provide (all-defined-out))",
        "(struct posn (x y) #:transparent)",
        "(define (f a #:b [b 1] . rest) (for/list ([i (in-range a)]) (λ (x) x)))",
        "#| block #| nested |# |# (displayln \"hi\") {1 2} [3]",
    ];
    let s = |t: &str| scheme_scope_of(Lang::Racket, &l, t);
    assert_eq!(s("#lang"), "keyword.declaration.lisp");
    assert_eq!(s("racket/base"), "entity.name.namespace.lisp");
    assert_eq!(s("require"), "keyword.control.import.lisp");
    assert_eq!(s("posn"), "entity.name.struct.lisp");
    assert_eq!(s("transparent"), "keyword.other.symbol.lisp");
    assert_eq!(s("f"), "entity.name.function.lisp");
    assert_eq!(s("a"), "variable.parameter.lisp");
    assert_eq!(s("b"), "keyword.other.symbol.lisp", "`#:b`");
    assert_eq!(s("for/list"), "keyword.control.loop.lisp");
    assert_eq!(s("λ"), "keyword.declaration.function.inline.lisp");
    assert_eq!(s("displayln"), "support.function.lisp");
    assert_eq!(
        s("{"),
        "meta.depth-1.lisp punctuation.section.parens.begin.lisp",
        "a list, colored by its depth (X-030)"
    );
    let (mut spans, mut state) = (Vec::new(), State::default());
    for line in l {
        lex(Lang::Racket, line, &mut state, &mut spans);
    }
    assert_eq!(state, State::default());
}

/// Guile's `#{…}#` symbols, quoted and across lines (`'#{,}#`, autofrisk.scm's `'#{` text `}#`).
#[test]
fn guile_extended_symbols() {
    let l = [
        "(eq? sym '#{,}#) '#{",
        "  any [text] (",
        "}#) (f #{g69 x1-0})",
    ];
    let (mut spans, mut state) = (Vec::new(), State::default());
    for line in l {
        lex(Lang::Scheme, line, &mut state, &mut spans);
    }
    assert_eq!(state, State::default());
    let s = |t: &str| scheme_scope_of(Lang::Scheme, &l, t);
    assert_eq!(s(","), "variable.other.lisp");
    assert_eq!(s("  any [text] ("), "variable.other.lisp");
    assert_eq!(s("f"), "variable.function.lisp");
}

/// `|#%kernel|` is a symbol in Scheme, not a stray `|#`; Typed Racket's `#{x : T}` is a list.
#[test]
fn scheme_pipes_and_typed_racket() {
    let l = ["(|#%require| (for-syntax '|#%kernel|))"];
    assert_eq!(
        scheme_scope_of(Lang::Scheme, &l, "#%kernel"),
        "constant.other.symbol.lisp"
    );
    let r = ["(let-values ([(#{x : Integer} y) (values 1 2)]) x)"];
    let (mut spans, mut state) = (Vec::new(), State::default());
    lex(Lang::Racket, r[0], &mut state, &mut spans);
    assert_eq!(state, State::default());
    assert_eq!(
        scheme_scope_of(Lang::Racket, &r, "Integer"),
        "variable.other.lisp"
    );
}

/// Clojure's call heads (a design of ada's own; bat calls every one a function): special forms
/// and `clojure.core`'s macros by what they do, its functions as builtins, the rest calls. Only
/// a head: the same words elsewhere stay symbols.
#[test]
fn clojure_heads() {
    let l = [
        "(if-some [[m a b] (re-matches #\"(\\w+)\" s)] (println a b) (my-fn \"no\"))",
        "(let [x 1] (loop [i 0] (when (< i 3) (recur (inc i)))))",
        "(ns a.b (:require [c.d :as d])) (->> xs (map f) (filter even?)) (and p q)",
        "(try (throw (ex-info \"e\" {})) (catch Exception e nil)) (binding [*out* w] x)",
        "#(* % 2) (/ 1 2) (If-Some x) (str/join xs) (map identity [println])",
    ];
    let s = |t: &str| scheme_scope_of(Lang::Clojure, &l, t);
    assert_eq!(s("if-some"), "keyword.control.conditional.clojure");
    assert_eq!(s("when"), "keyword.control.conditional.clojure");
    assert_eq!(s("re-matches"), "support.function.clojure");
    assert_eq!(s("println"), "support.function.clojure");
    assert_eq!(s("inc"), "support.function.clojure");
    assert_eq!(s("my-fn"), "variable.function.clojure");
    assert_eq!(s("let"), "keyword.declaration.variable.clojure");
    assert_eq!(s("binding"), "keyword.declaration.variable.clojure");
    assert_eq!(s("loop"), "keyword.control.loop.clojure");
    assert_eq!(s("recur"), "keyword.control.loop.clojure");
    assert_eq!(s("<"), "keyword.operator.clojure");
    assert_eq!(s("->>"), "keyword.operator.clojure");
    assert_eq!(
        s("*"),
        "keyword.operator.clojure",
        "an anonymous function's head"
    );
    assert_eq!(s("/"), "keyword.operator.clojure");
    assert_eq!(s("and"), "keyword.operator.logical.clojure");
    assert_eq!(s("ns"), "keyword.control.import.clojure");
    assert_eq!(s("try"), "keyword.control.clojure");
    assert_eq!(s("throw"), "keyword.control.clojure");
    assert_eq!(s("If-Some"), "variable.function.clojure", "case-sensitive");
    assert_eq!(
        s("str/join"),
        "variable.function.clojure",
        "qualified: not core's"
    );
    assert_eq!(s("even?"), "(none)", "not a head");
    assert_eq!(s("identity"), "(none)", "not a head");
}

/// A keyword's `:`, `::` and Racket's `#:` sit under the keyword's scope, so a theme colors the
/// keyword whole, where bat's accessor is its sibling (X-009); `pkg:sym`'s stays an accessor.
#[test]
fn a_keywords_mark_is_the_keywords() {
    const MARK: &str = "keyword.other.symbol.lisp punctuation.definition.keyword.lisp";
    assert_eq!(scope_of(&["(f :keys)"], ":"), MARK);
    assert_eq!(scope_of(&["(f ::keys)"], "::"), MARK);
    assert_eq!(scope_of(&["(cl:car x)"], ":"), "punctuation.accessor.lisp");
    let racket = marked(Lang::Racket, &["(f #:kw 1)"]);
    assert!(racket.contains(&("#:", MARK)), "{racket:?}");
}

/// Lines of `pieces` in a fixed pseudo-random order, 16 to ~70 bytes: a step of sixteen
/// meets each piece at every offset across the lines. Clojure's test too.
#[cfg(target_arch = "x86_64")]
pub(super) fn step_lines(pieces: &[&str]) -> Vec<String> {
    let mut x = 0x9e37_79b9_u32;
    let mut lines = Vec::new();
    for _ in 0..600 {
        let mut l = String::new();
        let len = 16 + (x % 56) as usize;
        while l.len() < len {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            l.push_str(pieces[x as usize % pieces.len()]);
        }
        lines.push(l);
    }
    lines
}

/// An identifier's run sixteen bytes a step (X-039) ends where a byte a step ends it, with the
/// same digit and escape answers, from every place of lines holding what a step stops at but
/// the run takes (control bytes, Unicode's letters and digits, escapes) and what ends it.
#[cfg(target_arch = "x86_64")]
#[test]
fn identifier_runs_step_as_bytes() {
    use super::{Dialect, Scan};
    use crate::emit::Push;
    let pieces = [
        "abc",
        "x1",
        "12",
        ".",
        "-",
        "*v*",
        "\u{1}",
        "\u{1f}",
        "\u{7f}",
        "é",
        "٣",
        "\u{3000}",
        "\u{a0}",
        "\\(",
        "\\é",
        "\\",
        "|",
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        "\t",
        " ",
        "'",
        ",",
        ":",
        ";",
        "\"",
        "foo-bar-baz-quux",
        "0123456789.0123",
    ];
    let mut spans = Vec::new();
    for dialect in [Dialect::Common, Dialect::Scheme] {
        for line in step_lines(&pieces) {
            let mut out = crate::Out::new(&mut spans);
            let s = Scan::fresh(line.as_bytes(), dialect, &State::default(), &mut out, Push);
            for p in 0..=line.len() {
                assert_eq!(s.run_wide(p), s.run_bytes(p), "{dialect:?} {line:?} at {p}");
            }
        }
    }
}
