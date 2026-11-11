//! A header's language by its text (X-134): bat's set names every `.h` Objective-C, its last
//! claimant after C++, C and Objective-C++, so the name says nothing of it. The head of the file
//! (its first [`HEAD`] bytes) is read past comments, strings, characters and directives' lines,
//! and its words decide: Objective-C where an `@` keyword of Objective-C's, `#import` or a word of
//! Foundation's stands; C++ where a word only C++ has, or `::`, stands outside an
//! `#if … __cplusplus` block (a C header's C++ wrapper); Objective-C++ where both; else C.
//!
//! Against headers labeled by the sources beside them (vault `ada-parse.md`, X-134): C++'s 92%,
//! C's 99.4%, Objective-C's 88.6% (+3% Objective-C++); the rest are, on inspection, headers of the
//! other language in the same directory (C++ beside `.mm`, C beside `.m`).

use crate::Lang;

/// How much of a header is read.
const HEAD: usize = 64 * 1024;

/// The words after `@` that only Objective-C has.
const OBJC_AT: &[&[u8]] = &[
    b"interface",
    b"protocol",
    b"end",
    b"class",
    b"property",
    b"implementation",
    b"import",
    b"optional",
    b"required",
    b"synthesize",
    b"dynamic",
    b"selector",
    b"encode",
    b"autoreleasepool",
    b"synchronized",
    b"try",
    b"catch",
    b"finally",
    b"throw",
    b"compatibility_alias",
    b"package",
    b"public",
    b"private",
    b"protected",
    b"defs",
    b"available",
];

/// Foundation's words: an Objective-C header that declares only C functions and types still
/// names them. Not `BOOL` (Windows has it), `YES`, `NO` or `nil` (C macros may).
const NS_WORDS: &[&[u8]] = &[
    b"NS_ASSUME_NONNULL_BEGIN",
    b"NS_ENUM",
    b"NS_OPTIONS",
    b"NS_CLOSED_ENUM",
    b"NS_ERROR_ENUM",
    b"NS_TYPED_ENUM",
    b"NS_SWIFT_NAME",
    b"FOUNDATION_EXPORT",
    b"FOUNDATION_EXTERN",
    b"NSString",
    b"NSInteger",
    b"NSUInteger",
    b"NSObject",
    b"NSArray",
    b"NSDictionary",
    b"NSError",
    b"NSData",
    b"NSNumber",
    b"NSURL",
    b"NSTimeInterval",
    b"NSRange",
    b"instancetype",
    b"IBOutlet",
    b"IBAction",
];

/// The words only C++ has: not those C has as macros (`static_assert`, `alignas`,
/// `thread_local`, `char16_t`) nor those C and Objective-C use as names (`new`, `delete`,
/// `this`, `try`, `catch`, `throw`, `final`, `export`, access words).
const CPP_WORDS: &[&[u8]] = &[
    b"class",
    b"namespace",
    b"template",
    b"typename",
    b"virtual",
    b"constexpr",
    b"nullptr",
    b"operator",
    b"using",
    b"noexcept",
    b"decltype",
    b"static_cast",
    b"reinterpret_cast",
    b"const_cast",
    b"dynamic_cast",
    b"mutable",
    b"explicit",
    b"friend",
    b"consteval",
    b"constinit",
    b"concept",
    b"requires",
    b"co_await",
    b"co_return",
    b"co_yield",
];

fn word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The language of a header whose text starts with `text`.
pub(crate) fn language(text: &[u8]) -> Lang {
    let b = &text[..text.len().min(HEAD)];
    let n = b.len();
    let (mut objc, mut cpp) = (false, false);
    // the `#if`s open, and the depth an `#if … __cplusplus` opened at (0: none)
    let (mut depth, mut in_cpp) = (0u32, 0u32);
    let mut line_start = true;
    let mut i = 0;
    while i < n {
        let c = b[i];
        match c {
            b'\n' => {
                line_start = true;
                i += 1;
            }
            b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c' => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = memchr(b, i, b'\n');
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = b[i + 2..]
                    .windows(2)
                    .position(|w| w == b"*/")
                    .map_or(n, |k| i + 2 + k + 2);
            }
            b'"' | b'\'' => {
                let mut j = i + 1;
                while j < n && b[j] != c && b[j] != b'\n' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                i = j + 1;
                line_start = false;
            }
            b'#' if line_start => {
                let mut s = i + 1;
                while s < n && matches!(b[s], b' ' | b'\t') {
                    s += 1;
                }
                let mut e = s;
                while e < n && b[e].is_ascii_lowercase() {
                    e += 1;
                }
                // the directive's line, its `\` continuations with it
                let mut end = i;
                loop {
                    end = memchr(b, end, b'\n');
                    if end < n && end > 0 && b[end - 1] == b'\\' {
                        end += 1;
                        continue;
                    }
                    break;
                }
                match &b[s..e] {
                    b"import" => objc = true,
                    b"if" | b"ifdef" | b"ifndef" => {
                        depth += 1;
                        if in_cpp == 0 && contains(&b[e..end], b"__cplusplus") {
                            in_cpp = depth;
                        }
                    }
                    b"endif" => {
                        if in_cpp == depth {
                            in_cpp = 0;
                        }
                        depth = depth.saturating_sub(1);
                    }
                    // `#ifdef __cplusplus … #else`: the other branch is C again
                    b"else" if in_cpp == depth => in_cpp = 0,
                    _ => {}
                }
                i = end;
            }
            b'@' => {
                line_start = false;
                let s = i + 1;
                let mut e = s;
                while e < n && word_byte(b[e]) {
                    e += 1;
                }
                objc |= e > s && OBJC_AT.contains(&&b[s..e]);
                i = e.max(s);
            }
            b':' if b.get(i + 1) == Some(&b':') => {
                line_start = false;
                cpp |= in_cpp == 0;
                i += 2;
            }
            _ if word_start(c) => {
                line_start = false;
                let mut e = i + 1;
                while e < n && word_byte(b[e]) {
                    e += 1;
                }
                let w = &b[i..e];
                if !cpp && in_cpp == 0 && CPP_WORDS.contains(&w) {
                    cpp = true;
                } else if !objc && NS_WORDS.contains(&w) {
                    objc = true;
                }
                i = e;
            }
            _ => {
                line_start = false;
                i += 1;
            }
        }
    }
    match (objc, cpp) {
        (true, true) => Lang::ObjCpp,
        (true, false) => Lang::ObjC,
        (false, true) => Lang::Cpp,
        (false, false) => Lang::C,
    }
}

/// Where the next `c` from `p` is, else `b`'s end.
fn memchr(b: &[u8], p: usize, c: u8) -> usize {
    b[p..]
        .iter()
        .position(|&x| x == c)
        .map_or(b.len(), |k| p + k)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_by_their_text() {
        let objc = b"// a\n#import <Foundation/Foundation.h>\n@interface Foo : NSObject\n@end\n";
        assert_eq!(language(objc), Lang::ObjC);
        let ns = b"#include <stdint.h>\nNS_ASSUME_NONNULL_BEGIN\nNSString *f(void);\n";
        assert_eq!(language(ns), Lang::ObjC);
        let cpp = b"#pragma once\nnamespace a {\nclass B;\n}\n";
        assert_eq!(language(cpp), Lang::Cpp);
        let objcpp = b"#include <vector>\n@class Foo;\nstd::vector<int> v;\n";
        assert_eq!(language(objcpp), Lang::ObjCpp);
        // a C header's C++ wrapper, comments and strings don't make it C++
        let c = b"#ifndef A_H\n#define A_H\n#ifdef __cplusplus\nextern \"C\" {\ntemplate<class T> T f();\n#endif\n\
                  /* class Foo, namespace */\n// @interface\nconst char *s = \"std::x\";\nint f(int);\n\
                  #ifdef __cplusplus\n}\n#endif\n#endif\n";
        assert_eq!(language(c), Lang::C);
        // `#ifdef __cplusplus … #else` is C again after the `#else`
        let alt = b"#ifdef __cplusplus\n#else\nclass X;\n#endif\n";
        assert_eq!(language(alt), Lang::Cpp);
        assert_eq!(language(b""), Lang::C);
        // a directive's continued line is the directive's
        assert_eq!(language(b"#define X \\\n  class\nint y;\n"), Lang::C);
    }
}
