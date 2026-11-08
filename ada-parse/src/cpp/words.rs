//! The words bat's C and C++ grammars tell apart, in the order their patterns try them.

use super::text::{is_upper, is_word, word_end};
use super::{
    BREAK_KW, C, CONTINUE_KW, GLOBAL_MAC, GOTO_KW, LANGUAGE, MAC_CONSTANT, MAC_SUPPORT_CONSTANT,
    MAC_TYPE, MEMBER_VAR, MICROSOFT_TYPE, PTHREAD_TYPE, RETURN_KW, STATIC_MAC, STDINT_TYPE,
    SUPPORT_CONSTANT, SYS_TYPE, THROW_KW, VAR_LANGUAGE, WINDOWS_TYPE,
};
use crate::cc::support::{SUPPORT, Support};
use crate::words::{Words, text_len};

/// What a whole word is to C++'s expressions, the first of their patterns that takes it (a
/// pattern that also needs what follows the word says so).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kw {
    Other,
    /// `case`, `default`.
    Case,
    Typedef,
    Using,
    /// `const_cast` and the others.
    Cast,
    /// `sizeof`: `keywords-parens` before `(`, else C's `keyword.operator.word`.
    Sizeof,
    /// `alignof`, `typeid`, `static_assert`: `keywords-parens` before `(`.
    Parens,
    /// `break`, `continue`, `goto`, `return`, `throw`: their leaf.
    Flow(u16),
    /// The rest of `keyword.control`'s list.
    Control,
    Delete,
    New,
    /// `and`, `or`, `noexcept` and the others.
    WordOp,
    Operator,
    Alignas,
    Attribute,
    Declspec,
    /// `unique-modifiers`.
    Modifier,
    /// `char16_t`, `char32_t`, `wchar_t`, `nullptr_t`, `class`: C++'s `storage.type`.
    CxxType,
    Decltype,
    /// C's `storage.type` words.
    CType,
    /// A variable's or a constant's leaf: `this`, `mFoo`, `m_foo`, `gFoo`, `sFoo`, `nullptr`
    /// (C++'s), or one of C's (with [`C`]).
    Value(u16),
    /// A word of C's support lists: its leaf, with [`C`].
    Support(u16),
    /// One of C99's functions, which only a call colors.
    C99,
}

/// The words [`lookup`] classes by themselves, whatever their shape.
const KEYWORDS: [(&[u8], Kw); 98] = {
    use Kw::*;
    [
        (b"_Pragma", Control),
        (b"__attribute__", Attribute),
        (b"__declspec", Declspec),
        (b"__restrict__", Modifier),
        (b"__restrict", Modifier),
        (b"__asm__", CType),
        (b"_Bool", CType),
        (b"_Complex", CType),
        (b"_Imaginary", CType),
        (b"__func__", Value(LANGUAGE | C)),
        (b"__FILE__", Value(SUPPORT_CONSTANT | C)),
        (b"__FUNCTION__", Value(SUPPORT_CONSTANT | C)),
        (b"__LINE__", Value(SUPPORT_CONSTANT | C)),
        (b"alignof", Parens),
        (b"and", WordOp),
        (b"and_eq", WordOp),
        (b"alignas", Alignas),
        (b"asm", CType),
        (b"auto", CType),
        (b"break", Flow(BREAK_KW)),
        (b"bitand", WordOp),
        (b"bitor", WordOp),
        (b"bool", CType),
        (b"case", Case),
        (b"const_cast", Cast),
        (b"continue", Flow(CONTINUE_KW)),
        (b"catch", Control),
        (b"compl", WordOp),
        (b"const", Modifier),
        (b"constexpr", Modifier),
        (b"char16_t", CxxType),
        (b"char32_t", CxxType),
        (b"class", CxxType),
        (b"char", CType),
        (b"default", Case),
        (b"dynamic_cast", Cast),
        (b"do", Control),
        (b"delete", Delete),
        (b"decltype", Decltype),
        (b"double", CType),
        (b"else", Control),
        (b"export", Modifier),
        (b"extern", Modifier),
        (b"explicit", Modifier),
        (b"enum", CType),
        (b"for", Control),
        (b"friend", Modifier),
        (b"float", CType),
        (b"false", Value(LANGUAGE | C)),
        (b"goto", Flow(GOTO_KW)),
        (b"if", Control),
        (b"inline", Modifier),
        (b"int", CType),
        (b"long", CType),
        (b"mutable", Modifier),
        (b"new", New),
        (b"not", WordOp),
        (b"not_eq", WordOp),
        (b"noexcept", WordOp),
        (b"nullptr_t", CxxType),
        (b"nullptr", Value(LANGUAGE)),
        (b"noErr", Value(MAC_SUPPORT_CONSTANT | C)),
        (b"NULL", Value(LANGUAGE | C)),
        (b"or", WordOp),
        (b"or_eq", WordOp),
        (b"operator", Operator),
        (b"reinterpret_cast", Cast),
        (b"return", Flow(RETURN_KW)),
        (b"register", Modifier),
        (b"restrict", Modifier),
        (b"static_cast", Cast),
        (b"sizeof", Sizeof),
        (b"static_assert", Parens),
        (b"switch", Control),
        (b"static", Modifier),
        (b"short", CType),
        (b"signed", CType),
        (b"struct", CType),
        (b"typedef", Typedef),
        (b"typeid", Parens),
        (b"throw", Flow(THROW_KW)),
        (b"try", Control),
        (b"thread_local", Modifier),
        (b"typename", Modifier),
        (b"this", Value(VAR_LANGUAGE)),
        (b"true", Value(LANGUAGE | C)),
        (b"TRUE", Value(LANGUAGE | C)),
        (b"FALSE", Value(LANGUAGE | C)),
        (b"using", Using),
        (b"union", CType),
        (b"unsigned", CType),
        (b"virtual", Modifier),
        (b"volatile", Modifier),
        (b"void", CType),
        (b"while", Control),
        (b"wchar_t", CxxType),
        (b"xor", WordOp),
        (b"xor_eq", WordOp),
    ]
};

/// The word `w` to C++'s expressions: a keyword's class, else a variable's or constant's by
/// its shape, else a support list's.
pub fn lookup(w: &[u8]) -> Kw {
    lookup_inline(w)
}

/// [`lookup`], inlined where a scan's loop asks it a name a turn (X-057): the word's class read
/// by its id ([`BY_ID`]), and a variable's shape tested only on a word that isn't a keyword
/// and starts as one can.
#[inline(always)]
pub fn lookup_inline(w: &[u8]) -> Kw {
    let k = TABLE.value(w, &BY_ID);
    if !matches!(k, Kw::Other | Kw::Support(_) | Kw::C99) {
        return k;
    }
    if matches!(w.first(), Some(b'f' | b'm' | b'g' | b's' | b'k'))
        && let Some(v) = value_shape(w)
    {
        return Kw::Value(v);
    }
    k
}

/// What [`TABLE`]'s words are by id, before a variable's shape: a keyword's class, a support
/// word's leaf (with [`C`]) or C99's function; [`Kw::Other`] at 0, for none.
static BY_ID: [Kw; WORDS + 1] = {
    let mut t = [Kw::Other; WORDS + 1];
    let mut i = 0;
    while i < WORDS {
        t[i + 1] = if i < KEYWORDS.len() {
            KEYWORDS[i].1
        } else {
            match SUPPORT[i - KEYWORDS.len()].1 {
                Support::C99 => Kw::C99,
                Support::Sys => Kw::Support(SYS_TYPE | C),
                Support::Pthread => Kw::Support(PTHREAD_TYPE | C),
                Support::Stdint => Kw::Support(STDINT_TYPE | C),
                Support::Microsoft => Kw::Support(MICROSOFT_TYPE | C),
                Support::Windows => Kw::Support(WINDOWS_TYPE | C),
                Support::Mac => Kw::Support(MAC_TYPE | C),
            }
        };
        i += 1;
    }
    t
};

/// The variables and constants a word's shape gives, in the grammar's order: `\b(f|m)
/// [\p{Lu}]\w*\b` and `\bm_[\p{L}\p{N}_]+\b` (C++'s members), `\bg[A-Z]\w*\b`, `\bs[A-Z]\w*\b`
/// (C's), then C's `\bk[A-Z]\w*\b`.
fn value_shape(w: &[u8]) -> Option<u16> {
    if w.len() < 2 {
        return None;
    }
    let leaf = match w[0] {
        b'f' | b'm' if is_upper(w, 1) => return Some(MEMBER_VAR),
        b'm' if w[1] == b'_' && w.len() > 2 && super::text::ident_end(w, 2) == w.len() => {
            return Some(MEMBER_VAR);
        }
        b'g' => GLOBAL_MAC | C,
        b's' => STATIC_MAC | C,
        b'k' => MAC_CONSTANT | C,
        _ => return None,
    };
    (w[1].is_ascii_uppercase() && word_end(w, 2) == w.len()).then_some(leaf)
}

/// C's own value words, for C's expressions: `\bg[A-Z]\w*\b`, `\bs[A-Z]\w*\b`, then its
/// constants.
pub fn c_value(w: &[u8]) -> Option<u16> {
    match w {
        b"__func__" | b"NULL" | b"true" | b"false" | b"TRUE" | b"FALSE" => {
            return Some(LANGUAGE | C);
        }
        b"__FILE__" | b"__FUNCTION__" | b"__LINE__" => return Some(SUPPORT_CONSTANT | C),
        b"noErr" => return Some(MAC_SUPPORT_CONSTANT | C),
        _ => {}
    }
    if w.len() < 2 || !w[1].is_ascii_uppercase() || word_end(w, 2) != w.len() {
        return None;
    }
    match w[0] {
        b'g' => Some(GLOBAL_MAC | C),
        b's' => Some(STATIC_MAC | C),
        b'k' => Some(MAC_CONSTANT | C),
        _ => None,
    }
}

/// C's `storage.type` words.
pub fn is_c_type(w: &[u8]) -> bool {
    matches!(
        w,
        b"asm"
            | b"__asm__"
            | b"auto"
            | b"bool"
            | b"_Bool"
            | b"char"
            | b"_Complex"
            | b"double"
            | b"float"
            | b"_Imaginary"
            | b"int"
            | b"long"
            | b"short"
            | b"signed"
            | b"unsigned"
            | b"void"
            | b"struct"
            | b"union"
            | b"enum"
    )
}

/// C's `storage.modifier` words (`__declspec(…)` aside).
pub fn is_c_modifier(w: &[u8]) -> bool {
    matches!(
        w,
        b"static"
            | b"extern"
            | b"register"
            | b"const"
            | b"volatile"
            | b"inline"
            | b"restrict"
            | b"__restrict__"
            | b"__restrict"
    )
}

/// C's `keywords`, after their flow words: `keyword.control`'s list.
pub fn is_c_control(w: &[u8]) -> bool {
    matches!(
        w,
        b"break"
            | b"case"
            | b"continue"
            | b"default"
            | b"do"
            | b"else"
            | b"for"
            | b"goto"
            | b"if"
            | b"_Pragma"
            | b"return"
            | b"switch"
            | b"while"
    )
}

/// How many words [`TABLE`] holds: the keywords, then the support words.
const WORDS: usize = KEYWORDS.len() + SUPPORT.len();

/// The words of [`KEYWORDS`] and [`SUPPORT`], in that order: [`TABLE`]'s list.
const LIST: [&[u8]; WORDS] = {
    let mut l: [&[u8]; WORDS] = [b""; WORDS];
    let mut i = 0;
    while i < WORDS {
        l[i] = if i < KEYWORDS.len() {
            KEYWORDS[i].0
        } else {
            SUPPORT[i - KEYWORDS.len()].0
        };
        i += 1;
    }
    l
};

/// The keywords and [`SUPPORT`]'s words: a word's id is its number in [`LIST`] plus one.
static TABLE: Words<2048, WORDS, { text_len(&LIST) }> = Words::new(&LIST);

/// [`SUPPORT`]'s lists, by number.
static LISTS: [Support; SUPPORT.len()] = {
    let mut k = [Support::C99; SUPPORT.len()];
    let mut i = 0;
    while i < k.len() {
        k[i] = SUPPORT[i].1;
        i += 1;
    }
    k
};

/// `w`'s number in [`LIST`].
#[inline]
fn find_word(w: &[u8]) -> Option<usize> {
    usize::from(TABLE.get(w)).checked_sub(1)
}

/// Where `w` is in [`SUPPORT`].
fn find(w: &[u8]) -> Option<usize> {
    find_word(w)?.checked_sub(KEYWORDS.len())
}

/// The leaf, with [`C`], of [`SUPPORT`]'s word `i` (none for C99's functions).
fn support_leaf(i: usize) -> Option<u16> {
    Some(
        match LISTS[i] {
            Support::C99 => return None,
            Support::Sys => SYS_TYPE,
            Support::Pthread => PTHREAD_TYPE,
            Support::Stdint => STDINT_TYPE,
            Support::Microsoft => MICROSOFT_TYPE,
            Support::Windows => WINDOWS_TYPE,
            Support::Mac => MAC_TYPE,
        } | C,
    )
}

/// A word of C's support lists: its leaf, with [`C`].
pub fn support(w: &[u8]) -> Option<u16> {
    support_leaf(find(w)?)
}

/// Whether `w` is one of C99's functions (`support.function.C99.c`).
pub fn is_c99(w: &[u8]) -> bool {
    find(w).is_some_and(|i| LISTS[i] == Support::C99)
}

/// Whether the word at `p` of `b` is one of C99's functions: `\b(…)\b`.
pub fn c99_at(b: &[u8], p: usize) -> Option<usize> {
    if super::text::word_before(b, p) || !is_word(b, p) {
        return None;
    }
    let e = word_end(b, p);
    is_c99(&b[p..e]).then_some(e)
}
