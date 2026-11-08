//! Rust's tokens as a logos DFA: the ceiling for a C-family language. Keywords, the primitive
//! types, identifiers, lifetimes, chars, strings (byte and C strings; raw strings and nested
//! block comments by callback), numbers with their suffixes, and punctuation, the multi-byte
//! operators whole. ASCII identifiers only: a non-ASCII one reads as an error token.

use logos::{Lexer, Logos};

fn raw_string(lex: &mut Lexer<Rust>) -> bool {
    let hashes = lex.slice().bytes().filter(|&b| b == b'#').count();
    let rest = lex.remainder().as_bytes();
    let mut i = 0;
    while let Some(q) = memchr(b'"', &rest[i..]) {
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

fn block_comment(lex: &mut Lexer<Rust>) -> bool {
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

fn memchr(needle: u8, hay: &[u8]) -> Option<usize> {
    hay.iter().position(|&b| b == needle)
}

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+")]
pub enum Rust {
    #[token("as")]
    #[token("async")]
    #[token("await")]
    #[token("break")]
    #[token("const")]
    #[token("continue")]
    #[token("crate")]
    #[token("dyn")]
    #[token("else")]
    #[token("enum")]
    #[token("extern")]
    #[token("false")]
    #[token("fn")]
    #[token("for")]
    #[token("if")]
    #[token("impl")]
    #[token("in")]
    #[token("let")]
    #[token("loop")]
    #[token("match")]
    #[token("mod")]
    #[token("move")]
    #[token("mut")]
    #[token("pub")]
    #[token("ref")]
    #[token("return")]
    #[token("self")]
    #[token("Self")]
    #[token("static")]
    #[token("struct")]
    #[token("super")]
    #[token("trait")]
    #[token("true")]
    #[token("type")]
    #[token("union")]
    #[token("unsafe")]
    #[token("use")]
    #[token("where")]
    #[token("while")]
    Keyword,
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
    Type,
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
