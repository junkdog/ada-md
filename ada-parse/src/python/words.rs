//! The words of bat's Python grammar that its patterns tell apart: keywords, and the name
//! lists `name-specials` tries. One table, hashed at compile time, gives a word's keyword and
//! its name class in one lookup.

/// Which of the grammar's name lists a whole word is in, the first in the order
/// `name-specials` tries them (`builtin-functions`, `builtin-types`, `builtin-exceptions`,
/// `illegal-names`, `magic-function-names`, `magic-variable-names`, `language-variables`):
/// the index of a leaf in a group of names (`super::L::Name`). The lists past the first that
/// holds a word don't hold it, so the class answers each list's own question too.
pub(super) const GENERIC: u16 = 0;
pub(super) const BUILTIN_FUNCTION: u16 = 1;
pub(super) const BUILTIN_TYPE: u16 = 2;
pub(super) const EXCEPTION: u16 = 3;
pub(super) const ILLEGAL: u16 = 4;
pub(super) const MAGIC_FUNCTION: u16 = 5;
pub(super) const MAGIC_VARIABLE: u16 = 6;
pub(super) const LANGUAGE: u16 = 7;

// The keywords the contexts' patterns match by name.
pub(super) const K_NONE: u8 = 0;
pub(super) const K_IMPORT: u8 = 1;
pub(super) const K_FROM: u8 = 2;
pub(super) const K_RAISE: u8 = 3;
pub(super) const K_ASSERT: u8 = 4;
pub(super) const K_DEL: u8 = 5;
pub(super) const K_PRINT: u8 = 6;
pub(super) const K_EXEC: u8 = 7;
pub(super) const K_RETURN: u8 = 8;
pub(super) const K_BREAK: u8 = 9;
pub(super) const K_CONTINUE: u8 = 10;
pub(super) const K_PASS: u8 = 11;
pub(super) const K_ASYNC: u8 = 12;
pub(super) const K_FOR: u8 = 13;
pub(super) const K_WITH: u8 = 14;
pub(super) const K_EXCEPT: u8 = 15;
pub(super) const K_IF: u8 = 16;
pub(super) const K_WHILE: u8 = 17;
pub(super) const K_ELSE: u8 = 18;
pub(super) const K_TRY: u8 = 19;
pub(super) const K_FINALLY: u8 = 20;
pub(super) const K_ELIF: u8 = 21;
pub(super) const K_CLASS: u8 = 22;
pub(super) const K_DEF: u8 = 23;
pub(super) const K_GLOBAL: u8 = 24;
pub(super) const K_NONLOCAL: u8 = 25;
pub(super) const K_LAMBDA: u8 = 26;
pub(super) const K_AWAIT: u8 = 27;
pub(super) const K_YIELD: u8 = 28;
/// `and`, `is`, `not`, `or`: `keyword.operator.logical` (`in` too, with a name of its own).
pub(super) const K_LOGICAL: u8 = 29;
pub(super) const K_IN: u8 = 30;
pub(super) const K_AS: u8 = 31;
/// `constants`: `None`, `True`, `False`, `Ellipsis`, `NotImplemented`, `__debug__`.
pub(super) const K_CONSTANT: u8 = 32;

/// Every word the grammar tells apart, its keyword and its name class, sorted.
const ENTRIES: &[(&[u8], u8, u16)] = &[
    (b"ArithmeticError", K_NONE, EXCEPTION),
    (b"AssertionError", K_NONE, EXCEPTION),
    (b"AttributeError", K_NONE, EXCEPTION),
    (b"BaseException", K_NONE, EXCEPTION),
    (b"BlockingIOError", K_NONE, EXCEPTION),
    (b"BrokenPipeError", K_NONE, EXCEPTION),
    (b"BufferError", K_NONE, EXCEPTION),
    (b"BytesWarning", K_NONE, EXCEPTION),
    (b"ChildProcessError", K_NONE, EXCEPTION),
    (b"ConnectionAbortedError", K_NONE, EXCEPTION),
    (b"ConnectionError", K_NONE, EXCEPTION),
    (b"ConnectionRefusedError", K_NONE, EXCEPTION),
    (b"ConnectionResetError", K_NONE, EXCEPTION),
    (b"DeprecationWarning", K_NONE, EXCEPTION),
    (b"EOFError", K_NONE, EXCEPTION),
    (b"Ellipsis", K_CONSTANT, GENERIC),
    (b"EnvironmentError", K_NONE, EXCEPTION),
    (b"Exception", K_NONE, EXCEPTION),
    (b"False", K_CONSTANT, GENERIC),
    (b"FileExistsError", K_NONE, EXCEPTION),
    (b"FileNotFoundError", K_NONE, EXCEPTION),
    (b"FloatingPointError", K_NONE, EXCEPTION),
    (b"FutureWarning", K_NONE, EXCEPTION),
    (b"GeneratorExit", K_NONE, EXCEPTION),
    (b"IOError", K_NONE, EXCEPTION),
    (b"ImportError", K_NONE, EXCEPTION),
    (b"ImportWarning", K_NONE, EXCEPTION),
    (b"IndentationError", K_NONE, EXCEPTION),
    (b"IndexError", K_NONE, EXCEPTION),
    (b"InterruptedError", K_NONE, EXCEPTION),
    (b"IsADirectoryErrorError", K_NONE, EXCEPTION),
    (b"KeyError", K_NONE, EXCEPTION),
    (b"KeyboardInterrupt", K_NONE, EXCEPTION),
    (b"LookupError", K_NONE, EXCEPTION),
    (b"MemoryError", K_NONE, EXCEPTION),
    (b"NameError", K_NONE, EXCEPTION),
    (b"None", K_CONSTANT, GENERIC),
    (b"NotADirectoryError", K_NONE, EXCEPTION),
    (b"NotImplemented", K_CONSTANT, EXCEPTION),
    (b"NotImplementedError", K_NONE, EXCEPTION),
    (b"OSError", K_NONE, EXCEPTION),
    (b"OverflowError", K_NONE, EXCEPTION),
    (b"PendingDeprecationWarning", K_NONE, EXCEPTION),
    (b"PermissionError", K_NONE, EXCEPTION),
    (b"ProcessLookupError", K_NONE, EXCEPTION),
    (b"ReferenceError", K_NONE, EXCEPTION),
    (b"RuntimeError", K_NONE, EXCEPTION),
    (b"RuntimeWarning", K_NONE, EXCEPTION),
    (b"StandardError", K_NONE, EXCEPTION),
    (b"StopIteration", K_NONE, EXCEPTION),
    (b"SyntaxError", K_NONE, EXCEPTION),
    (b"SyntaxWarning", K_NONE, EXCEPTION),
    (b"SystemError", K_NONE, EXCEPTION),
    (b"SystemExit", K_NONE, EXCEPTION),
    (b"TabError", K_NONE, EXCEPTION),
    (b"TimeoutError", K_NONE, EXCEPTION),
    (b"True", K_CONSTANT, GENERIC),
    (b"TypeError", K_NONE, EXCEPTION),
    (b"UnboundLocalError", K_NONE, EXCEPTION),
    (b"UnicodeDecodeError", K_NONE, EXCEPTION),
    (b"UnicodeEncodeError", K_NONE, EXCEPTION),
    (b"UnicodeError", K_NONE, EXCEPTION),
    (b"UnicodeTranslateError", K_NONE, EXCEPTION),
    (b"UnicodeWarning", K_NONE, EXCEPTION),
    (b"UserWarning", K_NONE, EXCEPTION),
    (b"VMSError", K_NONE, EXCEPTION),
    (b"ValueError", K_NONE, EXCEPTION),
    (b"Warning", K_NONE, EXCEPTION),
    (b"WindowsError", K_NONE, EXCEPTION),
    (b"ZeroDivisionError", K_NONE, EXCEPTION),
    (b"__abs__", K_NONE, MAGIC_FUNCTION),
    (b"__add__", K_NONE, MAGIC_FUNCTION),
    (b"__aenter__", K_NONE, MAGIC_FUNCTION),
    (b"__aexit__", K_NONE, MAGIC_FUNCTION),
    (b"__aiter__", K_NONE, MAGIC_FUNCTION),
    (b"__all__", K_NONE, MAGIC_VARIABLE),
    (b"__and__", K_NONE, MAGIC_FUNCTION),
    (b"__anext__", K_NONE, MAGIC_FUNCTION),
    (b"__annotations__", K_NONE, MAGIC_VARIABLE),
    (b"__await__", K_NONE, MAGIC_FUNCTION),
    (b"__bases__", K_NONE, MAGIC_VARIABLE),
    (b"__bool__", K_NONE, MAGIC_FUNCTION),
    (b"__bytes__", K_NONE, MAGIC_FUNCTION),
    (b"__call__", K_NONE, MAGIC_FUNCTION),
    (b"__class__", K_NONE, MAGIC_VARIABLE),
    (b"__class_getitem__", K_NONE, MAGIC_FUNCTION),
    (b"__closure__", K_NONE, MAGIC_VARIABLE),
    (b"__cmp__", K_NONE, MAGIC_FUNCTION),
    (b"__code__", K_NONE, MAGIC_VARIABLE),
    (b"__coerce__", K_NONE, MAGIC_FUNCTION),
    (b"__complex__", K_NONE, MAGIC_FUNCTION),
    (b"__contains__", K_NONE, MAGIC_FUNCTION),
    (b"__debug__", K_CONSTANT, GENERIC),
    (b"__defaults__", K_NONE, MAGIC_VARIABLE),
    (b"__del__", K_NONE, MAGIC_FUNCTION),
    (b"__delattr__", K_NONE, MAGIC_FUNCTION),
    (b"__delete__", K_NONE, MAGIC_FUNCTION),
    (b"__delitem__", K_NONE, MAGIC_FUNCTION),
    (b"__delslice__", K_NONE, MAGIC_FUNCTION),
    (b"__dict__", K_NONE, MAGIC_VARIABLE),
    (b"__dir__", K_NONE, MAGIC_FUNCTION),
    (b"__div__", K_NONE, MAGIC_FUNCTION),
    (b"__divmod__", K_NONE, MAGIC_FUNCTION),
    (b"__doc__", K_NONE, MAGIC_VARIABLE),
    (b"__enter__", K_NONE, MAGIC_FUNCTION),
    (b"__eq__", K_NONE, MAGIC_FUNCTION),
    (b"__exit__", K_NONE, MAGIC_FUNCTION),
    (b"__file__", K_NONE, MAGIC_VARIABLE),
    (b"__float__", K_NONE, MAGIC_FUNCTION),
    (b"__floordiv__", K_NONE, MAGIC_FUNCTION),
    (b"__format__", K_NONE, MAGIC_FUNCTION),
    (b"__fspath__", K_NONE, MAGIC_FUNCTION),
    (b"__func__", K_NONE, MAGIC_VARIABLE),
    (b"__ge__", K_NONE, MAGIC_FUNCTION),
    (b"__get__", K_NONE, MAGIC_FUNCTION),
    (b"__getattr__", K_NONE, MAGIC_FUNCTION),
    (b"__getattribute__", K_NONE, MAGIC_FUNCTION),
    (b"__getitem__", K_NONE, MAGIC_FUNCTION),
    (b"__getnewargs__", K_NONE, MAGIC_FUNCTION),
    (b"__getnewargs_ex__", K_NONE, MAGIC_FUNCTION),
    (b"__getslice__", K_NONE, MAGIC_FUNCTION),
    (b"__getstate__", K_NONE, MAGIC_FUNCTION),
    (b"__globals__", K_NONE, MAGIC_VARIABLE),
    (b"__gt__", K_NONE, MAGIC_FUNCTION),
    (b"__hash__", K_NONE, MAGIC_FUNCTION),
    (b"__hex__", K_NONE, MAGIC_FUNCTION),
    (b"__iadd__", K_NONE, MAGIC_FUNCTION),
    (b"__iand__", K_NONE, MAGIC_FUNCTION),
    (b"__idiv__", K_NONE, MAGIC_FUNCTION),
    (b"__ifloordiv__", K_NONE, MAGIC_FUNCTION),
    (b"__ilshift__", K_NONE, MAGIC_FUNCTION),
    (b"__imod__", K_NONE, MAGIC_FUNCTION),
    (b"__import__", K_NONE, BUILTIN_FUNCTION),
    (b"__imul__", K_NONE, MAGIC_FUNCTION),
    (b"__index__", K_NONE, MAGIC_FUNCTION),
    (b"__init__", K_NONE, MAGIC_FUNCTION),
    (b"__init_subclass__", K_NONE, MAGIC_FUNCTION),
    (b"__instancecheck__", K_NONE, MAGIC_FUNCTION),
    (b"__int__", K_NONE, MAGIC_FUNCTION),
    (b"__invert__", K_NONE, MAGIC_FUNCTION),
    (b"__ior__", K_NONE, MAGIC_FUNCTION),
    (b"__ipow__", K_NONE, MAGIC_FUNCTION),
    (b"__irshift__", K_NONE, MAGIC_FUNCTION),
    (b"__isub__", K_NONE, MAGIC_FUNCTION),
    (b"__iter__", K_NONE, MAGIC_FUNCTION),
    (b"__itruediv__", K_NONE, MAGIC_FUNCTION),
    (b"__ixor__", K_NONE, MAGIC_FUNCTION),
    (b"__kwdefaults__", K_NONE, MAGIC_VARIABLE),
    (b"__le__", K_NONE, MAGIC_FUNCTION),
    (b"__len__", K_NONE, MAGIC_FUNCTION),
    (b"__length_hint__", K_NONE, MAGIC_FUNCTION),
    (b"__long__", K_NONE, MAGIC_FUNCTION),
    (b"__lshift__", K_NONE, MAGIC_FUNCTION),
    (b"__lt__", K_NONE, MAGIC_FUNCTION),
    (b"__members__", K_NONE, MAGIC_VARIABLE),
    (b"__metaclass__", K_NONE, MAGIC_VARIABLE),
    (b"__methods__", K_NONE, MAGIC_VARIABLE),
    (b"__missing__", K_NONE, MAGIC_FUNCTION),
    (b"__mod__", K_NONE, MAGIC_FUNCTION),
    (b"__module__", K_NONE, MAGIC_VARIABLE),
    (b"__mro__", K_NONE, MAGIC_VARIABLE),
    (b"__mro_entries__", K_NONE, MAGIC_FUNCTION),
    (b"__mul__", K_NONE, MAGIC_FUNCTION),
    (b"__name__", K_NONE, MAGIC_VARIABLE),
    (b"__ne__", K_NONE, MAGIC_FUNCTION),
    (b"__neg__", K_NONE, MAGIC_FUNCTION),
    (b"__new__", K_NONE, MAGIC_FUNCTION),
    (b"__next__", K_NONE, MAGIC_FUNCTION),
    (b"__nonzero__", K_NONE, MAGIC_FUNCTION),
    (b"__oct__", K_NONE, MAGIC_FUNCTION),
    (b"__or__", K_NONE, MAGIC_FUNCTION),
    (b"__package__", K_NONE, MAGIC_VARIABLE),
    (b"__pos__", K_NONE, MAGIC_FUNCTION),
    (b"__post_init__", K_NONE, MAGIC_FUNCTION),
    (b"__pow__", K_NONE, MAGIC_FUNCTION),
    (b"__prepare__", K_NONE, MAGIC_VARIABLE),
    (b"__qualname__", K_NONE, MAGIC_VARIABLE),
    (b"__radd__", K_NONE, MAGIC_FUNCTION),
    (b"__rand__", K_NONE, MAGIC_FUNCTION),
    (b"__rcmp__", K_NONE, MAGIC_FUNCTION),
    (b"__rdiv__", K_NONE, MAGIC_FUNCTION),
    (b"__rdivmod__", K_NONE, MAGIC_FUNCTION),
    (b"__reduce__", K_NONE, MAGIC_FUNCTION),
    (b"__reduce_ex__", K_NONE, MAGIC_FUNCTION),
    (b"__repr__", K_NONE, MAGIC_FUNCTION),
    (b"__reversed__", K_NONE, MAGIC_FUNCTION),
    (b"__rfloordiv__", K_NONE, MAGIC_FUNCTION),
    (b"__rlshift__", K_NONE, MAGIC_FUNCTION),
    (b"__rmod__", K_NONE, MAGIC_FUNCTION),
    (b"__rmul__", K_NONE, MAGIC_FUNCTION),
    (b"__ror__", K_NONE, MAGIC_FUNCTION),
    (b"__round__", K_NONE, MAGIC_FUNCTION),
    (b"__rpow__", K_NONE, MAGIC_FUNCTION),
    (b"__rrshift__", K_NONE, MAGIC_FUNCTION),
    (b"__rshift__", K_NONE, MAGIC_FUNCTION),
    (b"__rsub__", K_NONE, MAGIC_FUNCTION),
    (b"__rtruediv__", K_NONE, MAGIC_FUNCTION),
    (b"__rxor__", K_NONE, MAGIC_FUNCTION),
    (b"__self__", K_NONE, MAGIC_VARIABLE),
    (b"__set__", K_NONE, MAGIC_FUNCTION),
    (b"__set_name__", K_NONE, MAGIC_FUNCTION),
    (b"__setattr__", K_NONE, MAGIC_FUNCTION),
    (b"__setitem__", K_NONE, MAGIC_FUNCTION),
    (b"__setslice__", K_NONE, MAGIC_FUNCTION),
    (b"__setstate__", K_NONE, MAGIC_FUNCTION),
    (b"__slots__", K_NONE, MAGIC_VARIABLE),
    (b"__str__", K_NONE, MAGIC_FUNCTION),
    (b"__sub__", K_NONE, MAGIC_FUNCTION),
    (b"__subclasscheck__", K_NONE, MAGIC_FUNCTION),
    (b"__subclasses__", K_NONE, MAGIC_FUNCTION),
    (b"__truediv__", K_NONE, MAGIC_FUNCTION),
    (b"__unicode__", K_NONE, MAGIC_FUNCTION),
    (b"__xor__", K_NONE, MAGIC_FUNCTION),
    (b"abs", K_NONE, BUILTIN_FUNCTION),
    (b"all", K_NONE, BUILTIN_FUNCTION),
    (b"and", K_LOGICAL, ILLEGAL),
    (b"any", K_NONE, BUILTIN_FUNCTION),
    (b"apply", K_NONE, BUILTIN_FUNCTION),
    (b"as", K_AS, ILLEGAL),
    (b"ascii", K_NONE, BUILTIN_FUNCTION),
    (b"assert", K_ASSERT, ILLEGAL),
    (b"async", K_ASYNC, GENERIC),
    (b"await", K_AWAIT, GENERIC),
    (b"basestring", K_NONE, BUILTIN_TYPE),
    (b"bin", K_NONE, BUILTIN_FUNCTION),
    (b"bool", K_NONE, BUILTIN_TYPE),
    (b"break", K_BREAK, ILLEGAL),
    (b"breakpoint", K_NONE, BUILTIN_FUNCTION),
    (b"buffer", K_NONE, BUILTIN_TYPE),
    (b"bytearray", K_NONE, BUILTIN_TYPE),
    (b"bytes", K_NONE, BUILTIN_TYPE),
    (b"callable", K_NONE, BUILTIN_FUNCTION),
    (b"chr", K_NONE, BUILTIN_FUNCTION),
    (b"class", K_CLASS, ILLEGAL),
    (b"classmethod", K_NONE, BUILTIN_FUNCTION),
    (b"cls", K_NONE, LANGUAGE),
    (b"cmp", K_NONE, BUILTIN_FUNCTION),
    (b"coerce", K_NONE, BUILTIN_FUNCTION),
    (b"compile", K_NONE, BUILTIN_FUNCTION),
    (b"complex", K_NONE, BUILTIN_TYPE),
    (b"continue", K_CONTINUE, ILLEGAL),
    (b"def", K_DEF, ILLEGAL),
    (b"del", K_DEL, ILLEGAL),
    (b"delattr", K_NONE, BUILTIN_FUNCTION),
    (b"dict", K_NONE, BUILTIN_TYPE),
    (b"dir", K_NONE, BUILTIN_FUNCTION),
    (b"divmod", K_NONE, BUILTIN_FUNCTION),
    (b"elif", K_ELIF, ILLEGAL),
    (b"else", K_ELSE, ILLEGAL),
    (b"enumerate", K_NONE, BUILTIN_FUNCTION),
    (b"eval", K_NONE, BUILTIN_FUNCTION),
    (b"except", K_EXCEPT, ILLEGAL),
    (b"exec", K_EXEC, BUILTIN_FUNCTION),
    (b"execfile", K_NONE, BUILTIN_FUNCTION),
    (b"filter", K_NONE, BUILTIN_FUNCTION),
    (b"finally", K_FINALLY, ILLEGAL),
    (b"float", K_NONE, BUILTIN_TYPE),
    (b"for", K_FOR, ILLEGAL),
    (b"format", K_NONE, BUILTIN_FUNCTION),
    (b"from", K_FROM, ILLEGAL),
    (b"frozenset", K_NONE, BUILTIN_TYPE),
    (b"getattr", K_NONE, BUILTIN_FUNCTION),
    (b"global", K_GLOBAL, ILLEGAL),
    (b"globals", K_NONE, BUILTIN_FUNCTION),
    (b"hasattr", K_NONE, BUILTIN_FUNCTION),
    (b"hash", K_NONE, BUILTIN_FUNCTION),
    (b"help", K_NONE, BUILTIN_FUNCTION),
    (b"hex", K_NONE, BUILTIN_FUNCTION),
    (b"id", K_NONE, BUILTIN_FUNCTION),
    (b"if", K_IF, ILLEGAL),
    (b"import", K_IMPORT, ILLEGAL),
    (b"in", K_IN, ILLEGAL),
    (b"input", K_NONE, BUILTIN_FUNCTION),
    (b"int", K_NONE, BUILTIN_TYPE),
    (b"intern", K_NONE, BUILTIN_FUNCTION),
    (b"is", K_LOGICAL, ILLEGAL),
    (b"isinstance", K_NONE, BUILTIN_FUNCTION),
    (b"issubclass", K_NONE, BUILTIN_FUNCTION),
    (b"iter", K_NONE, BUILTIN_FUNCTION),
    (b"lambda", K_LAMBDA, ILLEGAL),
    (b"len", K_NONE, BUILTIN_FUNCTION),
    (b"list", K_NONE, BUILTIN_TYPE),
    (b"locals", K_NONE, BUILTIN_FUNCTION),
    (b"long", K_NONE, BUILTIN_TYPE),
    (b"map", K_NONE, BUILTIN_FUNCTION),
    (b"max", K_NONE, BUILTIN_FUNCTION),
    (b"memoryview", K_NONE, BUILTIN_TYPE),
    (b"min", K_NONE, BUILTIN_FUNCTION),
    (b"next", K_NONE, BUILTIN_FUNCTION),
    (b"nonlocal", K_NONLOCAL, GENERIC),
    (b"not", K_LOGICAL, ILLEGAL),
    (b"object", K_NONE, BUILTIN_TYPE),
    (b"oct", K_NONE, BUILTIN_FUNCTION),
    (b"open", K_NONE, BUILTIN_FUNCTION),
    (b"or", K_LOGICAL, ILLEGAL),
    (b"ord", K_NONE, BUILTIN_FUNCTION),
    (b"pass", K_PASS, ILLEGAL),
    (b"pow", K_NONE, BUILTIN_FUNCTION),
    (b"print", K_PRINT, BUILTIN_FUNCTION),
    (b"property", K_NONE, BUILTIN_FUNCTION),
    (b"raise", K_RAISE, ILLEGAL),
    (b"range", K_NONE, BUILTIN_FUNCTION),
    (b"raw_input", K_NONE, BUILTIN_FUNCTION),
    (b"reduce", K_NONE, BUILTIN_FUNCTION),
    (b"reload", K_NONE, BUILTIN_FUNCTION),
    (b"repr", K_NONE, BUILTIN_FUNCTION),
    (b"return", K_RETURN, ILLEGAL),
    (b"reversed", K_NONE, BUILTIN_FUNCTION),
    (b"round", K_NONE, BUILTIN_FUNCTION),
    (b"self", K_NONE, LANGUAGE),
    (b"set", K_NONE, BUILTIN_TYPE),
    (b"setattr", K_NONE, BUILTIN_FUNCTION),
    (b"slice", K_NONE, BUILTIN_TYPE),
    (b"sorted", K_NONE, BUILTIN_FUNCTION),
    (b"staticmethod", K_NONE, BUILTIN_FUNCTION),
    (b"str", K_NONE, BUILTIN_TYPE),
    (b"sum", K_NONE, BUILTIN_FUNCTION),
    (b"super", K_NONE, BUILTIN_FUNCTION),
    (b"try", K_TRY, ILLEGAL),
    (b"tuple", K_NONE, BUILTIN_TYPE),
    (b"type", K_NONE, BUILTIN_FUNCTION),
    (b"unichr", K_NONE, BUILTIN_FUNCTION),
    (b"unicode", K_NONE, BUILTIN_TYPE),
    (b"vars", K_NONE, BUILTIN_FUNCTION),
    (b"while", K_WHILE, ILLEGAL),
    (b"with", K_WITH, ILLEGAL),
    (b"xrange", K_NONE, BUILTIN_TYPE),
    (b"yield", K_YIELD, ILLEGAL),
    (b"zip", K_NONE, BUILTIN_FUNCTION),
];

const SIZE: usize = 1024;

/// FNV-1a over the word's bytes.
const fn hash(w: &[u8]) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < w.len() {
        h ^= w[i] as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    (h as usize) & (SIZE - 1)
}

/// The table: each slot an entry's index plus one, 0 for none; open addressing.
static TABLE: [u16; SIZE] = {
    let mut t = [0u16; SIZE];
    let mut i = 0;
    while i < ENTRIES.len() {
        let mut h = hash(ENTRIES[i].0);
        while t[h] != 0 {
            h = (h + 1) & (SIZE - 1);
        }
        t[h] = i as u16 + 1;
        i += 1;
    }
    t
};

/// The longest word in the table: longer ones are none of them.
const LONGEST: usize = 25;

/// Word `w`'s keyword and name class.
#[inline]
pub(super) fn lookup(w: &[u8]) -> (u8, u16) {
    if w.len() > LONGEST || w.is_empty() {
        return (K_NONE, GENERIC);
    }
    let mut h = hash(w);
    loop {
        let i = TABLE[h];
        if i == 0 {
            return (K_NONE, GENERIC);
        }
        let (e, k, s) = ENTRIES[usize::from(i - 1)];
        if e == w {
            return (k, s);
        }
        h = (h + 1) & (SIZE - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_finds_its_words() {
        for &(w, k, s) in ENTRIES {
            assert_eq!(lookup(w), (k, s), "{}", String::from_utf8_lossy(w));
        }
        assert_eq!(lookup(b"range"), (K_NONE, BUILTIN_FUNCTION));
        assert_eq!(lookup(b"print"), (K_PRINT, BUILTIN_FUNCTION));
        assert_eq!(lookup(b"and"), (K_LOGICAL, ILLEGAL));
        assert_eq!(lookup(b"NotImplemented"), (K_CONSTANT, EXCEPTION));
        assert_eq!(lookup(b"IsADirectoryError"), (K_NONE, GENERIC));
        assert_eq!(lookup(b"__init__"), (K_NONE, MAGIC_FUNCTION));
        assert_eq!(lookup(b"alpha"), (K_NONE, GENERIC));
    }
}
