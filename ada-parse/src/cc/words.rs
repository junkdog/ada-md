//! Every word C's and C++'s scanner tells apart, in one table (X-057's redesign): the keywords by
//! leaf, C's support types by list, C99's functions; a word's id reads what it is in C++, in C, in
//! Objective-C and in Objective-C++ from [`INFO`], a word the language hasn't (`class` in C, `self`
//! in C++) plain there. Cocoa's words are a table of their own ([`COCOA_WORDS`]), looked up only
//! in Objective-C's rows, so C's and C++'s table stays as it was.

use super::cocoa::{COCOA, Cocoa};
use super::sql_calls::SQL_NAMES;
use super::support::{SUPPORT, Support};
use super::*;
use crate::words::{Words, text_len};

/// What a listed word does past its leaf ([`INFO`]'s high byte).
pub(super) const A_LEAF: u16 = 0;
/// `case`, `default`: the `:` that ends the label is a separator.
pub(super) const A_CASE: u16 = 1;
/// `public`, `private`, `protected`: a `:` after is a class's section.
pub(super) const A_ACCESS: u16 = 2;
/// `class`, `struct`, `union`: the next `{` opens a class's body, the name before it the
/// class's.
pub(super) const A_CLASS: u16 = 3;
/// `enum`: the next `{` opens its enumerators.
pub(super) const A_ENUM: u16 = 4;
/// `extern` (`extern "C" {`): the next `{` opens a namespace's level.
pub(super) const A_NS: u16 = 5;
/// One of C99's functions: its leaf only where called.
pub(super) const A_C99: u16 = 6;
/// `namespace`: its name next, the `{` after opening a namespace's level.
pub(super) const A_NAMESPACE: u16 = 7;
/// `typedef`: the name before its `;` is the type's.
pub(super) const A_TYPEDEF: u16 = 8;
/// `using`: a name before `=` is the alias's.
pub(super) const A_USING: u16 = 9;
/// `operator`: with its symbol (or the word after it) a function's name.
pub(super) const A_OPERATOR: u16 = 10;
/// `struct`, `union`: as [`A_CLASS`], their names' leaves their own.
pub(super) const A_STRUCT: u16 = 11;
pub(super) const A_UNION: u16 = 12;
/// `template`: a `<` next opens its parameters.
pub(super) const A_TEMPLATE: u16 = 13;
/// `typename`: a modifier, but a type's word in a template's parameters.
pub(super) const A_TYPENAME: u16 = 14;
/// A cast's word: a `<` next opens its type.
pub(super) const A_CAST: u16 = 15;
/// `delete`: a modifier after a definition's `=` (`= delete;`), as `default` is.
pub(super) const A_DELETE: u16 = 16;
/// A regex engine's name ([`ENGINES`]): a plain name, but a string after it on its line may be
/// a regex ([`super::regex_calls`]).
pub(super) const A_REGEX: u16 = 17;
/// Objective-C's `id`: a `<` next opens its protocols (`id<NSCopying>`).
pub(super) const A_ID: u16 = 18;
/// A SQL call's name ([`super::sql_calls::SQL_NAMES`]): a plain name, but a string after it on
/// its line may be SQL.
pub(super) const A_SQL: u16 = 19;

/// Where an [`INFO`] entry's action starts.
pub(super) const ACT: u16 = 8;

/// A word in both C and C++ (and so in Objective-C and Objective-C++), in C++ only (and
/// Objective-C++), in Objective-C and Objective-C++ only.
const BOTH: u8 = 3;
const CXX: u8 = 1;
const OBJC: u8 = 4;

/// The keywords: word, leaf, action, the languages that have it.
const KEYWORDS: [(&[u8], u16, u16, u8); 118] = [
    // C's `storage.type` words, `typedef` among them
    (b"asm", STORAGE_TYPE, A_LEAF, BOTH),
    (b"__asm__", STORAGE_TYPE, A_LEAF, BOTH),
    (b"auto", STORAGE_TYPE, A_LEAF, BOTH),
    (b"bool", STORAGE_TYPE, A_LEAF, BOTH),
    (b"_Bool", STORAGE_TYPE, A_LEAF, BOTH),
    (b"char", STORAGE_TYPE, A_LEAF, BOTH),
    (b"_Complex", STORAGE_TYPE, A_LEAF, BOTH),
    (b"double", STORAGE_TYPE, A_LEAF, BOTH),
    (b"float", STORAGE_TYPE, A_LEAF, BOTH),
    (b"_Imaginary", STORAGE_TYPE, A_LEAF, BOTH),
    (b"int", STORAGE_TYPE, A_LEAF, BOTH),
    (b"long", STORAGE_TYPE, A_LEAF, BOTH),
    (b"short", STORAGE_TYPE, A_LEAF, BOTH),
    (b"signed", STORAGE_TYPE, A_LEAF, BOTH),
    (b"unsigned", STORAGE_TYPE, A_LEAF, BOTH),
    (b"void", STORAGE_TYPE, A_LEAF, BOTH),
    (b"typedef", STORAGE_TYPE, A_TYPEDEF, BOTH),
    (b"struct", STORAGE_TYPE, A_STRUCT, BOTH),
    (b"union", STORAGE_TYPE, A_UNION, BOTH),
    (b"enum", STORAGE_TYPE, A_ENUM, BOTH),
    // C's modifiers
    (b"static", MODIFIER, A_LEAF, BOTH),
    (b"extern", MODIFIER, A_NS, BOTH),
    (b"register", MODIFIER, A_LEAF, BOTH),
    (b"const", MODIFIER, A_LEAF, BOTH),
    (b"volatile", MODIFIER, A_LEAF, BOTH),
    (b"inline", MODIFIER, A_LEAF, BOTH),
    (b"restrict", MODIFIER, A_LEAF, BOTH),
    (b"__restrict__", MODIFIER, A_LEAF, BOTH),
    (b"__restrict", MODIFIER, A_LEAF, BOTH),
    (b"__attribute__", MODIFIER, A_LEAF, BOTH),
    (b"__declspec", MODIFIER, A_LEAF, BOTH),
    // C's control words
    (b"break", BREAK_KW, A_LEAF, BOTH),
    (b"continue", CONTINUE_KW, A_LEAF, BOTH),
    (b"goto", GOTO_KW, A_LEAF, BOTH),
    (b"return", RETURN_KW, A_LEAF, BOTH),
    (b"case", CONTROL, A_CASE, BOTH),
    (b"default", CONTROL, A_CASE, BOTH),
    (b"do", CONTROL, A_LEAF, BOTH),
    (b"else", CONTROL, A_LEAF, BOTH),
    (b"for", CONTROL, A_LEAF, BOTH),
    (b"if", CONTROL, A_LEAF, BOTH),
    (b"_Pragma", CONTROL, A_LEAF, BOTH),
    (b"switch", CONTROL, A_LEAF, BOTH),
    (b"while", CONTROL, A_LEAF, BOTH),
    (b"sizeof", WORD_OP, A_LEAF, BOTH),
    // C's values
    (b"__func__", LANGUAGE, A_LEAF, BOTH),
    (b"NULL", LANGUAGE, A_LEAF, BOTH),
    (b"true", LANGUAGE, A_LEAF, BOTH),
    (b"false", LANGUAGE, A_LEAF, BOTH),
    (b"TRUE", LANGUAGE, A_LEAF, BOTH),
    (b"FALSE", LANGUAGE, A_LEAF, BOTH),
    (b"__FILE__", SUPPORT_CONSTANT, A_LEAF, BOTH),
    (b"__FUNCTION__", SUPPORT_CONSTANT, A_LEAF, BOTH),
    (b"__LINE__", SUPPORT_CONSTANT, A_LEAF, BOTH),
    (b"noErr", MAC_SUPPORT_CONSTANT, A_LEAF, BOTH),
    // C++'s own
    (b"class", STORAGE_TYPE, A_CLASS, CXX),
    (b"typename", MODIFIER, A_TYPENAME, CXX),
    (b"template", TEMPLATE_KW, A_TEMPLATE, CXX),
    (b"concept", STORAGE_TYPE, A_LEAF, CXX),
    (b"namespace", CONTROL, A_NAMESPACE, CXX),
    (b"using", CONTROL, A_USING, CXX),
    (b"new", CONTROL, A_LEAF, CXX),
    (b"delete", CONTROL, A_DELETE, CXX),
    (b"operator", CONTROL, A_OPERATOR, CXX),
    (b"try", CONTROL, A_LEAF, CXX),
    (b"catch", CONTROL, A_LEAF, CXX),
    (b"requires", CONTROL, A_LEAF, CXX),
    (b"throw", THROW_KW, A_LEAF, CXX),
    (b"co_return", RETURN_KW, A_LEAF, CXX),
    (b"co_yield", RETURN_KW, A_LEAF, CXX),
    (b"this", VAR_LANGUAGE, A_LEAF, CXX),
    (b"nullptr", LANGUAGE, A_LEAF, CXX),
    (b"nullptr_t", STORAGE_TYPE, A_LEAF, CXX),
    (b"wchar_t", STORAGE_TYPE, A_LEAF, CXX),
    (b"char8_t", STORAGE_TYPE, A_LEAF, CXX),
    (b"char16_t", STORAGE_TYPE, A_LEAF, CXX),
    (b"char32_t", STORAGE_TYPE, A_LEAF, CXX),
    (b"public", MODIFIER, A_ACCESS, CXX),
    (b"private", MODIFIER, A_ACCESS, CXX),
    (b"protected", MODIFIER, A_ACCESS, CXX),
    (b"virtual", MODIFIER, A_LEAF, CXX),
    (b"explicit", MODIFIER, A_LEAF, CXX),
    (b"friend", MODIFIER, A_LEAF, CXX),
    (b"mutable", MODIFIER, A_LEAF, CXX),
    (b"constexpr", MODIFIER, A_LEAF, CXX),
    (b"consteval", MODIFIER, A_LEAF, CXX),
    (b"constinit", MODIFIER, A_LEAF, CXX),
    (b"thread_local", MODIFIER, A_LEAF, CXX),
    (b"override", MODIFIER, A_LEAF, CXX),
    (b"final", MODIFIER, A_LEAF, CXX),
    (b"export", MODIFIER, A_LEAF, CXX),
    (b"noexcept", MODIFIER, A_LEAF, CXX),
    (b"alignas", MODIFIER, A_LEAF, CXX),
    (b"and", WORD_OP, A_LEAF, CXX),
    (b"and_eq", WORD_OP, A_LEAF, CXX),
    (b"or", WORD_OP, A_LEAF, CXX),
    (b"or_eq", WORD_OP, A_LEAF, CXX),
    (b"not", WORD_OP, A_LEAF, CXX),
    (b"not_eq", WORD_OP, A_LEAF, CXX),
    (b"xor", WORD_OP, A_LEAF, CXX),
    (b"xor_eq", WORD_OP, A_LEAF, CXX),
    (b"bitand", WORD_OP, A_LEAF, CXX),
    (b"bitor", WORD_OP, A_LEAF, CXX),
    (b"compl", WORD_OP, A_LEAF, CXX),
    (b"alignof", WORD_OP, A_LEAF, CXX),
    (b"typeid", WORD_OP, A_LEAF, CXX),
    (b"static_assert", WORD_OP, A_LEAF, CXX),
    (b"decltype", STORAGE_TYPE, A_LEAF, CXX),
    (b"co_await", WORD_OP, A_LEAF, CXX),
    (b"const_cast", CAST_KW, A_CAST, CXX),
    (b"dynamic_cast", CAST_KW, A_CAST, CXX),
    (b"reinterpret_cast", CAST_KW, A_CAST, CXX),
    (b"static_cast", CAST_KW, A_CAST, CXX),
    // the raw string prefixes, plain but before a quote ([`A_LEAF`] with no leaf)
    (b"R", PLAIN, A_LEAF, CXX),
    (b"LR", PLAIN, A_LEAF, CXX),
    (b"uR", PLAIN, A_LEAF, CXX),
    (b"UR", PLAIN, A_LEAF, CXX),
    (b"u8R", PLAIN, A_LEAF, CXX),
];

/// The names a regex engine's call has before its pattern on the line ([`A_REGEX`],
/// [`super::regex_calls`]), and the languages that have them: a string is checked for one only
/// after one of these on its line.
const ENGINES: [(&[u8], u8); 20] = [
    (b"regex", CXX),
    (b"wregex", CXX),
    (b"sregex", CXX),
    (b"cregex", CXX),
    (b"wsregex", CXX),
    (b"wcregex", CXX),
    (b"QRegularExpression", CXX),
    (b"QRegExp", CXX),
    (b"setFilterRegularExpression", CXX),
    (b"setFilterRegExp", CXX),
    (b"RE2", CXX),
    (b"ctre", CXX),
    (b"ctll", CXX),
    (b"regcomp", BOTH),
    (b"pcre2_compile", BOTH),
    (b"pcre2_compile_8", BOTH),
    (b"pcre2_compile_16", BOTH),
    (b"pcre2_compile_32", BOTH),
    (b"pcre_compile", BOTH),
    (b"pcre_compile2", BOTH),
];

/// Objective-C's words (X-134): word, leaf, action. bat's (`self`, `nil`, `SEL`, `_cmd`…), with
/// ada's own: `instancetype` a type; ARC's and the nullability qualifiers modifiers (bat's plain).
const OBJC_WORDS: [(&[u8], u16, u16); 37] = [
    (b"self", VAR_LANGUAGE, A_LEAF),
    (b"super", VAR_LANGUAGE, A_LEAF),
    (b"_cmd", SELECTOR_VAR, A_LEAF),
    (b"NSApp", NSAPP, A_LEAF),
    (b"YES", LANGUAGE, A_LEAF),
    (b"NO", LANGUAGE, A_LEAF),
    (b"nil", LANGUAGE, A_LEAF),
    (b"Nil", LANGUAGE, A_LEAF),
    (b"id", ID_TYPE, A_ID),
    (b"SEL", STORAGE_TYPE, A_LEAF),
    (b"IMP", STORAGE_TYPE, A_LEAF),
    (b"Class", STORAGE_TYPE, A_LEAF),
    (b"unichar", STORAGE_TYPE, A_LEAF),
    (b"IBOutlet", STORAGE_TYPE, A_LEAF),
    (b"IBAction", STORAGE_TYPE, A_LEAF),
    (b"instancetype", STORAGE_TYPE, A_LEAF),
    (b"NS_DURING", MACRO_CONTROL, A_LEAF),
    (b"NS_HANDLER", MACRO_CONTROL, A_LEAF),
    (b"NS_ENDHANDLER", MACRO_CONTROL, A_LEAF),
    (b"__weak", MODIFIER, A_LEAF),
    (b"__strong", MODIFIER, A_LEAF),
    (b"__block", MODIFIER, A_LEAF),
    (b"__unsafe_unretained", MODIFIER, A_LEAF),
    (b"__autoreleasing", MODIFIER, A_LEAF),
    (b"__kindof", MODIFIER, A_LEAF),
    (b"__bridge", MODIFIER, A_LEAF),
    (b"__bridge_transfer", MODIFIER, A_LEAF),
    (b"__bridge_retained", MODIFIER, A_LEAF),
    (b"__covariant", MODIFIER, A_LEAF),
    (b"__contravariant", MODIFIER, A_LEAF),
    (b"_Nullable", MODIFIER, A_LEAF),
    (b"_Nonnull", MODIFIER, A_LEAF),
    (b"_Null_unspecified", MODIFIER, A_LEAF),
    (b"_Nullable_result", MODIFIER, A_LEAF),
    (b"__nullable", MODIFIER, A_LEAF),
    (b"__nonnull", MODIFIER, A_LEAF),
    (b"__null_unspecified", MODIFIER, A_LEAF),
];

/// Words C's lists have that are Objective-C's otherwise: `BOOL` its type (Windows' in C's).
const OBJC_OVERRIDES: [(&[u8], u16); 1] = [(b"BOOL", STORAGE_TYPE)];

/// Where [`ENGINES`] start in [`LIST`].
const E0: usize = KEYWORDS.len() + SUPPORT.len();
/// Where [`OBJC_WORDS`] start in [`LIST`].
const O0: usize = E0 + ENGINES.len();
/// Where [`SQL_NAMES`] start in [`LIST`].
const S0: usize = O0 + OBJC_WORDS.len();
/// How many words [`WORDS`] holds: the keywords, the support words, the engines' names,
/// Objective-C's words, the SQL calls' names.
const N: usize = S0 + SQL_NAMES.len();

const LIST: [&[u8]; N] = {
    let mut l: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        l[i] = if i < KEYWORDS.len() {
            KEYWORDS[i].0
        } else if i < E0 {
            SUPPORT[i - KEYWORDS.len()].0
        } else if i < O0 {
            ENGINES[i - E0].0
        } else if i < S0 {
            OBJC_WORDS[i - O0].0
        } else {
            SQL_NAMES[i - S0]
        };
        i += 1;
    }
    l
};

/// Every word, a word's id its number in [`LIST`] plus one.
pub(super) static WORDS: Words<2048, N, { text_len(&LIST) }> = Words::new(&LIST);

/// The id of the first raw string prefix: an id from it to the last keyword is one.
pub(super) const RAW_PREFIX: u16 = 114;
const _: () = assert!(
    KEYWORDS[RAW_PREFIX as usize - 1].0.len() == 1
        && KEYWORDS[RAW_PREFIX as usize - 1].0[0] == b'R'
);

/// What each word id is in C++ (`[0]`), C (`[1]`), Objective-C (`[2]`) and Objective-C++
/// (`[3]`, [`L_CPP`]…): its leaf, its action at [`ACT`]; 0 for a plain name (id 0, and a word the
/// language hasn't). Objective-C has C's words and its own, Objective-C++ C++'s and Objective-C's.
pub(super) static INFO: [[u16; N + 1]; 4] = {
    let mut t = [[0; N + 1]; 4];
    let mut i = 0;
    while i < N {
        let (leaf, act, langs) = if i < KEYWORDS.len() {
            (KEYWORDS[i].1, KEYWORDS[i].2, KEYWORDS[i].3)
        } else if i >= S0 {
            // C's functions in every row; the members and types only C++ reads
            let w = SQL_NAMES[i - S0];
            let c = w[0] == b's' || w[0] == b'P';
            (PLAIN, A_SQL, if c { BOTH } else { CXX })
        } else if i >= O0 {
            (OBJC_WORDS[i - O0].1, OBJC_WORDS[i - O0].2, OBJC)
        } else if i >= E0 {
            (PLAIN, A_REGEX, ENGINES[i - E0].1)
        } else {
            let leaf = match SUPPORT[i - KEYWORDS.len()].1 {
                Support::C99 => C99,
                Support::Sys => SYS_TYPE,
                Support::Pthread => PTHREAD_TYPE,
                Support::Stdint => STDINT_TYPE,
                Support::Microsoft => MICROSOFT_TYPE,
                Support::Windows => WINDOWS_TYPE,
                Support::Mac => MAC_TYPE,
            };
            let act = if leaf == C99 { A_C99 } else { A_LEAF };
            (leaf, act, BOTH)
        };
        let v = leaf | act << ACT;
        if langs & CXX != 0 {
            t[L_CPP][i + 1] = v;
            t[L_OBJCPP][i + 1] = v;
        }
        if langs & 2 != 0 {
            t[L_C][i + 1] = v;
            t[L_OBJC][i + 1] = v;
        }
        if langs & OBJC != 0 {
            t[L_OBJC][i + 1] = v;
            t[L_OBJCPP][i + 1] = v;
        }
        i += 1;
    }
    let mut o = 0;
    while o < OBJC_OVERRIDES.len() {
        let id = find(&LIST, OBJC_OVERRIDES[o].0);
        t[L_OBJC][id + 1] = OBJC_OVERRIDES[o].1;
        t[L_OBJCPP][id + 1] = OBJC_OVERRIDES[o].1;
        o += 1;
    }
    t
};

/// `w`'s index in `list`, at compile time.
const fn find(list: &[&[u8]], w: &[u8]) -> usize {
    let mut i = 0;
    while i < list.len() {
        let l = list[i];
        if l.len() == w.len() {
            let mut k = 0;
            while k < w.len() && l[k] == w[k] {
                k += 1;
            }
            if k == w.len() {
                return i;
            }
        }
        i += 1;
    }
    panic!("an override's word isn't listed")
}

/// Cocoa's words ([`super::cocoa`]), a table of their own: looked up only in Objective-C's rows,
/// a name starting with `N` or `C` that [`WORDS`] hasn't.
pub(super) static COCOA_WORDS: Words<2048, { COCOA.len() }, { text_len(&COCOA_LIST) }> =
    Words::new(&COCOA_LIST);

const COCOA_LIST: [&[u8]; COCOA.len()] = {
    let mut l: [&[u8]; COCOA.len()] = [b""; COCOA.len()];
    let mut i = 0;
    while i < COCOA.len() {
        l[i] = COCOA[i].0;
        i += 1;
    }
    l
};

/// What each Cocoa word id is: its leaf, as [`INFO`]'s entries ([`A_LEAF`]).
pub(super) static COCOA_INFO: [u16; COCOA.len() + 1] = {
    let mut t = [0; COCOA.len() + 1];
    let mut i = 0;
    while i < COCOA.len() {
        t[i + 1] = match COCOA[i].1 {
            Cocoa::Class => COCOA_CLASS,
            Cocoa::ClassLeopard => COCOA_CLASS_LEOPARD,
            Cocoa::ClassQuartz => QUARTZ_CLASS,
            Cocoa::Type => COCOA_TYPE,
            Cocoa::TypeLeopard => COCOA_TYPE_LEOPARD,
            Cocoa::TypeQuartz => QUARTZ_TYPE,
            Cocoa::Function => COCOA_FUNCTION,
            Cocoa::FunctionLeopard => COCOA_FUNCTION_LEOPARD,
        };
        i += 1;
    }
    t
};
