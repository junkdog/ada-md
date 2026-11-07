//! The words bat's C and C++ grammars tell apart, in the order their patterns try them.

use super::text::{is_upper, is_word, word_end};
use super::{
    BREAK_KW, C, CONTINUE_KW, GLOBAL_MAC, GOTO_KW, LANGUAGE, MAC_CONSTANT, MAC_SUPPORT_CONSTANT,
    MAC_TYPE, MEMBER_VAR, MICROSOFT_TYPE, PTHREAD_TYPE, RETURN_KW, STATIC_MAC, STDINT_TYPE,
    SUPPORT_CONSTANT, SYS_TYPE, THROW_KW, VAR_LANGUAGE, WINDOWS_TYPE,
};
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

/// Which list of C's a support word is in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Support {
    C99,
    Sys,
    Pthread,
    Stdint,
    Microsoft,
    Windows,
    Mac,
}

/// C's support words: the C99 functions and the types, by list.
#[allow(clippy::large_const_arrays)] // read only where the table is built: not in the binary
const SUPPORT: [(&[u8], Support); 794] = [
    (b"APIENTRY", Support::Windows),
    (b"ATOM", Support::Windows),
    (b"AbsoluteTime", Support::Mac),
    (b"BOOL", Support::Windows),
    (b"BOOLEAN", Support::Windows),
    (b"BYTE", Support::Windows),
    (b"Boolean", Support::Mac),
    (b"Byte", Support::Mac),
    (b"ByteCount", Support::Mac),
    (b"ByteOffset", Support::Mac),
    (b"BytePtr", Support::Mac),
    (b"CALLBACK", Support::Windows),
    (b"CCHAR", Support::Windows),
    (b"CHAR", Support::Windows),
    (b"COLORREF", Support::Windows),
    (b"CONST", Support::Windows),
    (b"CompTimeValue", Support::Mac),
    (b"ConstLogicalAddress", Support::Mac),
    (b"ConstStrFileNameParam", Support::Mac),
    (b"ConstStringPtr", Support::Mac),
    (b"DWORD", Support::Windows),
    (b"DWORD32", Support::Windows),
    (b"DWORD64", Support::Windows),
    (b"DWORDLONG", Support::Windows),
    (b"DWORD_PTR", Support::Windows),
    (b"Duration", Support::Mac),
    (b"FLOAT", Support::Windows),
    (b"Fixed", Support::Mac),
    (b"FixedPtr", Support::Mac),
    (b"Float32", Support::Mac),
    (b"Float32Point", Support::Mac),
    (b"Float64", Support::Mac),
    (b"Float80", Support::Mac),
    (b"Float96", Support::Mac),
    (b"FourCharCode", Support::Mac),
    (b"Fract", Support::Mac),
    (b"FractPtr", Support::Mac),
    (b"HACCEL", Support::Windows),
    (b"HALF_PTR", Support::Windows),
    (b"HANDLE", Support::Windows),
    (b"HBITMAP", Support::Windows),
    (b"HBRUSH", Support::Windows),
    (b"HCOLORSPACE", Support::Windows),
    (b"HCONV", Support::Windows),
    (b"HCONVLIST", Support::Windows),
    (b"HCURSOR", Support::Windows),
    (b"HDC", Support::Windows),
    (b"HDDEDATA", Support::Windows),
    (b"HDESK", Support::Windows),
    (b"HDROP", Support::Windows),
    (b"HDWP", Support::Windows),
    (b"HENHMETAFILE", Support::Windows),
    (b"HFILE", Support::Windows),
    (b"HFONT", Support::Windows),
    (b"HGDIOBJ", Support::Windows),
    (b"HGLOBAL", Support::Windows),
    (b"HHOOK", Support::Windows),
    (b"HICON", Support::Windows),
    (b"HINSTANCE", Support::Windows),
    (b"HKEY", Support::Windows),
    (b"HKL", Support::Windows),
    (b"HLOCAL", Support::Windows),
    (b"HMENU", Support::Windows),
    (b"HMETAFILE", Support::Windows),
    (b"HMODULE", Support::Windows),
    (b"HMONITOR", Support::Windows),
    (b"HPALETTE", Support::Windows),
    (b"HPEN", Support::Windows),
    (b"HRESULT", Support::Windows),
    (b"HRGN", Support::Windows),
    (b"HRSRC", Support::Windows),
    (b"HSZ", Support::Windows),
    (b"HWINSTA", Support::Windows),
    (b"HWND", Support::Windows),
    (b"Handle", Support::Mac),
    (b"INT", Support::Windows),
    (b"INT16", Support::Windows),
    (b"INT32", Support::Windows),
    (b"INT64", Support::Windows),
    (b"INT8", Support::Windows),
    (b"INT_PTR", Support::Windows),
    (b"ItemCount", Support::Mac),
    (b"LANGID", Support::Windows),
    (b"LCID", Support::Windows),
    (b"LCTYPE", Support::Windows),
    (b"LGRPID", Support::Windows),
    (b"LONG", Support::Windows),
    (b"LONG32", Support::Windows),
    (b"LONG64", Support::Windows),
    (b"LONGLONG", Support::Windows),
    (b"LONG_PTR", Support::Windows),
    (b"LPARAM", Support::Windows),
    (b"LPBOOL", Support::Windows),
    (b"LPBYTE", Support::Windows),
    (b"LPCOLORREF", Support::Windows),
    (b"LPCSTR", Support::Windows),
    (b"LPCTSTR", Support::Windows),
    (b"LPCVOID", Support::Windows),
    (b"LPCWSTR", Support::Windows),
    (b"LPDWORD", Support::Windows),
    (b"LPHANDLE", Support::Windows),
    (b"LPINT", Support::Windows),
    (b"LPLONG", Support::Windows),
    (b"LPSTR", Support::Windows),
    (b"LPTSTR", Support::Windows),
    (b"LPVOID", Support::Windows),
    (b"LPWORD", Support::Windows),
    (b"LPWSTR", Support::Windows),
    (b"LRESULT", Support::Windows),
    (b"LogicalAddress", Support::Mac),
    (b"OSErr", Support::Mac),
    (b"OSStatus", Support::Mac),
    (b"OSType", Support::Mac),
    (b"OSTypePtr", Support::Mac),
    (b"OptionBits", Support::Mac),
    (b"PBOOL", Support::Windows),
    (b"PBOOLEAN", Support::Windows),
    (b"PBYTE", Support::Windows),
    (b"PCHAR", Support::Windows),
    (b"PCSTR", Support::Windows),
    (b"PCTSTR", Support::Windows),
    (b"PCWSTR", Support::Windows),
    (b"PDWORD", Support::Windows),
    (b"PDWORD32", Support::Windows),
    (b"PDWORD64", Support::Windows),
    (b"PDWORDLONG", Support::Windows),
    (b"PDWORD_PTR", Support::Windows),
    (b"PFLOAT", Support::Windows),
    (b"PHALF_PTR", Support::Windows),
    (b"PHANDLE", Support::Windows),
    (b"PHKEY", Support::Windows),
    (b"PINT", Support::Windows),
    (b"PINT16", Support::Windows),
    (b"PINT32", Support::Windows),
    (b"PINT64", Support::Windows),
    (b"PINT8", Support::Windows),
    (b"PINT_PTR", Support::Windows),
    (b"PLCID", Support::Windows),
    (b"PLONG", Support::Windows),
    (b"PLONG32", Support::Windows),
    (b"PLONG64", Support::Windows),
    (b"PLONGLONG", Support::Windows),
    (b"PLONG_PTR", Support::Windows),
    (b"POINTER_32", Support::Windows),
    (b"POINTER_64", Support::Windows),
    (b"POINTER_SIGNED", Support::Windows),
    (b"POINTER_UNSIGNED", Support::Windows),
    (b"PSHORT", Support::Windows),
    (b"PSIZE_T", Support::Windows),
    (b"PSSIZE_T", Support::Windows),
    (b"PSTR", Support::Windows),
    (b"PTBYTE", Support::Windows),
    (b"PTCHAR", Support::Windows),
    (b"PTSTR", Support::Windows),
    (b"PUCHAR", Support::Windows),
    (b"PUHALF_PTR", Support::Windows),
    (b"PUINT", Support::Windows),
    (b"PUINT16", Support::Windows),
    (b"PUINT32", Support::Windows),
    (b"PUINT64", Support::Windows),
    (b"PUINT8", Support::Windows),
    (b"PUINT_PTR", Support::Windows),
    (b"PULONG", Support::Windows),
    (b"PULONG32", Support::Windows),
    (b"PULONG64", Support::Windows),
    (b"PULONGLONG", Support::Windows),
    (b"PULONG_PTR", Support::Windows),
    (b"PUSHORT", Support::Windows),
    (b"PVOID", Support::Windows),
    (b"PWCHAR", Support::Windows),
    (b"PWORD", Support::Windows),
    (b"PWSTR", Support::Windows),
    (b"PhysicalAddress", Support::Mac),
    (b"ProcHandle", Support::Mac),
    (b"ProcessSerialNumber", Support::Mac),
    (b"ProcessSerialNumberPtr", Support::Mac),
    (b"Ptr", Support::Mac),
    (b"QWORD", Support::Windows),
    (b"ResType", Support::Mac),
    (b"ResTypePtr", Support::Mac),
    (b"SC_HANDLE", Support::Windows),
    (b"SC_LOCK", Support::Windows),
    (b"SERVICE_STATUS_HANDLE", Support::Windows),
    (b"SHORT", Support::Windows),
    (b"SIZE_T", Support::Windows),
    (b"SInt16", Support::Mac),
    (b"SInt32", Support::Mac),
    (b"SInt64", Support::Mac),
    (b"SInt8", Support::Mac),
    (b"SSIZE_T", Support::Windows),
    (b"ShortFixed", Support::Mac),
    (b"ShortFixedPtr", Support::Mac),
    (b"SignedByte", Support::Mac),
    (b"Size", Support::Mac),
    (b"StrFileName", Support::Mac),
    (b"StringHandle", Support::Mac),
    (b"StringPtr", Support::Mac),
    (b"TBYTE", Support::Windows),
    (b"TCHAR", Support::Windows),
    (b"TimeBase", Support::Mac),
    (b"TimeRecord", Support::Mac),
    (b"TimeScale", Support::Mac),
    (b"TimeValue", Support::Mac),
    (b"TimeValue64", Support::Mac),
    (b"UCHAR", Support::Windows),
    (b"UHALF_PTR", Support::Windows),
    (b"UINT", Support::Windows),
    (b"UINT16", Support::Windows),
    (b"UINT32", Support::Windows),
    (b"UINT64", Support::Windows),
    (b"UINT8", Support::Windows),
    (b"UINT_PTR", Support::Windows),
    (b"UInt16", Support::Mac),
    (b"UInt32", Support::Mac),
    (b"UInt64", Support::Mac),
    (b"UInt8", Support::Mac),
    (b"ULONG", Support::Windows),
    (b"ULONG32", Support::Windows),
    (b"ULONG64", Support::Windows),
    (b"ULONGLONG", Support::Windows),
    (b"ULONG_PTR", Support::Windows),
    (b"UNICODE_STRING", Support::Windows),
    (b"USHORT", Support::Windows),
    (b"USN", Support::Windows),
    (b"UTF16Char", Support::Mac),
    (b"UTF32Char", Support::Mac),
    (b"UTF8Char", Support::Mac),
    (b"UniChar", Support::Mac),
    (b"UniCharCount", Support::Mac),
    (b"UniCharCountPtr", Support::Mac),
    (b"UniCharPtr", Support::Mac),
    (b"UnicodeScalarValue", Support::Mac),
    (b"UniversalProcHandle", Support::Mac),
    (b"UniversalProcPtr", Support::Mac),
    (b"UnsignedFixed", Support::Mac),
    (b"UnsignedFixedPtr", Support::Mac),
    (b"UnsignedWide", Support::Mac),
    (b"VOID", Support::Windows),
    (b"WCHAR", Support::Windows),
    (b"WINAPI", Support::Windows),
    (b"WORD", Support::Windows),
    (b"WPARAM", Support::Windows),
    (b"_Exit", Support::C99),
    (b"__int16", Support::Microsoft),
    (b"__int32", Support::Microsoft),
    (b"__int64", Support::Microsoft),
    (b"__int8", Support::Microsoft),
    (b"abort", Support::C99),
    (b"abs", Support::C99),
    (b"acos", Support::C99),
    (b"acosf", Support::C99),
    (b"acosh", Support::C99),
    (b"acoshf", Support::C99),
    (b"acoshl", Support::C99),
    (b"acosl", Support::C99),
    (b"asctime", Support::C99),
    (b"asin", Support::C99),
    (b"asinf", Support::C99),
    (b"asinh", Support::C99),
    (b"asinhf", Support::C99),
    (b"asinhl", Support::C99),
    (b"asinl", Support::C99),
    (b"assert", Support::C99),
    (b"atan", Support::C99),
    (b"atan2", Support::C99),
    (b"atan2f", Support::C99),
    (b"atan2l", Support::C99),
    (b"atanf", Support::C99),
    (b"atanh", Support::C99),
    (b"atanhf", Support::C99),
    (b"atanhl", Support::C99),
    (b"atanl", Support::C99),
    (b"atexit", Support::C99),
    (b"atof", Support::C99),
    (b"atoi", Support::C99),
    (b"atol", Support::C99),
    (b"atoll", Support::C99),
    (b"blkcnt_t", Support::Sys),
    (b"blksize_t", Support::Sys),
    (b"bsearch", Support::C99),
    (b"btowc", Support::C99),
    (b"cabs", Support::C99),
    (b"cabsf", Support::C99),
    (b"cabsl", Support::C99),
    (b"cacos", Support::C99),
    (b"cacosf", Support::C99),
    (b"cacosh", Support::C99),
    (b"cacoshf", Support::C99),
    (b"cacoshl", Support::C99),
    (b"cacosl", Support::C99),
    (b"caddr_t", Support::Sys),
    (b"calloc", Support::C99),
    (b"carg", Support::C99),
    (b"cargf", Support::C99),
    (b"cargl", Support::C99),
    (b"casin", Support::C99),
    (b"casinf", Support::C99),
    (b"casinh", Support::C99),
    (b"casinhf", Support::C99),
    (b"casinhl", Support::C99),
    (b"casinl", Support::C99),
    (b"catan", Support::C99),
    (b"catanf", Support::C99),
    (b"catanh", Support::C99),
    (b"catanhf", Support::C99),
    (b"catanhl", Support::C99),
    (b"catanl", Support::C99),
    (b"cbrt", Support::C99),
    (b"cbrtf", Support::C99),
    (b"cbrtl", Support::C99),
    (b"ccos", Support::C99),
    (b"ccosf", Support::C99),
    (b"ccosh", Support::C99),
    (b"ccoshf", Support::C99),
    (b"ccosl", Support::C99),
    (b"ceil", Support::C99),
    (b"ceilf", Support::C99),
    (b"ceill", Support::C99),
    (b"cexp", Support::C99),
    (b"cexpf", Support::C99),
    (b"cexpl", Support::C99),
    (b"cimag", Support::C99),
    (b"cimagf", Support::C99),
    (b"cimagl", Support::C99),
    (b"clearerr", Support::C99),
    (b"clock", Support::C99),
    (b"clock_t", Support::Sys),
    (b"clog", Support::C99),
    (b"clogf", Support::C99),
    (b"clogl", Support::C99),
    (b"conj", Support::C99),
    (b"conjf", Support::C99),
    (b"conjl", Support::C99),
    (b"copysign", Support::C99),
    (b"copysignf", Support::C99),
    (b"copysignl", Support::C99),
    (b"cos", Support::C99),
    (b"cosf", Support::C99),
    (b"cosh", Support::C99),
    (b"coshf", Support::C99),
    (b"coshl", Support::C99),
    (b"cosl", Support::C99),
    (b"cpow", Support::C99),
    (b"cpowf", Support::C99),
    (b"cpowl", Support::C99),
    (b"cproj", Support::C99),
    (b"cprojf", Support::C99),
    (b"cprojl", Support::C99),
    (b"creal", Support::C99),
    (b"crealf", Support::C99),
    (b"creall", Support::C99),
    (b"csin", Support::C99),
    (b"csinf", Support::C99),
    (b"csinh", Support::C99),
    (b"csinhf", Support::C99),
    (b"csinhl", Support::C99),
    (b"csinl", Support::C99),
    (b"csqrt", Support::C99),
    (b"csqrtf", Support::C99),
    (b"csqrtl", Support::C99),
    (b"ctan", Support::C99),
    (b"ctanf", Support::C99),
    (b"ctanh", Support::C99),
    (b"ctanhf", Support::C99),
    (b"ctanhl", Support::C99),
    (b"ctanl", Support::C99),
    (b"ctime", Support::C99),
    (b"daddr_t", Support::Sys),
    (b"dev_t", Support::Sys),
    (b"difftime", Support::C99),
    (b"div", Support::C99),
    (b"erf", Support::C99),
    (b"erfc", Support::C99),
    (b"erfcf", Support::C99),
    (b"erfcl", Support::C99),
    (b"erff", Support::C99),
    (b"erfl", Support::C99),
    (b"exit", Support::C99),
    (b"exp", Support::C99),
    (b"exp2", Support::C99),
    (b"exp2f", Support::C99),
    (b"exp2l", Support::C99),
    (b"expf", Support::C99),
    (b"expl", Support::C99),
    (b"expm1", Support::C99),
    (b"expm1f", Support::C99),
    (b"expm1l", Support::C99),
    (b"fabs", Support::C99),
    (b"fabsf", Support::C99),
    (b"fabsl", Support::C99),
    (b"fclose", Support::C99),
    (b"fdim", Support::C99),
    (b"fdimf", Support::C99),
    (b"fdiml", Support::C99),
    (b"feclearexcept", Support::C99),
    (b"fegetenv", Support::C99),
    (b"fegetexceptflag", Support::C99),
    (b"fegetround", Support::C99),
    (b"feholdexcept", Support::C99),
    (b"feof", Support::C99),
    (b"feraiseexcept", Support::C99),
    (b"ferror", Support::C99),
    (b"fesetenv", Support::C99),
    (b"fesetexceptflag", Support::C99),
    (b"fesetround", Support::C99),
    (b"fetestexcept", Support::C99),
    (b"feupdateenv", Support::C99),
    (b"fflush", Support::C99),
    (b"fgetc", Support::C99),
    (b"fgetpos", Support::C99),
    (b"fgets", Support::C99),
    (b"fgetwc", Support::C99),
    (b"fgetws", Support::C99),
    (b"fixpt_t", Support::Sys),
    (b"floor", Support::C99),
    (b"floorf", Support::C99),
    (b"floorl", Support::C99),
    (b"fma", Support::C99),
    (b"fmaf", Support::C99),
    (b"fmal", Support::C99),
    (b"fmax", Support::C99),
    (b"fmaxf", Support::C99),
    (b"fmaxl", Support::C99),
    (b"fmin", Support::C99),
    (b"fminf", Support::C99),
    (b"fminl", Support::C99),
    (b"fmod", Support::C99),
    (b"fmodf", Support::C99),
    (b"fmodl", Support::C99),
    (b"fopen", Support::C99),
    (b"fpclassify", Support::C99),
    (b"fprintf", Support::C99),
    (b"fputc", Support::C99),
    (b"fputs", Support::C99),
    (b"fputwc", Support::C99),
    (b"fputws", Support::C99),
    (b"fread", Support::C99),
    (b"free", Support::C99),
    (b"freopen", Support::C99),
    (b"frexp", Support::C99),
    (b"frexpf", Support::C99),
    (b"frexpl", Support::C99),
    (b"fscanf", Support::C99),
    (b"fseek", Support::C99),
    (b"fsetpos", Support::C99),
    (b"ftell", Support::C99),
    (b"fwide", Support::C99),
    (b"fwprintf", Support::C99),
    (b"fwrite", Support::C99),
    (b"fwscanf", Support::C99),
    (b"getc", Support::C99),
    (b"getchar", Support::C99),
    (b"getenv", Support::C99),
    (b"gets", Support::C99),
    (b"getwc", Support::C99),
    (b"getwchar", Support::C99),
    (b"gid_t", Support::Sys),
    (b"gmtime", Support::C99),
    (b"hypot", Support::C99),
    (b"hypotf", Support::C99),
    (b"hypotl", Support::C99),
    (b"id_t", Support::Sys),
    (b"ilogb", Support::C99),
    (b"ilogbf", Support::C99),
    (b"ilogbl", Support::C99),
    (b"imaxabs", Support::C99),
    (b"imaxdiv", Support::C99),
    (b"in_addr_t", Support::Sys),
    (b"in_port_t", Support::Sys),
    (b"ino_t", Support::Sys),
    (b"int16_t", Support::Stdint),
    (b"int32_t", Support::Stdint),
    (b"int64_t", Support::Stdint),
    (b"int8_t", Support::Stdint),
    (b"int_fast16_t", Support::Stdint),
    (b"int_fast32_t", Support::Stdint),
    (b"int_fast64_t", Support::Stdint),
    (b"int_fast8_t", Support::Stdint),
    (b"int_least16_t", Support::Stdint),
    (b"int_least32_t", Support::Stdint),
    (b"int_least64_t", Support::Stdint),
    (b"int_least8_t", Support::Stdint),
    (b"intmax_t", Support::Stdint),
    (b"intptr_t", Support::Stdint),
    (b"isalnum", Support::C99),
    (b"isalpha", Support::C99),
    (b"isblank", Support::C99),
    (b"iscntrl", Support::C99),
    (b"isdigit", Support::C99),
    (b"isfinite", Support::C99),
    (b"isgraph", Support::C99),
    (b"isgreater", Support::C99),
    (b"isgreaterequal", Support::C99),
    (b"isinf", Support::C99),
    (b"isless", Support::C99),
    (b"islessequal", Support::C99),
    (b"islessgreater", Support::C99),
    (b"islower", Support::C99),
    (b"isnan", Support::C99),
    (b"isnormal", Support::C99),
    (b"isprint", Support::C99),
    (b"ispunct", Support::C99),
    (b"isspace", Support::C99),
    (b"isunordered", Support::C99),
    (b"isupper", Support::C99),
    (b"iswalnum", Support::C99),
    (b"iswalpha", Support::C99),
    (b"iswblank", Support::C99),
    (b"iswcntrl", Support::C99),
    (b"iswctype", Support::C99),
    (b"iswdigit", Support::C99),
    (b"iswgraph", Support::C99),
    (b"iswlower", Support::C99),
    (b"iswprint", Support::C99),
    (b"iswpunct", Support::C99),
    (b"iswspace", Support::C99),
    (b"iswupper", Support::C99),
    (b"iswxdigit", Support::C99),
    (b"isxdigit", Support::C99),
    (b"key_t", Support::Sys),
    (b"labs", Support::C99),
    (b"ldexp", Support::C99),
    (b"ldexpf", Support::C99),
    (b"ldexpl", Support::C99),
    (b"ldiv", Support::C99),
    (b"lgamma", Support::C99),
    (b"lgammaf", Support::C99),
    (b"lgammal", Support::C99),
    (b"llabs", Support::C99),
    (b"lldiv", Support::C99),
    (b"llrint", Support::C99),
    (b"llrintf", Support::C99),
    (b"llrintl", Support::C99),
    (b"llround", Support::C99),
    (b"llroundf", Support::C99),
    (b"llroundl", Support::C99),
    (b"localeconv", Support::C99),
    (b"localtime", Support::C99),
    (b"log", Support::C99),
    (b"log10", Support::C99),
    (b"log10f", Support::C99),
    (b"log10l", Support::C99),
    (b"log1p", Support::C99),
    (b"log1pf", Support::C99),
    (b"log1pl", Support::C99),
    (b"log2", Support::C99),
    (b"log2f", Support::C99),
    (b"log2l", Support::C99),
    (b"logb", Support::C99),
    (b"logbf", Support::C99),
    (b"logbl", Support::C99),
    (b"logf", Support::C99),
    (b"logl", Support::C99),
    (b"longjmp", Support::C99),
    (b"lrint", Support::C99),
    (b"lrintf", Support::C99),
    (b"lrintl", Support::C99),
    (b"lround", Support::C99),
    (b"lroundf", Support::C99),
    (b"lroundl", Support::C99),
    (b"malloc", Support::C99),
    (b"mblen", Support::C99),
    (b"mbrlen", Support::C99),
    (b"mbrtowc", Support::C99),
    (b"mbsinit", Support::C99),
    (b"mbsrtowcs", Support::C99),
    (b"mbstowcs", Support::C99),
    (b"mbtowc", Support::C99),
    (b"memchr", Support::C99),
    (b"memcmp", Support::C99),
    (b"memcpy", Support::C99),
    (b"memmove", Support::C99),
    (b"memset", Support::C99),
    (b"mktime", Support::C99),
    (b"mode_t", Support::Sys),
    (b"modf", Support::C99),
    (b"modff", Support::C99),
    (b"modfl", Support::C99),
    (b"nan", Support::C99),
    (b"nanf", Support::C99),
    (b"nanl", Support::C99),
    (b"nearbyint", Support::C99),
    (b"nearbyintf", Support::C99),
    (b"nearbyintl", Support::C99),
    (b"nextafter", Support::C99),
    (b"nextafterf", Support::C99),
    (b"nextafterl", Support::C99),
    (b"nexttoward", Support::C99),
    (b"nexttowardf", Support::C99),
    (b"nexttowardl", Support::C99),
    (b"nlink_t", Support::Sys),
    (b"off_t", Support::Sys),
    (b"perror", Support::C99),
    (b"pid_t", Support::Sys),
    (b"pow", Support::C99),
    (b"powf", Support::C99),
    (b"powl", Support::C99),
    (b"printf", Support::C99),
    (b"pthread_attr_t", Support::Pthread),
    (b"pthread_cond_t", Support::Pthread),
    (b"pthread_condattr_t", Support::Pthread),
    (b"pthread_key_t", Support::Pthread),
    (b"pthread_mutex_t", Support::Pthread),
    (b"pthread_mutexattr_t", Support::Pthread),
    (b"pthread_once_t", Support::Pthread),
    (b"pthread_rwlock_t", Support::Pthread),
    (b"pthread_rwlockattr_t", Support::Pthread),
    (b"pthread_t", Support::Pthread),
    (b"ptrdiff_t", Support::Sys),
    (b"putc", Support::C99),
    (b"putchar", Support::C99),
    (b"puts", Support::C99),
    (b"putwc", Support::C99),
    (b"putwchar", Support::C99),
    (b"qaddr_t", Support::Sys),
    (b"qsort", Support::C99),
    (b"quad_t", Support::Sys),
    (b"raise", Support::C99),
    (b"rand", Support::C99),
    (b"realloc", Support::C99),
    (b"remainder", Support::C99),
    (b"remainderf", Support::C99),
    (b"remainderl", Support::C99),
    (b"remove", Support::C99),
    (b"remquo", Support::C99),
    (b"remquof", Support::C99),
    (b"remquol", Support::C99),
    (b"rename", Support::C99),
    (b"rewind", Support::C99),
    (b"rint", Support::C99),
    (b"rintf", Support::C99),
    (b"rintl", Support::C99),
    (b"round", Support::C99),
    (b"roundf", Support::C99),
    (b"roundl", Support::C99),
    (b"scalbln", Support::C99),
    (b"scalblnf", Support::C99),
    (b"scalblnl", Support::C99),
    (b"scalbn", Support::C99),
    (b"scalbnf", Support::C99),
    (b"scalbnl", Support::C99),
    (b"scanf", Support::C99),
    (b"segsz_t", Support::Sys),
    (b"setbuf", Support::C99),
    (b"setjmp", Support::C99),
    (b"setlocale", Support::C99),
    (b"setvbuf", Support::C99),
    (b"signal", Support::C99),
    (b"signbit", Support::C99),
    (b"sin", Support::C99),
    (b"sinf", Support::C99),
    (b"sinh", Support::C99),
    (b"sinhf", Support::C99),
    (b"sinhl", Support::C99),
    (b"sinl", Support::C99),
    (b"size_t", Support::Sys),
    (b"snprintf", Support::C99),
    (b"sprintf", Support::C99),
    (b"sqrt", Support::C99),
    (b"sqrtf", Support::C99),
    (b"sqrtl", Support::C99),
    (b"srand", Support::C99),
    (b"sscanf", Support::C99),
    (b"ssize_t", Support::Sys),
    (b"strcat", Support::C99),
    (b"strchr", Support::C99),
    (b"strcmp", Support::C99),
    (b"strcoll", Support::C99),
    (b"strcpy", Support::C99),
    (b"strcspn", Support::C99),
    (b"strerror", Support::C99),
    (b"strftime", Support::C99),
    (b"strlen", Support::C99),
    (b"strncat", Support::C99),
    (b"strncmp", Support::C99),
    (b"strncpy", Support::C99),
    (b"strpbrk", Support::C99),
    (b"strrchr", Support::C99),
    (b"strspn", Support::C99),
    (b"strstr", Support::C99),
    (b"strtod", Support::C99),
    (b"strtof", Support::C99),
    (b"strtoimax", Support::C99),
    (b"strtok", Support::C99),
    (b"strtol", Support::C99),
    (b"strtold", Support::C99),
    (b"strtoll", Support::C99),
    (b"strtoul", Support::C99),
    (b"strtoull", Support::C99),
    (b"strtoumax", Support::C99),
    (b"strxfrm", Support::C99),
    (b"suseconds_t", Support::Sys),
    (b"swblk_t", Support::Sys),
    (b"swprintf", Support::C99),
    (b"swscanf", Support::C99),
    (b"system", Support::C99),
    (b"tan", Support::C99),
    (b"tanf", Support::C99),
    (b"tanh", Support::C99),
    (b"tanhf", Support::C99),
    (b"tanhl", Support::C99),
    (b"tanl", Support::C99),
    (b"tgamma", Support::C99),
    (b"tgammaf", Support::C99),
    (b"tgammal", Support::C99),
    (b"time", Support::C99),
    (b"time_t", Support::Sys),
    (b"tmpfile", Support::C99),
    (b"tmpnam", Support::C99),
    (b"tolower", Support::C99),
    (b"toupper", Support::C99),
    (b"trunc", Support::C99),
    (b"truncf", Support::C99),
    (b"truncl", Support::C99),
    (b"u_char", Support::Sys),
    (b"u_int", Support::Sys),
    (b"u_long", Support::Sys),
    (b"u_quad_t", Support::Sys),
    (b"u_short", Support::Sys),
    (b"uid_t", Support::Sys),
    (b"uint", Support::Sys),
    (b"uint16_t", Support::Stdint),
    (b"uint32_t", Support::Stdint),
    (b"uint64_t", Support::Stdint),
    (b"uint8_t", Support::Stdint),
    (b"uint_fast16_t", Support::Stdint),
    (b"uint_fast32_t", Support::Stdint),
    (b"uint_fast64_t", Support::Stdint),
    (b"uint_fast8_t", Support::Stdint),
    (b"uint_least16_t", Support::Stdint),
    (b"uint_least32_t", Support::Stdint),
    (b"uint_least64_t", Support::Stdint),
    (b"uint_least8_t", Support::Stdint),
    (b"uintmax_t", Support::Stdint),
    (b"uintptr_t", Support::Stdint),
    (b"ungetc", Support::C99),
    (b"ungetwc", Support::C99),
    (b"useconds_t", Support::Sys),
    (b"ushort", Support::Sys),
    (b"va_arg", Support::C99),
    (b"va_copy", Support::C99),
    (b"va_end", Support::C99),
    (b"va_start", Support::C99),
    (b"vfprintf", Support::C99),
    (b"vfscanf", Support::C99),
    (b"vfwprintf", Support::C99),
    (b"vfwscanf", Support::C99),
    (b"vprintf", Support::C99),
    (b"vscanf", Support::C99),
    (b"vsnprintf", Support::C99),
    (b"vsprintf", Support::C99),
    (b"vsscanf", Support::C99),
    (b"vswprintf", Support::C99),
    (b"vswscanf", Support::C99),
    (b"vwprintf", Support::C99),
    (b"vwscanf", Support::C99),
    (b"wcrtomb", Support::C99),
    (b"wcscat", Support::C99),
    (b"wcschr", Support::C99),
    (b"wcscmp", Support::C99),
    (b"wcscoll", Support::C99),
    (b"wcscpy", Support::C99),
    (b"wcscspn", Support::C99),
    (b"wcsftime", Support::C99),
    (b"wcslen", Support::C99),
    (b"wcsncat", Support::C99),
    (b"wcsncmp", Support::C99),
    (b"wcsncpy", Support::C99),
    (b"wcspbrk", Support::C99),
    (b"wcsrchr", Support::C99),
    (b"wcsrtombs", Support::C99),
    (b"wcsspn", Support::C99),
    (b"wcsstr", Support::C99),
    (b"wcstod", Support::C99),
    (b"wcstof", Support::C99),
    (b"wcstoimax", Support::C99),
    (b"wcstok", Support::C99),
    (b"wcstol", Support::C99),
    (b"wcstold", Support::C99),
    (b"wcstoll", Support::C99),
    (b"wcstombs", Support::C99),
    (b"wcstoul", Support::C99),
    (b"wcstoull", Support::C99),
    (b"wcstoumax", Support::C99),
    (b"wcsxfrm", Support::C99),
    (b"wctob", Support::C99),
    (b"wctomb", Support::C99),
    (b"wmemchr", Support::C99),
    (b"wmemcmp", Support::C99),
    (b"wmemcpy", Support::C99),
    (b"wmemmove", Support::C99),
    (b"wmemset", Support::C99),
    (b"wprintf", Support::C99),
    (b"wscanf", Support::C99),
];
