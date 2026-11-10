//! C#'s words in one table (X-122): a name is read once and looked up once, its row the leaf it
//! takes where it is a keyword and an action for the words whose meaning hangs on where they
//! stand (a type's, a modifier, a statement's, a contextual keyword).

use super::*;
use crate::words::{Words, text_len};

// A word's action ([`INFO`]'s high byte); its leaf is the low byte.
/// A keyword with nothing to do but its leaf: the token before it an operand's.
pub(super) const A_VALUE: u8 = 0;
/// A type's keyword (`int`, `string`…): a type where one can stand.
pub(super) const A_TYPE: u8 = 1;
/// `var`, `dynamic`: a type where a declaration's can stand, else a name.
pub(super) const A_VAR: u8 = 2;
/// A modifier that is always one (`public`, `static`…).
pub(super) const A_MOD: u8 = 3;
/// A contextual modifier (`async`, `partial`, `required`, `file`, `scoped`): one before a
/// declaration's type or another modifier, else a name.
pub(super) const A_CMOD: u8 = 4;
/// `class`, `struct`, `interface`, `enum`, `namespace`, `delegate`, `event`: an entity's word.
pub(super) const A_ENTITY: u8 = 5;
/// `record`: an entity's word before a name, `class` or `struct`, else a name.
pub(super) const A_RECORD: u8 = 6;
/// `if`, `while`, `switch`, `lock`, `fixed`, `catch`, `foreach`: a statement whose `(` is its
/// head ([`F_COND`]).
pub(super) const A_COND: u8 = 7;
/// `for`: its `(` holds statements ([`F_FOR`]).
pub(super) const A_FOR: u8 = 8;
/// `else`, `do`, `try`, `finally`, `checked`, `unchecked`, `unsafe`: a statement follows.
pub(super) const A_BLOCK: u8 = 9;
/// `return`, `throw`: an expression follows.
pub(super) const A_EXPR: u8 = 10;
/// `break`, `continue`.
pub(super) const A_JUMP: u8 = 11;
/// `goto`: a label follows.
pub(super) const A_GOTO: u8 = 12;
/// `yield`: `yield return`, `yield break` one token, as bat's.
pub(super) const A_YIELD: u8 = 13;
/// `case`: a pattern to its `:`.
pub(super) const A_CASE: u8 = 14;
/// `default`: a `case`'s word before `:`, `default(T)` a type's, else a value.
pub(super) const A_DEFAULT: u8 = 15;
/// `when`: a `case`'s or a `catch`'s condition.
pub(super) const A_WHEN: u8 = 16;
/// `this`, `base`: a value; `this[` an indexer's declaration at a type's level.
pub(super) const A_THIS: u8 = 17;
/// `new`: an object's creation, or a modifier at a declaration's start.
pub(super) const A_NEW: u8 = 18;
/// `is`, `as`: a type or a pattern follows.
pub(super) const A_IS: u8 = 19;
/// `typeof`, `sizeof`: a type in parentheses follows.
pub(super) const A_TYPEOF: u8 = 20;
/// `get`, `set`, `init`, `add`, `remove`: an accessor's in an accessor list, else a name.
pub(super) const A_ACCESSOR: u8 = 22;
/// `where`: a constraint's in a declaration's head, a query's clause in a query, else a name.
pub(super) const A_WHERE: u8 = 23;
/// A query's word (`from`, `select`…): a keyword in a query, else a name.
pub(super) const A_QUERY: u8 = 24;
/// `in`, `out`, `ref`, `params`: a parameter's or an argument's modifier, `in` a `foreach`'s.
pub(super) const A_PARAM: u8 = 25;
/// `operator`: an operator's declaration.
pub(super) const A_OPERATOR: u8 = 26;
/// `and`, `or`, `not`: a pattern's combinators, in a pattern, else names.
pub(super) const A_PATTERN: u8 = 27;
/// `with`: a record's copy, `x with { … }`, after an operand, else a name.
pub(super) const A_WITH: u8 = 28;
/// `using`: a directive at a namespace's level, a statement in code.
pub(super) const A_USING: u8 = 29;
/// `global`: `global using`, `global::`.
pub(super) const A_GLOBAL: u8 = 30;
/// `value`: the setter's value, as bat's (in any code).
pub(super) const A_NAMEVALUE: u8 = 31;
/// `true`, `false`, `null`.
pub(super) const A_LITERAL: u8 = 32;
/// `delegate`: a delegate type's declaration, or an anonymous method in an expression.
pub(super) const A_DELEGATE: u8 = 33;
/// `static`: a modifier, `using static`'s word.
pub(super) const A_STATIC: u8 = 34;
/// `nameof`: its operand an expression's, a type's path where it starts upper-case (bat's).
pub(super) const A_NAMEOF: u8 = 35;
/// `checked`, `unchecked`: a block's word, or an expression's before `(`.
pub(super) const A_CHECKED: u8 = 36;
/// `stackalloc`: a type follows.
pub(super) const A_STACKALLOC: u8 = 37;
/// `await`: an expression follows; a name where one is declared.
pub(super) const A_AWAIT: u8 = 38;
/// `implicit`, `explicit`: a conversion's modifier.
pub(super) const A_CONVERSION: u8 = 39;
/// `unmanaged`, `notnull`, `managed`: a constraint's word in a `where`, else a name.
pub(super) const A_CONSTRAINT: u8 = 40;

/// What an entity's word names ([`Scan::ent`]).
pub(super) const E_CLASS: u8 = 1;
pub(super) const E_STRUCT: u8 = 2;
pub(super) const E_INTERFACE: u8 = 3;
pub(super) const E_ENUM: u8 = 4;
pub(super) const E_NAMESPACE: u8 = 5;
pub(super) const E_DELEGATE: u8 = 6;
pub(super) const E_EVENT: u8 = 7;
pub(super) const E_RECORD: u8 = 8;

/// The words, each with its leaf and its action.
const WORD_LIST: &[(&[u8], u16, u8)] = &[
    // types
    (b"bool", STORAGE_TYPE, A_TYPE),
    (b"byte", STORAGE_TYPE, A_TYPE),
    (b"sbyte", STORAGE_TYPE, A_TYPE),
    (b"char", STORAGE_TYPE, A_TYPE),
    (b"decimal", STORAGE_TYPE, A_TYPE),
    (b"double", STORAGE_TYPE, A_TYPE),
    (b"float", STORAGE_TYPE, A_TYPE),
    (b"int", STORAGE_TYPE, A_TYPE),
    (b"uint", STORAGE_TYPE, A_TYPE),
    (b"long", STORAGE_TYPE, A_TYPE),
    (b"ulong", STORAGE_TYPE, A_TYPE),
    (b"short", STORAGE_TYPE, A_TYPE),
    (b"ushort", STORAGE_TYPE, A_TYPE),
    (b"object", STORAGE_TYPE, A_TYPE),
    (b"string", STORAGE_TYPE, A_TYPE),
    (b"void", STORAGE_TYPE, A_TYPE),
    // C# 9's native integers, keywords as `int` is (bat's names them as types)
    (b"nint", STORAGE_TYPE, A_TYPE),
    (b"nuint", STORAGE_TYPE, A_TYPE),
    (b"var", VAR_KW, A_VAR),
    (b"dynamic", VAR_KW, A_VAR),
    // modifiers
    (b"public", ACCESS, A_MOD),
    (b"private", ACCESS, A_MOD),
    (b"protected", ACCESS, A_MOD),
    (b"internal", ACCESS, A_MOD),
    (b"abstract", MODIFIER, A_MOD),
    (b"const", MODIFIER, A_MOD),
    (b"extern", MODIFIER, A_MOD),
    (b"override", MODIFIER, A_MOD),
    (b"readonly", MODIFIER, A_MOD),
    (b"sealed", MODIFIER, A_MOD),
    (b"static", MODIFIER, A_STATIC),
    (b"unsafe", MODIFIER, A_MOD),
    (b"virtual", MODIFIER, A_MOD),
    (b"volatile", MODIFIER, A_MOD),
    (b"async", MODIFIER, A_CMOD),
    (b"partial", MODIFIER, A_CMOD),
    (b"required", MODIFIER, A_CMOD),
    (b"file", MODIFIER, A_CMOD),
    (b"scoped", MODIFIER, A_CMOD),
    (b"implicit", MODIFIER, A_CONVERSION),
    (b"explicit", MODIFIER, A_CONVERSION),
    (b"event", MODIFIER, A_ENTITY),
    // entities
    (b"class", CLASS_KW, A_ENTITY),
    (b"struct", STRUCT_KW, A_ENTITY),
    (b"interface", INTERFACE_KW, A_ENTITY),
    (b"enum", ENUM_KW, A_ENTITY),
    (b"namespace", NAMESPACE_KW, A_ENTITY),
    (b"delegate", DELEGATE_KW, A_DELEGATE),
    (b"record", CLASS_KW, A_RECORD),
    // statements
    (b"if", IF_KW, A_COND),
    (b"while", WHILE_KW, A_COND),
    (b"switch", SWITCH_KW, A_COND),
    (b"lock", LOCK_KW, A_COND),
    (b"fixed", FIXED_KW, A_COND),
    (b"catch", CATCH_KW, A_COND),
    (b"foreach", FOREACH_KW, A_COND),
    (b"for", FOR_KW, A_FOR),
    (b"else", ELSE_KW, A_BLOCK),
    (b"do", DO_KW, A_BLOCK),
    (b"try", TRY_KW, A_BLOCK),
    (b"finally", FINALLY_KW, A_BLOCK),
    (b"checked", KW_OTHER, A_CHECKED),
    (b"unchecked", KW_OTHER, A_CHECKED),
    (b"return", RETURN_KW, A_EXPR),
    (b"throw", THROW_KW, A_EXPR),
    (b"break", BREAK_KW, A_JUMP),
    (b"continue", BREAK_KW, A_JUMP),
    (b"goto", GOTO_KW, A_GOTO),
    (b"yield", RETURN_KW, A_YIELD),
    (b"case", CASE_KW, A_CASE),
    (b"default", KW_OTHER, A_DEFAULT),
    (b"when", WHEN_KW, A_WHEN),
    (b"using", USING_KW, A_USING),
    // expressions
    (b"this", VAR_LANGUAGE, A_THIS),
    (b"base", VAR_LANGUAGE, A_THIS),
    (b"value", VAR_LANGUAGE, A_NAMEVALUE),
    (b"new", NEW_OP, A_NEW),
    (b"is", REFLECTION, A_IS),
    (b"as", REFLECTION, A_IS),
    (b"typeof", REFLECTION, A_TYPEOF),
    (b"sizeof", KW_OTHER, A_TYPEOF),
    (b"nameof", KW_OTHER, A_NAMEOF),
    (b"await", AWAIT, A_AWAIT),
    (b"stackalloc", KW_OTHER, A_STACKALLOC),
    (b"true", LANGUAGE, A_LITERAL),
    (b"false", LANGUAGE, A_LITERAL),
    (b"null", LANGUAGE, A_LITERAL),
    (b"with", KW_OTHER, A_WITH),
    (b"and", LOGICAL, A_PATTERN),
    (b"or", LOGICAL, A_PATTERN),
    (b"not", LOGICAL, A_PATTERN),
    // parameters
    (b"in", PARAM_MOD, A_PARAM),
    (b"out", PARAM_MOD, A_PARAM),
    (b"ref", PARAM_MOD, A_PARAM),
    (b"params", PARAM_MOD, A_PARAM),
    (b"operator", MOD_OTHER, A_OPERATOR),
    // accessors
    (b"get", ACC_GET, A_ACCESSOR),
    (b"set", ACC_SET, A_ACCESSOR),
    (b"init", ACC_INIT, A_ACCESSOR),
    (b"add", ACC_ADD, A_ACCESSOR),
    (b"remove", ACC_REMOVE, A_ACCESSOR),
    // constraints
    (b"where", MODIFIER, A_WHERE),
    (b"unmanaged", STORAGE_TYPE, A_CONSTRAINT),
    (b"notnull", STORAGE_TYPE, A_CONSTRAINT),
    (b"managed", STORAGE_TYPE, A_CONSTRAINT),
    (b"global", KW_OTHER, A_GLOBAL),
    // queries
    (b"from", SQL, A_QUERY),
    (b"select", SQL, A_QUERY),
    (b"let", SQL, A_QUERY),
    (b"join", SQL, A_QUERY),
    (b"on", SQL, A_QUERY),
    (b"equals", SQL, A_QUERY),
    (b"orderby", SQL, A_QUERY),
    (b"ascending", SQL, A_QUERY),
    (b"descending", SQL, A_QUERY),
    (b"group", SQL, A_QUERY),
    (b"by", SQL, A_QUERY),
    (b"into", SQL, A_QUERY),
];

const N: usize = WORD_LIST.len();
static WORD_TEXT: [&[u8]; N] = {
    let mut t: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        t[i] = WORD_LIST[i].0;
        i += 1;
    }
    t
};
static WORDS: Words<256, N, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);

/// A word's row by its id: its leaf in the low byte, its action in the high one; 0 for a name.
static INFO: [u16; N + 1] = {
    let mut t = [0u16; N + 1];
    let mut i = 0;
    while i < N {
        assert!(WORD_LIST[i].1 < 256);
        t[i + 1] = WORD_LIST[i].1 | (WORD_LIST[i].2 as u16) << 8;
        i += 1;
    }
    t
};

/// What word `w` is ([`INFO`]'s row), 0 for a name.
#[inline(always)]
pub(super) fn info(w: &[u8]) -> u16 {
    WORDS.value(w, &INFO)
}

/// What an entity's word names, by its leaf (`event`'s is a modifier's).
pub(super) fn entity(leaf: u16) -> u8 {
    match leaf {
        CLASS_KW => E_CLASS,
        STRUCT_KW => E_STRUCT,
        INTERFACE_KW => E_INTERFACE,
        ENUM_KW => E_ENUM,
        NAMESPACE_KW => E_NAMESPACE,
        DELEGATE_KW => E_DELEGATE,
        _ => E_EVENT,
    }
}

/// Whether a word (its [`INFO`] row) can't be the name a declaration declares: a reserved word
/// or an operator's (`is`, `as`, `when`, `and`…), so `x is y` and `T when` aren't declarations.
#[inline(always)]
pub(super) fn not_a_name(info: u16) -> bool {
    info != 0
        && !matches!(
            (info >> 8) as u8,
            A_VAR
                | A_CMOD
                | A_RECORD
                | A_ACCESSOR
                | A_QUERY
                | A_CONSTRAINT
                | A_NAMEVALUE
                | A_GLOBAL
                | A_AWAIT
                | A_WITH
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_is_found_with_its_row() {
        for (i, (w, leaf, act)) in WORD_LIST.iter().enumerate() {
            assert_eq!(WORDS.get(w), i as u16 + 1, "{}", String::from_utf8_lossy(w));
            let r = info(w);
            assert_eq!(r & 0xff, *leaf);
            assert_eq!((r >> 8) as u8, *act);
        }
        for w in [&b"Value"[..], b"values", b"i", b"intx", b"Int"] {
            assert_eq!(info(w), 0);
        }
    }
}
