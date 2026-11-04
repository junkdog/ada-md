//! The words C#'s patterns test, in one table on the C family's core ([`crate::clike::Words`],
//! X-015): a name is looked up once, and its groups ([`flags`]) answer what the grammar asks
//! with a list each (`{{reserved}}`, `{{base_type}}`, the LINQ words, `keywords`' words,
//! `line_of_code`'s statements, `method_declaration`'s modifiers). Every word is ASCII
//! lowercase, so a name that doesn't start with a lowercase letter isn't looked up.

use crate::clike;

/// `{{reserved}}`.
pub(super) const RES: u16 = 1;
/// `{{base_type}}`.
pub(super) const BASE: u16 = 2;
/// The LINQ words `line_of_code_in_no_semicolon` takes.
pub(super) const LINQ: u16 = 4;
/// `true`, `false`, `null`.
pub(super) const LANG: u16 = 8;
/// `{{visibility}}`'s words.
pub(super) const VIS: u16 = 16;
/// A word `keywords` acts on (each with its own conditions).
pub(super) const KWS: u16 = 32;
/// A word `line_of_code`'s statements start with.
pub(super) const STMT: u16 = 64;
/// `method_declaration`'s modifiers.
pub(super) const MODS: u16 = 128;

macro_rules! words {
    ($($id:ident = $w:literal, $f:expr;)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, clippy::enum_variant_names, dead_code)]
        #[repr(u8)]
        enum W { _None, $($id,)* Count }
        $(#[allow(dead_code)] pub(super) const $id: u8 = W::$id as u8;)*
        static LIST: [&[u8]; W::Count as usize - 1] = [$($w,)*];
        static FLAGS: [u16; W::Count as usize] = [0, $($f,)*];
    };
}

words! {
    W_ABSTRACT = b"abstract", RES | MODS;
    W_AS = b"as", RES | KWS;
    W_BASE = b"base", RES | KWS;
    W_BREAK = b"break", RES | STMT;
    W_CASE = b"case", RES | STMT;
    W_CATCH = b"catch", RES | STMT;
    W_CHECKED = b"checked", RES | KWS;
    W_CLASS = b"class", RES;
    W_CONST = b"const", RES | MODS;
    W_CONTINUE = b"continue", RES | STMT;
    W_DEFAULT = b"default", RES | KWS;
    W_DELEGATE = b"delegate", RES;
    W_DO = b"do", RES | STMT;
    W_ELSE = b"else", RES | STMT;
    W_ENUM = b"enum", RES;
    W_EVENT = b"event", RES;
    W_EXPLICIT = b"explicit", RES;
    W_EXTERN = b"extern", RES | MODS;
    W_FINALLY = b"finally", RES | STMT;
    W_FIXED = b"fixed", RES | STMT;
    W_FOR = b"for", RES | STMT;
    W_FOREACH = b"foreach", RES | STMT;
    W_GOTO = b"goto", RES | STMT;
    W_IF = b"if", RES | STMT;
    W_IMPLICIT = b"implicit", RES;
    W_IN = b"in", RES | LINQ;
    W_INTERFACE = b"interface", RES;
    W_INTERNAL = b"internal", RES | VIS | KWS;
    W_IS = b"is", RES | KWS;
    W_LOCK = b"lock", RES | STMT;
    W_NAMEOF = b"nameof", RES | KWS;
    W_NAMESPACE = b"namespace", RES;
    W_NEW = b"new", RES | KWS | MODS;
    W_NULL = b"null", RES | LANG;
    W_OPERATOR = b"operator", RES;
    W_OUT = b"out", RES;
    W_OVERRIDE = b"override", RES | MODS;
    W_PARAMS = b"params", RES;
    W_PRIVATE = b"private", RES | VIS | KWS;
    W_PROTECTED = b"protected", RES | VIS | KWS;
    W_PUBLIC = b"public", RES | VIS | KWS;
    W_READONLY = b"readonly", RES | MODS;
    W_REF = b"ref", RES | MODS;
    W_RETURN = b"return", RES | STMT;
    W_SEALED = b"sealed", RES | MODS;
    W_SIZEOF = b"sizeof", RES;
    W_STACKALLOC = b"stackalloc", RES;
    W_STATIC = b"static", RES | MODS;
    W_STRING = b"string", RES | BASE;
    W_STRUCT = b"struct", RES;
    W_SWITCH = b"switch", RES | STMT;
    W_THIS = b"this", RES | KWS;
    W_THROW = b"throw", RES | STMT;
    W_TRY = b"try", RES | STMT;
    W_TYPEOF = b"typeof", RES | KWS;
    W_UNCHECKED = b"unchecked", RES | KWS;
    W_UNSAFE = b"unsafe", RES | KWS | MODS;
    W_USING = b"using", RES | STMT;
    W_VIRTUAL = b"virtual", RES | MODS;
    W_VOLATILE = b"volatile", RES | MODS;
    W_WHILE = b"while", RES | STMT;
    W_BOOL = b"bool", BASE;
    W_BYTE = b"byte", BASE;
    W_SBYTE = b"sbyte", BASE;
    W_CHAR = b"char", BASE;
    W_DECIMAL = b"decimal", BASE;
    W_DOUBLE = b"double", BASE;
    W_FLOAT = b"float", BASE;
    W_INT = b"int", BASE;
    W_UINT = b"uint", BASE;
    W_LONG = b"long", BASE;
    W_ULONG = b"ulong", BASE;
    W_SHORT = b"short", BASE;
    W_USHORT = b"ushort", BASE;
    W_OBJECT = b"object", BASE;
    W_VOID = b"void", BASE;
    W_LET = b"let", LINQ;
    W_SELECT = b"select", LINQ;
    W_EQUALS = b"equals", LINQ;
    W_FROM = b"from", LINQ;
    W_WHERE = b"where", LINQ;
    W_JOIN = b"join", LINQ;
    W_ORDERBY = b"orderby", LINQ;
    W_GROUP = b"group", LINQ;
    W_BY = b"by", LINQ;
    W_ON = b"on", LINQ;
    W_INTO = b"into", LINQ;
    W_ASCENDING = b"ascending", LINQ;
    W_DESCENDING = b"descending", LINQ;
    W_TRUE = b"true", LANG;
    W_FALSE = b"false", LANG;
    W_AWAIT = b"await", KWS;
    W_YIELD = b"yield", STMT;
    W_ASYNC = b"async", MODS;
    W_VALUE = b"value", 0;
    W_VAR = b"var", 0;
    W_DYNAMIC = b"dynamic", 0;
    W_GLOBAL = b"global", 0;
    W_PARTIAL = b"partial", 0;
    W_GET = b"get", 0;
    W_SET = b"set", 0;
    W_ADD = b"add", 0;
    W_REMOVE = b"remove", 0;
    W_WHEN = b"when", 0;
}

static TABLE: clike::Words<256> = clike::Words::new(&LIST);

/// Word `w`'s id, 0 for none.
#[inline(always)]
pub(super) fn get(w: &[u8]) -> u8 {
    TABLE.get(w)
}

/// The groups of the word with id `id` (0 for none: no group).
#[inline(always)]
pub(super) fn flags(id: u8) -> u16 {
    FLAGS[usize::from(id)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csharp::re;

    #[test]
    fn the_groups_are_the_grammars_lists() {
        for w in re::RESERVED {
            assert!(flags(get(w)) & RES != 0, "{}", String::from_utf8_lossy(w));
        }
        for w in re::BASE {
            assert!(flags(get(w)) & BASE != 0, "{}", String::from_utf8_lossy(w));
        }
        for w in crate::csharp::ctx::SQL_WORDS {
            assert!(flags(get(w)) & LINQ != 0, "{}", String::from_utf8_lossy(w));
        }
        for (i, w) in LIST.iter().enumerate() {
            let f = flags(i as u8 + 1);
            assert_eq!(get(w), i as u8 + 1);
            assert_eq!(
                f & RES != 0,
                re::is_reserved(w),
                "{}",
                String::from_utf8_lossy(w)
            );
            assert_eq!(
                f & BASE != 0,
                re::BASE.contains(w),
                "{}",
                String::from_utf8_lossy(w)
            );
            assert_eq!(
                f & LINQ != 0,
                crate::csharp::ctx::SQL_WORDS.contains(w),
                "{}",
                String::from_utf8_lossy(w)
            );
            assert!(w[0].is_ascii_lowercase());
        }
        for w in [&b"Value"[..], b"values", b"i", b"", b"intx"] {
            assert_eq!(get(w), 0);
        }
    }
}
