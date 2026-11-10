//! Dockerfiles by a scanner: bat's `Dockerfile` compiled by hand. Every pattern but an
//! instruction's arguments' is anchored at a line's start, so a line is decided by its first
//! bytes: a comment, an instruction (its arguments then run to a line's end that isn't
//! continued by `\`, blanks after it or not, and an instruction alone on its line ends with it,
//! as Docker reads them: X-116), `FROM`, or text bat marks `invalid`. A string left open at a line's end
//! turns the next line `invalid`, as bat's does. bat's `Dockerfile (with bash)`, which colors
//! `RUN` as shell, is a syntax of its own that no file name selects, and stays on syntect.
//!
//! **Here-documents** (BuildKit's, Dockerfile 1.4; X-105, ada's own: bat has none and reads
//! their bodies as instructions or `invalid`): `<<EOF`, `<<-EOF` (tabs before the closer) and a
//! quoted word, at a word's start in the arguments of `RUN`, `COPY` and `ADD`, colored as bash's
//! opener is; the bodies follow on the lines after, in the openers' order. A `RUN` whose command
//! is the body (only flags and the opener, or a shell such as `bash` with the body its stdin) has
//! it on bash's scanner, bash's state carried between its lines; any other body (`COPY`'s and
//! `ADD`'s files, `RUN cat <<EOF > f`'s stdin) is a here-document's text, as bash's body is.
//!
//! Its readings are held by its unit tests and `tests/readings.rs`' snapshots (X-116).

use crate::bash::hd;
use crate::emit::{Push, Runs, Sink, To};
use crate::heredoc::{Label, Lead, Queue, Tail};
use crate::matcher::{Atom, atoms, is_space};
use crate::{Out, Span, State};

atoms! {
    STR_BEGIN = "punctuation.definition.string.begin.dockerfile",
    COMMENT_LEADING = "punctuation.whitespace.comment.leading.dockerfile",
    COMMENT_S = "comment.dockerfile",
    COMMENT_MARK = "punctuation.definition.comment.dockerfile",
    KEYWORD = "keyword.control.dockerfile",
    SPECIAL = "keyword.other.special-method.dockerfile",
    STAGE = "variable.stage-name",
    OPERATOR = "keyword.operator.dockerfile",
    DOUBLE = "string.quoted.double.dockerfile",
    STR_END = "punctuation.definition.string.end.dockerfile",
    ESCAPED = "constant.character.escaped.dockerfile",
    TAG_DIGEST = "entity.name.enum.tag-digest",
    INVALID_S = "invalid",
    SINGLE = "string.quoted.single.dockerfile",
}

/// The format's kinds are this plus their index in [`KINDS`]; bash's (a `RUN`'s script body,
/// a here-document's opener and text) are below it, and bash's twins above.
pub const DOCKER: u16 = 0x100;

const COMMENT_LEADING: u16 = DOCKER;
const COMMENT: u16 = DOCKER + 1;
const COMMENT_MARK: u16 = DOCKER + 2;
const KEYWORD: u16 = DOCKER + 3;
const ONBUILD: u16 = DOCKER + 4;
const OPERATOR: u16 = DOCKER + 5;
const STAGE: u16 = DOCKER + 7;
const TAG: u16 = DOCKER + 8;
const INVALID: u16 = DOCKER + 9;
const ESCAPED: u16 = DOCKER + 10;
/// A double-quoted string's kinds; a single-quoted one's are the same, [`IN_SINGLE`] on.
const DOUBLE: u16 = DOCKER + 11;
const DOUBLE_BEGIN: u16 = DOCKER + 12;
const DOUBLE_END: u16 = DOCKER + 13;
const DOUBLE_ESCAPED: u16 = DOCKER + 14;
const IN_SINGLE: u16 = 4;

/// Each kind's scopes under `source.dockerfile`, by kind.
static KINDS: [&[Atom]; 19] = [
    &[s::COMMENT_LEADING],
    &[s::COMMENT_S],
    &[s::COMMENT_S, s::COMMENT_MARK],
    &[s::KEYWORD],
    &[s::KEYWORD, s::SPECIAL],
    &[s::OPERATOR],
    &[s::OPERATOR, s::SPECIAL],
    &[s::STAGE],
    &[s::TAG_DIGEST],
    &[s::INVALID_S],
    &[s::ESCAPED],
    &[s::DOUBLE],
    &[s::DOUBLE, s::STR_BEGIN],
    &[s::DOUBLE, s::STR_END],
    &[s::DOUBLE, s::ESCAPED],
    &[s::SINGLE],
    &[s::SINGLE, s::STR_BEGIN],
    &[s::SINGLE, s::STR_END],
    &[s::SINGLE, s::ESCAPED],
];

/// The scopes under `source.dockerfile` a kind carries, outermost first: the format's own, or
/// bash's.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    match kind.checked_sub(DOCKER).map(usize::from) {
        Some(i) if i < KINDS.len() => KINDS[i].iter().map(|&a| ATOMS[usize::from(a)]).collect(),
        _ => crate::bash::scopes(kind),
    }
}

/// The context open at the scan's place: the top of bat's stack.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// `main`, before the first `FROM`: comments, `ARG` and `FROM` only.
    Main,
    /// `body`, which each `FROM` pushes and nothing pops.
    Body,
    /// `args`: an instruction's arguments, to a line's end that isn't continued.
    Args,
    /// A string in the arguments, `"` or `'`.
    Double,
    Single,
    /// `invalid`, where a string left open at a line's end set it: the next line that starts
    /// with something other than an ASCII capital is `invalid`.
    Invalid,
}

// The state's word: the open context in bits 0–3, an instruction's here-document class in 4–5
// (while its arguments are open).
const INST_SHIFT: u32 = 4;

// An instruction's here-document class.
/// Not `RUN`, `COPY` or `ADD`: no here-documents.
const I_NONE: u64 = 0;
/// `RUN`, no command yet: a here-document's body is the script.
const I_RUN: u64 = 1;
/// `RUN` with a shell as its command (`bash`, `sh`): the body is its script, on stdin.
const I_SHELL: u64 = 2;
/// `COPY`, `ADD`, or `RUN` with another command: a body is text.
const I_TEXT: u64 = 3;

// Here-documents' bodies, in the state's `arg` (0: none open, the word as above).
/// Bodies of text waiting, their labels a [`Queue`] in the levels from 0, its length at
/// [`HD_LEN_SHIFT`] (each label's flag: [`LABEL_TABS`]).
const HD_TEXT: u16 = 1;
/// A `RUN`'s script: bash's state in the levels below [`SCRIPT_HELD`] and the state's depth, the
/// label's bits in the four levels from there.
const HD_SCRIPT: u16 = 2;
/// The script's `<<-`: blanks before its closer.
const HD_TABS: u16 = 4;
const HD_LEN_SHIFT: u16 = 3;
/// A label's flag in the queue: `<<-`.
const LABEL_TABS: u8 = 1;
/// How many labels wait at most: a fifth on a line is dropped, its body read as instructions.
type Bodies = Queue<4>;
/// The levels bash's state keeps under a `RUN`'s script (of [`crate::stack::HELD`]); a deeper
/// stack drops its bottom levels, as bash's own does past its 28.
const SCRIPT_HELD: usize = crate::stack::HELD - 4;

/// Lex `b`, a line without its `\n`, from `state`: its word holds the open context and an open
/// instruction's here-document class; a here-document's body, its `arg`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, &mut To::new(out, &mut Push));
}

/// [`line`]'s runs by theme rule written to `k` (X-074, X-068's sink).
pub(crate) fn line_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    line_on(b, state, &mut To::new(out, k));
}

/// [`line`] on a sink.
#[inline(always)]
fn line_on<K: Sink>(b: &[u8], state: &mut State, out: &mut To<'_, '_, K>) {
    if state.flags() != 0 {
        return body(b, state, out);
    }
    let word = state.word();
    let mut ctx = match word & 0xf {
        1 => Ctx::Body,
        2 => Ctx::Args,
        3 => Ctx::Double,
        4 => Ctx::Single,
        5 => Ctx::Invalid,
        _ => Ctx::Main,
    };
    let mut inst = word >> INST_SHIFT & 3;
    let blank = || b.iter().all(|&c| is_space(c));
    // the patterns anchored at the line's start; past them only the arguments' and strings'
    // patterns can match
    let p = match ctx {
        Ctx::Main | Ctx::Body => {
            if comment(b, out) {
                return save(state, ctx, inst);
            }
            if ctx == Ctx::Body
                && let Some((e, i)) = directive(b, out)
            {
                ctx = Ctx::Args;
                inst = i;
                // an instruction alone on its line ends with it, as Docker reads it (X-116: bat's
                // keyword `\s` took the `\n`, and nothing popped the arguments)
                if e > b.len() {
                    return save(state, Ctx::Body, inst);
                }
                e
            } else {
                if ctx == Ctx::Main && arg(b) {
                    out.push(0, 4.min(b.len()), KEYWORD);
                } else if ctx == Ctx::Body && invalid(b) {
                    out.push(0, b.len(), INVALID);
                } else if from(b, out) {
                    ctx = Ctx::Body;
                }
                return save(state, ctx, inst);
            }
        }
        // `^\s*$` takes a blank line and its `\n`: the arguments go on
        Ctx::Args if blank() || comment(b, out) => return save(state, ctx, inst),
        Ctx::Double | Ctx::Single if blank() => {
            let kind = if ctx == Ctx::Double {
                DOUBLE
            } else {
                DOUBLE + IN_SINGLE
            };
            out.push(0, b.len(), kind);
            return save(state, ctx, inst);
        }
        Ctx::Args | Ctx::Double | Ctx::Single => 0,
        Ctx::Invalid => {
            if invalid(b) {
                out.push(0, b.len(), INVALID);
                ctx = Ctx::Body;
            }
            return save(state, ctx, inst);
        }
    };
    // the line's openers queue their labels in the state, [`HD_TEXT`]'s way
    let ctx = args(b, p, ctx, inst, state, out);
    // a `RUN`'s command read only where it matters: a body opened, or the arguments going on
    // without a command yet
    if inst == I_RUN && (state.flags() != 0 || ctx != Ctx::Body) {
        inst = run_command(b, p);
    }
    if state.flags() != 0 {
        return open_bodies(state, inst);
    }
    save(state, ctx, inst);
}

/// A `RUN`'s first word from `p` that is neither a flag nor a here-document's opener: a shell
/// ([`I_SHELL`]), another command ([`I_TEXT`]), or none on the line ([`I_RUN`]).
#[cold]
#[inline(never)]
fn run_command(b: &[u8], mut p: usize) -> u64 {
    let n = b.len();
    loop {
        while p < n && is_space(b[p]) {
            p += 1;
        }
        if p >= n {
            return I_RUN;
        }
        let e = crate::prim::find_in(b, p, n, *b" \t");
        let w = &b[p..e];
        if w.starts_with(b"--") || w.starts_with(b"<<") {
            p = e;
            continue;
        }
        if w == b"\\" {
            return I_RUN;
        }
        let name = w.rsplit(|&c| c == b'/').next().unwrap_or(w);
        return if matches!(name, b"sh" | b"bash" | b"ash" | b"dash" | b"zsh" | b"ksh") {
            I_SHELL
        } else {
            I_TEXT
        };
    }
}

/// `<<` at `k` in the arguments: a here-document's opener where it starts a word (BuildKit reads
/// one from a shell word: `<<EOF`, `<<-EOF`, `<<'EOF'`, `<<"EOF"`), its label queued in `state`
/// (the line's word read already); where it ends, or `None` for a `<` that opens none.
#[cold]
#[inline(never)]
fn opener<K: Sink>(
    b: &[u8],
    k: usize,
    state: &mut State,
    out: &mut To<'_, '_, K>,
) -> Option<usize> {
    let n = b.len();
    if b.get(k + 1) != Some(&b'<') || k > 0 && !is_space(b[k - 1]) {
        return None;
    }
    let mut q = k + 2;
    let tabs = b.get(q) == Some(&b'-');
    if tabs {
        q += 1;
    }
    let quote = match b.get(q) {
        Some(&c @ (b'\'' | b'"')) => Some(c),
        _ => None,
    };
    let s = q + usize::from(quote.is_some());
    if s >= n || !(b[s].is_ascii_alphabetic() || b[s] == b'_') {
        return None;
    }
    let mut e = s + 1;
    while e < n && crate::prim::WORD[usize::from(b[e])] {
        e += 1;
    }
    let end = match quote {
        Some(c) if b.get(e) == Some(&c) => e + 1,
        Some(_) => return None,
        None => e,
    };
    out.push(k, q, hd::REDIRECT);
    if quote.is_some() {
        out.push(q, s, hd::STR_BEGIN);
    }
    out.push(s, e, hd::HEREDOC_TOKEN);
    if end > e {
        out.push(e, end, hd::STR_END);
    }
    let mut bodies = if state.flags() == 0 {
        Bodies::new()
    } else {
        text_bodies(state)
    };
    if bodies.push(Label::of(&b[s..e]), if tabs { LABEL_TABS } else { 0 }) {
        save_text(state, &bodies);
    }
    Some(end)
}

/// The bodies a line's openers queued in `state`, waiting from the next line: a `RUN`'s one
/// script on bash's state, else text.
#[cold]
#[inline(never)]
fn open_bodies(state: &mut State, inst: u64) {
    let bodies = text_bodies(state);
    *state = State::default();
    match bodies.front() {
        Some((label, flags)) if bodies.len() == 1 && (inst == I_RUN || inst == I_SHELL) => {
            for (i, c) in label.bits().to_le_bytes().into_iter().enumerate() {
                state.set_level(SCRIPT_HELD + i, c);
            }
            let tabs = if flags & LABEL_TABS != 0 { HD_TABS } else { 0 };
            state.set_flags(HD_SCRIPT | tabs);
        }
        _ => save_text(state, &bodies),
    }
}

/// The text bodies `state` keeps.
fn text_bodies(state: &State) -> Bodies {
    let len = usize::from(state.flags() >> HD_LEN_SHIFT & 7);
    let bytes: [u8; Bodies::BYTES] = std::array::from_fn(|i| state.level(i));
    Bodies::load(&bytes, len)
}

/// The text bodies still waiting, kept in `state`.
fn save_text(state: &mut State, bodies: &Bodies) {
    let mut bytes = [0; Bodies::BYTES];
    bodies.save(&mut bytes);
    for (i, c) in bytes.into_iter().enumerate() {
        state.set_level(i, c);
    }
    state.set_flags(HD_TEXT | (bodies.len() as u16) << HD_LEN_SHIFT);
}

/// The last body done: the instructions again.
fn bodies_done(state: &mut State) {
    *state = State::default();
    save(state, Ctx::Body, I_NONE);
}

/// A line in a here-document's body, or its closer.
#[inline(never)]
fn body<K: Sink>(b: &[u8], state: &mut State, out: &mut To<'_, '_, K>) {
    let flags = state.flags();
    if flags & HD_SCRIPT != 0 {
        let bits = [0, 1, 2, 3].map(|i| state.level(SCRIPT_HELD + i));
        let label = Label::from_bits(u32::from_le_bytes(bits));
        let lead = if flags & HD_TABS != 0 {
            Lead::Blanks
        } else {
            Lead::None
        };
        if let Some(c) = label.close(b, lead, Tail::Blanks) {
            out.push(c.start, c.end, hd::HEREDOC_TOKEN);
            return bodies_done(state);
        }
        return script(b, state, out);
    }
    let mut bodies = text_bodies(state);
    let Some((label, f)) = bodies.front() else {
        return bodies_done(state);
    };
    let lead = if f & LABEL_TABS != 0 {
        Lead::Blanks
    } else {
        Lead::None
    };
    if let Some(c) = label.close(b, lead, Tail::Blanks) {
        out.push(c.start, c.end, hd::HEREDOC_TOKEN);
        bodies.pop();
        if bodies.is_empty() {
            return bodies_done(state);
        }
        return save_text(state, &bodies);
    }
    out.push(0, b.len(), hd::HEREDOC);
}

std::thread_local! {
    static SHELL: std::cell::RefCell<Vec<Span>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A line of a `RUN`'s script on bash's scanner, from the state the script's lines before it left.
#[inline(never)]
fn script<K: Sink>(b: &[u8], state: &mut State, out: &mut To<'_, '_, K>) {
    let mut sh = State {
        depth: state.depth,
        ..State::default()
    };
    for i in 0..SCRIPT_HELD {
        sh.set_level(i, state.level(i));
    }
    SHELL.with_borrow_mut(|spans| {
        spans.clear();
        crate::bash::line_spans(b, &mut sh, &mut Out::new(spans));
        for sp in spans.iter() {
            out.push(sp.start as usize, sp.end as usize, sp.kind);
        }
    });
    // bash's stack, its bottom levels dropped past what the script keeps
    let held = sh.held();
    let from = held.saturating_sub(SCRIPT_HELD);
    for i in 0..SCRIPT_HELD {
        state.set_level(
            i,
            if from + i < held {
                sh.level(from + i)
            } else {
                0
            },
        );
    }
    state.depth = (held - from) as u8;
}

/// The arguments or a string in them, from `p` to the line's end: the context the line
/// leaves open.
fn args<K: Sink>(
    b: &[u8],
    mut p: usize,
    mut ctx: Ctx,
    inst: u64,
    state: &mut State,
    out: &mut To<'_, '_, K>,
) -> Ctx {
    loop {
        if ctx == Ctx::Args {
            let k = crate::prim::find_in(b, p, b.len(), *b"\\\"'<");
            let Some(k) = (k < b.len()).then_some(k) else {
                // `\n` pops the arguments
                return Ctx::Body;
            };
            match b[k] {
                b'<' if inst != I_NONE => p = opener(b, k, state, out).unwrap_or(k + 1),
                b'<' => p = k + 1,
                b'\\' => match escaped(b, k) {
                    Some(e) => {
                        out.push(k, e, ESCAPED);
                        p = e;
                    }
                    // a `\` that continues the line: the arguments go on
                    None => return ctx,
                },
                q => {
                    let single = if q == b'"' { 0 } else { IN_SINGLE };
                    out.push(k, k + 1, DOUBLE_BEGIN + single);
                    p = k + 1;
                    ctx = if single == 0 {
                        Ctx::Double
                    } else {
                        Ctx::Single
                    };
                }
            }
            continue;
        }
        let (quote, single) = match ctx {
            Ctx::Double => (b'"', 0),
            _ => (b'\'', IN_SINGLE),
        };
        let k = crate::prim::find_in(b, p, b.len(), [b'\\', quote]);
        let Some(k) = (k < b.len()).then_some(k) else {
            // `\n` sets `invalid`
            out.push(p, b.len(), DOUBLE + single);
            return Ctx::Invalid;
        };
        out.push(p, k, DOUBLE + single);
        if b[k] == quote {
            out.push(k, k + 1, DOUBLE_END + single);
            p = k + 1;
            ctx = Ctx::Args;
            continue;
        }
        match escaped(b, k) {
            Some(e) => {
                out.push(k, e, DOUBLE_ESCAPED + single);
                p = e;
            }
            // a `\` that continues the line: the string goes on
            None => {
                out.push(k, b.len(), DOUBLE + single);
                return ctx;
            }
        }
    }
}

/// Keeps the context a line leaves open and an open instruction's here-document class in
/// `state`.
fn save(state: &mut State, ctx: Ctx, inst: u64) {
    let inst = if matches!(ctx, Ctx::Args | Ctx::Double | Ctx::Single) {
        inst
    } else {
        I_NONE
    };
    let ctx = match ctx {
        Ctx::Main => 0,
        Ctx::Body => 1,
        Ctx::Args => 2,
        Ctx::Double => 3,
        Ctx::Single => 4,
        Ctx::Invalid => 5,
    };
    state.set_word(inst << INST_SHIFT | ctx);
}

/// `\\.` at `k`: where it ends; `None` for a `\` that continues the line, blanks after it or
/// not, as Docker reads `\\[ \t]*$` (X-116: bat's `\ ` an escape, and the arguments ending).
fn escaped(b: &[u8], k: usize) -> Option<usize> {
    if b[k + 1..].iter().all(|&c| c == b' ' || c == b'\t') {
        return None;
    }
    Some(k + 1 + char_len(b, k + 1))
}

/// `^(\s*)((#).*(?m:$)\n?)`: the whole line, `\n` and all.
fn comment<K: Sink>(b: &[u8], out: &mut To<'_, '_, K>) -> bool {
    let Some(q) = b.iter().position(|&c| !is_space(c)) else {
        return false;
    };
    if b[q] != b'#' {
        return false;
    }
    out.push(0, q, COMMENT_LEADING);
    out.push(q, q + 1, COMMENT_MARK);
    out.push(q + 1, b.len(), COMMENT);
    true
}

/// Whether `word` starts at `at`, ignoring ASCII case.
fn word_at(b: &[u8], at: usize, word: &[u8]) -> bool {
    b.get(at..at + word.len())
        .is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// Whether `\s` matches at `at`, the line's `\n` at its end.
fn space_at(b: &[u8], at: usize) -> bool {
    b.get(at).is_none_or(|&c| is_space(c)) && at <= b.len()
}

/// Where the spaces at `at` end.
fn spaces(b: &[u8], at: usize) -> usize {
    at + b[at.min(b.len())..]
        .iter()
        .take_while(|&&c| is_space(c))
        .count()
}

/// What a line's first word is, as the instructions' patterns read it: an instruction that
/// takes arguments (`RUN`, `ENV`, …), `MAINTAINER`, `COPY`, `CMD` or `ENTRYPOINT`, `ONBUILD`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Word {
    Instruction,
    /// `RUN` and `ADD`, instructions whose arguments may open here-documents.
    Run,
    Add,
    Maintainer,
    Copy,
    Command,
    Onbuild,
    Other,
}

/// A word of lowercase letters as [`word`] folds one.
const fn key_of(w: &[u8]) -> u128 {
    let mut k = 0u128;
    let mut i = w.len();
    while i > 0 {
        i -= 1;
        k = k << 8 | w[i] as u128;
    }
    k
}

const K_ADD: u128 = key_of(b"add");
const K_ARG: u128 = key_of(b"arg");
const K_ENV: u128 = key_of(b"env");
const K_RUN: u128 = key_of(b"run");
const K_USER: u128 = key_of(b"user");
const K_LABEL: u128 = key_of(b"label");
const K_SHELL: u128 = key_of(b"shell");
const K_EXPOSE: u128 = key_of(b"expose");
const K_VOLUME: u128 = key_of(b"volume");
const K_WORKDIR: u128 = key_of(b"workdir");
const K_STOPSIGNAL: u128 = key_of(b"stopsignal");
const K_HEALTHCHECK: u128 = key_of(b"healthcheck");
const K_CMD: u128 = key_of(b"cmd");
const K_ENTRYPOINT: u128 = key_of(b"entrypoint");
const K_COPY: u128 = key_of(b"copy");
const K_ONBUILD: u128 = key_of(b"onbuild");
const K_MAINTAINER: u128 = key_of(b"maintainer");

/// The word of ASCII letters at `at` (ignoring case), if `\s` follows it (the line's `\n`
/// among them): which it is, and where it ends. The words are letters, so a word the
/// patterns name ends where the letters do.
fn word(b: &[u8], at: usize) -> (Word, usize) {
    let mut e = at;
    while e < b.len() && b[e].is_ascii_alphabetic() {
        e += 1;
    }
    if !space_at(b, e) {
        return (Word::Other, e);
    }
    // the word folded to lowercase, a byte a letter, as one key: the longest word is 11
    let w = &b[at..e];
    if w.len() > 11 {
        return (Word::Other, e);
    }
    let key = w
        .iter()
        .rev()
        .fold(0u128, |k, &c| k << 8 | u128::from(c | 0x20));
    let word = match key {
        K_RUN => Word::Run,
        K_ADD => Word::Add,
        K_ARG | K_ENV | K_USER | K_LABEL | K_SHELL | K_EXPOSE | K_VOLUME | K_WORKDIR
        | K_STOPSIGNAL | K_HEALTHCHECK => Word::Instruction,
        K_CMD | K_ENTRYPOINT => Word::Command,
        K_COPY => Word::Copy,
        K_ONBUILD => Word::Onbuild,
        K_MAINTAINER => Word::Maintainer,
        _ => Word::Other,
    };
    (word, e)
}

/// `directives`, in the grammar's order: an instruction that takes arguments (`RUN`, `ENV`,
/// …), `MAINTAINER`, `COPY` and `CMD` or `ENTRYPOINT`, `ONBUILD` before any but `MAINTAINER`
/// (`^\s*(?i:(onbuild)\s+)?(?i:<words>)\s`). Where the match ends, its arguments' start: past
/// the line's end when the keyword's `\s` is its `\n`, and the instruction's here-document class.
/// The line's first word is read once.
fn directive<K: Sink>(b: &[u8], out: &mut To<'_, '_, K>) -> Option<(usize, u64)> {
    // caps 0 the keyword, 1 `onbuild` within it
    let whole = |e: usize, onbuild: Option<usize>, kind: u16, out: &mut To<'_, '_, K>| {
        let end = e.min(b.len());
        match onbuild {
            Some(o) => {
                out.push(0, o, kind);
                out.push(o, o + 7, kind + 1);
                out.push(o + 7, end, kind);
            }
            None => out.push(0, end, kind),
        }
        e
    };
    let q = spaces(b, 0);
    let (first, e) = word(b, q);
    let (w, e, onbuild) = match first {
        // `(onbuild)\s+` and the word after it; `MAINTAINER` takes none
        Word::Onbuild => {
            let k = spaces(b, e);
            if k == e {
                return None;
            }
            match word(b, k) {
                (Word::Maintainer, _) => return None,
                (w, e) => (w, e, Some(q)),
            }
        }
        w => (w, e, None),
    };
    match w {
        Word::Instruction | Word::Maintainer => Some((whole(e + 1, onbuild, KEYWORD, out), I_NONE)),
        Word::Run => Some((whole(e + 1, onbuild, KEYWORD, out), I_RUN)),
        Word::Add => Some((whole(e + 1, onbuild, KEYWORD, out), I_TEXT)),
        Word::Copy => copy(b, out).map(|e| (e, I_TEXT)),
        Word::Command => Some((whole(e + 1, onbuild, OPERATOR, out), I_NONE)),
        Word::Onbuild | Word::Other => None,
    }
}

/// `^\s*((?i:(onbuild)\s+)?(?i:copy))(?:\s+--from=(\S+))?\s`: caps 1 the keyword, 2 `onbuild`,
/// 3 the stage.
fn copy<K: Sink>(b: &[u8], out: &mut To<'_, '_, K>) -> Option<usize> {
    let q = spaces(b, 0);
    let (kw_end, onbuild) = match word_at(b, q, b"onbuild") {
        true => {
            let k = spaces(b, q + 7);
            if k > q + 7 && word_at(b, k, b"copy") {
                (k + 4, true)
            } else if word_at(b, q, b"copy") {
                (q + 4, false)
            } else {
                return None;
            }
        }
        false if word_at(b, q, b"copy") => (q + 4, false),
        false => return None,
    };
    let keyword = |out: &mut To<'_, '_, K>| {
        if onbuild {
            out.push(q, q + 7, ONBUILD);
            out.push(q + 7, kw_end, KEYWORD);
        } else {
            out.push(q, kw_end, KEYWORD);
        }
    };
    let s = spaces(b, kw_end);
    if s > kw_end && b.get(s..s + 7) == Some(b"--from=") {
        let e = s + 7 + b[s + 7..].iter().take_while(|&&c| !is_space(c)).count();
        if e > s + 7 && space_at(b, e) {
            keyword(out);
            out.push(s + 7, e, STAGE);
            return Some(e + 1);
        }
    }
    space_at(b, kw_end).then(|| {
        keyword(out);
        kw_end + 1
    })
}

/// `^(?i:(from))\s+[^\s:@]+(?:[:@](\S+))?(?:\s+(?i:(as))\s+(\S+))?`: caps 1 `from`, 2 the tag
/// or digest, 3 `as`, 4 the stage.
fn from<K: Sink>(b: &[u8], out: &mut To<'_, '_, K>) -> bool {
    if !word_at(b, 0, b"from") {
        return false;
    }
    let image = spaces(b, 4);
    let e = image
        + b[image..]
            .iter()
            .take_while(|&&c| !is_space(c) && c != b':' && c != b'@')
            .count();
    if image == 4 || e == image {
        return false;
    }
    out.push(0, 4, KEYWORD);
    let mut end = e;
    if b.get(e).is_some_and(|&c| c == b':' || c == b'@') {
        let t = e + 1 + b[e + 1..].iter().take_while(|&&c| !is_space(c)).count();
        if t > e + 1 {
            out.push(e + 1, t, TAG);
            end = t;
        }
    }
    let a = spaces(b, end);
    if a > end && word_at(b, a, b"as") {
        let n = spaces(b, a + 2);
        let ne = n + b[n..].iter().take_while(|&&c| !is_space(c)).count();
        if n > a + 2 && ne > n {
            out.push(a, a + 2, KEYWORD);
            out.push(n, ne, STAGE);
        }
    }
    true
}

/// `^[^A-Z\n](.*)(?m:$)`
fn invalid(b: &[u8]) -> bool {
    b.first().is_some_and(|c| !c.is_ascii_uppercase())
}

/// `^(?i:arg)\s`
fn arg(b: &[u8]) -> bool {
    word_at(b, 0, b"arg") && space_at(b, 3)
}

/// The length of the character at `i`, a valid UTF-8 line's.
fn char_len(b: &[u8], i: usize) -> usize {
    match b[i] {
        0..0x80 => 1,
        0xc0..0xe0 => 2,
        0xe0..0xf0 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use crate::{Lang, State};

    /// Each line's tokens as (text, leaf scope), the lines lexed one at a time.
    fn leaves(lines: &[&str]) -> Vec<Vec<(String, String)>> {
        let (mut st, mut v) = (State::default(), Vec::new());
        lines
            .iter()
            .map(|l| {
                crate::lex(Lang::Dockerfile, l, &mut st, &mut v);
                v.iter()
                    .map(|s| {
                        let sc = Lang::Dockerfile.scopes(s.kind);
                        (
                            l[s.start as usize..s.end as usize].to_owned(),
                            sc.last().copied().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    fn has(line: &[(String, String)], text: &str, leaf: &str) -> bool {
        line.iter().any(|(t, s)| t == text && s == leaf)
    }

    const TOKEN: &str = "keyword.control.heredoc-token.shell";
    const TEXT: &str = "string.unquoted.heredoc.shell";

    /// A `RUN` whose command is its here-document: the body on bash's scanner, bash's state
    /// carried between its lines (a string across two), then the instructions again.
    #[test]
    fn a_run_script_is_bash() {
        let got = leaves(&[
            "FROM x",
            "RUN --mount=type=cache,target=/c <<EOF",
            "if [ -f x ]; then",
            "  echo \"a $x",
            "b\" EOF",
            "fi",
            "EOF",
            "COPY a b",
        ]);
        assert!(
            has(
                &got[1],
                "<<",
                "keyword.operator.assignment.redirection.shell"
            ),
            "{got:?}"
        );
        assert!(has(&got[1], "EOF", TOKEN), "{got:?}");
        assert!(
            has(&got[2], "if", "keyword.control.conditional.if.shell"),
            "{got:?}"
        );
        assert!(
            has(&got[3], "x", "variable.other.readwrite.shell"),
            "{got:?}"
        );
        assert!(has(&got[4], "b", "string.quoted.double.shell"), "{got:?}");
        assert!(
            has(&got[5], "fi", "keyword.control.conditional.end.shell"),
            "{got:?}"
        );
        assert_eq!(got[6], [("EOF".to_owned(), TOKEN.to_owned())]);
        assert!(
            has(&got[7], "COPY", "keyword.control.dockerfile"),
            "{got:?}"
        );
        // a shell as the command: its stdin is the script
        let got = leaves(&["FROM x", "RUN bash -e <<'EOF'", "set -x", "EOF"]);
        assert!(has(&got[2], "set", "support.function.set.shell"), "{got:?}");
    }

    /// An instruction alone on its line ends with it, and a `\` with blanks after it continues
    /// the arguments, as Docker reads them (X-116: bat kept a bare `RUN`'s arguments open into
    /// the next line, and took `\ ` as an escape that ended them).
    #[test]
    fn lines_end_and_continue_as_docker_reads_them() {
        const KW: &str = "keyword.control.dockerfile";
        let got = leaves(&["FROM x", "RUN", "COPY a b", "WORKDIR", "USER me"]);
        assert!(has(&got[2], "COPY", KW), "{got:?}");
        assert!(has(&got[4], "USER ", KW), "{got:?}");
        let got = leaves(&["FROM x", "RUN a \\ ", "  \"b\" c", "RUN d \\\t", "COPY"]);
        // the continued arguments: a string, not an instruction
        assert!(
            has(
                &got[2],
                "\"",
                "punctuation.definition.string.begin.dockerfile"
            ),
            "{got:?}"
        );
        assert!(!got[2].iter().any(|(_, s)| s == KW), "{got:?}");
        assert!(!got[4].iter().any(|(_, s)| s == KW), "{got:?}");
        // a `\` with text after it is an escape, the arguments ending with the line
        let got = leaves(&["FROM x", "RUN a \\ b", "COPY a b"]);
        assert!(has(&got[2], "COPY", KW), "{got:?}");
    }

    /// Bodies of text (`COPY`'s, `ADD`'s, a `RUN` command's stdin) in the openers' order, a
    /// `<<-` closer after blanks, and a fifth opener on a line past the queue, its body read as
    /// instructions.
    #[test]
    fn bodies_of_text_follow_their_openers() {
        let got = leaves(&[
            "FROM x",
            "COPY <<A <<-\"B\" /dst/",
            "a = $x",
            "A",
            "b",
            "\t B",
            "RUN cat <<EOF > /etc/x",
            "if x",
            "EOF",
        ]);
        assert!(
            has(&got[1], "\"", "punctuation.definition.string.begin.shell"),
            "{got:?}"
        );
        assert_eq!(got[2], [("a = $x".to_owned(), TEXT.to_owned())]);
        assert_eq!(got[3], [("A".to_owned(), TOKEN.to_owned())]);
        assert_eq!(got[4], [("b".to_owned(), TEXT.to_owned())]);
        assert_eq!(got[5], [("B".to_owned(), TOKEN.to_owned())]);
        assert!(
            has(&got[6], "RUN ", "keyword.control.dockerfile"),
            "{got:?}"
        );
        assert_eq!(got[7], [("if x".to_owned(), TEXT.to_owned())]);
        assert_eq!(got[8], [("EOF".to_owned(), TOKEN.to_owned())]);
        let got = leaves(&[
            "FROM x",
            "COPY <<A <<B <<C <<D <<E /d/",
            "A",
            "B",
            "C",
            "D",
            "E",
            "RUN x",
        ]);
        for (i, w) in ["A", "B", "C", "D"].iter().enumerate() {
            assert_eq!(got[2 + i], [(w.to_string(), TOKEN.to_owned())], "{got:?}");
        }
        assert!(got[6].iter().all(|(_, s)| s != TOKEN), "{got:?}");
        assert!(
            has(&got[7], "RUN ", "keyword.control.dockerfile"),
            "{got:?}"
        );
        // not an opener: in a string, after a word's start, `<<<`
        let got = leaves(&["FROM x", "RUN echo \"<<A\" x<<B <<<C", "RUN y"]);
        assert!(
            has(&got[2], "RUN ", "keyword.control.dockerfile"),
            "{got:?}"
        );
    }

    /// The lines as one block lex as they do one at a time, the queue and bash's state past
    /// their caps.
    #[test]
    fn a_block_lexes_as_its_lines() {
        let deep = format!("{}cmd", "{ ".repeat(30));
        let text = [
            "FROM x",
            "COPY <<A <<B <<C <<D <<E /d/",
            "A",
            "B",
            "C",
            "D",
            "E",
            "RUN <<-EOF",
            "cat <<IN",
            "EOF",
            &deep,
            "IN",
            "\tEOF",
            "RUN x",
        ]
        .join("\n");
        let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
        for line in text.split('\n') {
            crate::lex(Lang::Dockerfile, line, &mut state, &mut spans);
            want.push((line.to_owned(), spans.clone()));
        }
        let (mut got, mut state, mut buf) = (Vec::new(), State::default(), String::new());
        crate::lex_lines(
            Lang::Dockerfile,
            &text,
            usize::MAX,
            &mut state,
            (&mut spans, &mut buf),
            |_, _| false,
            |l, s| got.push((l.to_owned(), s.to_vec())),
        );
        assert_eq!(got, want);
        // the closer of the `RUN`'s body ends it, inside bash's own here-document
        assert!(want[9].1.iter().all(|s| {
            Lang::Dockerfile
                .scopes(s.kind)
                .join(" ")
                .contains("heredoc-token")
        }));
        assert!(want[13].1.iter().any(|s| {
            Lang::Dockerfile
                .scopes(s.kind)
                .join(" ")
                .contains("keyword.control.dockerfile")
        }));
    }
}
