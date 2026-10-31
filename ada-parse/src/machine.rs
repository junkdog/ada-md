//! The stack machine the context grammars run on (TOML, …): syntect's, so that a line gets the
//! scopes syntect gives it on bat's grammar, but with each regex a matcher of ada's own that
//! scans in linear time, and the stack between lines packed into a [`State`].
//!
//! As syntect's `ParseState`: a stack of contexts, each a list of patterns; at each step the
//! pattern whose match starts leftmost wins, ties going to the first; a match pushes, sets or
//! pops contexts and gives its text scopes, its groups' scopes and the contexts' meta scopes,
//! in syntect's order. A push that matched nothing can't be followed by a pop that matches
//! nothing at the same place (the machine steps a character instead), a pattern that matches
//! nothing and does nothing doesn't count, and a pattern's match is kept for the line while it
//! lies ahead (syntect's search cache). A line is matched with its `\n`, as syntect sees it.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::{Out, State};

/// A scope, as an index into a grammar's [`Grammar::atoms`].
pub type Atom = u8;

/// A matcher: the leftmost match of a pattern's regex at or after a position.
pub type Find = fn(&Line, usize) -> Option<M>;

/// What a match does to the stack.
#[derive(Clone, Copy)]
pub enum Op {
    None,
    Push(&'static [u8]),
    Set(&'static [u8]),
    Pop,
}

/// A pattern: its matcher, its scopes, its groups' scopes, what it does.
pub struct Pat {
    pub find: Find,
    pub scope: &'static [Atom],
    pub caps: &'static [(u8, &'static [Atom])],
    pub op: Op,
    /// It reads [`Line::arg`], so a match found before says nothing (syntect doesn't keep
    /// those of a regex with a back-reference either).
    pub nocache: bool,
    /// A `with_prototype` context: its patterns apply in the contexts the match opens and
    /// every context above them, until those are popped.
    pub with: Option<u8>,
}

impl Pat {
    pub const fn new(find: Find) -> Pat {
        Pat {
            find,
            scope: &[],
            caps: &[],
            op: Op::None,
            nocache: false,
            with: None,
        }
    }

    pub const fn with(mut self, ctx: u8) -> Pat {
        self.with = Some(ctx);
        self
    }

    pub const fn nocache(mut self) -> Pat {
        self.nocache = true;
        self
    }

    pub const fn sc(mut self, scope: &'static [Atom]) -> Pat {
        self.scope = scope;
        self
    }

    pub const fn caps(mut self, caps: &'static [(u8, &'static [Atom])]) -> Pat {
        self.caps = caps;
        self
    }

    pub const fn push(mut self, ctx: &'static [u8]) -> Pat {
        self.op = Op::Push(ctx);
        self
    }

    pub const fn set(mut self, ctx: &'static [u8]) -> Pat {
        self.op = Op::Set(ctx);
        self
    }

    pub const fn pop(mut self) -> Pat {
        self.op = Op::Pop;
        self
    }
}

/// A context's entry: a pattern, or another context's entries.
pub enum It {
    P(Pat),
    I(u8),
}

/// A context: its meta scopes (on its text and the matches that open and close it), its meta
/// content scopes (on its text only), its entries; whether it takes the grammar's prototype
/// (`meta_include_prototype`, before [`protos`] leaves out those the prototype reaches), and
/// whether its patterns refer back to group 1 of the match that opened it.
pub struct Ctx {
    pub meta: &'static [Atom],
    pub content: &'static [Atom],
    pub items: &'static [It],
    pub proto: bool,
    pub backref: bool,
}

impl Ctx {
    pub const fn new(items: &'static [It]) -> Ctx {
        Ctx {
            meta: &[],
            content: &[],
            items,
            proto: true,
            backref: false,
        }
    }

    pub const fn meta(mut self, meta: &'static [Atom]) -> Ctx {
        self.meta = meta;
        self
    }

    pub const fn content(mut self, content: &'static [Atom]) -> Ctx {
        self.content = content;
        self
    }

    pub const fn no_proto(mut self) -> Ctx {
        self.proto = false;
        self
    }

    pub const fn backref(mut self) -> Ctx {
        self.backref = true;
        self
    }
}

/// Which contexts take the prototype `proto`, as syntect links them: those that ask for it,
/// but for the prototype and every context it includes, pushes or sets, and theirs in turn.
pub const fn protos<const N: usize>(ctxs: &[Ctx], proto: Option<u8>) -> [bool; N] {
    assert!(ctxs.len() == N);
    let mut out = [false; N];
    let Some(proto) = proto else {
        return out;
    };
    let mut reached = [false; N];
    let mut todo = [0u8; N];
    let mut n = 1;
    todo[0] = proto;
    reached[proto as usize] = true;
    while n > 0 {
        n -= 1;
        let items = ctxs[todo[n] as usize].items;
        let mut i = 0;
        while i < items.len() {
            let refs: &[u8] = match &items[i] {
                It::I(c) => std::slice::from_ref(c),
                It::P(p) => match p.op {
                    Op::Push(r) | Op::Set(r) => r,
                    _ => &[],
                },
            };
            let mut j = 0;
            while j < refs.len() {
                let c = refs[j] as usize;
                if !reached[c] {
                    reached[c] = true;
                    todo[n] = refs[j];
                    n += 1;
                }
                j += 1;
            }
            i += 1;
        }
    }
    let mut i = 0;
    while i < N {
        out[i] = ctxs[i].proto && !reached[i];
        i += 1;
    }
    out
}

/// A grammar: its contexts by id, the stack a block starts with, and the scope names.
pub struct Grammar {
    pub ctxs: &'static [Ctx],
    pub start: &'static [u8],
    pub atoms: &'static [&'static str],
    /// Each context's first slot in a line's cache of matches, as [`slots`] gives them.
    pub slots: &'static [u16],
    /// The prototype context, if the grammar has one.
    pub proto: Option<u8>,
    /// Which contexts take it, as [`protos`] gives them.
    pub protos: &'static [bool],
    /// The stacks of scopes met so far: the kinds.
    pub kinds: &'static Kinds,
}

impl Grammar {
    /// The scopes of kind `kind`, outermost first; none for a kind not handed out yet.
    pub fn scopes(&self, kind: u16) -> Vec<&'static str> {
        let mut out = Vec::new();
        let mut node = u32::from(kind);
        while node != ROOT {
            let Some((parent, atom)) = self.kinds.node(node) else {
                return Vec::new();
            };
            out.push(self.atoms[atom as usize]);
            node = parent;
        }
        out.reverse();
        out
    }
}

/// A grammar's kinds: each stack of scopes its text has had, as a node of a trie whose root is
/// the base scope alone, a node's children each one more scope. Lexing interns a stack the
/// first time it meets it, so that a kind costs nothing until some text has it, and its id
/// holds for the rest of the process. Shared by every thread, without a lock: a node is
/// written before the slot that leads to it is published.
pub struct Kinds {
    /// Each node's parent and scope, `parent << 8 | atom`, plus [`TAKEN`]; 0 while unwritten.
    nodes: [AtomicU32; NODES],
    /// Nodes handed out past the root. All of a `Kinds` starts zero, so that its statics take
    /// no room in the binary (`.bss`, not `.data`).
    len: AtomicU32,
    /// From a parent and a scope to the child: `key << 32 | node`, by [`key`]; 0 when free.
    slots: [AtomicU64; SLOTS],
}

/// The most kinds a grammar has; past it, a deeper stack takes its parent's kind.
const NODES: usize = 4096;
const SLOTS: usize = 2 * NODES;
const ROOT: u32 = 0;
const TAKEN: u32 = 1 << 31;

/// A parent and a scope as a key of [`Kinds::slots`], never 0.
fn key(parent: u32, atom: Atom) -> u32 {
    (parent << 8 | u32::from(atom)) + 1
}

impl Kinds {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Kinds {
        Kinds {
            nodes: [const { AtomicU32::new(0) }; NODES],
            len: AtomicU32::new(0),
            slots: [const { AtomicU64::new(0) }; SLOTS],
        }
    }

    fn node(&self, id: u32) -> Option<(u32, Atom)> {
        let n = self.nodes.get(id as usize)?.load(Ordering::Acquire);
        (n & TAKEN != 0).then_some(((n & !TAKEN) >> 8, n as u8))
    }

    /// The kind of `parent`'s stack with `atom` on it, interned if new; `None` when the
    /// trie is full.
    fn child(&self, parent: u32, atom: Atom) -> Option<u32> {
        let k = key(parent, atom);
        let mut h = (k.wrapping_mul(0x9e37_79b1) >> 19) as usize % SLOTS;
        loop {
            let v = self.slots[h].load(Ordering::Acquire);
            if v == 0 {
                let id = self.len.fetch_add(1, Ordering::Relaxed) + 1;
                if id as usize >= NODES {
                    self.len.store(NODES as u32, Ordering::Relaxed);
                    return None;
                }
                self.nodes[id as usize]
                    .store(TAKEN | parent << 8 | u32::from(atom), Ordering::Release);
                let want = u64::from(k) << 32 | u64::from(id);
                match self.slots[h].compare_exchange(0, want, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => return Some(id),
                    // another thread took the slot: its node if it is this one, else probe on
                    // (this id stays unused)
                    Err(v) if (v >> 32) as u32 == k => return Some(v as u32),
                    Err(_) => {}
                }
            } else if (v >> 32) as u32 == k {
                return Some(v as u32);
            }
            h = (h + 1) % SLOTS;
        }
    }
}

/// A match: where it starts and ends, and where those of its groups that took part do, by
/// their number in the regex (up to [`GROUPS`] of them).
#[derive(Clone, Copy, Debug)]
pub struct M {
    pub s: u32,
    pub e: u32,
    n: u8,
    nums: [u8; GROUPS],
    spans: [(u32, u32); GROUPS],
}

/// The most groups a match keeps, after the whole match.
pub const GROUPS: usize = 8;

impl M {
    pub fn new(s: usize, e: usize) -> M {
        M {
            s: s as u32,
            e: e as u32,
            n: 0,
            nums: [0; GROUPS],
            spans: [(0, 0); GROUPS],
        }
    }

    /// Group `n` (from 1) spans `s..e`.
    pub fn g(mut self, n: usize, s: usize, e: usize) -> M {
        let i = self.nums[..usize::from(self.n)]
            .iter()
            .position(|&x| usize::from(x) == n)
            .unwrap_or_else(|| {
                self.n += 1;
                usize::from(self.n) - 1
            });
        self.nums[i] = n as u8;
        self.spans[i] = (s as u32, e as u32);
        self
    }

    /// Where group `n` (from 1; 0 for the whole match) spans, if it took part.
    pub fn group(&self, n: u8) -> Option<(usize, usize)> {
        if n == 0 {
            return Some((self.s as usize, self.e as usize));
        }
        let i = self.nums[..usize::from(self.n)]
            .iter()
            .position(|&x| x == n)?;
        let (s, e) = self.spans[i];
        Some((s as usize, e as usize))
    }
}

/// A line as the machine matches it: its bytes, then a `\n` at [`Line::nl`]; positions run to
/// [`Line::end`], past the `\n`.
pub struct Line<'a> {
    pub b: &'a [u8],
    /// What the level being matched holds for a back-reference: the length of the group 1
    /// that opened it (all a grammar here refers back to is a run of spaces).
    pub arg: Cell<u16>,
}

impl Line<'_> {
    /// Where the `\n` is: the line's length.
    pub fn nl(&self) -> usize {
        self.b.len()
    }

    /// The end of the text matched, after the `\n`.
    pub fn end(&self) -> usize {
        self.b.len() + 1
    }

    /// The byte at `i`: the `\n` at [`Line::nl`], none at or after [`Line::end`].
    pub fn at(&self, i: usize) -> Option<u8> {
        match self.b.get(i) {
            Some(&c) => Some(c),
            None if i == self.b.len() => Some(b'\n'),
            None => None,
        }
    }

    /// Whether the byte at `i` is in `set`.
    pub fn is(&self, i: usize, set: impl Fn(u8) -> bool) -> bool {
        self.at(i).is_some_and(set)
    }

    /// Where a run of bytes in `set` from `i` ends.
    pub fn run(&self, mut i: usize, set: impl Fn(u8) -> bool) -> usize {
        while self.is(i, &set) {
            i += 1;
        }
        i
    }

    /// The length of the character at `i`, for `.` (a byte of a broken one counts alone);
    /// none at the `\n` or past it.
    pub fn char_len(&self, i: usize) -> Option<usize> {
        let &c = self.b.get(i)?;
        let n = match c {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        let whole = i + n <= self.b.len() && self.b[i + 1..i + n].iter().all(|&c| c & 0xc0 == 0x80);
        Some(if whole { n } else { 1 })
    }

    /// `$` in Oniguruma's multi-line sense: before the `\n`, or at the end.
    pub fn eol(&self, i: usize) -> bool {
        i == self.nl() || i == self.end()
    }

    /// The leftmost position at or after `from` where `at` matches, with what it gives.
    pub fn find(&self, from: usize, at: impl Fn(usize) -> Option<M>) -> Option<M> {
        (from..=self.end()).find_map(at)
    }

    /// The leftmost `lit` at or after `from`.
    pub fn lit(&self, from: usize, lit: &[u8]) -> Option<M> {
        let hay = self.b.get(from..)?;
        let i = hay.windows(lit.len()).position(|w| w == lit)?;
        Some(M::new(from + i, from + i + lit.len()))
    }
}

/// A grammar's scopes, each a constant in a module `s` naming its index in `ATOMS`.
macro_rules! atoms {
    ($($name:ident = $scope:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[repr(u8)]
        enum Atoms {
            $($name,)*
        }

        /// The scopes, as indices into [`ATOMS`].
        mod s {
            $(pub const $name: crate::machine::Atom = super::Atoms::$name as u8;)*
        }

        const ATOMS: &[&str] = &[$($scope,)*];
    };
}

pub(crate) use atoms;

/// The first `c` of the line at or after `from`.
pub fn first(l: &Line, from: usize, c: u8) -> Option<usize> {
    let hay = l.b.get(from..)?;
    hay.iter().position(|&x| x == c).map(|i| from + i)
}

/// The leftmost `c`, as a match.
pub fn lit1(l: &Line, from: usize, c: u8) -> Option<M> {
    first(l, from, c).map(|k| M::new(k, k + 1))
}

/// Where the `[ \t]` before `at` start, not before `from`.
pub fn back_blanks(l: &Line, from: usize, mut at: usize) -> usize {
    while at > from && is_blank(l.b[at - 1]) {
        at -= 1;
    }
    at
}

/// The empty regex.
pub fn empty(l: &Line, from: usize) -> Option<M> {
    (from <= l.end()).then(|| M::new(from, from))
}

/// `\S`: a character, not a space.
pub fn non_space(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| !is_space(c))?;
    Some(M::new(i, i + l.char_len(i).unwrap_or(1)))
}

/// `\w` as far as ASCII goes; any non-ASCII byte counts as one, as Oniguruma's Unicode `\w`
/// takes letters.
pub fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// `[ \t]`.
pub fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// `\s`.
pub fn is_space(b: u8) -> bool {
    crate::is_space(b)
}

/// How many levels a line's stack holds, more than syntect lets a push reach; past it the
/// bottom ones are dropped, still counted toward the depth.
pub(crate) const DEPTH: usize = 128;

/// How deep syntect lets a push take the stack.
pub(crate) const MAX_PUSH: usize = 100;

/// The levels a [`State`] keeps between lines.
pub const KEPT: usize = 14;

/// The machine on one line.
struct Run<'a, 'b, 'c> {
    g: &'static Grammar,
    l: Line<'a>,
    stack: [u8; DEPTH],
    /// Each level's [`Line::arg`].
    args: [u16; DEPTH],
    /// Each level's `with_prototype` context, plus 1; 0 for none.
    withs: [u8; DEPTH],
    /// syntect's `proto_starts`: the heights at which a `with_prototype`'s match opened
    /// contexts, below which no `with_prototype` applies.
    proto_starts: [usize; 8],
    nstarts: usize,
    depth: usize,
    /// The levels dropped from the bottom, past [`DEPTH`].
    dropped: usize,
    exact: bool,
    scopes: [Atom; 64],
    /// The kind of each prefix of `scopes`: `nodes[i]` that of `scopes[..i]`.
    nodes: [u32; 65],
    nscopes: usize,
    /// Scopes pushed past what `scopes` holds.
    over: usize,
    /// Where the spans so far end.
    at: usize,
    out: &'b mut Out<'c>,
    /// Each pattern's cached match, by its slot ([`Grammar::slots`]): `UNKNOWN`, `NO_MATCH`, or where it starts.
    cache: [u32; CACHE],
}

const CACHE: usize = 256;
const UNKNOWN: u32 = u32::MAX;
const NO_MATCH: u32 = u32::MAX - 1;

/// The best match at a step.
struct Best {
    pat: &'static Pat,
    m: M,
    would_loop: bool,
    /// It came from a level's `with_prototype`.
    from_with: bool,
}

/// Lex `b`, a line without its `\n`, on `g`, from `state`.
#[inline(never)]
pub fn line(g: &'static Grammar, b: &[u8], state: &mut State, out: &mut Out) {
    let mut run = Run {
        g,
        l: Line {
            b,
            arg: Cell::new(0),
        },
        stack: [0; DEPTH],
        args: [0; DEPTH],
        withs: [0; DEPTH],
        proto_starts: [0; 8],
        nstarts: 0,
        depth: 0,
        dropped: 0,
        exact: state.exact(),
        scopes: [0; 64],
        nodes: [ROOT; 65],
        nscopes: 0,
        over: 0,
        at: 0,
        out,
        cache: [UNKNOWN; CACHE],
    };
    match state.levels() {
        [] => {
            for &c in g.start {
                run.stack[run.depth] = c;
                run.depth += 1;
            }
        }
        levels => {
            run.stack[..levels.len()].copy_from_slice(levels);
            run.depth = levels.len();
            if let Some(i) = run.top_backref() {
                run.args[i] = state.arg;
            }
            run.withs[..levels.len()].copy_from_slice(state.withs());
        }
    }
    for i in 0..run.depth {
        let ctx = &g.ctxs[run.stack[i] as usize];
        for &a in ctx.meta.iter().chain(ctx.content) {
            run.push_scope(a);
        }
    }
    run.parse();
    let len = run.l.nl();
    run.flush(len);
    let arg = run.top_backref().map_or(0, |i| run.args[i]);
    // a `with_prototype`'s height isn't kept between lines: a line that ends above one is
    // not exact
    run.exact &= run.nstarts == 0;
    *state = State::pack(
        &run.stack[..run.depth],
        &run.withs[..run.depth],
        arg,
        run.exact,
    );
}

/// Where each context's patterns start in the cache, for [`Grammar::slots`]: one slot an
/// entry, so that a pattern has the same one whichever context includes it.
pub const fn slots<const N: usize>(ctxs: &[Ctx]) -> [u16; N] {
    assert!(ctxs.len() == N);
    let mut slots = [0; N];
    let mut i = 1;
    while i < N {
        slots[i] = slots[i - 1] + ctxs[i - 1].items.len() as u16;
        i += 1;
    }
    assert!(
        slots[N - 1] as usize + ctxs[N - 1].items.len() <= CACHE,
        "too many patterns"
    );
    slots
}

impl Run<'_, '_, '_> {
    /// The topmost level whose context refers back to what opened it.
    fn top_backref(&self) -> Option<usize> {
        (0..self.depth)
            .rev()
            .find(|&i| self.g.ctxs[self.stack[i] as usize].backref)
    }

    fn parse(&mut self) {
        let mut start = 0;
        let mut push_at = (usize::MAX, 0usize);
        loop {
            if self.depth == 0 {
                return;
            }
            while self.nstarts > 0 && self.proto_starts[self.nstarts - 1] >= self.depth {
                self.nstarts -= 1;
            }
            let check_loop = push_at == (start, self.depth + self.dropped);
            let Some(best) = self.best(start, check_loop) else {
                return;
            };
            if best.would_loop {
                // syntect steps a character; at the `\n` there is none after it
                match self.l.char_len(start) {
                    Some(n) => {
                        start += n;
                        continue;
                    }
                    None => return,
                }
            }
            let end = best.m.e as usize;
            if end == start && matches!(best.pat.op, Op::Push(_)) {
                push_at = (end, self.depth + self.dropped + 1);
            }
            start = end;
            if best.from_with {
                if self.nstarts == self.proto_starts.len() {
                    self.proto_starts.copy_within(1.., 0);
                    self.nstarts -= 1;
                    self.exact = false;
                }
                self.proto_starts[self.nstarts] = self.depth;
                self.nstarts += 1;
            }
            self.exec(&best);
        }
    }

    /// syntect's `find_best_match`: the prototype's patterns, if the top context takes it,
    /// then the top context's.
    fn best(&mut self, start: usize, check_loop: bool) -> Option<Best> {
        let top = self.stack[self.depth - 1];
        let mut found = Found {
            min: usize::MAX,
            best: None,
            would_loop: false,
            from_with: false,
            stop: false,
        };
        // the `with_prototype`s of the levels from the last one a match of theirs opened, from
        // the bottom up
        let from = self.proto_starts[..self.nstarts]
            .last()
            .copied()
            .unwrap_or(0);
        for i in from..self.depth {
            if let Some(with) = self.withs[i].checked_sub(1) {
                found.from_with = true;
                self.l.arg.set(self.args[i]);
                self.walk(with, start, check_loop, &mut found);
                found.from_with = false;
                if found.stop {
                    return found.best;
                }
            }
        }
        if let Some(proto) = self.g.proto
            && self.g.protos[top as usize]
        {
            self.l.arg.set(0);
            self.walk(proto, start, check_loop, &mut found);
            if found.stop {
                return found.best;
            }
        }
        self.l.arg.set(self.args[self.depth - 1]);
        self.walk(top, start, check_loop, &mut found);
        found.best
    }

    fn walk(&mut self, ctx: u8, start: usize, check_loop: bool, found: &mut Found) {
        let g = self.g;
        for (i, item) in g.ctxs[ctx as usize].items.iter().enumerate() {
            let pat = match item {
                It::I(inner) => {
                    self.walk(*inner, start, check_loop, found);
                    if found.stop {
                        return;
                    }
                    continue;
                }
                It::P(pat) => pat,
            };
            let slot = g.slots[ctx as usize] as usize + i;
            let Some(m) = self.search(pat, slot, start) else {
                continue;
            };
            let s = m.s as usize;
            if s < found.min || (s == found.min && found.would_loop) {
                found.min = s;
                found.would_loop = check_loop && m.e as usize <= start && matches!(pat.op, Op::Pop);
                if matches!(pat.op, Op::Push(_)) && self.depth + self.dropped >= MAX_PUSH {
                    found.best = None;
                    found.stop = true;
                    return;
                }
                found.best = Some(Best {
                    pat,
                    m,
                    would_loop: found.would_loop,
                    from_with: found.from_with,
                });
                if s == start && !found.would_loop {
                    found.stop = true;
                    return;
                }
            }
        }
    }

    /// syntect's `search`: a pattern's leftmost match, kept for the line while it lies ahead.
    fn search(&mut self, pat: &'static Pat, slot: usize, start: usize) -> Option<M> {
        if pat.nocache {
            return (pat.find)(&self.l, start).filter(|m| match pat.op {
                Op::None => m.s != m.e,
                Op::Push(_) => self.depth + self.dropped < MAX_PUSH,
                _ => true,
            });
        }
        let cached = self.cache[slot];
        if cached == NO_MATCH {
            return None;
        }
        if cached != UNKNOWN && cached as usize >= start {
            return (pat.find)(&self.l, cached as usize);
        }
        match (pat.find)(&self.l, start) {
            Some(m) => {
                let does_something = match pat.op {
                    Op::None => m.s != m.e,
                    Op::Push(_) => self.depth + self.dropped < MAX_PUSH,
                    _ => true,
                };
                if !does_something {
                    return None;
                }
                self.cache[slot] = m.s;
                Some(m)
            }
            None => {
                self.cache[slot] = NO_MATCH;
                None
            }
        }
    }

    fn ctx(&self, id: u8) -> &'static Ctx {
        &self.g.ctxs[id as usize]
    }

    /// syntect's `exec_pattern`: the match's scopes, then what it does to the stack.
    fn exec(&mut self, best: &Best) {
        let (s, e) = (best.m.s as usize, best.m.e as usize);
        let pat = best.pat;
        let cur = self.ctx(self.stack[self.depth - 1]);
        // the meta scopes before the match
        match pat.op {
            Op::Pop => self.pop_scopes(s, cur.content.len()),
            Op::Push(refs) | Op::Set(refs) => {
                for &r in refs {
                    for &a in self.ctx(r).meta {
                        self.at(s);
                        self.push_scope(a);
                    }
                }
            }
            Op::None => {}
        }
        for &a in pat.scope {
            self.at(s);
            self.push_scope(a);
        }
        if !pat.caps.is_empty() {
            // as syntect orders them: by start, the longer first, a group's end before
            // anything else there
            let mut ops = [(0usize, 0i64, 0u8, false); 2 * GROUPS + 2];
            let mut n = 0;
            for (ci, &(group, scopes)) in pat.caps.iter().enumerate() {
                let Some((gs, ge)) = best.m.group(group) else {
                    continue;
                };
                if n == ops.len() {
                    break;
                }
                if gs == ge {
                    continue;
                }
                ops[n] = (gs, -((ge - gs) as i64), ci as u8, true);
                ops[n + 1] = (ge, i64::MIN, scopes.len() as u8, false);
                n += 2;
            }
            ops[..n].sort_by_key(|o| (o.0, o.1));
            for &(at, _, x, push) in &ops[..n] {
                if push {
                    for &a in pat.caps[x as usize].1 {
                        self.at(at);
                        self.push_scope(a);
                    }
                } else {
                    self.pop_scopes(at, x as usize);
                }
            }
        }
        self.pop_scopes(e, pat.scope.len());
        // the meta scopes after the match
        match pat.op {
            Op::Pop => self.pop_scopes(e, cur.meta.len()),
            Op::Push(refs) | Op::Set(refs) => {
                let is_set = matches!(pat.op, Op::Set(_));
                let repush = (is_set && !(cur.meta.is_empty() && cur.content.is_empty()))
                    || refs.iter().any(|&r| !self.ctx(r).content.is_empty());
                if repush {
                    let mut n: usize = refs.iter().map(|&r| self.ctx(r).meta.len()).sum();
                    if is_set {
                        n += cur.content.len() + cur.meta.len();
                    }
                    self.pop_scopes(e, n);
                    for &r in refs {
                        let ctx = self.ctx(r);
                        for &a in ctx.meta.iter().chain(ctx.content) {
                            self.at(e);
                            self.push_scope(a);
                        }
                    }
                }
            }
            Op::None => {}
        }
        match pat.op {
            // down to the levels dropped, if any: the line stops there
            Op::Pop => self.depth -= 1,
            Op::Set(refs) => {
                self.depth -= 1;
                // the first context set keeps the `with_prototype` of the one it replaces
                let kept = self.withs[self.depth];
                self.push_levels(refs, &best.m, pat.with, kept);
            }
            Op::Push(refs) => self.push_levels(refs, &best.m, pat.with, 0),
            Op::None => {}
        }
    }

    fn push_levels(&mut self, refs: &[u8], m: &M, with: Option<u8>, kept: u8) {
        for (i, &r) in refs.iter().enumerate() {
            // the last context opened takes the match's `with_prototype`; one level holds one
            let mut w = if i == 0 { kept } else { 0 };
            if i + 1 == refs.len()
                && let Some(with) = with
            {
                self.exact &= w == 0;
                w = with + 1;
            }
            if self.depth == DEPTH {
                // past what a line holds: drop the bottom level
                self.stack.copy_within(1.., 0);
                self.args.copy_within(1.., 0);
                self.withs.copy_within(1.., 0);
                self.depth -= 1;
                self.dropped += 1;
                self.exact = false;
            }
            self.stack[self.depth] = r;
            self.withs[self.depth] = w;
            self.args[self.depth] = match self.g.ctxs[r as usize].backref {
                true => m
                    .group(1)
                    .map_or(0, |(s, e)| (e - s).min(usize::from(u16::MAX)) as u16),
                false => 0,
            };
            self.depth += 1;
        }
    }

    /// The scopes so far cover the text up to `pos`.
    fn at(&mut self, pos: usize) {
        self.flush(pos);
    }

    fn push_scope(&mut self, a: Atom) {
        if self.nscopes == self.scopes.len() {
            self.over += 1;
            self.exact = false;
            return;
        }
        let parent = self.nodes[self.nscopes];
        let node = self.g.kinds.child(parent, a).unwrap_or_else(|| {
            // the trie is full: the stack takes its parent's kind
            self.exact = false;
            parent
        });
        self.scopes[self.nscopes] = a;
        self.nscopes += 1;
        self.nodes[self.nscopes] = node;
    }

    fn pop_scopes(&mut self, pos: usize, n: usize) {
        if n > 0 {
            self.flush(pos);
            let over = n.min(self.over);
            self.over -= over;
            self.nscopes = self.nscopes.saturating_sub(n - over);
        }
    }

    /// Give the text from where the spans end to `pos` (never past the line) the current
    /// scopes' kind.
    fn flush(&mut self, pos: usize) {
        let pos = pos.min(self.l.nl());
        if pos <= self.at {
            return;
        }
        let kind = self.nodes[self.nscopes];
        if kind != ROOT {
            self.out.push_merged(self.at, pos, kind as u16);
        }
        self.at = pos;
    }
}

struct Found {
    min: usize,
    best: Option<Best>,
    would_loop: bool,
    /// The walk under way is a `with_prototype`'s.
    from_with: bool,
    stop: bool,
}
