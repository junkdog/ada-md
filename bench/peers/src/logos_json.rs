//! JSON's tokens as a logos DFA: the ceiling for a format, tokens only (no scopes, no keys told
//! from values). Comments are taken as JSONC has them, since the census has some.

use logos::Logos;

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+")]
pub enum Json {
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("null")]
    Null,
    #[regex(r#""([^"\\\n]|\\[^\n])*""#)]
    String,
    #[regex(r"-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?")]
    Number,
    #[regex(r"//[^\n]*", allow_greedy = true)]
    LineComment,
    #[regex(r"/\*([^*]|\*+[^*/])*\*+/")]
    BlockComment,
}
