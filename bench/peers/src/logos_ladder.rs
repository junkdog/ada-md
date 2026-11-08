//! The logos highlighter ladder for Rust: from logos' tokens up toward what ada's scanner does,
//! a rung at a time, to price keywords told while the name is read against ada's read-then-look-up.
//!
//! Both DFAs below tell the same words ada's `rust.rs` `lookup` tells (bat's keywords by leaf,
//! the `support.type` names, the primitive types and `Self`), so the words' answers match:
//! - [`Told`]: every such word its own pattern in the DFA, its class known at the name's end;
//! - [`Idents`]: one identifier pattern, each name then given to [`lookup`], a copy of ada's
//!   (`ada-head/ada-parse/src/rust.rs`, `b35c407`), its first byte the branch and a `match` on
//!   the slice.
//!
//! The rungs ([`Rung`]): `Told` tokens counted; `Told` spans written into a `Vec<Span>`, the
//! whole file; the same line by line from the lines split outside the timing, the Vec cleared a
//! line as `ada_parse::lex` clears it (a block comment or raw string open at a line's end isn't
//! carried, a small undercount of work on the few files that have them, the same on both last
//! rungs); `Idents` with `lookup`, line by line. The last two rungs differ only in where a word is
//! told.

use ada_parse::Span;
use logos::{Lexer, Logos};

fn raw_string<'s, T: Logos<'s, Source = str>>(lex: &mut Lexer<'s, T>) -> bool {
    let hashes = lex.slice().bytes().filter(|&b| b == b'#').count();
    let rest = lex.remainder().as_bytes();
    let mut i = 0;
    while let Some(q) = rest[i..].iter().position(|&b| b == b'"') {
        let end = i + q + 1;
        if rest[end..].len() >= hashes && rest[end..end + hashes].iter().all(|&b| b == b'#') {
            lex.bump(end + hashes);
            return true;
        }
        i = end;
    }
    lex.bump(rest.len());
    true
}

fn block_comment<'s, T: Logos<'s, Source = str>>(lex: &mut Lexer<'s, T>) -> bool {
    let rest = lex.remainder().as_bytes();
    let mut depth = 1u32;
    let mut i = 0;
    while i + 1 < rest.len() {
        match (rest[i], rest[i + 1]) {
            (b'/', b'*') => {
                depth += 1;
                i += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    lex.bump(i);
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    lex.bump(rest.len());
    true
}

/// The tokens both DFAs share, after the words one of them adds.
macro_rules! rust_tokens {
    ($name:ident { $($words:tt)* }) => {
        #[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
        #[logos(skip r"[ \t\r\n]+")]
        pub enum $name {
            $($words)*
            #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
            Ident,
            #[regex(r"r#[a-zA-Z_][a-zA-Z0-9_]*")]
            RawIdent,
            #[regex(r"'[a-zA-Z_][a-zA-Z0-9_]*")]
            Lifetime,
            #[regex(r"b?'([^'\\\n]|\\[^\n]|\\u\{[0-9a-fA-F_]+\})'")]
            Char,
            #[regex(r#"[bc]?"([^"\\]|\\[\s\S])*""#)]
            String,
            #[regex(r#"[bc]?r#*""#, raw_string)]
            RawString,
            #[regex(r"[0-9][0-9_]*(\.[0-9][0-9_]*)?([eE][+-]?[0-9_]+)?([iuf](8|16|32|64|128|size))?")]
            #[regex(r"0x[0-9a-fA-F_]+([iu](8|16|32|64|128|size))?")]
            #[regex(r"0o[0-7_]+([iu](8|16|32|64|128|size))?")]
            #[regex(r"0b[01_]+([iu](8|16|32|64|128|size))?")]
            Number,
            #[regex(r"//[^\n]*", allow_greedy = true)]
            LineComment,
            #[token("/*", block_comment)]
            BlockComment,
            #[token("::")]
            #[token("->")]
            #[token("=>")]
            #[token("==")]
            #[token("!=")]
            #[token("<=")]
            #[token(">=")]
            #[token("&&")]
            #[token("||")]
            #[token("+=")]
            #[token("-=")]
            #[token("*=")]
            #[token("/=")]
            #[token("..")]
            #[token("..=")]
            #[token("...")]
            #[regex(r"[+\-*/%\^!&|=<>@.,;:#$?~(){}\[\]]")]
            Punct,
        }
    };
}

rust_tokens!(Told {
    #[token("as")]
    #[token("box")]
    #[token("in")]
    Operator,
    #[token("alignof")]
    #[token("abstract")]
    #[token("become")]
    #[token("do")]
    #[token("final")]
    #[token("macro")]
    #[token("offsetof")]
    #[token("override")]
    #[token("priv")]
    #[token("proc")]
    #[token("pure")]
    #[token("sizeof")]
    #[token("typeof")]
    #[token("unsized")]
    #[token("virtual")]
    #[token("yield")]
    Invalid,
    #[token("break")]
    #[token("continue")]
    #[token("else")]
    #[token("for")]
    #[token("if")]
    #[token("loop")]
    #[token("match")]
    #[token("return")]
    #[token("while")]
    Control,
    #[token("const")]
    #[token("let")]
    #[token("static")]
    Storage,
    #[token("crate")]
    #[token("extern")]
    #[token("use")]
    #[token("where")]
    Keyword,
    #[token("enum")]
    Enum,
    #[token("fn")]
    Fn,
    #[token("impl")]
    Impl,
    #[token("mod")]
    Module,
    #[token("mut")]
    #[token("move")]
    #[token("pub")]
    #[token("ref")]
    #[token("unsafe")]
    Modifier,
    #[token("struct")]
    Struct,
    #[token("type")]
    Type,
    #[token("trait")]
    Trait,
    #[token("false")]
    #[token("true")]
    Language,
    #[token("bool")]
    #[token("char")]
    #[token("f32")]
    #[token("f64")]
    #[token("i8")]
    #[token("i16")]
    #[token("i32")]
    #[token("i64")]
    #[token("i128")]
    #[token("isize")]
    #[token("str")]
    #[token("u8")]
    #[token("u16")]
    #[token("u32")]
    #[token("u64")]
    #[token("u128")]
    #[token("usize")]
    #[token("Self")]
    Primitive,
    #[token("AsRef")]
    #[token("AsMut")]
    #[token("Box")]
    #[token("Copy")]
    #[token("Clone")]
    #[token("Drop")]
    #[token("Default")]
    #[token("DoubleEndedIterator")]
    #[token("Eq")]
    #[token("Extend")]
    #[token("ExactSizeIterator")]
    #[token("Err")]
    #[token("Fn")]
    #[token("FnMut")]
    #[token("FnOnce")]
    #[token("From")]
    #[token("Into")]
    #[token("Iterator")]
    #[token("IntoIterator")]
    #[token("None")]
    #[token("Ord")]
    #[token("Option")]
    #[token("Ok")]
    #[token("PartialEq")]
    #[token("PartialOrd")]
    #[token("Result")]
    #[token("Send")]
    #[token("Sized")]
    #[token("Sync")]
    #[token("Some")]
    #[token("SliceConcatExt")]
    #[token("String")]
    #[token("ToOwned")]
    #[token("ToString")]
    #[token("Vec")]
    Support,
});

rust_tokens!(Idents {});

// ada's leaves for the words (rust.rs), and stand-ins for the other tokens
const PLAIN: u16 = 0;
const KEYWORD: u16 = 61;
const CONTROL: u16 = 62;
const OPERATOR: u16 = 63;
const STORAGE: u16 = 70;
const FN: u16 = 72;
const STRUCT: u16 = 73;
const IMPL: u16 = 74;
const ENUM: u16 = 75;
const TYPE: u16 = 76;
const TRAIT: u16 = 77;
const MODULE: u16 = 78;
const MODIFIER: u16 = 79;
const SUPPORT_TYPE: u16 = 100;
const LANGUAGE: u16 = 102;
const INVALID: u16 = 108;
const OTHER: [u16; 10] = [1, 2, 3, 4, 13, 14, 15, 16, 17, 18];

/// What a word is to bat's grammar, as ada's `Name`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Name {
    Other,
    Keyword(u16),
    Support,
    Primitive,
}

/// ada's `rust.rs` `lookup` at `b35c407`, copied: the word among bat's keywords and type names,
/// dispatching on its first byte.
fn lookup(w: &[u8]) -> Name {
    use Name::{Keyword as K, Other, Primitive, Support};
    let Some(&c) = w.first() else {
        return Other;
    };
    match c {
        b'a' => match w {
            b"as" => K(OPERATOR),
            b"alignof" | b"abstract" => K(INVALID),
            _ => Other,
        },
        b'b' => match w {
            b"break" => K(CONTROL),
            b"box" => K(OPERATOR),
            b"become" => K(INVALID),
            b"bool" => Primitive,
            _ => Other,
        },
        b'c' => match w {
            b"const" => K(STORAGE),
            b"crate" => K(KEYWORD),
            b"continue" => K(CONTROL),
            b"char" => Primitive,
            _ => Other,
        },
        b'd' => match w {
            b"do" => K(INVALID),
            _ => Other,
        },
        b'e' => match w {
            b"else" => K(CONTROL),
            b"enum" => K(ENUM),
            b"extern" => K(KEYWORD),
            _ => Other,
        },
        b'f' => match w {
            b"fn" => K(FN),
            b"for" => K(CONTROL),
            b"false" => K(LANGUAGE),
            b"final" => K(INVALID),
            b"f32" | b"f64" => Primitive,
            _ => Other,
        },
        b'i' => match w {
            b"if" => K(CONTROL),
            b"impl" => K(IMPL),
            b"in" => K(OPERATOR),
            b"i8" | b"i16" | b"i32" | b"i64" | b"i128" | b"isize" => Primitive,
            _ => Other,
        },
        b'l' => match w {
            b"let" => K(STORAGE),
            b"loop" => K(CONTROL),
            _ => Other,
        },
        b'm' => match w {
            b"mod" => K(MODULE),
            b"mut" | b"move" => K(MODIFIER),
            b"match" => K(CONTROL),
            b"macro" => K(INVALID),
            _ => Other,
        },
        b'o' => match w {
            b"offsetof" | b"override" => K(INVALID),
            _ => Other,
        },
        b'p' => match w {
            b"pub" => K(MODIFIER),
            b"priv" | b"proc" | b"pure" => K(INVALID),
            _ => Other,
        },
        b'r' => match w {
            b"ref" => K(MODIFIER),
            b"return" => K(CONTROL),
            _ => Other,
        },
        b's' => match w {
            b"static" => K(STORAGE),
            b"struct" => K(STRUCT),
            b"sizeof" => K(INVALID),
            b"str" => Primitive,
            _ => Other,
        },
        b't' => match w {
            b"type" => K(TYPE),
            b"trait" => K(TRAIT),
            b"true" => K(LANGUAGE),
            b"typeof" => K(INVALID),
            _ => Other,
        },
        b'u' => match w {
            b"use" => K(KEYWORD),
            b"unsafe" => K(MODIFIER),
            b"unsized" => K(INVALID),
            b"u8" | b"u16" | b"u32" | b"u64" | b"u128" | b"usize" => Primitive,
            _ => Other,
        },
        b'v' => match w {
            b"virtual" => K(INVALID),
            _ => Other,
        },
        b'w' => match w {
            b"where" => K(KEYWORD),
            b"while" => K(CONTROL),
            _ => Other,
        },
        b'y' => match w {
            b"yield" => K(INVALID),
            _ => Other,
        },
        b'A' => match w {
            b"AsRef" | b"AsMut" => Support,
            _ => Other,
        },
        b'B' => match w {
            b"Box" => Support,
            _ => Other,
        },
        b'C' => match w {
            b"Copy" | b"Clone" => Support,
            _ => Other,
        },
        b'D' => match w {
            b"Drop" | b"Default" | b"DoubleEndedIterator" => Support,
            _ => Other,
        },
        b'E' => match w {
            b"Eq" | b"Extend" | b"ExactSizeIterator" | b"Err" => Support,
            _ => Other,
        },
        b'F' => match w {
            b"Fn" | b"FnMut" | b"FnOnce" | b"From" => Support,
            _ => Other,
        },
        b'I' => match w {
            b"Into" | b"Iterator" | b"IntoIterator" => Support,
            _ => Other,
        },
        b'N' => match w {
            b"None" => Support,
            _ => Other,
        },
        b'O' => match w {
            b"Ord" | b"Option" | b"Ok" => Support,
            _ => Other,
        },
        b'P' => match w {
            b"PartialEq" | b"PartialOrd" => Support,
            _ => Other,
        },
        b'R' => match w {
            b"Result" => Support,
            _ => Other,
        },
        b'S' => match w {
            b"Send" | b"Sized" | b"Sync" | b"Some" | b"SliceConcatExt" | b"String" => Support,
            b"Self" => Primitive,
            _ => Other,
        },
        b'T' => match w {
            b"ToOwned" | b"ToString" => Support,
            _ => Other,
        },
        b'V' => match w {
            b"Vec" => Support,
            _ => Other,
        },
        _ => Other,
    }
}

/// A word's leaf, ada's way: a keyword its own, a support name `support.type`, a primitive type
/// `storage.type`.
fn leaf(name: Name) -> u16 {
    match name {
        Name::Keyword(k) => k,
        Name::Support => SUPPORT_TYPE,
        Name::Primitive => STORAGE,
        Name::Other => PLAIN,
    }
}

impl Told {
    fn kind(self) -> u16 {
        match self {
            Told::Operator => OPERATOR,
            Told::Invalid => INVALID,
            Told::Control => CONTROL,
            Told::Storage => STORAGE,
            Told::Keyword => KEYWORD,
            Told::Enum => ENUM,
            Told::Fn => FN,
            Told::Impl => IMPL,
            Told::Module => MODULE,
            Told::Modifier => MODIFIER,
            Told::Struct => STRUCT,
            Told::Type => TYPE,
            Told::Trait => TRAIT,
            Told::Language => LANGUAGE,
            Told::Primitive => STORAGE,
            Told::Support => SUPPORT_TYPE,
            Told::Ident => PLAIN,
            Told::RawIdent => OTHER[0],
            Told::Lifetime => OTHER[1],
            Told::Char => OTHER[2],
            Told::String => OTHER[3],
            Told::RawString => OTHER[4],
            Told::Number => OTHER[5],
            Told::LineComment => OTHER[6],
            Told::BlockComment => OTHER[7],
            Told::Punct => OTHER[8],
        }
    }
}

impl Idents {
    /// The token's leaf, an identifier's by `lookup` on its text.
    fn kind(self, text: &str) -> u16 {
        match self {
            Idents::Ident => leaf(lookup(text.as_bytes())),
            Idents::RawIdent => OTHER[0],
            Idents::Lifetime => OTHER[1],
            Idents::Char => OTHER[2],
            Idents::String => OTHER[3],
            Idents::RawString => OTHER[4],
            Idents::Number => OTHER[5],
            Idents::LineComment => OTHER[6],
            Idents::BlockComment => OTHER[7],
            Idents::Punct => OTHER[8],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rung {
    /// `Told`'s tokens counted, the whole file
    Tokens,
    /// `Told`'s spans written, the whole file
    Spans,
    /// `Told`'s spans written, line by line
    Lines,
    /// `Idents`' spans written, each name looked up, line by line
    LookedUp,
}

impl Rung {
    pub const ALL: [Rung; 4] = [Rung::Tokens, Rung::Spans, Rung::Lines, Rung::LookedUp];

    pub fn name(self) -> &'static str {
        match self {
            Rung::Tokens => "logos told, tokens",
            Rung::Spans => "logos told, spans",
            Rung::Lines => "logos told, by line",
            Rung::LookedUp => "logos + lookup, line",
        }
    }
}

fn span(r: std::ops::Range<usize>, kind: u16) -> Span {
    Span { start: r.start as u32, end: r.end as u32, kind }
}

/// One file through a rung: (output units, error tokens). `lines` are the file's lines, split
/// outside the timing.
pub fn pass(rung: Rung, text: &str, lines: &[&str], spans: &mut Vec<Span>) -> (u64, u64) {
    let (mut n, mut err) = (0u64, 0u64);
    match rung {
        Rung::Tokens => {
            for t in Told::lexer(text) {
                if t.is_ok() { n += 1 } else { err += 1 }
            }
        }
        Rung::Spans => {
            spans.clear();
            let mut lex = Told::lexer(text);
            while let Some(t) = lex.next() {
                match t {
                    Ok(t) => spans.push(span(lex.span(), t.kind())),
                    Err(()) => err += 1,
                }
            }
            n = spans.len() as u64;
        }
        Rung::Lines => {
            for line in lines {
                spans.clear();
                let mut lex = Told::lexer(line);
                while let Some(t) = lex.next() {
                    match t {
                        Ok(t) => spans.push(span(lex.span(), t.kind())),
                        Err(()) => err += 1,
                    }
                }
                n += spans.len() as u64;
            }
        }
        Rung::LookedUp => {
            for line in lines {
                spans.clear();
                let mut lex = Idents::lexer(line);
                while let Some(t) = lex.next() {
                    match t {
                        Ok(t) => spans.push(span(lex.span(), t.kind(lex.slice()))),
                        Err(()) => err += 1,
                    }
                }
                n += spans.len() as u64;
            }
        }
    }
    (n, err)
}

/// A check outside the timing: both last rungs give every span the same leaf.
pub fn same_leaves(lines: &[&str]) -> Result<u64, String> {
    let mut words = 0;
    for (i, line) in lines.iter().enumerate() {
        let a: Vec<_> = {
            let mut lex = Told::lexer(line);
            std::iter::from_fn(|| lex.next().map(|t| (lex.span(), t.map(Told::kind)))).collect()
        };
        let b: Vec<_> = {
            let mut lex = Idents::lexer(line);
            std::iter::from_fn(|| lex.next().map(|t| (lex.span(), t.map(|t| t.kind(lex.slice())))))
                .collect()
        };
        if a != b {
            return Err(format!("line {i}: {line:?}"));
        }
        words += a.iter().filter(|(_, k)| k.is_ok_and(|k| k != PLAIN && !OTHER.contains(&k))).count() as u64;
    }
    Ok(words)
}
