//! Java's tokens as a logos DFA: the ceiling, as `logos_rust.rs` is for Rust. Keywords, the
//! literals `true`, `false` and `null`, identifiers (with `$`), annotations, chars, strings and
//! text blocks (by callback), numbers with underscores, suffixes and hex floats, line and block
//! comments (a block comment by callback), and punctuation, the multi-byte operators whole. ASCII
//! identifiers only: a non-ASCII one reads as an error token. A comment is one token: what's in a
//! Javadoc comment isn't read.

use logos::{Lexer, Logos};

fn text_block(lex: &mut Lexer<Java>) -> bool {
    let rest = lex.remainder().as_bytes();
    let mut i = 0;
    while i + 2 < rest.len() {
        match rest[i] {
            b'\\' => i += 2,
            b'"' if rest[i + 1] == b'"' && rest[i + 2] == b'"' => {
                lex.bump(i + 3);
                return true;
            }
            _ => i += 1,
        }
    }
    lex.bump(rest.len());
    true
}

fn block_comment(lex: &mut Lexer<Java>) -> bool {
    let rest = lex.remainder().as_bytes();
    let mut i = 0;
    while i + 1 < rest.len() {
        if rest[i] == b'*' && rest[i + 1] == b'/' {
            lex.bump(i + 2);
            return true;
        }
        i += 1;
    }
    lex.bump(rest.len());
    true
}

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n\x0C]+")]
pub enum Java {
    #[token("abstract")]
    #[token("assert")]
    #[token("break")]
    #[token("case")]
    #[token("catch")]
    #[token("class")]
    #[token("const")]
    #[token("continue")]
    #[token("default")]
    #[token("do")]
    #[token("else")]
    #[token("enum")]
    #[token("extends")]
    #[token("final")]
    #[token("finally")]
    #[token("for")]
    #[token("goto")]
    #[token("if")]
    #[token("implements")]
    #[token("import")]
    #[token("instanceof")]
    #[token("interface")]
    #[token("native")]
    #[token("new")]
    #[token("package")]
    #[token("private")]
    #[token("protected")]
    #[token("public")]
    #[token("return")]
    #[token("static")]
    #[token("strictfp")]
    #[token("super")]
    #[token("switch")]
    #[token("synchronized")]
    #[token("this")]
    #[token("throw")]
    #[token("throws")]
    #[token("transient")]
    #[token("try")]
    #[token("var")]
    #[token("volatile")]
    #[token("while")]
    #[token("record")]
    #[token("yield")]
    Keyword,
    #[token("boolean")]
    #[token("byte")]
    #[token("char")]
    #[token("double")]
    #[token("float")]
    #[token("int")]
    #[token("long")]
    #[token("short")]
    #[token("void")]
    Type,
    #[token("true")]
    #[token("false")]
    #[token("null")]
    Literal,
    #[regex(r"[a-zA-Z_$][a-zA-Z0-9_$]*")]
    Ident,
    #[regex(r"@[a-zA-Z_$][a-zA-Z0-9_$]*")]
    Annotation,
    #[regex(r"'([^'\\\n]|\\[^\n]|\\u+[0-9a-fA-F]{4})'")]
    Char,
    #[regex(r#""([^"\\\n]|\\[^\n])*""#)]
    String,
    #[token(r#"""""#, text_block)]
    TextBlock,
    #[regex(r"[0-9][0-9_]*[lL]?")]
    #[regex(r"[0-9][0-9_]*\.[0-9_]*([eE][+-]?[0-9_]+)?[fFdD]?")]
    #[regex(r"\.[0-9][0-9_]*([eE][+-]?[0-9_]+)?[fFdD]?")]
    #[regex(r"[0-9][0-9_]*[eE][+-]?[0-9_]+[fFdD]?")]
    #[regex(r"[0-9][0-9_]*[fFdD]")]
    #[regex(r"0[xX][0-9a-fA-F_]+[lL]?")]
    #[regex(r"0[xX][0-9a-fA-F_]*\.?[0-9a-fA-F_]*[pP][+-]?[0-9_]+[fFdD]?")]
    #[regex(r"0[bB][01_]+[lL]?")]
    Number,
    #[regex(r"//[^\n]*", allow_greedy = true)]
    LineComment,
    #[token("/*", block_comment)]
    BlockComment,
    #[token("::")]
    #[token("->")]
    #[token("==")]
    #[token("!=")]
    #[token("<=")]
    #[token(">=")]
    #[token("&&")]
    #[token("||")]
    #[token("++")]
    #[token("--")]
    #[token("+=")]
    #[token("-=")]
    #[token("*=")]
    #[token("/=")]
    #[token("%=")]
    #[token("&=")]
    #[token("|=")]
    #[token("^=")]
    #[token("<<")]
    #[token("<<=")]
    #[token(">>=")]
    #[token(">>>=")]
    #[token("...")]
    #[regex(r"[+\-*/%\^!&|=<>.,;:?~(){}\[\]]")]
    Punct,
}
