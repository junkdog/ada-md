//! Every word C's and C++'s scanner tells apart, in one table (X-057's redesign): the keywords by
//! leaf, C's support types by list, C99's functions; a word's id reads what it is in C++ and in C
//! from [`INFO`], a word C hasn't (`class`, `this`) plain there.

use super::support::{SUPPORT, Support};
use super::*;
use crate::words::{Words, text_len};

/// What a listed word does past its leaf ([`INFO`]'s high byte).
pub(super) const A_LEAF: u16 = 0;
/// `case`, `default`: the `:` that ends the label is a separator.
pub(super) const A_CASE: u16 = 1;
/// `public`, `private`, `protected`: a `:` after is a class's section.
pub(super) const A_ACCESS: u16 = 2;
/// `class`, `struct`, `union`: the next `{` opens a class's body.
pub(super) const A_CLASS: u16 = 3;
/// `enum`: the next `{` opens its enumerators.
pub(super) const A_ENUM: u16 = 4;
/// `namespace`, `extern` (`extern "C" {`): the next `{` opens a namespace's level.
pub(super) const A_NS: u16 = 5;
/// One of C99's functions: its leaf only where called.
pub(super) const A_C99: u16 = 6;

/// Where an [`INFO`] entry's action starts.
pub(super) const ACT: u16 = 8;

/// A word in both languages, in C++ only, in C only.
const BOTH: u8 = 3;
const CXX: u8 = 1;

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
    (b"typedef", STORAGE_TYPE, A_LEAF, BOTH),
    (b"struct", STORAGE_TYPE, A_CLASS, BOTH),
    (b"union", STORAGE_TYPE, A_CLASS, BOTH),
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
    (b"typename", STORAGE_TYPE, A_LEAF, CXX),
    (b"template", TEMPLATE_KW, A_LEAF, CXX),
    (b"concept", STORAGE_TYPE, A_LEAF, CXX),
    (b"namespace", CONTROL, A_NS, CXX),
    (b"using", CONTROL, A_LEAF, CXX),
    (b"new", CONTROL, A_LEAF, CXX),
    (b"delete", CONTROL, A_LEAF, CXX),
    (b"operator", CONTROL, A_LEAF, CXX),
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
    (b"decltype", WORD_OP, A_LEAF, CXX),
    (b"co_await", WORD_OP, A_LEAF, CXX),
    (b"const_cast", CAST_KW, A_LEAF, CXX),
    (b"dynamic_cast", CAST_KW, A_LEAF, CXX),
    (b"reinterpret_cast", CAST_KW, A_LEAF, CXX),
    (b"static_cast", CAST_KW, A_LEAF, CXX),
    // the raw string prefixes, plain but before a quote ([`A_LEAF`] with no leaf)
    (b"R", PLAIN, A_LEAF, CXX),
    (b"LR", PLAIN, A_LEAF, CXX),
    (b"uR", PLAIN, A_LEAF, CXX),
    (b"UR", PLAIN, A_LEAF, CXX),
    (b"u8R", PLAIN, A_LEAF, CXX),
];

/// How many words [`WORDS`] holds: the keywords, then the support words.
const N: usize = KEYWORDS.len() + SUPPORT.len();

const LIST: [&[u8]; N] = {
    let mut l: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        l[i] = if i < KEYWORDS.len() {
            KEYWORDS[i].0
        } else {
            SUPPORT[i - KEYWORDS.len()].0
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

/// What each word id is in C++ (`[0]`) and in C (`[1]`): its leaf, its action at [`ACT`]; 0 for
/// a plain name (id 0, and a word the language hasn't).
pub(super) static INFO: [[u16; N + 1]; 2] = {
    let mut t = [[0; N + 1]; 2];
    let mut i = 0;
    while i < N {
        let (leaf, act, langs) = if i < KEYWORDS.len() {
            (KEYWORDS[i].1, KEYWORDS[i].2, KEYWORDS[i].3)
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
            t[0][i + 1] = v;
        }
        if langs & 2 != 0 {
            t[1][i + 1] = v;
        }
        i += 1;
    }
    t
};
