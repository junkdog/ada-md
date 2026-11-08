//! The words the scan tells apart: bat's `support-objects` lists (the parent's
//! [`super::lists`] entries, generated from the grammar) and the keywords, in two tables
//! by where a name stands: [`WORDS`] the words a name's start reads (keywords, builtin and DOM
//! classes, support functions), each word's meaning there ([`Info`]); [`DOT_WORDS`] the members
//! a `.` reads ([`Dot`]). Built at compile time; one lookup a name, in the smaller table where
//! most names stand, so most miss at its first-byte and length filter.

use super::lists as w;
use super::*;
use crate::words::{Words, text_len};

/// What a word is to the scan at a name's start.
#[derive(Clone, Copy)]
pub(super) struct Info {
    /// [`W_NONE`], [`W_KW`], …
    pub class: u8,
    /// Its leaf where it is one by itself.
    pub leaf: u16,
    /// The previous token it leaves ([`W_KW`]), or the member list it opens ([`W_OBJ`]).
    pub after: u8,
}

// The classes.
/// A name, nothing more here (a member word, or a contextual word the place decides).
pub(super) const W_NONE: u8 = 0;
/// A keyword: its leaf, the previous token it leaves.
pub(super) const W_KW: u8 = 1;
/// A keyword that starts something of its own (`const`, `function`, `class`, `case`, `new`,
/// `import`, …): [`super::Scan::keyword`]'s, out of the loop.
pub(super) const W_KW2: u8 = 6;
/// A literal (`true`, `null`, `this`, …) or a support name (a builtin class, a DOM variable):
/// its leaf, a value.
pub(super) const W_VALUE: u8 = 2;
/// A support function: its leaf before a `(`, else a name.
pub(super) const W_SUPPORT_FN: u8 = 3;
/// `Math`, `console`, `JSON`, `process`, `module`: its leaf, and a member list after its `.`
/// ([`Info::after`] the previous token that reads it).
pub(super) const W_OBJ: u8 = 4;
/// A word whose meaning is the place's (`async`, `get`, `type`, `as`, …): its id is read.
pub(super) const W_CONTEXT: u8 = 5;

/// What a member word is after a `.`: its leaf where no call follows, and where one does (0
/// for a name's).
#[derive(Clone, Copy)]
pub(super) struct Dot {
    pub plain: u16,
    pub call: u16,
}

// The keywords and the contextual words: (word, class, leaf, after).
const KW: &[(&[u8], u8, u16, u8)] = &[
    (b"const", W_KW2, STORAGE, P_DECL),
    (b"let", W_KW2, STORAGE, P_DECL),
    (b"var", W_KW2, STORAGE, P_DECL),
    (b"function", W_KW2, FUNCTION_KW, P_OP),
    (b"class", W_KW2, CLASS_KW, P_OP),
    (b"return", W_KW, FLOW, P_OP),
    (b"await", W_KW, FLOW, P_OP),
    (b"yield", W_KW2, FLOW, P_OP),
    (b"if", W_KW, CONDITIONAL, P_OP),
    (b"else", W_KW, CONDITIONAL, P_START),
    (b"for", W_KW, LOOP, P_OP),
    (b"while", W_KW, LOOP, P_OP),
    (b"do", W_KW, LOOP, P_START),
    (b"break", W_KW, LOOP, P_JUMP),
    (b"continue", W_KW, LOOP, P_JUMP),
    (b"goto", W_KW, LOOP, P_JUMP),
    (b"switch", W_KW, SWITCH, P_OP),
    (b"case", W_KW2, SWITCH, P_OP),
    (b"default", W_KW2, SWITCH, P_OP),
    (b"try", W_KW, TRY, P_START),
    (b"catch", W_KW, TRY, P_CATCH),
    (b"finally", W_KW, TRY, P_START),
    (b"throw", W_KW, TRY, P_OP),
    (b"new", W_KW2, NEW, P_NEW),
    (b"delete", W_KW, DELETE, P_OP),
    (b"void", W_KW, VOID, P_OP),
    (b"typeof", W_KW, TYPEOF, P_OP),
    (b"instanceof", W_KW, INSTANCEOF, P_TYPE_AFTER),
    (b"in", W_KW, IN, P_OP),
    (b"of", W_CONTEXT, OF, P_OP),
    (b"keyof", W_KW, KEYOF, P_OP),
    (b"debugger", W_KW, DEBUGGER, P_VALUE),
    (b"with", W_KW, WITH, P_OP),
    (b"import", W_KW2, IMPORT, P_IMPORT),
    (b"export", W_KW2, EXPORT, P_IMPORT),
    (b"enum", W_KW, ENUM_KW, P_ENUM_NAME),
    (b"interface", W_CONTEXT, INTERFACE_KW, P_OP),
    (b"type", W_CONTEXT, TYPE_KW, P_OP),
    (b"namespace", W_CONTEXT, NAMESPACE_KW, P_MODULE_NAME),
    (b"module", W_OBJ, MODULE, P_DOT_MODULE),
    (b"declare", W_CONTEXT, MODIFIER, P_START),
    (b"abstract", W_CONTEXT, MODIFIER, P_START),
    (b"async", W_CONTEXT, ASYNC, P_OP),
    (b"from", W_CONTEXT, FROM, P_OP),
    (b"as", W_CONTEXT, AS, P_OP),
    (b"satisfies", W_CONTEXT, AS, P_OP),
    (b"get", W_CONTEXT, PROPERTY_KW, P_ACCESSOR),
    (b"set", W_CONTEXT, PROPERTY_KW, P_ACCESSOR),
    (b"static", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"public", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"private", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"protected", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"readonly", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"override", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"accessor", W_CONTEXT, MODIFIER, P_MEMBER),
    (b"constructor", W_CONTEXT, STORAGE, P_KEYED),
    (b"extends", W_CONTEXT, MODIFIER, P_EXTENDS),
    (b"implements", W_CONTEXT, MODIFIER, P_EXTENDS),
    (b"require", W_SUPPORT_FN, SUPPORT_FN, P_VALUE),
    (b"global", W_CONTEXT, NODE_OBJ, P_VALUE),
    (b"true", W_VALUE, TRUE, P_VALUE),
    (b"false", W_VALUE, FALSE, P_VALUE),
    (b"null", W_VALUE, NULL, P_VALUE),
    (b"undefined", W_VALUE, UNDEFINED, P_VALUE),
    (b"NaN", W_VALUE, NAN, P_VALUE),
    (b"Infinity", W_VALUE, INFINITY, P_VALUE),
    (b"this", W_VALUE, THIS, P_VALUE),
    (b"super", W_VALUE, SUPER, P_VALUE),
    (b"arguments", W_VALUE, ARGUMENTS, P_VALUE),
    (b"Promise", W_VALUE, PROMISE, P_VALUE),
    (b"Math", W_OBJ, MATH, P_DOT_MATH),
    (b"console", W_OBJ, CONSOLE, P_DOT_CONSOLE),
    (b"JSON", W_OBJ, JSON, P_DOT_JSON),
    (b"process", W_OBJ, PROCESS, P_DOT_PROCESS),
    (b"exports", W_VALUE, MODULE, P_VALUE),
    (b"GLOBAL", W_VALUE, NODE_OBJ, P_VALUE),
    (b"root", W_VALUE, NODE_OBJ, P_VALUE),
    (b"__dirname", W_VALUE, NODE_OBJ, P_VALUE),
    (b"__filename", W_VALUE, NODE_OBJ, P_VALUE),
    (b"Error", W_VALUE, ERROR_CLASS, P_VALUE),
    (b"EvalError", W_VALUE, ERROR_CLASS, P_VALUE),
    (b"InternalError", W_VALUE, ERROR_CLASS, P_VALUE),
    (b"RangeError", W_VALUE, ERROR_CLASS, P_VALUE),
    (b"ReferenceError", W_VALUE, ERROR_CLASS, P_VALUE),
    (b"SyntaxError", W_VALUE, ERROR_CLASS, P_VALUE),
    (b"TypeError", W_VALUE, ERROR_CLASS, P_VALUE),
    (b"URIError", W_VALUE, ERROR_CLASS, P_VALUE),
];

// Member words beyond the generated lists: (word, leaf where no call follows, leaf before a
// call; [`NO_LEAF`] for a name's). `Number`'s constants are plain either way.
const MEMBERS: &[(&[u8], u16, u16)] = &[
    (b"constructor", SUPPORT_PROP, NO_LEAF),
    (b"length", SUPPORT_PROP, NO_LEAF),
    (b"prototype", SUPPORT_PROP, NO_LEAF),
    (b"__proto__", SUPPORT_PROP, NO_LEAF),
    (b"EPSILON", PLAIN, PLAIN),
    (b"MAX_SAFE_INTEGER", PLAIN, PLAIN),
    (b"MAX_VALUE", PLAIN, PLAIN),
    (b"MIN_SAFE_INTEGER", PLAIN, PLAIN),
    (b"MIN_VALUE", PLAIN, PLAIN),
    (b"NEGATIVE_INFINITY", PLAIN, PLAIN),
    (b"POSITIVE_INFINITY", PLAIN, PLAIN),
    (b"all", NO_LEAF, PROMISE_FN),
    (b"catch", NO_LEAF, PROMISE_FN),
    (b"finally", NO_LEAF, PROMISE_FN),
    (b"race", NO_LEAF, PROMISE_FN),
    (b"reject", NO_LEAF, PROMISE_FN),
    (b"resolve", NO_LEAF, PROMISE_FN),
    (b"then", NO_LEAF, PROMISE_FN),
    (b"parse", NO_LEAF, NO_LEAF),
    (b"stringify", NO_LEAF, NO_LEAF),
    (b"meta", NO_LEAF, NO_LEAF),
    (b"target", NO_LEAF, NO_LEAF),
    (b"id", NO_LEAF, NO_LEAF),
    (b"filename", NO_LEAF, NO_LEAF),
    (b"loaded", NO_LEAF, NO_LEAF),
    (b"parent", NO_LEAF, NO_LEAF),
    (b"children", NO_LEAF, NO_LEAF),
];

const E: usize = w::ENTRIES.len();

/// The generated bits a name's start reads, and those a member after a `.` reads.
const AT_NAME: u32 =
    w::BUILTIN_CLASS | w::DOM_VAR | w::DOM_CLASS | w::NODE_CLASS | w::SUPPORT_FN | w::KW;
const AT_DOT: u32 = w::DOM_CONST
    | w::DOM_PROP
    | w::EVENT_HANDLER
    | w::BROWSER_FN
    | w::DOM_FN
    | w::MEMBER
    | w::MATH_FN
    | w::MATH_CONST
    | w::CONSOLE_FN
    | w::PROCESS_PROP
    | w::PROCESS_FN;

const fn count(mask: u32) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < E {
        if w::ENTRIES[i].1 & mask != 0 {
            n += 1;
        }
        i += 1;
    }
    n
}

const NE: usize = count(AT_NAME);
const DE: usize = count(AT_DOT);
const N_ALL: usize = NE + KW.len();
const D_ALL: usize = DE + MEMBERS.len();

/// The words a name's start reads: the generated lists' and the keywords.
const NAME_LIST: [&[u8]; N_ALL] = {
    let mut l: [&[u8]; N_ALL] = [b""; N_ALL];
    let (mut i, mut j) = (0, 0);
    while i < E {
        if w::ENTRIES[i].1 & AT_NAME != 0 {
            l[j] = w::ENTRIES[i].0;
            j += 1;
        }
        i += 1;
    }
    let mut k = 0;
    while k < KW.len() {
        l[NE + k] = KW[k].0;
        k += 1;
    }
    l
};

/// The words a member after a `.` reads.
const DOT_LIST: [&[u8]; D_ALL] = {
    let mut l: [&[u8]; D_ALL] = [b""; D_ALL];
    let (mut i, mut j) = (0, 0);
    while i < E {
        if w::ENTRIES[i].1 & AT_DOT != 0 {
            l[j] = w::ENTRIES[i].0;
            j += 1;
        }
        i += 1;
    }
    let mut m = 0;
    while m < MEMBERS.len() {
        l[DE + m] = MEMBERS[m].0;
        m += 1;
    }
    l
};

/// Every word's id in its table: its first place in the list plus one (a word listed twice
/// keeps its first).
const WORDS_C: Words<2048, N_ALL, { text_len(&NAME_LIST) }> = Words::new(&NAME_LIST);
pub(super) static WORDS: Words<2048, N_ALL, { text_len(&NAME_LIST) }> = WORDS_C;
const DOT_WORDS_C: Words<4096, D_ALL, { text_len(&DOT_LIST) }> = Words::new(&DOT_LIST);
pub(super) static DOT_WORDS: Words<4096, D_ALL, { text_len(&DOT_LIST) }> = DOT_WORDS_C;

/// A member word's generated bits, by its [`DOT_WORDS`] id.
pub(super) static BITS: [u32; D_ALL + 1] = {
    let mut t = [0u32; D_ALL + 1];
    let mut i = 0;
    while i < E {
        if w::ENTRIES[i].1 & AT_DOT != 0 {
            let id = DOT_WORDS_C.find(w::ENTRIES[i].0) as usize;
            t[id] |= w::ENTRIES[i].1;
        }
        i += 1;
    }
    t
};

/// What a word is at a name's start, by its [`WORDS`] id.
pub(super) static INFO: [Info; N_ALL + 1] = {
    let none = Info {
        class: W_NONE,
        leaf: PLAIN,
        after: P_VALUE,
    };
    let mut t = [none; N_ALL + 1];
    // the generated lists first, in the order bat's `support-objects` reads a name: a builtin
    // class, a DOM variable, a DOM class, a Node class; a support function before `(`
    let mut i = 0;
    while i < E {
        let bits = w::ENTRIES[i].1;
        if bits & AT_NAME == 0 {
            i += 1;
            continue;
        }
        let id = WORDS_C.find(w::ENTRIES[i].0) as usize;
        let leaf = if bits & w::BUILTIN_CLASS != 0 {
            BUILTIN_CLASS
        } else if bits & w::DOM_VAR != 0 {
            DOM_VAR
        } else if bits & w::DOM_CLASS != 0 {
            DOM_CLASS
        } else if bits & w::NODE_CLASS != 0 {
            NODE_CLASS
        } else {
            PLAIN
        };
        if leaf != PLAIN {
            t[id] = Info {
                class: W_VALUE,
                leaf,
                after: P_VALUE,
            };
        } else if bits & w::SUPPORT_FN != 0 {
            t[id] = Info {
                class: W_SUPPORT_FN,
                leaf: SUPPORT_FN,
                after: P_VALUE,
            };
        }
        i += 1;
    }
    let mut k = 0;
    while k < KW.len() {
        let id = WORDS_C.find(KW[k].0) as usize;
        t[id] = Info {
            class: KW[k].1,
            leaf: KW[k].2,
            after: KW[k].3,
        };
        k += 1;
    }
    t
};

/// No member leaf: the member is a name ([`Dot`]).
pub(super) const NO_LEAF: u16 = u16::MAX;

/// What a word is after a `.`, by id (bat's `support-objects` first pattern).
pub(super) static DOT: [Dot; D_ALL + 1] = {
    let mut t = [Dot {
        plain: NO_LEAF,
        call: NO_LEAF,
    }; D_ALL + 1];
    let mut i = 0;
    while i < E {
        let bits = t_bits(i);
        if bits & AT_DOT == 0 {
            i += 1;
            continue;
        }
        let id = DOT_WORDS_C.find(w::ENTRIES[i].0) as usize;
        if bits & w::DOM_CONST != 0 {
            t[id].plain = DOM_CONST;
        } else if bits & w::DOM_PROP != 0 {
            t[id].plain = DOM_PROP;
        }
        if bits & w::EVENT_HANDLER != 0 {
            t[id].call = EVENT_HANDLER;
        } else if bits & w::BROWSER_FN != 0 {
            t[id].call = SUPPORT_FN;
        } else if bits & w::DOM_FN != 0 {
            t[id].call = SUPPORT_FN_DOM;
        }
        i += 1;
    }
    let mut m = 0;
    while m < MEMBERS.len() {
        let id = DOT_WORDS_C.find(MEMBERS[m].0) as usize;
        let (_, plain, call) = MEMBERS[m];
        if plain != NO_LEAF {
            t[id].plain = plain;
        }
        if call != NO_LEAF {
            t[id].call = call;
        }
        m += 1;
    }
    t
};

const fn t_bits(i: usize) -> u32 {
    w::ENTRIES[i].1
}

/// Word ids (in [`WORDS`]) the scan compares against.
pub(super) const fn id_of(word: &[u8]) -> u16 {
    let id = WORDS_C.find(word);
    assert!(id != 0, "not listed");
    id
}
