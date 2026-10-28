//! Makefile lines (G-002): bat's `Makefile` grammar, and the shell its recipes embed, run by a
//! stack machine of ada's own in place of syntect on Oniguruma. The grammar decides whether a
//! line is a rule or a variable by a lookahead of nested optional groups (`just_eat`) that
//! backtracks ~n³ on unbalanced parens: a fuzzed line of 305 bytes took 3.1 s.
//!
//! The machine is syntect's (`ParseState`): a stack of contexts, each a list of patterns; at
//! each step the pattern whose match starts leftmost wins, ties going to the first; a match
//! pushes, sets or pops contexts and gives its text scopes; `with_prototype` patterns ride
//! along the contexts above the one that brought them. Each regex is a matcher of its own
//! (`grammar.rs`, `shell.rs`) that finds its leftmost match by scanning, in linear time, and
//! `just_eat` is decided from the line's paren groups ([`scan::Parens`]). So a line gets the
//! scopes syntect gives it, and themes color it as before.
//!
//! Recipe lines are shell (bat's `Bourne Again Shell`, with make's `$(VAR)` on top). Its
//! contexts are ported but for function definitions, coprocesses and here-documents, and some
//! of `${…}`'s operators, which recipes hardly use: those lines may color differently.

mod grammar;
mod scan;
mod shell;

use std::collections::HashMap;

use scan::{Line, M};

macro_rules! scopes {
    ($($v:ident = $s:literal,)*) => {
        /// A scope the Makefile grammar or its shell gives text.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Sc {
            $($v,)*
        }

        impl Sc {
            pub fn name(self) -> &'static str {
                match self {
                    $(Sc::$v => $s,)*
                }
            }
        }
    };
}

scopes! {
    // Makefile
    MetaGroup = "meta.group.makefile",
    MetaString = "meta.string.makefile",
    StrSingle = "string.quoted.single.makefile",
    StrDouble = "string.quoted.double.makefile",
    StrBegin = "punctuation.definition.string.begin.makefile",
    StrEnd = "punctuation.definition.string.end.makefile",
    GroupBegin = "punctuation.section.group.begin.makefile",
    GroupEnd = "punctuation.section.group.end.makefile",
    Separator = "punctuation.separator.makefile",
    StrayParen = "invalid.illegal.stray.paren.makefile",
    Vpath = "keyword.control.vpath.makefile",
    Unquoted = "string.unquoted.makefile",
    Import = "keyword.control.import.makefile",
    Conditional = "keyword.control.conditional.makefile",
    Percent = "variable.language.makefile",
    Wildcard = "variable.language.wildcard.makefile",
    MetaFunction = "meta.function.makefile",
    EntityFunction = "entity.name.function.makefile",
    Assign = "keyword.operator.assignment.makefile",
    FnArgs = "meta.function.arguments.makefile",
    CommentMark = "punctuation.definition.comment.makefile",
    Comment = "comment.line.number-sign.makefile",
    FnBody = "meta.function.body.makefile",
    Terminator = "punctuation.terminator.makefile",
    ConstLang = "constant.language.makefile",
    ExpectedSpaces = "invalid.illegal.inconsistent.expected.spaces.makefile",
    ExpectedTab = "invalid.illegal.inconsistent.expected.tab.makefile",
    Continuation = "punctuation.separator.continuation.line.makefile",
    Escape = "constant.character.escape.makefile",
    CallArgs = "meta.function-call.arguments.makefile",
    BlockEnd = "keyword.other.block.end.makefile",
    BlockBegin = "keyword.other.block.begin.makefile",
    Call = "meta.function-call.makefile",
    CallLang = "constant.language.call.makefile",
    VarFunction = "variable.function.makefile",
    Builtin = "support.function.builtin.makefile",
    Control = "keyword.control.makefile",
    VarOther = "variable.other.makefile",
    Interpolation = "meta.interpolation.makefile",
    InterpBegin = "punctuation.section.interpolation.begin.makefile",
    InterpEnd = "punctuation.section.interpolation.end.makefile",
    SubstMark = "punctuation.definition.substitution.makefile",
    AssignMark = "punctuation.definition.assignment.makefile",
    VarParam = "variable.parameter.makefile",
    Automatic = "variable.language.automatic.makefile",
    SingleChar = "keyword.other.single-character-variable.makefile",
    StrayEndef = "invalid.illegal.stray.endef.makefile",
    // the shell
    Source = "source.shell",
    ShComment = "comment.line.number-sign.shell",
    ShCommentMark = "punctuation.definition.comment.begin.shell",
    ShContinuation = "punctuation.separator.continuation.line.shell",
    ShExtraSpaces = "invalid.illegal.extraneous-spaces-after-line-continuation.shell",
    ShEscape = "constant.character.escape.shell",
    FnCall = "meta.function-call.shell",
    FnName = "variable.function.shell",
    FnArgsSh = "meta.function-call.arguments.shell",
    ShStrDouble = "string.quoted.double.shell",
    ShStrSingle = "string.quoted.single.shell",
    ShStrAnsiC = "string.quoted.single.ansi-c.shell",
    ShStrLocale = "string.quoted.double.locale.shell",
    ShStrBegin = "punctuation.definition.string.begin.shell",
    ShStrEnd = "punctuation.definition.string.end.shell",
    ShUnquoted = "string.unquoted.shell",
    ShOption = "variable.parameter.option.shell",
    OptionMark = "punctuation.definition.parameter.shell",
    OptionAssign = "keyword.operator.assignment.option.shell",
    EndOfOptions = "keyword.operator.end-of-options.shell",
    Continue = "keyword.operator.logical.continue.shell",
    Or = "keyword.operator.logical.or.shell",
    Pipe = "keyword.operator.logical.pipe.shell",
    And = "keyword.operator.logical.and.shell",
    Job = "keyword.operator.logical.job.shell",
    Logical = "keyword.operator.logical.shell",
    LogicalNot = "keyword.operator.logical.not.shell",
    CompoundBegin = "punctuation.definition.compound.begin.shell",
    CompoundEnd = "punctuation.definition.compound.end.shell",
    BracesBegin = "punctuation.definition.compound.braces.begin.shell",
    BracesEnd = "punctuation.definition.compound.braces.end.shell",
    DoubleBraceBegin = "support.function.double-brace.begin.shell",
    DoubleBraceEnd = "support.function.double-brace.end.shell",
    TestBegin = "support.function.test.begin.shell",
    TestEnd = "support.function.test.end.shell",
    Let = "support.function.let.bash",
    If = "keyword.control.conditional.if.shell",
    Then = "keyword.control.conditional.then.shell",
    Elif = "keyword.control.conditional.elseif.shell",
    CondEnd = "keyword.control.conditional.end.shell",
    Else = "keyword.control.conditional.else.shell",
    For = "keyword.control.loop.for.shell",
    Do = "keyword.control.loop.do.shell",
    Done = "keyword.control.loop.end.shell",
    While = "keyword.control.loop.while.shell",
    Until = "keyword.control.loop.until.shell",
    Case = "keyword.control.conditional.case.shell",
    ContinueKw = "keyword.control.flow.continue.shell",
    Break = "keyword.control.flow.break.shell",
    In = "keyword.control.in.shell",
    GroupFor = "meta.group.for.shell",
    CaseMeta = "meta.conditional.case.shell",
    PatternsBegin = "keyword.control.conditional.patterns.begin.shell",
    PatternsEnd = "keyword.control.conditional.patterns.end.shell",
    CasePatterns = "meta.conditional.case.clause.patterns.shell",
    CaseCommands = "meta.conditional.case.clause.commands.shell",
    CaseEnd = "punctuation.terminator.case.clause.shell",
    ParensBegin = "punctuation.section.parens.begin.shell",
    ParensEnd = "punctuation.section.parens.end.shell",
    BracketsBegin = "punctuation.section.brackets.begin.shell",
    BracketsEnd = "punctuation.section.brackets.end.shell",
    SectionBracesBegin = "punctuation.section.braces.begin.shell",
    SectionBracesEnd = "punctuation.section.braces.end.shell",
    FunctionKw = "storage.type.function.shell",
    FunctionName = "entity.name.function.shell",
    Function = "meta.function.shell",
    ArrayVar = "variable.language.array.shell",
    Arithmetic = "meta.group.arithmetic.shell",
    ArithmeticBegin = "punctuation.section.arithmetic.begin.shell",
    ArithmeticEnd = "punctuation.section.arithmetic.end.shell",
    Modifier = "storage.modifier.shell",
    Alias = "support.function.alias.shell",
    AliasName = "entity.name.function.alias.shell",
    VarAssign = "variable.other.readwrite.assignment.shell",
    ShAssign = "keyword.operator.assignment.shell",
    Fd = "constant.numeric.integer.decimal.file-descriptor.shell",
    Redirect = "keyword.operator.assignment.redirection.shell",
    RedirectProcess = "keyword.operator.assignment.redirection.process.shell",
    FdEnd = "punctuation.terminator.file-descriptor.shell",
    HereString = "keyword.operator.herestring.shell",
    History = "variable.language.history.shell",
    HistoryMark = "punctuation.definition.history.shell",
    Backticks = "meta.group.expansion.command.backticks.shell",
    CommandParens = "meta.group.expansion.command.parens.shell",
    ShGroupBegin = "punctuation.section.group.begin.shell",
    ShGroupEnd = "punctuation.section.group.end.shell",
    VarMark = "punctuation.definition.variable.shell",
    Parameter = "meta.group.expansion.parameter.shell",
    ParameterBegin = "punctuation.section.expansion.parameter.begin.shell",
    ParameterEnd = "punctuation.section.expansion.parameter.end.shell",
    ReadWrite = "variable.other.readwrite.shell",
    LangVar = "variable.language.shell",
    Indirection = "keyword.operator.indirection.shell",
    ArithOp = "keyword.operator.arithmetic.shell",
    ExpansionOp = "keyword.operator.expansion.shell",
    Quantifier = "keyword.operator.regexp.quantifier.shell",
    SetBegin = "keyword.control.regexp.set.begin.shell",
    SetEnd = "keyword.control.regexp.set.end.shell",
    WordOp = "keyword.operator.word.shell",
    ClassBegin = "punctuation.separator.character-class.begin.shell",
    ClassEnd = "punctuation.separator.character-class.end.shell",
    CollateBegin = "punctuation.separator.collate.begin.shell",
    CollateEnd = "punctuation.separator.collate.end.shell",
    EquivalenceBegin = "punctuation.separator.equivalence-class.begin.shell",
    EquivalenceEnd = "punctuation.separator.equivalence-class.end.shell",
    HeredocToken = "keyword.control.heredoc-token.shell",
    Heredoc = "string.unquoted.heredoc.shell",
    HeredocSpaces = "invalid.illegal.no-spaces-allowed-after-heredoc-token.shell",
    BacktickEscape = "constant.character.escape.backtick.shell",
    StringMark = "punctuation.definition.string.shell",
    Substitution = "keyword.operator.substitution.shell",
    Switch = "variable.parameter.switch.shell",
    SubstringBegin = "keyword.operator.substring.begin.shell",
    SubstringEnd = "keyword.operator.substring.end.shell",
    TildeGroup = "meta.group.expansion.tilde",
    Tilde = "variable.language.tilde.shell",
    BraceGroup = "meta.group.expansion.brace.shell",
    BraceBegin = "punctuation.section.expansion.brace.begin.shell",
    BraceEnd = "punctuation.section.expansion.brace.end.shell",
    ShSeparator = "punctuation.separator.shell",
    ArithExpansion = "meta.group.expansion.arithmetic.shell",
    JobGroup = "meta.group.expansion.job.shell",
    JobMark = "punctuation.definition.variable.job.shell",
    JobVar = "variable.language.job.shell",
    JobNumber = "constant.numeric.integer.decimal.job.shell",
    Decimal = "constant.numeric.integer.decimal.shell",
    Hex = "constant.numeric.integer.hexadecimal.shell",
    Octal = "constant.numeric.integer.octal.shell",
    OtherBase = "constant.numeric.integer.other.shell",
    Base = "punctuation.definition.numeric.base.shell",
    NotHex = "invalid.illegal.not-a-hex-character.shell",
    NotOctal = "invalid.illegal.not-an-octal-character.shell",
    Bitwise = "keyword.operator.bitwise.shell",
    Ternary = "keyword.operator.ternary.shell",
    ParensGroup = "meta.group.parens.shell",
    Regexp = "meta.regexp.shell",
    BuiltinSet = "support.function.set.shell",
    BuiltinHelp = "support.function.help.shell",
    BuiltinMapfile = "support.function.mapfile.shell",
    BuiltinGetopts = "support.function.getopts.shell",
    BuiltinCd = "support.function.cd.shell",
    BuiltinSuspend = "support.function.suspend.shell",
    BuiltinColon = "support.function.colon.shell",
    BuiltinPrintf = "support.function.printf.shell",
    BuiltinHash = "support.function.hash.shell",
    BuiltinUlimit = "support.function.ulimit.shell",
    BuiltinUmask = "support.function.umask.shell",
    BuiltinSource = "support.function.source.shell",
    BuiltinBuiltin = "support.function.builtin.shell",
    BuiltinExit = "support.function.exit.shell",
    BuiltinType = "support.function.type.shell",
    BuiltinBg = "support.function.bg.shell",
    Return = "keyword.control.flow.return.shell",
    BuiltinExec = "support.function.exec.shell",
    BuiltinRead = "support.function.read.shell",
    BuiltinEnable = "support.function.enable.shell",
    BuiltinPwd = "support.function.pwd.shell",
    BuiltinLogout = "support.function.logout.shell",
    BuiltinEval = "support.function.eval.shell",
    BuiltinFg = "support.function.fg.shell",
    BuiltinDisown = "support.function.disown.shell",
    BuiltinEcho = "support.function.echo.shell",
    BuiltinDot = "support.function.dot.shell",
    BuiltinWait = "support.function.wait.shell",
    BuiltinJobs = "support.function.jobs.shell",
    BuiltinBind = "support.function.bind.shell",
    BuiltinCaller = "support.function.caller.shell",
    BuiltinTimes = "support.function.times.shell",
    BuiltinUnalias = "support.function.unalias.shell",
    BuiltinCommand = "support.function.command.shell",
    BuiltinTrap = "support.function.trap.shell",
    BuiltinShift = "support.function.shift.shell",
    BuiltinKill = "support.function.kill.shell",
    BuiltinReadarray = "support.function.readarray.shell",
    BuiltinShopt = "support.function.shopt.shell",
    BuiltinUnset = "support.function.unset.shell",
}

/// The base scope of every stack, which [`State::stack`] leaves out.
pub const BASE: &str = "source.makefile";

/// Finds a pattern's leftmost match at or after a position; the `u16` is the pattern's own
/// argument.
type Find = fn(&Line, usize, u16) -> Option<M>;

/// What a match does to the stack of contexts.
#[derive(Clone, Copy)]
enum Op {
    None,
    Push(&'static [&'static Ctx]),
    Set(&'static [&'static Ctx]),
    Pop,
}

/// Up to three scopes, kept inline: a pattern's, a group's, a context's meta scopes.
#[derive(Clone, Copy)]
struct Scopes {
    n: u8,
    s: [Sc; 3],
}

impl Scopes {
    const NONE: Scopes = Scopes {
        n: 0,
        s: [Sc::MetaGroup; 3],
    };

    const fn of(scopes: &[Sc]) -> Scopes {
        assert!(scopes.len() <= 3, "at most three scopes");
        let mut s = [Sc::MetaGroup; 3];
        let mut i = 0;
        while i < scopes.len() {
            s[i] = scopes[i];
            i += 1;
        }
        Scopes {
            n: scopes.len() as u8,
            s,
        }
    }

    fn as_slice(&self) -> &[Sc] {
        &self.s[..usize::from(self.n)]
    }

    fn len(self) -> usize {
        usize::from(self.n)
    }

    fn is_empty(self) -> bool {
        self.n == 0
    }
}

/// The group of a [`Pat::caps`] entry past the pattern's last.
const NO_GROUP: u8 = u8::MAX;

/// A pattern: a matcher, the scopes its match and its groups get, and what it does.
struct Pat {
    find: Find,
    op: Op,
    with: Option<&'static Ctx>,
    caps: [(u8, Scopes); 5],
    scope: Scopes,
    arg: u16,
    /// Its matcher reads what the level hands it, or finds only matches that can win, so a
    /// match it found earlier on the line says nothing about later ones.
    nocache: bool,
}

impl Pat {
    const fn new(find: Find) -> Pat {
        Pat {
            find,
            op: Op::None,
            with: None,
            caps: [(NO_GROUP, Scopes::NONE); 5],
            scope: Scopes::NONE,
            arg: 0,
            nocache: false,
        }
    }

    const fn arg(mut self, arg: u16) -> Pat {
        self.arg = arg;
        self
    }

    const fn sc(mut self, scope: &[Sc]) -> Pat {
        self.scope = Scopes::of(scope);
        self
    }

    const fn caps(mut self, caps: &[(u8, &[Sc])]) -> Pat {
        assert!(caps.len() <= 5, "at most five groups");
        let mut i = 0;
        while i < caps.len() {
            self.caps[i] = (caps[i].0, Scopes::of(caps[i].1));
            i += 1;
        }
        self
    }

    const fn push(mut self, ctx: &'static [&'static Ctx]) -> Pat {
        self.op = Op::Push(ctx);
        self
    }

    const fn set(mut self, ctx: &'static [&'static Ctx]) -> Pat {
        self.op = Op::Set(ctx);
        self
    }

    const fn pop(mut self) -> Pat {
        self.op = Op::Pop;
        self
    }

    const fn with(mut self, ctx: &'static Ctx) -> Pat {
        self.with = Some(ctx);
        self
    }

    const fn nocache(mut self) -> Pat {
        self.nocache = true;
        self
    }
}

/// A context's entry: a pattern, or another context's patterns.
enum It {
    P(&'static Pat),
    I(&'static Ctx),
}

/// A context: its patterns, the scopes it gives what it holds, and whether the shell's
/// prototype (comments, continuations, escapes) applies in it.
struct Ctx {
    meta: Scopes,
    content: Scopes,
    clear: u8,
    proto: bool,
    /// The group a pattern in it refers back to (`\2`, `\3`, …), or 0.
    backref: u8,
    items: &'static [It],
}

impl Ctx {
    const fn new(items: &'static [It]) -> Ctx {
        Ctx {
            meta: Scopes::NONE,
            content: Scopes::NONE,
            clear: 0,
            proto: false,
            backref: 0,
            items,
        }
    }

    const fn meta(mut self, meta: &[Sc]) -> Ctx {
        self.meta = Scopes::of(meta);
        self
    }

    const fn content(mut self, content: &[Sc]) -> Ctx {
        self.content = Scopes::of(content);
        self
    }

    const fn clear(mut self, n: u8) -> Ctx {
        self.clear = n;
        self
    }

    /// The shell's contexts include its prototype unless they say otherwise.
    const fn sh(mut self) -> Ctx {
        self.proto = true;
        self
    }

    const fn backref(mut self, group: u8) -> Ctx {
        self.backref = group;
        self
    }
}

/// A level of the stack: a context, the `with_prototype` contexts that came with it, and what
/// its patterns read.
#[derive(Clone, Copy)]
struct Level {
    ctx: &'static Ctx,
    protos: [Option<&'static Ctx>; 2],
    /// Its backreference: 1 + its index in [`State::refs`], or 0.
    cap: u32,
    arg: u16,
}

/// Bytes `start..end` of a line have the scopes of [`State::stack`]`(stack)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub stack: u32,
}

/// Where the machine is between two lines, and the scope stacks it has seen.
pub struct State {
    levels: Vec<Level>,
    proto_starts: Vec<usize>,
    scopes: Vec<Sc>,
    cleared: Vec<Vec<Sc>>,
    /// The text the levels' backreferences stand for, a level's after those below it.
    refs: Vec<Box<[u8]>>,
    stacks: Vec<Box<[Sc]>>,
    ids: HashMap<Box<[Sc]>, u32>,
    buf: String,
    cache: Box<[Slot; CACHE]>,
    generation: u32,
}

/// A pattern's match on the current line, as syntect's search cache keeps it: valid while it
/// starts at or after where the search is.
#[derive(Clone, Copy)]
struct Slot {
    pat: usize,
    generation: u32,
    found: Option<M>,
}

/// Slots for the patterns a line tries: more than the grammars have, so none is ever evicted.
const CACHE: usize = 1024;

/// How deep the stack may grow, as syntect allows.
const MAX_DEPTH: usize = 100;

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            levels: vec![Level {
                ctx: &grammar::START,
                protos: [None; 2],
                cap: 0,
                arg: 0,
            }],
            proto_starts: Vec::new(),
            scopes: Vec::new(),
            cleared: Vec::new(),
            refs: Vec::new(),
            stacks: Vec::new(),
            ids: HashMap::new(),
            buf: String::new(),
            cache: Box::new(
                [Slot {
                    pat: 0,
                    generation: 0,
                    found: None,
                }; CACHE],
            ),
            generation: 0,
        }
    }

    /// The scopes, under [`BASE`], of a span's `stack`.
    pub fn stack(&self, id: u32) -> &[Sc] {
        &self.stacks[id as usize]
    }

    /// How many distinct stacks the spans so far have named.
    pub fn stack_count(&self) -> usize {
        self.stacks.len()
    }

    fn stack_id(&mut self) -> u32 {
        if let Some(&id) = self.ids.get(self.scopes.as_slice()) {
            return id;
        }
        let id = self.stacks.len() as u32;
        let stack: Box<[Sc]> = self.scopes.clone().into_boxed_slice();
        self.stacks.push(stack.clone());
        self.ids.insert(stack, id);
        id
    }
}

/// The spans of one line, `line` without its `\n`, into `spans` (cleared first), from where
/// the line before left `state`.
pub fn tokens(line: &str, state: &mut State, spans: &mut Vec<Span>) {
    spans.clear();
    let mut buf = std::mem::take(&mut state.buf);
    buf.clear();
    buf.push_str(line);
    buf.push('\n');
    state.generation = state.generation.wrapping_add(1);
    if state.generation == 0 {
        // after 2^32 lines: slots from the last time round would pass for this line's
        state.cache.iter_mut().for_each(|s| s.generation = 0);
        state.generation = 1;
    }
    let mut run = Run {
        l: Line::new(&buf),
        text: line.len(),
        at: 0,
        current: None,
        spans,
    };
    run.parse(state);
    run.emit(state, line.len());
    state.buf = buf;
}

/// The best match at a step: the pattern, its match, where it came from.
struct Best {
    pat: &'static Pat,
    m: M,
    from_with: bool,
    would_loop: bool,
}

/// One line's parse.
struct Run<'a, 'b> {
    l: Line<'a>,
    /// The line's length without its `\n`: spans stop there.
    text: usize,
    /// Where the spans so far end.
    at: usize,
    /// The id of the scopes since `at`, once looked up.
    current: Option<u32>,
    spans: &'b mut Vec<Span>,
}

enum Scan {
    Go,
    Stop(Option<Best>),
}

impl Run<'_, '_> {
    fn parse(&mut self, st: &mut State) {
        let mut start = 0;
        let mut push_at = (0usize, 0usize);
        loop {
            let check_loop = push_at == (start, st.levels.len());
            while st
                .proto_starts
                .last()
                .is_some_and(|&s| s >= st.levels.len())
            {
                st.proto_starts.pop();
            }
            let Some(best) = self.best(st, start, check_loop) else {
                return;
            };
            if best.would_loop {
                match self.l.s[start..].char_indices().nth(1) {
                    Some((i, _)) => {
                        start += i;
                        continue;
                    }
                    None => return,
                }
            }
            let end = best.m.e();
            if end == start && matches!(best.pat.op, Op::Push(_)) {
                push_at = (end, st.levels.len() + 1);
            }
            start = end;
            if best.from_with {
                st.proto_starts.push(st.levels.len());
            }
            self.exec(st, &best);
        }
    }

    /// syntect's `find_best_match`: the leftmost match among the `with_prototype` contexts
    /// from the bottom up, the prototype, then the context; a pop that would loop gives way to
    /// another match at the same place.
    fn best(&mut self, st: &mut State, start: usize, check_loop: bool) -> Option<Best> {
        let top = *st.levels.last()?;
        let from = st.proto_starts.last().copied().unwrap_or(0);
        let mut found = Found {
            min: usize::MAX,
            best: None,
            would_loop: false,
        };
        for i in from..st.levels.len() {
            let level = st.levels[i];
            for proto in level.protos.into_iter().flatten() {
                self.set_level(level, &st.refs);
                if let Scan::Stop(b) = self.walk(st, proto, start, check_loop, true, &mut found) {
                    return b;
                }
            }
        }
        if top.ctx.proto {
            self.l.cap.borrow_mut().clear();
            self.l.arg.set(top.arg);
            if let Scan::Stop(b) =
                self.walk(st, &shell::PROTOTYPE, start, check_loop, false, &mut found)
            {
                return b;
            }
        }
        self.set_level(top, &st.refs);
        if let Scan::Stop(b) = self.walk(st, top.ctx, start, check_loop, false, &mut found) {
            return b;
        }
        found.best
    }

    fn set_level(&self, level: Level, refs: &[Box<[u8]>]) {
        let mut cap = self.l.cap.borrow_mut();
        cap.clear();
        if level.cap > 0 {
            cap.extend_from_slice(&refs[level.cap as usize - 1]);
        }
        self.l.arg.set(level.arg);
    }

    fn walk(
        &mut self,
        st: &mut State,
        ctx: &'static Ctx,
        start: usize,
        check_loop: bool,
        from_with: bool,
        found: &mut Found,
    ) -> Scan {
        for item in ctx.items {
            let pat = match item {
                It::I(inner) => {
                    if let Scan::Stop(b) = self.walk(st, inner, start, check_loop, from_with, found)
                    {
                        return Scan::Stop(b);
                    }
                    continue;
                }
                It::P(pat) => *pat,
            };
            let Some(m) = self.search(st, pat, start) else {
                continue;
            };
            let s = m.s();
            if s < found.min || (s == found.min && found.would_loop) {
                found.min = s;
                found.would_loop = check_loop && m.e() <= start && matches!(pat.op, Op::Pop);
                if matches!(pat.op, Op::Push(_)) && st.levels.len() >= MAX_DEPTH {
                    return Scan::Stop(None);
                }
                found.best = Some(Best {
                    pat,
                    m,
                    from_with,
                    would_loop: found.would_loop,
                });
                if s == start && !found.would_loop {
                    return Scan::Stop(found.best.take());
                }
            }
        }
        Scan::Go
    }

    /// syntect's `search`: a pattern's leftmost match, cached for the line.
    fn search(&mut self, st: &mut State, pat: &'static Pat, start: usize) -> Option<M> {
        let key = pat as *const Pat as usize;
        // open addressing: every pattern a line tries keeps its slot for the line, as in
        // syntect, where a push found before is taken even at the depth limit
        let mut slot = (key >> 4) % CACHE;
        while st.cache[slot].generation == st.generation && st.cache[slot].pat != key {
            slot = (slot + 1) % CACHE;
        }
        if !pat.nocache {
            let s = st.cache[slot];
            if s.generation == st.generation {
                match s.found {
                    Some(m) if m.s() >= start => return Some(m),
                    Some(_) => {}
                    None => return None,
                }
            }
        }
        let found = (pat.find)(&self.l, start, pat.arg);
        let does = found.is_none_or(|m| match pat.op {
            Op::None => m.s() != m.e(),
            Op::Push(_) => st.levels.len() < MAX_DEPTH,
            _ => true,
        });
        // as syntect: a match that does nothing here is passed over, and not kept, for a
        // push refused at the depth limit may be taken once the stack is lower
        if !does {
            return None;
        }
        if !pat.nocache {
            st.cache[slot] = Slot {
                pat: key,
                generation: st.generation,
                found,
            };
        }
        found
    }

    fn exec(&mut self, st: &mut State, best: &Best) {
        let (s, e) = (best.m.s(), best.m.e());
        let pat = best.pat;
        let level = *st.levels.last().expect("a level");
        self.meta_ops(st, true, s, level.ctx, pat.op);
        for &sc in pat.scope.as_slice() {
            self.push(st, s, sc);
        }
        if pat.caps[0].0 != NO_GROUP {
            // by index; at one index pops first, then the longer group's pushes
            let mut ops: Vec<(usize, i64, &'static [Sc], usize)> = Vec::new();
            for (i, scopes) in pat.caps.iter().take_while(|c| c.0 != NO_GROUP) {
                let (cs, ce) = best.m.c[usize::from(*i)];
                if cs == u32::MAX || cs == ce {
                    continue;
                }
                let (cs, ce) = (cs as usize, ce as usize);
                ops.push((cs, -((ce - cs) as i64), scopes.as_slice(), 0));
                ops.push((ce, i64::MIN, &[], scopes.len()));
            }
            // an insertion sort: a match has at most a few groups
            for i in 1..ops.len() {
                let mut j = i;
                while j > 0 && (ops[j - 1].0, ops[j - 1].1) > (ops[j].0, ops[j].1) {
                    ops.swap(j - 1, j);
                    j -= 1;
                }
            }
            for (at, _, scopes, pops) in ops {
                if pops > 0 {
                    self.pop(st, at, pops);
                }
                for &sc in scopes {
                    self.push(st, at, sc);
                }
            }
        }
        if !pat.scope.is_empty() {
            self.pop(st, e, pat.scope.len());
        }
        self.meta_ops(st, false, e, level.ctx, pat.op);
        self.perform(st, best, level);
    }

    /// syntect's `push_meta_ops`, at the start of a match (`initial`) or at its end.
    fn meta_ops(&mut self, st: &mut State, initial: bool, at: usize, cur: &Ctx, op: Op) {
        match op {
            Op::Pop => {
                let v = if initial { cur.content } else { cur.meta };
                if !v.is_empty() {
                    self.pop(st, at, v.len());
                }
                if !initial && cur.clear > 0 {
                    self.restore(st, at);
                }
            }
            Op::Push(refs) | Op::Set(refs) => {
                let is_set = matches!(op, Op::Set(_));
                if initial {
                    if is_set && cur.clear > 0 {
                        self.restore(st, at);
                    }
                    for ctx in refs {
                        if !is_set && ctx.clear > 0 {
                            self.clear(st, at, ctx.clear);
                        }
                        for &sc in ctx.meta.as_slice() {
                            self.push(st, at, sc);
                        }
                    }
                } else {
                    let repush = (is_set && (!cur.meta.is_empty() || !cur.content.is_empty()))
                        || refs
                            .iter()
                            .any(|c| !c.content.is_empty() || (c.clear > 0 && is_set));
                    if repush {
                        let mut n: usize = refs.iter().map(|c| c.meta.len()).sum();
                        if is_set {
                            n += cur.content.len() + cur.meta.len();
                        }
                        if n > 0 {
                            self.pop(st, at, n);
                        }
                        for ctx in refs {
                            if is_set && ctx.clear > 0 {
                                self.clear(st, at, ctx.clear);
                            }
                            for &sc in ctx.meta.as_slice().iter().chain(ctx.content.as_slice()) {
                                self.push(st, at, sc);
                            }
                        }
                    }
                }
            }
            Op::None => {}
        }
    }

    fn perform(&mut self, st: &mut State, best: &Best, level: Level) {
        let pat = best.pat;
        let (refs, old) = match pat.op {
            Op::Push(refs) => (refs, None),
            Op::Set(refs) => (refs, pop_level(st).map(|l| l.protos)),
            Op::Pop => {
                pop_level(st);
                return;
            }
            Op::None => return,
        };
        let arg = if pat.arg >= shell::BUILTIN_ARG {
            pat.arg
        } else {
            level.arg
        };
        for (i, &ctx) in refs.iter().enumerate() {
            let mut protos = if i == 0 {
                old.unwrap_or([None; 2])
            } else {
                [None; 2]
            };
            if i == refs.len() - 1
                && let Some(with) = pat.with
            {
                match protos.iter_mut().find(|p| p.is_none()) {
                    Some(free) => *free = Some(with),
                    None => protos[1] = Some(with),
                }
            }
            let group = protos
                .iter()
                .flatten()
                .map(|p| p.backref)
                .fold(ctx.backref, u8::max);
            let cap = match best.m.c[usize::from(group)] {
                (s, e) if group > 0 && s != u32::MAX => {
                    st.refs.push(self.l.b[s as usize..e as usize].into());
                    st.refs.len() as u32
                }
                _ => 0,
            };
            st.levels.push(Level {
                ctx,
                protos,
                cap,
                arg,
            });
        }
    }

    fn push(&mut self, st: &mut State, at: usize, sc: Sc) {
        self.emit(st, at);
        st.scopes.push(sc);
        self.current = None;
    }

    fn pop(&mut self, st: &mut State, at: usize, n: usize) {
        self.emit(st, at);
        let keep = st.scopes.len().saturating_sub(n);
        st.scopes.truncate(keep);
        self.current = None;
    }

    fn clear(&mut self, st: &mut State, at: usize, n: u8) {
        self.emit(st, at);
        let keep = st.scopes.len().saturating_sub(usize::from(n));
        let cleared = st.scopes.split_off(keep);
        st.cleared.push(cleared);
        self.current = None;
    }

    fn restore(&mut self, st: &mut State, at: usize) {
        self.emit(st, at);
        if let Some(cleared) = st.cleared.pop() {
            st.scopes.extend(cleared);
        }
        self.current = None;
    }

    /// Close the span up to `at` under the scopes so far.
    fn emit(&mut self, st: &mut State, at: usize) {
        let at = at.min(self.text);
        if at <= self.at {
            return;
        }
        let id = match self.current {
            Some(id) => id,
            None => {
                let id = st.stack_id();
                self.current = Some(id);
                id
            }
        };
        match self.spans.last_mut() {
            Some(last) if last.stack == id && last.end as usize == self.at => last.end = at as u32,
            _ => self.spans.push(Span {
                start: self.at as u32,
                end: at as u32,
                stack: id,
            }),
        }
        self.at = at;
    }
}

struct Found {
    min: usize,
    best: Option<Best>,
    would_loop: bool,
}

/// Pop the top level, and the backreference it held.
fn pop_level(st: &mut State) -> Option<Level> {
    let level = st.levels.pop()?;
    if level.cap > 0 {
        st.refs.truncate(level.cap as usize - 1);
    }
    Some(level)
}
