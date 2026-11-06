//! Where a scanner's lexemes go (X-068): a line's [`Span`]s by kind, as [`crate::lex`] gives
//! them, or a highlighter's [`Run`]s by theme rule, as [`crate::lex_runs`] does. A scanner
//! generic over its [`Sink`] is compiled once a sink it's used with: ada uses only the runs, a
//! user of the span API only the spans, so neither carries the other's copy, and neither tests a
//! mode a span.

use std::marker::PhantomData;

use crate::{Out, Span};

/// An index into a theme's rules, as a highlighter numbers them.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rule(pub u16);

impl Rule {
    /// No rule: the theme's plain text.
    pub const NONE: u16 = u16::MAX;
    /// A kind's rule not resolved yet ([`Rules::table`]). No theme has this many rules.
    pub const UNSET: u16 = u16::MAX - 1;
}

/// A run of a line's bytes in one rule: from where the run before ends (the line's start for
/// the first) up to `end`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub end: u32,
    pub rule: Rule,
}

/// A theme's rules by a language's kinds, resolved as they're met.
pub trait Rules {
    /// The rules by kind so far: [`Rule::UNSET`] where a kind's isn't resolved, and past the
    /// end for a kind not met yet.
    fn table(&self) -> &[Rule];
    /// `kind`'s rule, resolved into [`Rules::table`] (which may move).
    fn resolve(&mut self, kind: u16) -> Rule;
}

/// Where a scanner writes a line's lexemes. A scanner hands over `out`, the spans' vector, on
/// each call: the span sink writes there, the run sink reads there only what another scanner
/// (an embedded one) pushed, between [`Sink::close`] and [`Sink::reopen`].
pub(crate) trait Sink: Copy {
    /// Bytes `s..e` are `kind`, `s` at or past the last lexeme's end; an empty one is nothing.
    /// `out` is the scan's field, not the vector's handle read from it: only a span sink's
    /// growth reads it, and a handle passed in was loaded at every span (+2.3% cycles alone).
    fn merge(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16);
    /// Opened for writing at `out`'s end, room made there for `n` more spans.
    fn open(&mut self, out: &mut Out, n: usize);
    /// Closed for another scanner to push spans to `out`.
    fn close(&mut self, out: &mut Out);
    /// Opened again after another scanner pushed spans to `out` ([`Sink::close`]).
    fn reopen(&mut self, out: &mut Out, n: usize);
    /// A line starts, on a sink open since an earlier one (a block's pass).
    fn start_line(&mut self);
}

/// A line's spans, on a write cursor in the scan's own fields: a span is three stores and a
/// step, which no store through the vector's buffer can change (X-065).
#[derive(Clone, Copy)]
pub(crate) struct Spans {
    /// Where the line's last span ends, `usize::MAX` for none: a span joins it only where it
    /// starts there, so that the test is a field's.
    end: usize,
    /// The write cursor, its vector's start and its limit, open from a line's start (a
    /// block's, in a block pass) to the line's end ([`Sink::open`], [`Sink::close`]).
    w: *mut Span,
    base: *mut Span,
    lim: *mut Span,
}

impl Spans {
    pub(crate) fn new() -> Self {
        Spans {
            end: usize::MAX,
            w: std::ptr::null_mut(),
            base: std::ptr::null_mut(),
            lim: std::ptr::null_mut(),
        }
    }

    /// The spans' room doubled, at the limit.
    #[cold]
    #[inline(never)]
    fn grow(&mut self, out: &mut Out) {
        self.close(out);
        let n = out.0.len().max(16);
        self.open(out, n);
    }
}

impl Sink for Spans {
    /// `Out::push_merged`, on the write cursor: a token is a scan's commonest step (X-065). Only
    /// a span that starts where the last ends ([`Spans::end`]) reads the last. A scanner on
    /// this sink is never embedded where spans aren't joined (HTML's), so they always are.
    #[inline(always)]
    fn merge(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        if s == self.end {
            // SAFETY: `end` is a span's only once one is written below the cursor, on this
            // line's open spans
            let last = unsafe { &mut *self.w.sub(1) };
            if last.kind == kind {
                last.end = e as u32;
                self.end = e;
                return;
            }
        }
        if e > s {
            if self.w == self.lim {
                self.grow(out);
            }
            // SAFETY: `w` is below `lim`, the vector's capacity, which nothing else changes
            // while the spans are open
            unsafe {
                self.w.write(Span {
                    start: s as u32,
                    end: e as u32,
                    kind,
                });
                self.w = self.w.add(1);
            }
            self.end = e;
        }
    }

    #[inline(always)]
    fn open(&mut self, out: &mut Out, n: usize) {
        let v = &mut *out.0;
        v.reserve(n);
        let base = v.as_mut_ptr();
        // SAFETY: both within (or one past) the vector's allocation
        unsafe {
            self.w = base.add(v.len());
            self.lim = base.add(v.capacity());
        }
        self.base = base;
        self.end = v.last().map_or(usize::MAX, |l| l.end as usize);
    }

    /// The vector's length up to the cursor.
    #[inline(always)]
    fn close(&mut self, out: &mut Out) {
        // SAFETY: the cursor is in the vector's allocation, at or past its start, every span
        // below it written
        unsafe {
            let n = self.w.offset_from(self.base) as usize;
            out.0.set_len(n);
        }
    }

    #[inline(always)]
    fn reopen(&mut self, out: &mut Out, n: usize) {
        self.open(out, n);
    }

    /// The line's spans from the vector's start.
    #[inline(always)]
    fn start_line(&mut self) {
        (self.w, self.end) = (self.base, usize::MAX);
    }
}

/// A highlighter's rule table as a run builder reads it: [`Rules::table`]'s, read again after
/// a kind is resolved.
#[derive(Clone, Copy)]
pub(crate) struct Table<'r> {
    tab: *const Rule,
    tlen: usize,
    rules: *mut (dyn Rules + 'r),
    _r: PhantomData<&'r mut ()>,
}

impl<'r> Table<'r> {
    #[inline(always)]
    pub(crate) fn new(rules: &'r mut dyn Rules) -> Self {
        let t = rules.table();
        Table {
            tab: t.as_ptr(),
            tlen: t.len(),
            rules,
            _r: PhantomData,
        }
    }

    /// `kind`'s rule, resolved where it isn't yet.
    #[inline(always)]
    fn get(&mut self, kind: u16) -> Rule {
        let k = usize::from(kind);
        let rule = if k < self.tlen {
            // SAFETY: below the table's length
            unsafe { *self.tab.add(k) }
        } else {
            Rule(Rule::UNSET)
        };
        if rule.0 == Rule::UNSET {
            return self.resolve(kind);
        }
        rule
    }

    /// `kind`'s rule resolved, the table read again.
    #[cold]
    #[inline(never)]
    fn resolve(&mut self, kind: u16) -> Rule {
        // SAFETY: the rules outlive the table, and nothing else holds them while it's read
        unsafe {
            let r = (*self.rules).resolve(kind);
            let t = (*self.rules).table();
            (self.tab, self.tlen) = (t.as_ptr(), t.len());
            r
        }
    }
}

/// A line's `spans`, of length `len`, as runs pushed to `runs`, by `t`'s rules: any scanner's,
/// for a language without a scan of its own on [`Runs`]. ada's loop as it was tuned there
/// (X-022, HL-004): the line's state in locals, which a store to the runs can't change; the
/// same on [`Runs`]' fields took ada's JSON path a third more cycles (X-068). Returns where the
/// runs end, `runs`' length.
#[inline(always)]
pub(crate) fn runs_of(len: usize, spans: &[Span], t: &mut Table, runs: &mut Vec<Run>) -> usize {
    let open = runs.len();
    // no `reserve` for a span and a gap each: the bound is twice what the theme leaves, and
    // a block's vector grown to it took JSON 57% more page faults and 4% of its time (X-022)

    // the line's last run's rule, held here rather than read back: a span whose rule is the
    // last one's stretches it, so the theme joins spans (X-022); unset before the first
    let mut last = Rule(Rule::UNSET);
    let mut put = |end: u32, rule: Rule| {
        if rule == last {
            let n = runs.len();
            runs[n - 1].end = end;
        } else {
            runs.push(Run { end, rule });
            last = rule;
        }
    };
    let mut at = 0;
    // whether a run has a rule, kept as they're pushed: no second pass over them (HL-004)
    let mut colored = false;
    // the table in locals, read again only after a kind is resolved: through `t` it was read a
    // span, ~4 instructions more each on JSON (X-068)
    let (mut tab, mut tlen) = (t.tab, t.tlen);
    for s in spans {
        let k = usize::from(s.kind);
        let mut rule = if k < tlen {
            // SAFETY: below the table's length
            unsafe { *tab.add(k) }
        } else {
            Rule(Rule::UNSET)
        };
        if rule.0 == Rule::UNSET {
            rule = t.resolve(s.kind);
            (tab, tlen) = (t.tab, t.tlen);
        }
        if s.start > at {
            put(s.start, Rule(Rule::NONE));
        }
        put(s.end, rule);
        colored |= rule.0 != Rule::NONE;
        at = s.end;
    }
    if !colored {
        runs.truncate(open);
    } else if (at as usize) < len {
        put(len as u32, Rule(Rule::NONE));
    }
    runs.len()
}

/// A line's runs by theme rule, joined by rule as they're written, on a write cursor in the
/// scan's own fields as [`Spans`]' is: a gap between lexemes is [`Rule::NONE`]'s, and a line
/// with no rule has no runs ([`Runs::end_line`]). Runs are a highlighter's own vector, from
/// line to line; the spans' vector holds only what an embedded scanner pushes, taken at once.
#[derive(Clone, Copy)]
pub(crate) struct Runs<'r> {
    t: Table<'r>,
    runs: *mut Vec<Run>,
    /// The last run's rule on the line, [`Rule::UNSET`] for none.
    last: Rule,
    /// Where the line's last run ends.
    end: usize,
    /// The line's first run.
    line: *mut Run,
    w: *mut Run,
    base: *mut Run,
    lim: *mut Run,
}

impl<'r> Runs<'r> {
    /// Runs written at `runs`' end, by `rules`; a line started.
    #[inline(always)]
    pub(crate) fn new(rules: &'r mut dyn Rules, runs: &'r mut Vec<Run>) -> Self {
        let mut r = Runs {
            t: Table::new(rules),
            runs,
            last: Rule(Rule::UNSET),
            end: 0,
            line: std::ptr::null_mut(),
            w: std::ptr::null_mut(),
            base: std::ptr::null_mut(),
            lim: std::ptr::null_mut(),
        };
        r.open_runs(64);
        r.line = r.w;
        r
    }

    /// The line's runs ended at `len`, its length: none where none has a rule, else a gap to
    /// the end [`Rule::NONE`]'s. Returns where the runs end, the vector's length.
    #[inline(always)]
    pub(crate) fn end_line(&mut self, len: usize) -> usize {
        // SAFETY: `line..w` are the line's runs, written
        let n = unsafe { self.w.offset_from(self.line) as usize };
        // runs next to each other differ in rule, so a line of more than one has one, and one of
        // one has one when its run's isn't plain: no flag kept a run
        let colored = n > 1 || (n == 1 && unsafe { (*self.line).rule.0 } != Rule::NONE);
        if !colored {
            self.w = self.line;
        } else if self.end < len {
            self.put(len, Rule(Rule::NONE));
        }
        // SAFETY: the cursor is in the vector's allocation, every run below it written
        unsafe {
            let n = self.w.offset_from(self.base) as usize;
            (*self.runs).set_len(n);
            n
        }
    }

    #[inline(always)]
    fn put(&mut self, end: usize, rule: Rule) {
        if rule == self.last {
            // SAFETY: `last` is a rule only once a run of the line is written below the cursor
            unsafe { (*self.w.sub(1)).end = end as u32 };
            return;
        }
        if self.w == self.lim {
            self.grow();
        }
        // SAFETY: below the limit, the vector's capacity, which nothing else changes while
        // the runs are open
        unsafe {
            self.w.write(Run {
                end: end as u32,
                rule,
            });
            self.w = self.w.add(1);
        }
        self.last = rule;
    }

    /// The runs opened at the vector's end, room made for `n` more.
    fn open_runs(&mut self, n: usize) {
        // SAFETY: the vector outlives the sink
        let v = unsafe { &mut *self.runs };
        v.reserve(n);
        let base = v.as_mut_ptr();
        // SAFETY: both within (or one past) the vector's allocation
        unsafe {
            self.w = base.add(v.len());
            self.lim = base.add(v.capacity());
        }
        self.base = base;
    }

    /// The runs' room doubled, at the limit, the line's start moved with them.
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        // SAFETY: the cursors are in the vector's allocation, every run below `w` written
        unsafe {
            let line = self.line.offset_from(self.base) as usize;
            let n = self.w.offset_from(self.base) as usize;
            (*self.runs).set_len(n);
            self.open_runs(n.max(64));
            self.line = self.base.add(line);
        }
    }
}

impl Sink for Runs<'_> {
    #[inline(always)]
    fn merge(&mut self, _: &mut &mut Out, s: usize, e: usize, kind: u16) {
        if e > s {
            self.merge_span(s, e, kind);
        }
    }

    #[inline(always)]
    fn open(&mut self, _: &mut Out, _: usize) {}

    #[inline(always)]
    fn close(&mut self, _: &mut Out) {}

    /// The spans another scanner pushed, as runs, the vector left empty.
    #[inline(always)]
    fn reopen(&mut self, out: &mut Out, _: usize) {
        if !out.0.is_empty() {
            self.take(out);
        }
    }

    #[inline(always)]
    fn start_line(&mut self) {
        (self.line, self.last, self.end) = (self.w, Rule(Rule::UNSET), 0);
    }
}

impl Runs<'_> {
    #[cold]
    #[inline(never)]
    fn take(&mut self, out: &mut Out) {
        for s in out.0.iter() {
            self.merge_span(s.start as usize, s.end as usize, s.kind);
        }
        out.0.clear();
    }

    /// [`Sink::merge`] of a span that isn't empty (a scanner's spans never are).
    #[inline(always)]
    fn merge_span(&mut self, s: usize, e: usize, kind: u16) {
        let rule = self.t.get(kind);
        // a gap before it plain, each run joined to the one before where their rules are the
        // same
        if s > self.end {
            self.put(s, Rule(Rule::NONE));
        }
        self.put(e, rule);
        self.end = e;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lang, State, lex, lex_lines, lex_lines_runs, lex_runs};

    /// A theme of a few rules over the kinds, plain for some, each resolved as it's met into a
    /// table grown as a highlighter's is.
    struct Lazy(Vec<Rule>);

    impl Rules for Lazy {
        fn table(&self) -> &[Rule] {
            &self.0
        }

        fn resolve(&mut self, kind: u16) -> Rule {
            let k = usize::from(kind);
            if k >= self.0.len() {
                self.0
                    .resize((2 * self.0.len()).max(k + 1), Rule(Rule::UNSET));
            }
            let rule = match kind % 5 {
                0 => Rule(Rule::NONE),
                r => Rule(r % 3),
            };
            self.0[k] = rule;
            rule
        }
    }

    const TEXT: &str = concat!(
        "fn main() {\n",
        "\tlet re = Regex::new(r\"^(?P<y>\\d{4})-[a-z]+$\").unwrap();\n",
        "    let q = sqlx::query!(\"SELECT a, b FROM t WHERE id = $1\", id);\n",
        "    // language=SQL\n",
        "    let s = r#\"UPDATE t\n",
        "\tSET a = 1\"#;\n",
        "\n",
        "    let plain = x;\n",
        "}\n",
        "/* open\n",
        "   still */ struct S;",
    );

    /// Runs from spans, the reference: each line's spans by [`lex_lines`], read into runs.
    fn from_spans(text: &str, prep: impl Fn(&str, &mut String) -> bool + Copy) -> Vec<Vec<Run>> {
        let (mut spans, mut buf, mut state) = (Vec::new(), String::new(), State::default());
        let (mut rules, mut runs, mut lines) = (Lazy(Vec::new()), Vec::new(), Vec::new());
        lex_lines(
            Lang::Rust,
            text,
            usize::MAX,
            &mut state,
            (&mut spans, &mut buf),
            prep,
            |l, s| {
                runs_of(l.len(), s, &mut Table::new(&mut rules), &mut runs);
                lines.push(std::mem::take(&mut runs));
            },
        );
        lines
    }

    /// Rust's runs written by its scan are those its spans give, on a block's pass and a line at
    /// a time: through tabs expanded apart, an embedded regex's and SQL's spans, a comment and
    /// a raw string over lines, and plain lines (no runs).
    #[test]
    fn rust_runs_are_its_spans() {
        let tabs = |l: &str, b: &mut String| {
            b.clear();
            b.push_str(&l.replace('\t', "    "));
            l.contains('\t')
        };
        let none = |_: &str, _: &mut String| false;
        for text in [
            TEXT,
            include_str!("rust/mod.rs"),
            include_str!("rust/code.rs"),
        ] {
            for (name, prep) in [
                ("tabs", &tabs as &dyn Fn(&str, &mut String) -> bool),
                ("none", &none),
            ] {
                let want = from_spans(text, |l, b| prep(l, b));
                let (mut spans, mut buf, mut state) = (Vec::new(), String::new(), State::default());
                let (mut rules, mut runs, mut ends) = (Lazy(Vec::new()), Vec::new(), vec![0]);
                let prep2 = |l: &str, b: &mut String| prep(l, b);
                let rest = lex_lines_runs(
                    Lang::Rust,
                    text,
                    usize::MAX,
                    &mut state,
                    (&mut spans, &mut buf),
                    prep2,
                    &mut rules,
                    &mut runs,
                    |n| ends.push(n),
                );
                assert_eq!(rest, None);
                let got: Vec<_> = ends.windows(2).map(|w| runs[w[0]..w[1]].to_vec()).collect();
                assert_eq!(got.len(), want.len(), "{name}");
                for (n, (g, w)) in got.iter().zip(&want).enumerate() {
                    assert_eq!(g, w, "{name}: line {n}");
                }
                // a line at a time, the state handed on
                let (mut state, mut rules, mut runs) =
                    (State::default(), Lazy(Vec::new()), Vec::new());
                for (n, line) in text.split('\n').enumerate() {
                    let line = if prep(line, &mut buf) {
                        buf.clone()
                    } else {
                        line.to_owned()
                    };
                    runs.clear();
                    lex_runs(
                        Lang::Rust,
                        &line,
                        &mut state,
                        &mut spans,
                        &mut rules,
                        &mut runs,
                    );
                    assert_eq!(runs, want[n], "{name}: line {n} alone");
                }
            }
        }
        // another language's runs are its spans' too, by the same rules
        let (mut state, mut spans, mut rules, mut runs) =
            (State::default(), Vec::new(), Lazy(Vec::new()), Vec::new());
        lex_runs(
            Lang::Java,
            "int a = 1; // x",
            &mut state,
            &mut spans,
            &mut rules,
            &mut runs,
        );
        let mut want = Vec::new();
        lex(
            Lang::Java,
            "int a = 1; // x",
            &mut State::default(),
            &mut spans,
        );
        runs_of(
            15,
            &spans,
            &mut Table::new(&mut Lazy(Vec::new())),
            &mut want,
        );
        assert_eq!(runs, want);
    }

    /// A gap between spans is plain, a run joins the one before of its rule, a line with no rule
    /// has no runs, and one with one ends at the line's end.
    #[test]
    fn runs_from_spans() {
        let span = |start, end, kind| Span { start, end, kind };
        let mut table = Lazy(vec![Rule(Rule::NONE), Rule(1), Rule(1), Rule(2)]);
        let mut runs = Vec::new();
        let t = &mut Table::new(&mut table);
        assert_eq!(runs_of(4, &[span(0, 4, 0)], t, &mut runs), 0);
        let line = [span(1, 2, 1), span(2, 3, 2), span(4, 5, 3)];
        assert_eq!(runs_of(9, &line, t, &mut runs), 5);
        assert_eq!(runs_of(3, &[span(0, 3, 3)], t, &mut runs), 6);
        let run = |end, rule| Run {
            end,
            rule: Rule(rule),
        };
        let none = Rule::NONE;
        assert_eq!(
            runs,
            [
                run(1, none),
                run(3, 1),
                run(4, none),
                run(5, 2),
                run(9, none),
                run(3, 2)
            ]
        );
    }
}
