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
    /// As [`Sink::merge`], but on a span sink not joined to the span before: the same where
    /// lexemes are joined by rule anyway.
    #[inline(always)]
    fn push(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        self.merge(out, s, e, kind);
    }
    /// As [`Sink::push`], for a scan whose writes sit in small helpers inlined at many places: on
    /// the run sink one call, so that each place isn't a copy of the run's write (the Lisp
    /// engine's, X-074); elsewhere the write itself.
    #[inline(always)]
    fn push_call(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        self.push(out, s, e, kind);
    }
    /// As [`Sink::merge`], one call on the run sink as [`Sink::push_call`] is.
    #[inline(always)]
    fn merge_call(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        self.merge(out, s, e, kind);
    }
    /// Opened for writing at `out`'s end, room made there for `n` more spans.
    fn open(&mut self, out: &mut Out, n: usize);
    /// Closed for another scanner to push spans to `out`.
    fn close(&mut self, out: &mut Out);
    /// Opened again after another scanner pushed spans to `out` ([`Sink::close`]).
    fn reopen(&mut self, out: &mut Out, n: usize);
    /// A line starts, on a sink open since an earlier one (a block's pass).
    fn start_line(&mut self);
    /// Where the writing stands, for [`Sink::undo`].
    fn mark(&self, out: &Out) -> Mark;
    /// What was written since `m` undone, as if it never was: a scan's guess at a line's
    /// lexemes taken back (Java's type arguments read ahead, Kotlin's string before `.toRegex(`,
    /// X-074).
    fn undo(&mut self, out: &mut Out, m: Mark);
    /// Whether this is the run sink, where [`Sink::host`] writes a guest's runs.
    const RUNS: bool = false;
    /// The run sink only ([`Sink::RUNS`]): another scanner's runs of the line's text from `at`
    /// written in place by `lex`, which is handed the rules with its kinds `base` above the
    /// host's (a view of the host's table) and the runs' vector at the line's runs so far, and
    /// pushes its runs as for a line of its own (its ends from `at`, a gap to its end plain):
    /// X-151's bash in Nix's scripts, with no spans between them. Every write before is at or
    /// before `at`.
    fn host(
        &mut self,
        _at: usize,
        _base: u16,
        _lex: &mut dyn FnMut(&mut dyn Rules, &mut Vec<Run>),
    ) {
        unreachable!("a guest's runs on a sink of spans");
    }
}

/// A host's rules for a guest's kinds, `base` below the host's ([`Sink::host`]): its table from
/// `base` on, read again by the guest's run sink after a kind is resolved.
struct Shifted<'a> {
    rules: &'a mut dyn Rules,
    base: u16,
}

impl Rules for Shifted<'_> {
    fn table(&self) -> &[Rule] {
        self.rules
            .table()
            .get(usize::from(self.base)..)
            .unwrap_or(&[])
    }

    fn resolve(&mut self, kind: u16) -> Rule {
        self.rules.resolve(kind + self.base)
    }
}

/// Where a sink's writing stood ([`Sink::mark`]): its spans' or runs' count, where the last ends
/// ([`u32::MAX`] for none on the line), and for a sink that keeps them the line's end and last
/// rule. Writes after it only add to the count or move the last one's end, so these undo them.
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    n: usize,
    prev: u32,
    end: usize,
    last: Rule,
}

impl Mark {
    /// A mark of nothing written, for a slot a scan fills before it reads it.
    pub(crate) const NONE: Mark = Mark {
        n: 0,
        prev: u32::MAX,
        end: 0,
        last: Rule(Rule::NONE),
    };
}

/// A line's spans pushed to the vector as [`Out`] pushes them, no cursor: the span API of a
/// scanner whose spans went that way before it was generic over its sink (the line-format
/// engine, X-069), so that API's code stays as it was.
#[derive(Clone, Copy)]
pub(crate) struct Push;

impl Sink for Push {
    #[inline(always)]
    fn merge(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        out.push_merged(s, e, kind);
    }

    #[inline(always)]
    fn push(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        out.push(s, e, kind);
    }

    #[inline(always)]
    fn open(&mut self, _: &mut Out, _: usize) {}

    #[inline(always)]
    fn close(&mut self, _: &mut Out) {}

    #[inline(always)]
    fn reopen(&mut self, _: &mut Out, _: usize) {}

    #[inline(always)]
    fn start_line(&mut self) {}

    #[inline(always)]
    fn mark(&self, out: &Out) -> Mark {
        Mark {
            n: out.0.len(),
            prev: out.0.last().map_or(u32::MAX, |l| l.end),
            end: 0,
            last: Rule(Rule::UNSET),
        }
    }

    #[inline(always)]
    fn undo(&mut self, out: &mut Out, m: Mark) {
        out.0.truncate(m.n);
        if let Some(l) = out.0.last_mut() {
            l.end = m.prev;
        }
    }
}

/// A sink beside the spans' vector, written as [`Out`] is (`push`, `push_merged`): for a scan of
/// free functions that took the vector (Dockerfile's, X-074), so that its code reads as
/// it did, and on [`Push`] compiles as it did.
pub(crate) struct To<'a, 'o, K: Sink> {
    out: &'a mut Out<'o>,
    k: &'a mut K,
}

impl<'a, 'o, K: Sink> To<'a, 'o, K> {
    #[inline(always)]
    pub(crate) fn new(out: &'a mut Out<'o>, k: &'a mut K) -> Self {
        To { out, k }
    }

    /// Bytes `s..e` are `kind`; an empty stretch is nothing ([`Out::push`]). One call on the run
    /// sink ([`Sink::push_call`]): these scans write a few spans a line from many places.
    #[inline(always)]
    pub(crate) fn push(&mut self, s: usize, e: usize, kind: u16) {
        self.k.push_call(&mut self.out, s, e, kind);
    }
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

    #[inline(always)]
    fn mark(&self, _: &Out) -> Mark {
        // SAFETY: the cursor is in the vector's allocation, past its start; `end` is a span's
        // only once one is written below it
        unsafe {
            Mark {
                n: self.w.offset_from(self.base) as usize,
                prev: if self.end == usize::MAX {
                    u32::MAX
                } else {
                    (*self.w.sub(1)).end
                },
                end: self.end,
                last: Rule(Rule::UNSET),
            }
        }
    }

    #[inline(always)]
    fn undo(&mut self, _: &mut Out, m: Mark) {
        // SAFETY: at or below the cursor, the spans there written; the cursor's room never
        // shrinks, so the count holds across a growth
        unsafe {
            self.w = self.base.add(m.n);
            if m.end != usize::MAX {
                (*self.w.sub(1)).end = m.prev;
            }
        }
        self.end = m.end;
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
            self.reload();
            r
        }
    }

    /// The table read again, after another reader of the rules resolved kinds ([`Sink::host`]).
    #[inline(always)]
    fn reload(&mut self) {
        // SAFETY: as above
        let t = unsafe { (*self.rules).table() };
        (self.tab, self.tlen) = (t.as_ptr(), t.len());
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

    #[inline(never)]
    fn push_call(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        self.merge(out, s, e, kind);
    }

    #[inline(never)]
    fn merge_call(&mut self, out: &mut &mut Out, s: usize, e: usize, kind: u16) {
        self.merge(out, s, e, kind);
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

    #[inline(always)]
    fn mark(&self, _: &Out) -> Mark {
        // SAFETY: the cursors are in the vector's allocation, `line` at or below `w`, the runs
        // below `w` written
        unsafe {
            Mark {
                n: self.w.offset_from(self.base) as usize,
                prev: if self.w > self.line {
                    (*self.w.sub(1)).end
                } else {
                    u32::MAX
                },
                end: self.end,
                last: self.last,
            }
        }
    }

    #[inline(always)]
    fn undo(&mut self, _: &mut Out, m: Mark) {
        // SAFETY: at or below the cursor, past the line's start where `prev` is a run's (a
        // growth moves `line` with the runs and keeps the count)
        unsafe {
            self.w = self.base.add(m.n);
            if m.prev != u32::MAX {
                (*self.w.sub(1)).end = m.prev;
            }
        }
        (self.end, self.last) = (m.end, m.last);
    }

    const RUNS: bool = true;

    /// The guest's runs pushed to the vector as it stands at the cursor (its length set there),
    /// after a plain gap to `at`; their ends moved from its text's to the line's, the first
    /// joined to the host's last where their rules are the same; the cursor, the table and the
    /// line's last rule and end read again from the vector.
    #[inline(never)]
    fn host(&mut self, at: usize, base: u16, lex: &mut dyn FnMut(&mut dyn Rules, &mut Vec<Run>)) {
        debug_assert!(self.end <= at);
        if self.end < at {
            self.put(at, Rule(Rule::NONE));
            self.end = at;
        }
        // SAFETY: the cursors are in the vector's allocation, every run below `w` written; the
        // vector and the rules outlive the sink, and nothing else holds them while the guest
        // writes
        unsafe {
            let line = self.line.offset_from(self.base) as usize;
            let n0 = self.w.offset_from(self.base) as usize;
            let v = &mut *self.runs;
            v.set_len(n0);
            lex(
                &mut Shifted {
                    rules: &mut *self.t.rules,
                    base,
                },
                v,
            );
            let n1 = v.len();
            for r in &mut v[n0..n1] {
                r.end += at as u32;
            }
            if n1 > n0 && n0 > line && v[n0 - 1].rule == v[n0].rule {
                v[n0 - 1].end = v[n0].end;
                v.remove(n0);
            }
            self.t.reload();
            self.open_runs(0);
            self.line = self.base.add(line);
            if self.w > self.line {
                let last = *self.w.sub(1);
                (self.last, self.end) = (last.rule, last.end as usize);
            }
        }
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
    pub(crate) fn merge_span(&mut self, s: usize, e: usize, kind: u16) {
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
        lang_from_spans(Lang::Rust, text, prep)
    }

    /// [`from_spans`] in `lang`.
    fn lang_from_spans(
        lang: Lang,
        text: &str,
        prep: impl Fn(&str, &mut String) -> bool + Copy,
    ) -> Vec<Vec<Run>> {
        let (mut spans, mut buf, mut state) = (Vec::new(), String::new(), State::default());
        let (mut rules, mut runs, mut lines) = (Lazy(Vec::new()), Vec::new(), Vec::new());
        lex_lines(
            lang,
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
        let tabs: crate::Prep = |l, b| {
            b.clear();
            b.push_str(&l.replace('\t', "    "));
            l.contains('\t')
        };
        for text in [TEXT, include_str!("rust/mod.rs"), include_str!("lib.rs")] {
            for (name, prep) in [("tabs", Some(tabs)), ("none", None)] {
                let prep2 = |l: &str, b: &mut String| prep.is_some_and(|p: crate::Prep| p(l, b));
                let want = from_spans(text, prep2);
                let (mut spans, mut buf, mut state) = (Vec::new(), String::new(), State::default());
                let (mut rules, mut runs, mut ends) = (Lazy(Vec::new()), Vec::new(), vec![0]);
                let rest = lex_lines_runs(
                    Lang::Rust,
                    text,
                    usize::MAX,
                    &mut state,
                    (&mut spans, &mut buf),
                    prep,
                    &mut rules,
                    &mut runs,
                    &mut ends,
                );
                assert_eq!(rest, None);
                let got: Vec<_> = ends
                    .windows(2)
                    .map(|w| runs[w[0] as usize..w[1] as usize].to_vec())
                    .collect();
                assert_eq!(got.len(), want.len(), "{name}");
                for (n, (g, w)) in got.iter().zip(&want).enumerate() {
                    assert_eq!(g, w, "{name}: line {n}");
                }
                // a line at a time, the state handed on
                let (mut state, mut rules, mut runs) =
                    (State::default(), Lazy(Vec::new()), Vec::new());
                for (n, line) in text.split('\n').enumerate() {
                    let line = if prep2(line, &mut buf) {
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

    /// The line-format engine's runs written by its scans (X-069) are those its spans give: a
    /// block's lines on one sink, in blocks of a few lines and a line at a time, through what a
    /// line leaves open (a value run on by `\`, a quote not closed), comments, plain lines and
    /// blank ones.
    #[test]
    fn line_format_runs_are_their_spans() {
        let texts: [(Lang, &str); 10] = [
            (
                Lang::Ini,
                "; c\n[sec]\nkey = value ; x\n\n  name=\"q\"\n[bad\nplain\n# h\n",
            ),
            (
                Lang::Properties,
                "# c\n! d\nkey = a \\\n  b\\\n c\nk2: v\\u00e9\n\nk3 v3\n=x",
            ),
            (
                Lang::DotEnv,
                "# c\nexport A=1\nB=\"x ${A} \\\"\n y\"\nC='z\n'\nD=$(cmd) # t\n\nE",
            ),
            (
                Lang::GitIgnore,
                "# c\n*.o\n!keep/**\n/build/\n[a-z]?.txt\n\n\\#x",
            ),
            (
                Lang::GitAttributes,
                "# c\n*.rs text eol=lf diff=rust\n[attr]bin -diff -merge\n* !x -y\n\n\"a b\" z",
            ),
            (
                Lang::GitMailmap,
                "# c\nA B <a@b> <c@d>\n<e@f> Name <g@h>\nbad <x\n\n",
            ),
            (Lang::GitLink, "gitdir: ../.git/modules/x\nother\n\n"),
            (
                Lang::Fstab,
                "# c\nUUID=ab-12 / ext4 defaults,noatime 0 1\n/dev/sda1\t/boot vfat rw 0 2\nbad\n\n",
            ),
            (
                Lang::PkgConfig,
                "# c\nprefix=@P@\nlibdir=${prefix}/lib # d\nName: x\nCflags: -I${a} \\\n  -DB\nRequires: y >= 1, z\n\n",
            ),
            (
                Lang::KnownHosts,
                "# c\nhost,1.2.3.4 ssh-ed25519 AAAAC3Nz\n[h]:22 ecdsa-sha2-nistp256 AAAA x\n|1|aGFz|aGFz ssh-rsa AAAA\n@revoked * ssh-rsa AAAA\n\n",
            ),
        ];
        for (lang, text) in texts {
            runs_are_spans(lang, text);
        }
    }

    /// `lang`'s runs of `text` are those its spans give, in blocks of a few lines and a line at a
    /// time, the state handed on.
    fn runs_are_spans(lang: Lang, text: &str) {
        let none = |_: &str, _: &mut String| false;
        let want = lang_from_spans(lang, text, none);
        for max in [usize::MAX, 1, 2, 3] {
            let (mut spans, mut buf, mut state) = (Vec::new(), String::new(), State::default());
            let (mut rules, mut runs, mut got) = (Lazy(Vec::new()), Vec::new(), Vec::new());
            let mut rest = Some(0);
            while let Some(p) = rest {
                let mut ends = vec![u32::try_from(runs.len()).unwrap()];
                let next = lex_lines_runs(
                    lang,
                    &text[p..],
                    max,
                    &mut state,
                    (&mut spans, &mut buf),
                    None,
                    &mut rules,
                    &mut runs,
                    &mut ends,
                );
                got.extend(
                    ends.windows(2)
                        .map(|w| runs[w[0] as usize..w[1] as usize].to_vec()),
                );
                rest = next.map(|q| p + q);
            }
            assert_eq!(got, want, "{lang:?} in blocks of {max}");
        }
        let (mut state, mut spans, mut rules, mut runs) =
            (State::default(), Vec::new(), Lazy(Vec::new()), Vec::new());
        for (n, line) in text.split('\n').enumerate() {
            runs.clear();
            lex_runs(lang, line, &mut state, &mut spans, &mut rules, &mut runs);
            assert_eq!(runs, want[n], "{lang:?}: line {n} alone");
        }
    }

    /// The runs written by the scans of X-072 are those their spans give: JSON's through keys,
    /// escapes, numbers, words, and a `/* */` comment over lines; TOML's; XML's.
    #[test]
    fn scans_runs_are_their_spans() {
        runs_are_spans(
            Lang::Json,
            concat!(
                "{\"a\": 1, \"b\\\"\" : [1.5, -2e3, true, null, NaN], // c\n",
                "  \"s\": \"x\\u00e9\\q\", /* open\n",
                "\n",
                "  still */ \"k\": {}, \"open\n",
                "  [ ] , \"/*\" /**/ }",
            ),
        );
        // TOML's through headers, inline tables and arrays over lines, multi-line strings, dates,
        // and bad keys and array items (spans joined)
        runs_are_spans(
            Lang::Toml,
            concat!(
                "# c\n[table]\nkey = \"v\" # t\n\"q k\".x = 1_000\n[[arr]]\nd = 1979-05-27T07:32:00Z\n",
                "a = [\n  1, 2.5, # c\n  \"x\",\n]\nt = { a = 1, b = { c = [true] } }\n",
                "m = \"\"\"\nmulti \\u00e9\n\"\"\"\nl = '''\nlit'''\nbad key = ?\nx = [1 2 @]\n[bad\n",
            ),
        );
        // XML's through a doctype and its subset, comments, CDATA and attributes over lines, and
        // processing instructions
        runs_are_spans(
            Lang::Xml,
            concat!(
                "<?xml version=\"1.0\"?>\n<!DOCTYPE a [\n  <!ELEMENT a (b|c)*>\n  <!ENTITY e \"x\">\n]>\n",
                "<a x=\"1\"\n   y='2'>text &e; &#x41;<!-- c\n still -->\n<![CDATA[ raw\n<b>]]>",
                "<b:c/></a>\n<?pi data?>\n< bad\n",
            ),
        );
    }

    /// HTML's runs written by its scan (X-076) are those its spans give: through a doctype, a
    /// comment, entities, attributes over lines, and its embeds, a `<style>` and a `<script>`
    /// over lines and `style="…"` and `on…="…"` (their spans read into the runs).
    #[test]
    fn html_runs_are_its_spans() {
        runs_are_spans(
            Lang::Html,
            concat!(
                "<!DOCTYPE html>\n<html lang=\"en\"><!-- c\n still --><head>\n",
                "<style>\n  a:hover { color: red; /* open\n  still */ }\n</style>\n",
                "<script type=\"module\">\n  const s = `t ${x}\n  more`; if (a < b) f();\n</script>\n",
                "<p class=\"x\" style=\"color: red; margin: 0\" onclick=\"go(1)\">a &amp; b &#x41;\n",
                "<a href='x'\n   title=\"t\">l</a><br/><custom-el data-x=y></custom-el>\n",
                "<div style=\"color:\n red\">x</div><script>let a = '</script>' <b>\n\n",
                "</p></html>",
            ),
        );
    }

    /// Java's runs written by its scan (X-074) are those its spans give: through type arguments
    /// read ahead and taken back (a `<` that doesn't close on its line, one that holds an
    /// annotation), an embedded regex and SQL, Javadoc, a text block and a comment over lines.
    #[test]
    fn java_runs_are_its_spans() {
        runs_are_spans(
            Lang::Java,
            concat!(
                "package a.b;\nimport java.util.*;\n/** Doc {@link Map}\n * @param x the <b>x</b>\n */\n",
                "@Deprecated(since = \"1\") public class A<T extends Map<String, List<Integer>>> {\n",
                "  private Map<String,\n      List<int[]>> m = new HashMap<>();\n",
                "  List<@NonNull String> n; int x = a < b ? 1 : c >> 2;\n",
                "  Pattern p = Pattern.compile(\"^(?<y>\\\\d{4})-[a-z]+$\");\n",
                "  @Language(\"SQL\") String q = \"SELECT a, b FROM t WHERE id = ?\";\n",
                "  String t = \"\"\"\n    text \\n block\n    \"\"\";\n",
                "  /* open\n  still */ void f(int... a) -> { return; }\n\n}\n",
            ),
        );
    }

    /// Kotlin's runs written by its scan (X-074, X-130) are those its spans give: through a string
    /// lexed again as a regex before `.toRegex(` (its text's run joined to its quote's where their
    /// rules are the same), templates (one open across lines), an embedded SQL string, raw strings,
    /// declarations' heads and parameters, and a comment over lines.
    #[test]
    fn kotlin_runs_are_its_spans() {
        runs_are_spans(
            Lang::Kotlin,
            concat!(
                "package a.b\nimport kotlin.text.*\n/** Doc [Map] */\n@JvmStatic fun <T> f(x: List<T>): Int {\n",
                "    val r = \"^(\\\\d{4})-[a-z]+$\".toRegex()\n    val p = \"a.b\".toPattern(); val s = \"x $y ${z + 1}\"\n",
                "    val q = \"SELECT a, b FROM t WHERE id = ?\" // language=SQL\n",
                "    @Language(\"SQL\") val u = \"UPDATE t SET a = 1\"\n",
                "    val t = \"\"\"\n      raw $v\n    \"\"\".trimIndent()\n",
                "    /* open\n  still */ return when (x) { is A -> 1; else -> 2 }\n\n}\n",
                "class C<in T>(private val a: Int = 1) : B(a) { val n = \"a ${f {\n x }} b\" }\n",
            ),
        );
    }

    /// TypeScript's and TSX's runs written by their scan (X-074) are those their spans give:
    /// through templates and their substitutions over lines, a regex literal and a regex string,
    /// JSDoc, types, JSX's elements, attributes and text, and a comment over lines.
    #[test]
    fn typescript_runs_are_its_spans() {
        let ts = concat!(
            "import { a, b as c } from \"./m\";\n/** Doc {@link Map} @param {string} x */\n",
            "export class A<T extends Map<string, number[]>> implements I {\n",
            "  private m: Record<string, (x: number) => void> = {};\n",
            "  f(x = /^(?<y>\\d{4})-[a-z]+$/gi, y = new RegExp(\"a+b\")): string {\n",
            "    return `t ${x + `in ${y}`}\n  more ${z}`;\n  }\n",
            "  /* open\n  still */ g = async <U,>(u: U) => u satisfies U ?? null;\n\n}\n",
        );
        runs_are_spans(Lang::TypeScript, ts);
        runs_are_spans(Lang::TypeScriptReact, ts);
        runs_are_spans(
            Lang::TypeScriptReact,
            concat!(
                "const C = ({ a }: P) => (\n  <div className=\"x\" onClick={() => f(a)}>\n",
                "    text {a ? <b>y</b> : null} &amp; more\n    <Foo.Bar {...p} k='v' />\n  </div>\n);\n",
            ),
        );
    }

    /// Python's runs written by its scan (X-074) are those its spans give: through f-strings and
    /// their fields, a regex string and a SQL string, docstrings and strings over lines, and
    /// decorators.
    #[test]
    fn python_runs_are_its_spans() {
        runs_are_spans(
            Lang::Python,
            concat!(
                "import re, os.path as p\nfrom a.b import (c,\n    d)\n@dataclass(frozen=True)\n",
                "class A(B, metaclass=M):\n    \"\"\"Doc\n    more\"\"\"\n",
                "    def f(self, x: int = 1, *a, **k) -> list[str]:\n",
                "        r = re.compile(r\"^(?P<y>\\d{4})-[a-z]+$\")\n",
                "        q = \"SELECT a, b FROM t WHERE id = ?\"\n",
                "        s = f\"x {a + 1!r:>{w}} y {{z}}\" b'\\x00' \\\n            'cont'\n",
                "        return [i ** 2 for i in range(10) if i % 2]  # c\n\nlambda: None\n",
            ),
        );
    }

    /// The Lisp engine's runs (four dialects) and Clojure's, written by their scans (X-074), are
    /// those their spans give: through `#e`/`#i`/`#d` read ahead and taken back (a run of the
    /// prefix's rule joined to the one before), `|…|` symbols, regexes, strings and comments
    /// over lines, and lists past the depths' kinds.
    #[test]
    fn lisp_runs_are_their_spans() {
        let lisp = concat!(
            "; c\n(defun f (x &optional (y 2)) \"doc\n  more\" (let ((z #e1.5) (w #ifoo) (v #x1F))\n",
            "  (format t \"~a~%\" '|odd sym| #\\Space #'car `(,x ,@y)) #| block\n still |# (((((((((x)))))))))))\n",
            "#!/usr/bin/env racket\n#lang racket\n(define (g [a 1] #:k [k 2]) (regexp-match #rx\"^a(b|c)+$\" a))\n",
            "[vector] {brace} #;(dropped) #(1 2) #e #d12 #i#x1F 1/2 -3.5e10\n",
        );
        for lang in [Lang::Lisp, Lang::EmacsLisp, Lang::Scheme, Lang::Racket] {
            runs_are_spans(lang, lisp);
        }
        runs_are_spans(
            Lang::Clojure,
            concat!(
                "(ns a.b (:require [c.d :as e]))\n;; c\n(defn f \"doc\n  more\" [x & {:keys [y]}]\n",
                "  (let [r #\"^a(b|c)+\\d$\" m {:k 1, 'q \\c} s #{1 2}] (re-find r \"s\\n\")))\n",
                "#?(:clj 1 :cljs 2) @(atom 1) ^:meta #_ (dropped) (((((((((x)))))))))\n",
            ),
        );
    }

    /// Dockerfile's runs written by its scan (X-074) are those its spans give, and Diff's (X-163)
    /// too.
    #[test]
    fn dockerfile_and_diff_runs_are_their_spans() {
        runs_are_spans(
            Lang::Dockerfile,
            concat!(
                "# syntax=docker/dockerfile:1\nARG V=1\nFROM rust:1.80 AS build\nWORKDIR /src\n",
                "COPY --from=build [\"a b\", \"/c\"]\nRUN cargo build \\\n    --release \"x\n  y\" 'z' && \\\n",
                "\n    echo \\\"q\\\" # c\nONBUILD RUN make\nENV A=\"1\" B=2\nbad line\n",
            ),
        );
        runs_are_spans(
            Lang::Diff,
            concat!(
                "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -1,3 +1,4 @@ fn main\n",
                " same\n-gone\n+new\n\\ No newline at end of file\n*** 1,2 ****\n--- 3,4 ----\n",
                "! changed\n> right\n< left\n---\nIndex: f.c\n==== //depot/a#1 - /b ====\n1c1\n",
            ),
        );
    }

    /// C#'s runs written by its scan (X-074) are those its spans give: through lone braces and
    /// blank lines (taken without the scan), `using` lines, an embedded regex, doc comments,
    /// interpolated and verbatim strings over lines, and a comment over lines.
    #[test]
    fn csharp_runs_are_its_spans() {
        runs_are_spans(
            Lang::CSharp,
            concat!(
                "using System;\nusing System.Text.RegularExpressions;\nnamespace A.B\n{\n",
                "    /// <summary>Doc <see cref=\"Map\"/></summary>\n    public class C<T> : I where T : new()\n    {\n",
                "\n        private Dictionary<string, List<int>> m = new();\n",
                "        bool r = Regex.IsMatch(s, @\"^(?<y>\\d{4})-[a-z]+$\");\n",
                "        string t = $\"x {a + 1:N2} y\", v = @\"multi\n  line\";\n",
                "        /* open\n  still */ void F(int a) => a switch { 1 => 2, _ => 3 };\n",
                "    }\n}\n",
            ),
        );
    }

    /// Go's and Lua's runs written by their scans (X-075) are those their spans give: Go's through
    /// an embedded regex and SQL, raw strings and a comment over lines; Lua's through long
    /// brackets of levels and `\z`.
    #[test]
    fn go_and_lua_runs_are_their_spans() {
        runs_are_spans(
            Lang::Go,
            concat!(
                "package main\nimport (\n\t\"fmt\"\n\tre \"regexp\"\n)\n// c\n",
                "func (s *S) F(x []int, m map[string]int) (int, error) {\n",
                "\tr := regexp.MustCompile(`^(?P<y>\\d{4})-[a-z]+$`)\n\tq := re.MustCompile(\"a+\\\\d\")\n",
                "\trows, _ := db.Query(\"SELECT a, b FROM t WHERE id = $1\", id)\n",
                "\t// language=SQL\n\tu := `UPDATE t\n\tSET a = 1`\n",
                "\ts := fmt.Sprintf(\"%d %v\\n\", 0x1F, 'c')\n\t/* open\n still */ return len(x), nil\n\n}\n",
            ),
        );
        runs_are_spans(
            Lang::Lua,
            concat!(
                "-- c\nlocal function f(a, ...)\n  local s = [==[long\n  ]] still]==] .. \"x\\z\n",
                "     y\" .. 'q\\n'\n  --[[ block\n  ]] return #a + 0x1F, a:m(b), t[1]\nend\n",
                "--[=[ open\n\n]=] goto done ::done:: print(not nil and true or false)\n",
            ),
        );
    }

    /// C++'s and C's runs written by their scan (X-057) are those their spans give: through the
    /// preprocessor (an `#if 0` branch, a macro over lines), a class and its methods, templates,
    /// a raw string over lines, members, calls, numbers, strings and comments.
    #[test]
    fn cpp_and_c_runs_are_their_spans() {
        runs_are_spans(
            Lang::Cpp,
            concat!(
                "#include <vector>\n#define MAX(a, b) \\\n  ((a) > (b))\n#if 0\nold();\n#else\n",
                "namespace ns {\ntemplate <typename T>\nclass Foo : public Bar<T> {\n public:\n",
                "  explicit Foo(int x = 0) : x_(x) {}\n  int get() const { return x_ + 0x1F + 1.5f; }\n",
                "  // c\n  std::vector<int> v{1, 2};\n};\n}  // ns\n#endif\n",
                "auto s = R\"d(one\n)\" still )d\"; /* open\n * still */ int main() {\n",
                "  for (auto& e : v) { e->call(a.b, \"x\\n%d\", 'c'); }\n  return mFoo == nullptr;\n}\n",
            ),
        );
        runs_are_spans(
            Lang::C,
            concat!(
                "#include \"a.h\"\nstatic const int k = 0x7fUL; /* c\n */\n",
                "struct s { int a; char *b; };\nint f(int a, char b[]) {\n",
                "  if (a > 10) return a.b + sizeof(int);\n  printf(\"%d\\n\", 1e3);\n  // c\n}\n",
            ),
        );
    }

    /// JavaScript's runs written by its scan (X-058) are those its spans give: through a
    /// function, an arrow and a class, a regex literal and `RegExp(…)`'s string (the regex
    /// scanner's spans taken as runs), a template over lines, JSX and a comment over lines.
    #[test]
    fn javascript_runs_are_its_spans() {
        runs_are_spans(
            Lang::JavaScript,
            concat!(
                "#!/usr/bin/env node\n'use strict';\nimport { a as b } from \"x\";\n",
                "function f(a, b = 1) { return a / b; }\nconst g = async (x, y) => x + y, h = z => z;\n",
                "class A extends B {\n  static get x() { return this.y; }\n  m(p) { super.m(p); }\n}\n",
                "let r = /^a[/.]b\\/(?<n>c)/gu, s = new RegExp(\"(\\\\d+)\", 'g');\n",
                "const t = `a ${b + `c${d}`} e\nf`;\nconst j = (\n  <div className=\"x\">{y}&amp;</div>\n);\n",
                "/* open\n still */ obj.key = function* () {}; o = { k: 1, 'q': [2, -3.5e1], m() {} };\n",
            ),
        );
    }

    /// SCSS's runs written by its scan (X-075) are those its spans give: through CSS's selectors,
    /// at-rules, functions, strings and a comment over lines, its front matter (YAML's spans
    /// taken as runs), interpolation, maps and `//` comments. (CSS's are read from its spans:
    /// HTML's embeds keep its span copy in ada, X-075.)
    #[test]
    fn scss_runs_are_its_spans() {
        let css = concat!(
            "@charset \"utf-8\";\n@import url(\"a.css\") screen and (min-width: 600px);\n",
            ":root { --main: #1f2; }\na.b > #c[href^='x' i]::before, li:nth-child(2n+1) {\n",
            "  color: rgb(0 128 255 / 50%); margin: -1.5em auto !important;\n",
            "  font: italic bold 12px/30px Georgia, \"Times New\", serif; /* open\n",
            "  still */ background: url(x.png) no-repeat;\n}\n",
            "@media (prefers-color-scheme: dark) { body { content: \"q\\\"\n\" } }\n",
            "@keyframes k { from { top: 0 } 50.5% { top: calc(100% - var(--h, 2px)) } }\n",
        );
        runs_are_spans(Lang::Scss, css);
        runs_are_spans(
            Lang::Scss,
            concat!(
                "---\ntitle: \"x\"\nlist: [1, 2]\n---\n@use \"sass:math\";\n$map: (a: 1, 'b': 2px);\n",
                "// c\n@mixin m($x, $y: 10px) { width: math.div($x, 2) + $y; }\n",
                ".a-#{$n} { &:hover { @include m(1px); } %ph { @extend .b; } }\n",
                "@each $k, $v in $map { .#{$k} { v: if($v > 1, $v * 2, null); } }\n",
                "@function f($a...) { @return nth($a, 1); } /* open\n still */ .c { d: e }\n",
            ),
        );
    }

    /// Nix's runs are those its spans give, where its scripts' lines are bash's runs written on
    /// its sink (X-151, [`Sink::host`]) and where they're bash's spans laid on it: a script's
    /// lines of text alone (its indentation stripped, comments and blank lines with nothing
    /// open, single quotes and `$$`, a here-document and a `$(` over lines), a script closed
    /// on its line and another opened after it, lines with an interpolation, an escape or a
    /// spliced string's close, a `"…"` script, writers' files (JSON's; bash's over lines, named
    /// by its shebang, in a script's interpolation), and a block cut anywhere.
    #[test]
    fn nix_runs_are_its_spans() {
        runs_are_spans(
            Lang::Nix,
            concat!(
                "{ pkgs, ... }: {\n  postPatch = ''\n    # a comment\n\n    echo 'a b' $$HOME\n",
                "    cat > $out/x <<EOF\n    text $(date\n    EOF\n    x=\"$(\n      pwd)\"\n",
                "    for f in *.c; do sed -i 's/a/b/' \"$f\"; done\n  '';\n",
                "  preBuild = ''make -j4 ''; postBuild = ''ls'' + ''\n    rm -rf x\n  '';\n",
                "  installPhase = ''\n    cp ${pkgs.foo}/bin/foo $out\n    echo ''$HOME '''x\n",
                "    ${lib.optionalString true ''\n      echo spliced\n    ''} done\n",
                "    if [ -e a ]; then\n      exit 1\n    fi'';\n",
                "  checkPhase = \"make check\\n  echo $x\";\n",
                "  passthru.f = writeText \"a.json\" ''\n    {\"a\": 1}\n  '';\n",
                "  passthru.g = writeText \"b.sh\" ''\n    # c\n    if [ -e \"$1\n    \" ]; then\n",
                "      echo $HOME 'q'\n    fi'';\n",
                "  passthru.h = writeScript \"h\" ''\n    #!/bin/sh\n    exec foo \"$@\"\n  '';\n",
                "  postFixup = ''\n    cp ${writeText \"c.sh\" ''echo x''} $out\n",
                "    cp ${writeText \"d.sh\" ''\n      echo y\n    ''} $out\n  '';\n",
                "  script = ''\n    echo \"open\n    still\" # c\n  '';\n}\n",
            ),
        );
    }

    /// bash's and Makefile's runs written by their scan (X-075, X-095) are those their spans
    /// give: bash's through here-documents, `$(…)`, `${…}`, arithmetic, tests and strings over
    /// lines; Makefile's through rules, variables, conditionals, `define` and its shell regions
    /// (recipes, `$(shell …)` nested and open over lines, `!=`, `bash -c`, tokens standing in the
    /// shell's text, a recipe's comment over tokens).
    #[test]
    fn bash_and_makefile_runs_are_their_spans() {
        runs_are_spans(
            Lang::Bash,
            concat!(
                "#!/usr/bin/env bash\nset -euo pipefail\nf() { local a=${1:-x} b=\"$(date +%s)\"; }\n",
                "if [[ -n \"$a\" && $b -gt 0 ]]; then echo 'q' >&2; fi\nfor i in {1..3}; do (( n += i * 2 )); done\n",
                "cat <<-EOF > \"$out\"\n\tline $HOME ${arr[@]}\nEOF\ns=\"open\n still\" # c\n",
                "case $x in a|b) echo `pwd`;; *) exit 1;; esac\narr=(a \"b c\") && export PATH=\"$PATH:/x\"\n",
            ),
        );
        runs_are_spans(
            Lang::Makefile,
            concat!(
                "# c\n.PHONY: all clean\nCC ?= gcc\nCFLAGS := -O2 $(EXTRA) ${X}\nSRC = $(wildcard src/*.c)\n",
                "all: $(SRC:.c=.o) | dir\n\t$(CC) $(CFLAGS) -o $@ $^ && echo \"done $$HOME\"\n",
                "ifeq ($(OS),Windows_NT)\n  EXE := .exe\nelse\n  EXE :=\nendif\n",
                "define tmpl\n$(1): ; @echo $(1)\nendef\n%.o: %.c\n\t@if [ -f $< ]; then \\\n\t  cc -c $<; fi\n",
                "export A = 1\nvpath %.h inc\n-include deps.mk\n",
                "V := $(shell git describe $(shell echo --tags) 2>/dev/null)\nW != ls -1 | wc -l\n",
                "U := $(shell printf '%s' \\\n  \"x y\")\nx:\n\t$(Q)echo $(ENV) CC=gcc $$f; bash -c 'a $$1' # $(X) c\n",
                "\t$(MAKE) -C sub $(build)=dir $(call f,$(shell \\\n\t  date)) $$$$ ok\n",
            ),
        );
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
