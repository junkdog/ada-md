use super::*;
use crate::{Span, State};

/// Each line's tokens of C++ (or C) as (text, scopes), lexed from the default state, each
/// line's spans checked: in order, not empty, inside the line.
fn lexed(c: bool, lines: &[&str]) -> Vec<(String, String)> {
    let mut state = State::default();
    let mut got = Vec::new();
    for l in lines {
        let mut spans: Vec<Span> = Vec::new();
        let mut out = Out::new(&mut spans);
        if c {
            line_c(l.as_bytes(), &mut state, &mut out);
        } else {
            line(l.as_bytes(), &mut state, &mut out);
        }
        let mut end = 0;
        for s in &spans {
            assert!(
                s.start >= end && s.end > s.start && s.end as usize <= l.len(),
                "{spans:?} on {l:?}"
            );
            end = s.end;
            let scopes = if c { scopes_c(s.kind) } else { scopes(s.kind) };
            got.push((
                l[s.start as usize..s.end as usize].to_owned(),
                scopes.join(" "),
            ));
        }
    }
    got
}

fn has(c: bool, lines: &[&str], text: &str, scopes: &str) {
    let got = lexed(c, lines);
    assert!(
        got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

fn plain(c: bool, lines: &[&str], text: &str) {
    let got = lexed(c, lines);
    assert!(
        !got.iter().any(|(t, _)| t == text),
        "{text:?} not plain in {got:#?}"
    );
}

#[test]
fn words_and_calls() {
    let l = ["int x = foo(1) + obj.bar(2) + p->q + printf(\"%d\\n\", n) + kMax + m_v;"];
    has(false, &l, "int", "storage.type.c++");
    has(false, &l, "foo", "variable.function.c++");
    has(false, &l, "bar", "variable.function.member.c++");
    has(false, &l, "q", "variable.other.readwrite.member.c++");
    has(false, &l, "printf", "support.function.C99.c++");
    has(
        false,
        &l,
        "%d",
        "string.quoted.double.c++ constant.other.placeholder.c++",
    );
    has(
        false,
        &l,
        "\\n",
        "string.quoted.double.c++ constant.character.escape.c++",
    );
    has(false, &l, "kMax", "constant.other.variable.mac-classic.c++");
    has(false, &l, "m_v", "variable.other.readwrite.member.c++");
    plain(false, &l, "x");
    plain(false, &l, "n");
    // C hasn't C++'s words nor its member shapes
    let l = ["class this; m_v = new_thing;"];
    plain(true, &l, "class");
    plain(true, &l, "this");
    plain(true, &l, "m_v");
    has(false, &l, "class", "storage.type.c++");
    has(false, &l, "this", "variable.language.c++");
}

#[test]
fn numbers() {
    let l = ["x = 0x1Fu + 1.5e3f + 0777 + 0b101 + 1'000ull + .5;"];
    has(
        false,
        &l,
        "0x",
        "constant.numeric.integer.hexadecimal.c++ punctuation.definition.numeric.base.c++",
    );
    has(false, &l, "1F", "constant.numeric.integer.hexadecimal.c++");
    has(
        false,
        &l,
        "u",
        "constant.numeric.integer.hexadecimal.c++ storage.type.numeric.c++",
    );
    has(false, &l, "5e3", "constant.numeric.float.decimal.c++");
    has(
        false,
        &l,
        "f",
        "constant.numeric.float.decimal.c++ storage.type.numeric.c++",
    );
    has(false, &l, "777", "constant.numeric.integer.octal.c++");
    has(false, &l, "101", "constant.numeric.integer.binary.c++");
    has(false, &l, "1'000", "constant.numeric.integer.decimal.c++");
    has(
        false,
        &l,
        "ull",
        "constant.numeric.integer.decimal.c++ storage.type.numeric.c++",
    );
}

#[test]
fn comments_and_strings_across_lines() {
    let l = [
        "a /* one",
        "two */ b; // x \\",
        "still",
        "c = \"s\\",
        "t\";",
    ];
    has(
        false,
        &l,
        "/*",
        "comment.block.c++ punctuation.definition.comment.c++",
    );
    has(false, &l, "two ", "comment.block.c++");
    let m = ["/*", " * one", " */", "a->b = c.d;"];
    has(
        false,
        &m,
        "*",
        "comment.block.c++ punctuation.definition.comment.c++",
    );
    has(false, &m, "b", "variable.other.readwrite.member.c++");
    plain(true, &m, "b");
    has(true, &m, "d", "variable.other.readwrite.member.c");
    has(false, &l, "still", "comment.line.double-slash.c++");
    has(false, &l, "t", "string.quoted.double.c++");
    // a string without a `\` ends at its line's end
    let l = ["s = \"open", "int x;"];
    has(false, &l, "int", "storage.type.c++");
    let l = ["auto r = R\"xy(raw ) \"body", "more)xy\";"];
    has(
        false,
        &l,
        "R",
        "string.quoted.double.c++ storage.type.string.c++",
    );
    has(false, &l, "raw ) \"body", "string.quoted.double.c++");
    has(
        false,
        &l,
        ")xy\"",
        "string.quoted.double.c++ punctuation.definition.string.end.c++",
    );
}

#[test]
fn colons() {
    let l = [
        "class Foo : public Bar {",
        "public:",
        "  Foo() : a(1) {}",
        "};",
        "void g() { switch (x) { case 1: y = c ? 1 : 2; } label: ; }",
    ];
    has(false, &l, ":", "punctuation.section.class.c++");
    has(false, &l, ":", "punctuation.separator.initializer-list.c++");
    has(false, &l, ":", "punctuation.separator.c++");
    has(false, &l, ":", "keyword.operator.ternary.c++");
    has(false, &l, "label", "entity.name.label.c++");
}

#[test]
fn directives() {
    let l = [
        "#include <stdio.h> // c",
        "#define BAR(a, b) ((a) + (b)) \\",
        "  * 2",
        "#if defined(X) && X > 2",
        "#if 1",
        "#endif",
        "#error \"no\" don't",
        "#if 0",
        "int dead;",
        "#if 1",
        "#endif",
        "#else",
        "int live;",
        "#endif",
    ];
    has(false, &l, "#include", "keyword.control.import.include.c++");
    has(
        false,
        &l,
        "stdio.h",
        "string.quoted.other.lt-gt.include.c++",
    );
    has(false, &l, " ", "meta.preprocessor.c++");
    has(false, &l, " c", "comment.line.double-slash.c++");
    has(false, &l, "BAR", "entity.name.function.preprocessor.c++");
    has(false, &l, "a", "variable.parameter.c++");
    has(false, &l, "  ", "meta.preprocessor.c++");
    has(false, &l, "2", "constant.numeric.integer.decimal.c++");
    has(false, &l, "defined", "keyword.control.c++");
    has(false, &l, "(X) && X > 2", "meta.preprocessor.c++");
    has(false, &l, "1", "constant.numeric.integer.decimal.c++");
    has(false, &l, "don't", "string.unquoted.c++");
    has(
        false,
        &l,
        "int dead;",
        "comment.block.preprocessor.if-branch.c++",
    );
    has(
        false,
        &l,
        "#if 1",
        "comment.block.preprocessor.if-branch.c++",
    );
    has(false, &l, "#else", "keyword.control.import.else.c++");
    has(false, &l, "int", "storage.type.c++");
}

#[test]
fn braces_and_state() {
    let mut state = State::default();
    let mut spans = Vec::new();
    for l in ["namespace a {", "class B {", "void f() {", "  { x; }"] {
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
    }
    assert_eq!(state.depth, 3);
    {
        let mut out = Out::new(&mut spans);
        let mut s = Scan::<Spans, false>::new(b"", &mut out, L_CPP, Spans::new());
        s.load(&state);
        // innermost first: the function's body, the class's, the namespace's
        assert_eq!(s.braces & 0x3f, u128::from(CODE | CLASS << 2 | NS << 4));
    }
    for l in ["}", "};", "}"] {
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
    }
    assert_eq!(state, State::default());
}

#[test]
fn a_block_lexes_as_its_lines() {
    let text = "#define X \\\n  1\nint a; /* b\n c */ s = \"x\\\ny\";\n#if 0\nz\n#endif\n";
    let mut state = State::default();
    let mut one = Vec::new();
    let mut spans = Vec::new();
    for l in text.lines() {
        spans.clear();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        one.push((spans.clone(), state));
    }
    let mut state = State::default();
    let mut all = Vec::new();
    let mut v = Vec::new();
    block(&mut state, &mut v, &mut |c| {
        for l in text.lines() {
            all.push(c.next(l.as_bytes()).to_vec());
        }
        None
    });
    assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
    assert_eq!(state, one.last().unwrap().1);
}

#[test]
fn definitions_and_parameters() {
    let l = [
        "static int",
        "foo(SSL *ssl, const char *name, int n = g(1, 2), int arr[]) {",
        "  return bar(ssl, n);",
        "}",
        "std::string Foo::get(std::map<int, int> m, unsigned) const;",
        "Foo::~Foo() {}",
        "bool operator==(const A &a);",
        "FOO(x);",
        "int x = make(1);",
        "void f2(int a,",
        "        int b) {}",
    ];
    for f in ["foo", "get", "~Foo", "operator==", "f2"] {
        has(false, &l, f, "entity.name.function.c++");
    }
    for v in ["ssl", "name", "n", "arr", "m", "unsigned", "a", "b"] {
        has(false, &l, v, "variable.parameter.c++");
    }
    for c in ["bar", "g", "FOO", "make"] {
        has(false, &l, c, "variable.function.c++");
    }
    has(false, &l, "int", "storage.type.c++");
    // a class's members: constructors, a macro's call, `operator` before a type
    let l = [
        "class EXPORT Foo final : public Bar {",
        " public:",
        "  Foo(int v);",
        "  DISALLOW_COPY(Foo);",
        "  operator bool() const;",
        "  void run() { go(1); }",
        "};",
    ];
    has(false, &l, "Foo", "entity.name.class.c++");
    plain(false, &l, "EXPORT");
    has(false, &l, "Foo", "entity.name.function.c++");
    has(false, &l, "v", "variable.parameter.c++");
    has(false, &l, "DISALLOW_COPY", "variable.function.c++");
    has(false, &l, "bool", "entity.name.function.c++");
    has(false, &l, "run", "entity.name.function.c++");
    has(false, &l, "go", "variable.function.c++");
    // C: `void` alone isn't a parameter, `restrict` before `)` is one
    let l = ["int f(void);", "int g(char *restrict);"];
    has(true, &l, "void", "storage.type.c");
    has(true, &l, "restrict", "variable.parameter.c");
    has(true, &l, "g", "entity.name.function.c");
}

#[test]
fn entity_names() {
    let l = [
        "namespace a::b {",
        "struct S;",
        "struct stat st;",
        "struct T { int x; };",
        "enum class E : int { A };",
        "union U { int i; };",
        "typedef struct node node_t;",
        "typedef struct { int x; } anon_t;",
        "typedef void (*cb_t)(int a);",
        "using Alias = std::vector<int>;",
        "using namespace std;",
        "}",
    ];
    has(false, &l, "a", "entity.name.namespace.c++");
    has(
        false,
        &l,
        "::",
        "entity.name.namespace.c++ punctuation.accessor.double-colon.c++",
    );
    has(false, &l, "b", "entity.name.namespace.c++");
    for s in ["S", "T"] {
        has(false, &l, s, "entity.name.struct.c++");
    }
    plain(false, &l, "stat");
    plain(false, &l, "st");
    has(false, &l, "E", "entity.name.enum.c++");
    has(false, &l, "U", "entity.name.union.c++");
    for t in ["node_t", "anon_t"] {
        has(false, &l, t, "entity.name.type.typedef.c++");
    }
    plain(false, &l, "node");
    plain(false, &l, "cb_t");
    plain(false, &l, "x");
    has(false, &l, "Alias", "entity.name.type.using.c++");
    plain(false, &l, "std");
}

#[test]
fn a_directive_keeps_the_statement() {
    // a parameter list across a directive, a `#define`'s body no definition
    let l = ["int f(int a,", "#define G(x) int h(x)", "      int b);"];
    has(false, &l, "b", "variable.parameter.c++");
    has(false, &l, "h", "variable.function.c++");
}

#[test]
fn bodies_after_branches_and_parameters() {
    // a struct among the parameters names no level: the body's calls stay calls
    let l = ["static void z(struct poly *p) {", "  memset_x(p);", "}"];
    has(true, &l, "memset_x", "variable.function.c");
    // each branch's header a definition, the body after them at the code's level
    let l = [
        "#if X",
        "int k(int a) {",
        "#else",
        "int k(int a, int b) {",
        "#endif",
        "  return go(a);",
        "}",
        "int m(int c);",
    ];
    has(true, &l, "go", "variable.function.c");
    has(true, &l, "m", "entity.name.function.c");
    has(true, &l, "c", "variable.parameter.c");
    // C's `extern "C" {` is a block of code; C++'s a namespace's level
    let l = ["extern \"C\" {", "int f(int a);", "}"];
    has(true, &l, "f", "variable.function.c");
    has(false, &l, "f", "entity.name.function.c++");
}

#[test]
fn macros_around_definitions() {
    let l = [
        "const STACK_OF(X509_EXTENSION) *X509_get0_extensions(const X509 *x);",
        "DECL_X(a) qux(int c);",
        "OPENSSL_MSVC_PRAGMA(warning(push, 3))",
        "#include <winsock2.h>",
        "OPENSSL_MSVC_PRAGMA(warning(pop))",
    ];
    for f in ["STACK_OF", "DECL_X", "OPENSSL_MSVC_PRAGMA"] {
        has(true, &l, f, "variable.function.c");
    }
    for f in ["X509_get0_extensions", "qux"] {
        has(true, &l, f, "entity.name.function.c");
    }
    let got = lexed(true, &l);
    assert!(
        !got.iter()
            .any(|(t, s)| t == "OPENSSL_MSVC_PRAGMA" && s.starts_with("entity")),
        "{got:#?}"
    );
}

#[test]
fn line_ends_and_nested_parameters() {
    let l = [
        "void* f1(size_t n) MI_FORWARD(go, n)",
        "void* f2(size_t n) MI_FORWARD(go, n)",
        "TEST_END",
        "TEST_BEGIN(test_x) {",
        "}",
        "static my_t",
        "bar(int b);",
        "void f(int (*cmp)(const void *a, int), const STACK_OF(X) *s, int n);",
        "Foo::Foo(int a)",
        "    : x_(a),",
        "      y_(g(a)) {}",
    ];
    for f in ["f1", "f2", "bar", "f", "Foo"] {
        has(false, &l, f, "entity.name.function.c++");
    }
    for c in ["MI_FORWARD", "TEST_BEGIN", "STACK_OF", "x_", "y_"] {
        has(false, &l, c, "variable.function.c++");
    }
    for v in ["b", "s", "n"] {
        has(false, &l, v, "variable.parameter.c++");
    }
    for p in ["test_x", "cmp", "X"] {
        plain(false, &l, p);
    }
}

#[test]
fn generics_and_templates() {
    const BEGIN: &str = "punctuation.section.generic.begin.c++";
    const END: &str = "punctuation.section.generic.end.c++";
    // a generic's shape after a name in code: template arguments, a call through them
    let l = ["void f() {", "  auto p = std::make_unique<Foo>(1);", "}"];
    has(false, &l, "<", BEGIN);
    has(false, &l, ">", END);
    has(false, &l, "make_unique", "variable.function.c++");
    // a comparison has no such end
    let l = ["void f() {", "  if (a < b) return;", "}"];
    has(false, &l, "<", "keyword.operator.comparison.c++");
    // in a declaration any `<`: a pointer's type, across lines; no call in the arguments
    let l = ["std::vector<Foo*> v;", "UniquePtr<STACK_OF(X509)> s;"];
    has(false, &l, "<", BEGIN);
    plain(false, &l, "STACK_OF");
    // a definition's name through its arguments; a type before a definition's name
    let l = [
        "template <> void foo<int>(int a);",
        "std::vector<int> g(int b);",
    ];
    has(false, &l, "foo", "entity.name.function.c++");
    has(false, &l, "g", "entity.name.function.c++");
    has(false, &l, "b", "variable.parameter.c++");
    // a template's parameters: `typename` and `class` a type's words, across lines
    let l = [
        "template <typename T,",
        "          class U = int>",
        "struct A;",
    ];
    has(false, &l, "typename", "storage.type.c++");
    has(false, &l, "class", "storage.type.c++");
    has(false, &l, ">", END);
    plain(false, &l, "U");
    // elsewhere `typename` a modifier; a cast's type; a member through `template`
    let l = [
        "typename T::type y = static_cast<int>(z);",
        "a.template get<0>();",
    ];
    has(false, &l, "typename", "storage.modifier.c++");
    has(false, &l, "<", BEGIN);
    has(false, &l, "get", "variable.function.member.c++");
    // C has no generics
    let l = ["int f(void) { return a<b>(c); }"];
    has(true, &l, "<", "keyword.operator.comparison.c");
}

#[test]
fn declared_names_calls_and_modifiers() {
    // the name a declaration declares is plain, not by its shape; used, it is
    let l = ["static const int kMax = 3;", "int y = kMax;"];
    let got = lexed(false, &l);
    let k: Vec<_> = got.iter().filter(|(t, _)| t == "kMax").collect();
    assert_eq!(k.len(), 1, "{got:#?}");
    assert_eq!(k[0].1, "constant.other.variable.mac-classic.c++");
    // C++'s `name{…}` a call's color; a base class's before a body isn't
    let l = ["struct S : Base {", "  uint8_t *secret{};", "};"];
    has(false, &l, "secret", "variable.function.c++");
    plain(false, &l, "Base");
    // `= default`, `= delete` after a definition's parameters; `decltype` a type
    let l = [
        "A::A() = default;",
        "A& operator=(const A&) = delete;",
        "decltype(x) y;",
    ];
    has(false, &l, "default", "storage.modifier.c++");
    has(false, &l, "delete", "storage.modifier.c++");
    has(false, &l, "decltype", "storage.type.c++");
    // a member named as a support type
    let l = ["x = a.Size;"];
    has(false, &l, "Size", "variable.other.readwrite.member.c++");
    // a constructor's initializer list on the line after its header
    let l = ["class A {", "  A(int a)", "      : a_(a) {}", "};"];
    has(false, &l, ":", "punctuation.separator.initializer-list.c++");
}

#[test]
fn directives_corners() {
    // `#if 1`'s `#else` branch is disabled, nested `#if`s counted; `#if 2`'s number plain
    let l = [
        "#if 1",
        "#ifdef X",
        "#else",
        "int kept;",
        "#endif",
        "#else",
        "int off;",
        "#endif",
        "#if 2",
    ];
    has(false, &l, "int", "storage.type.c++");
    has(
        false,
        &l,
        "int off;",
        "comment.block.preprocessor.if-branch.c++",
    );
    plain(false, &l, "2");
    // `#endif`'s rest outside the directive; an indented directive's indentation in it
    let l = ["#endif  // x"];
    plain(false, &l, "  ");
    has(
        false,
        &l,
        "//",
        "comment.line.double-slash.c++ punctuation.definition.comment.c++",
    );
    let got = lexed(false, &["  #ifdef Y"]);
    assert!(
        got.contains(&("  ".into(), "meta.preprocessor.c++".into())),
        "{got:#?}"
    );
    // a `#define`'s name under the directive's scope
    has(
        false,
        &["#define N 1"],
        "N",
        "meta.preprocessor.c++ entity.name.constant.preprocessor.c++",
    );
}

#[test]
fn a_column_zero_brace_closes_the_code() {
    // a `{` an `#if`'s branch leaves open: the function's `}` at a line's start closes all of
    // its code, so the next function is defined, not called
    let l = [
        "void f(void) {",
        "#if !defined X",
        "  for (;;) {",
        "#endif",
        "  }",
        "}",
        "void g(int a) {}",
    ];
    has(true, &l, "g", "entity.name.function.c");
    has(true, &l, "a", "variable.parameter.c");
}

#[test]
fn friends_trailing_returns_delete_and_pragma_mark() {
    // a friend's class names nothing; its function is a declaration's, as any member's
    let l = [
        "class A {",
        "  friend class Other;",
        "  friend void f(int n);",
        "};",
    ];
    plain(false, &l, "Other");
    has(false, &l, "f", "entity.name.function.c++");
    let l = ["enum class E { X };"];
    has(false, &l, "enum class", "storage.type.c++");
    // a trailing return type's `->` a separator, a member's an accessor
    let l = ["auto get() const -> int { return p->x; }"];
    let got = lexed(false, &l);
    let arrows: Vec<&str> = got
        .iter()
        .filter(|(t, _)| t == "->")
        .map(|(_, s)| s.as_str())
        .collect();
    assert_eq!(
        arrows,
        [
            "punctuation.separator.c++",
            "punctuation.accessor.arrow.c++"
        ]
    );
    has(
        false,
        &["  delete [] a;"],
        "delete []",
        "keyword.control.c++",
    );
    // `#pragma mark` one keyword, but not `#pragma marked`
    let l = ["#  pragma mark - Section"];
    has(true, &l, "#  pragma mark", "keyword.control.import.c");
    has(
        false,
        &["#pragma marked"],
        "#pragma",
        "keyword.control.import.c++",
    );
}

#[test]
fn a_block_keeps_what_state_keeps() {
    // what a line hands the next as `State` keeps it: an enum's `{` on the next line, two `?`s
    // open across a line, eight `(`s and four `<`s open, a class's word before its name's line
    let text = concat!(
        "enum E\n{\n  A = x ? y ? 1\n  : 2 : 3,\n};\n",
        "int v = f((((((((a,\n  b))))))));\n",
        "std::map<a<b<c<d<e, f>>>>,\n  g> m;\n",
        "class\nC\n{\n};\n",
    );
    for lang in [crate::Lang::Cpp, crate::Lang::C] {
        let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
        for l in text.split('\n') {
            crate::lex(lang, l, &mut state, &mut spans);
            want.push((spans.clone(), state));
        }
        let (mut got, mut buf) = (Vec::new(), String::new());
        let mut block = State::default();
        crate::lex_lines(
            lang,
            text,
            usize::MAX,
            &mut block,
            (&mut spans, &mut buf),
            |_, _| false,
            |_, s| got.push(s.to_vec()),
        );
        assert_eq!(block, want.last().unwrap().1, "{lang:?}");
        for (n, (g, (w, _))) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "{lang:?} line {}", n + 1);
        }
    }
}

/// Whether `text` holds a token of the regex's `leaf` (a regex scope's last part).
fn has_re(c: bool, lines: &[&str], text: &str, leaf: &str) {
    let s = if c {
        "string.quoted.double.c"
    } else {
        "string.quoted.double.c++"
    };
    let got = lexed(c, lines);
    assert!(
        got.iter()
            .any(|(t, sc)| t == text && sc.starts_with(s) && sc.ends_with(leaf)),
        "{text:?} as {leaf:?} in {got:#?}"
    );
}

/// Whether no token of `lines` is a regex's.
fn no_regex(c: bool, lines: &[&str]) {
    let got = lexed(c, lines);
    assert!(
        !got.iter().any(|(_, s)| s.contains("regexp")),
        "a regex in {got:#?}"
    );
}

#[test]
fn regexes_by_the_engines_call() {
    const ANCHOR: &str = "keyword.control.anchor.regexp";
    const CLASS: &str = "constant.other.character-class.escape.backslash.regexp";
    const GROUP: &str = "punctuation.definition.group.begin.regexp";
    const LOOK: &str = "meta.assertion.look-ahead.regexp";
    const POSIX: &str = "constant.other.character-class.posix.regexp";
    const NAME: &str = "entity.name.other.group.regexp";
    // `<regex>`: a call, a declaration, unqualified, braces; ECMAScript's lookahead
    let l = [r#"auto ok = std::regex_match(s, std::regex("^\\d+(?=x)$"));"#];
    has_re(false, &l, "^", ANCHOR);
    has_re(false, &l, r"\\d", CLASS);
    has_re(false, &l, "?=", LOOK);
    has(
        false,
        &l,
        "\"",
        "string.quoted.double.c++ punctuation.definition.string.begin.c++",
    );
    has_re(
        false,
        &[r#"static const std::wregex re{L"[a-z]+"};"#],
        "a",
        "range.regexp",
    );
    has_re(
        false,
        &[r#"regex re ("(a|b)");"#],
        "|",
        "keyword.operator.or.regexp",
    );
    // the host's escape: a character to the regex, the string's color
    has(
        false,
        &[r#"std::regex r("\"\\w\"");"#],
        "\\\"",
        "string.quoted.double.c++ constant.character.escape.c++",
    );
    // a raw string, its regex as it reads, across lines
    let l = [
        r#"const std::regex kRe(R"re(^(\w+)\s"#,
        r#"  [[:alpha:]]$)re", std::regex::icase);"#,
    ];
    has_re(false, &l, r"\w", CLASS);
    has(
        false,
        &l,
        ")re\"",
        "string.quoted.double.c++ punctuation.definition.string.end.c++",
    );
    // ECMAScript's class has no POSIX class: `[` and `:` its characters
    plain(false, &l, "[:alpha:]");
    // a flag after the pattern picks POSIX's grammar
    let l = [r#"std::regex e("a\\(b\\)\\{2,3\\}[[:digit:]\\d]", std::regex::basic);"#];
    has_re(false, &l, r"\\(", GROUP);
    has_re(false, &l, "2", "constant.literal.numeric.regexp");
    has_re(false, &l, "[:digit:]", POSIX);
    let l = [r#"std::regex e("(a|b)+[[:space:]]", std::regex_constants::extended);"#];
    has_re(false, &l, "(", GROUP);
    has_re(false, &l, "[:space:]", POSIX);
    // Boost's Perl, Qt's PCRE: Java's dialect; through a string's wrapper
    let l = [r#"boost::regex e("(?<y>\\d{4})");"#];
    has_re(false, &l, "y", NAME);
    let l = [r#"static const QRegularExpression re(QStringLiteral("^(?<n>\\w+)$"));"#];
    has_re(false, &l, "n", NAME);
    has_re(
        false,
        &[r#"auto m = QRegularExpression(u"(\\d+)"_s).match(t);"#],
        r"\\d",
        CLASS,
    );
    // not a regex: a wildcard, a fixed string, Boost's literal
    no_regex(
        false,
        &[r#"QRegExp rx("*.txt", Qt::CaseSensitive, QRegExp::Wildcard);"#],
    );
    no_regex(false, &[r#"boost::regex e("a+b", boost::regex::literal);"#]);
    // RE2: its own, a pattern second in its matchers
    let l = [r#"if (RE2::FullMatch("x", "(?P<k>\\w+)") && re2::RE2::PartialMatch(t, "\\pN")) {}"#];
    has_re(false, &l, "k", NAME);
    has_re(false, &l, r"\\p", "unicode-property.regexp");
    has_re(
        false,
        &[r#"RE2 re("a{2}");"#],
        "2",
        "constant.literal.numeric.regexp",
    );
    // CTRE's template argument
    has_re(
        false,
        &[r#"if (auto m = ctre::match<"([a-z]+)\\.?">(s)) {}"#],
        "(",
        GROUP,
    );
    has_re(
        false,
        &[r#"static constexpr auto p = ctll::fixed_string{ "h.*" };"#],
        "*",
        "quantifier.regexp",
    );
    // calls that aren't an engine's: plain strings
    no_regex(
        false,
        &[r#"printf("(%d)\n", x); foo("a+", "b*"); s.find("^");"#],
    );
    no_regex(
        false,
        &[r#"std::string s("^a+$"); f(x, "a+"); std::regex re(make(p), "a*");"#],
    );
}

#[test]
fn regexes_in_cs_calls() {
    const GROUP: &str = "punctuation.definition.group.begin.regexp";
    const POSIX: &str = "constant.other.character-class.posix.regexp";
    // POSIX's `regcomp(`: BRE, ERE with `REG_EXTENDED`; the first argument's brackets skipped
    let l = [r#"if (regcomp(&cfg[i].re, "^\\(a\\)*[[:alpha:]\\]x$", REG_NEWLINE)) return 1;"#];
    has_re(true, &l, r"\\(", GROUP);
    has_re(true, &l, "[:alpha:]", POSIX);
    // in a bracket expression a `\` is a character
    has(
        true,
        &l,
        r"\\",
        "string.quoted.double.c constant.other.character-class.set.regexp",
    );
    let l = [r#"ret = regcomp(&re, "(a|b)+$", REG_EXTENDED | REG_ICASE);"#];
    has_re(true, &l, "(", GROUP);
    has_re(true, &l, "|", "keyword.operator.or.regexp");
    // BRE's `(` and `|` are characters
    let l = [r#"regcomp(&re, "(a|b)", 0);"#];
    plain(true, &l, "|");
    has_re(true, &l, "(a|b)", "string.quoted.double.c");
    // PCRE2's, a cast before the pattern passed over
    let l = [
        r#"re = pcre2_compile((PCRE2_SPTR)"(?i)\\bfoo", PCRE2_ZERO_TERMINATED, 0, &e, &o, NULL);"#,
    ];
    has_re(true, &l, "?i", "keyword.other.option-toggle.regexp");
    no_regex(
        true,
        &[r#"pcre2_compile((PCRE2_SPTR)"a.b", 3, PCRE2_LITERAL, &e, &o, NULL);"#],
    );
    // C has no C++ engine
    no_regex(true, &[r#"regex re("a+"); RE2 r("b+");"#]);
}

#[test]
fn a_raw_regex_across_lines_in_a_block() {
    let text = "std::regex r(R\"abcdefgh((a|\n[b-c]{2}\n  )+)abcdefgh\");\nint x;\n";
    let mut state = State::default();
    let mut one = Vec::new();
    let mut spans = Vec::new();
    for l in text.lines() {
        spans.clear();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        one.push((spans.clone(), state));
    }
    let mut state = State::default();
    let mut all = Vec::new();
    let mut v = Vec::new();
    block(&mut state, &mut v, &mut |c| {
        for l in text.lines() {
            all.push(c.next(l.as_bytes()).to_vec());
        }
        None
    });
    assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
    assert_eq!(state, one.last().unwrap().1);
    let l: Vec<&str> = text.lines().collect();
    // the group opened on the first line closes on the third, the string ends there
    has_re(false, &l, ")", "punctuation.definition.group.end.regexp");
    has_re(false, &l, "2", "constant.literal.numeric.regexp");
    has(false, &l, "int", "storage.type.c++");
}

/// Each line's tokens in `lang` as (text, scopes), through [`crate::lex`] from the default
/// state, each line's spans checked as [`lexed`]'s.
fn lexed_lang(lang: crate::Lang, lines: &[&str]) -> Vec<(String, String)> {
    let mut state = State::default();
    let mut got = Vec::new();
    for l in lines {
        let mut spans: Vec<Span> = Vec::new();
        crate::lex(lang, l, &mut state, &mut spans);
        let mut end = 0;
        for s in &spans {
            assert!(
                s.start >= end && s.end > s.start && s.end as usize <= l.len(),
                "{spans:?} on {l:?}"
            );
            end = s.end;
            got.push((
                l[s.start as usize..s.end as usize].to_owned(),
                lang.scopes(s.kind).join(" "),
            ));
        }
    }
    got
}

fn has_in(lang: crate::Lang, lines: &[&str], text: &str, scopes: &str) {
    let got = lexed_lang(lang, lines);
    assert!(
        got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

fn plain_in(lang: crate::Lang, lines: &[&str], text: &str) {
    let got = lexed_lang(lang, lines);
    assert!(
        !got.iter().any(|(t, _)| t == text),
        "{text:?} not plain in {got:#?}"
    );
}

#[test]
fn objc_sends_heads_and_at_keywords() {
    use crate::Lang::ObjC;
    let l = [
        "@interface Foo : NSObject <NSCopying>",
        "@property (nonatomic, copy) NSArray<NSString *> *names;",
        "- (void)run:(void (^)(BOOL done))block NS_SWIFT_NAME(run(block:));",
        "- (instancetype)initWithName:(NSString *)name",
        "                       count:(NSInteger)count;",
        "@end",
        "@implementation Foo",
        "- (void)go {",
        "  id x = [[[Foo alloc] initWithName:@\"%@ n\" count:2] autorelease];",
        "  [self.view addConstraint:[NSLayoutConstraint constraintWithItem:x",
        "                                                        attribute:1]];",
        "  [UIView animateWithDuration:0.3 animations:^{",
        "    switch (y) { case 1: break; }",
        "  } completion:nil];",
        "  NSDictionary *d = @{ @\"k\": @1 };",
        "  return [self class];",
        "}",
        "@end",
    ];
    has_in(ObjC, &l, "Foo", "entity.name.type.objc");
    has_in(ObjC, &l, "NSObject", "entity.other.inherited-class.objc");
    has_in(
        ObjC,
        &l,
        "nonatomic",
        "keyword.other.property.attribute.objc",
    );
    has_in(ObjC, &l, "<", "punctuation.section.scope.begin.objc");
    // the method heads, a macro after one, a head over two lines
    for f in ["run", "initWithName", "go"] {
        has_in(ObjC, &l, f, "entity.name.function.objc");
    }
    has_in(
        ObjC,
        &l,
        "count",
        "entity.name.function.name-of-parameter.objc",
    );
    for v in ["block", "name"] {
        has_in(ObjC, &l, v, "variable.parameter.function.objc");
    }
    has_in(ObjC, &l, "NS_SWIFT_NAME", "variable.function.objc");
    // sends: their receivers a send, a member; parts over lines; a block's `case` its own
    for s in [
        "alloc",
        "autorelease",
        "addConstraint",
        "animateWithDuration",
        "class",
    ] {
        has_in(ObjC, &l, s, "support.function.any-method.objc");
    }
    for s in ["attribute", "completion", "animations"] {
        has_in(
            ObjC,
            &l,
            s,
            "support.function.any-method.name-of-parameter.objc",
        );
    }
    has_in(
        ObjC,
        &l,
        "%@",
        "string.quoted.double.objc constant.other.placeholder.objc",
    );
    has_in(ObjC, &l, ":", "punctuation.separator.objc");
    has_in(
        ObjC,
        &l,
        "@",
        "storage.type.objc punctuation.definition.storage.type.objc",
    );
    has_in(ObjC, &l, "NSInteger", "support.type.cocoa.leopard.objc");
    has_in(ObjC, &l, "self", "variable.language.objc");
    // C's scanner reads none of it
    plain_in(crate::Lang::C, &l, "initWithName");
}

#[test]
fn sql_in_strings() {
    use crate::Lang::{C, Cpp, ObjC};
    let c = [
        "rc = sqlite3_exec(db, \"CREATE TABLE t (\"",
        "                      \"  a INTEGER);\", NULL, NULL, &err);",
        "rc = sqlite3_prepare_v2(db,",
        "    \"DELETE FROM t\", -1, &s, 0);",
        "PQprepare(conn, \"name\", \"SELECT 1\", 0, NULL);",
        "printf(\"SELECT %d\", 1);",
    ];
    for k in ["CREATE", "DELETE", "SELECT"] {
        assert!(
            lexed_lang(C, &c)
                .iter()
                .any(|(t, s)| t == k && s.ends_with(".sql") && s.starts_with("string.quoted")),
            "{k}"
        );
    }
    has_in(C, &c, "INTEGER", "string.quoted.double.c storage.type.sql");
    has_in(C, &c, "SELECT ", "string.quoted.double.c");
    has_in(C, &c, "name", "string.quoted.double.c");
    let cpp = [
        "q.exec(R\"(",
        "  SELECT a -- x",
        "  FROM t)\");",
        "QSqlQuery query(\"UPDATE t SET a = 1\");",
        "proc.exec(\"ls\");",
    ];
    has_in(
        Cpp,
        &cpp,
        "FROM",
        "string.quoted.double.c++ keyword.other.DML.sql",
    );
    has_in(
        Cpp,
        &cpp,
        "UPDATE",
        "string.quoted.double.c++ keyword.other.DML.sql",
    );
    has_in(Cpp, &cpp, "ls", "string.quoted.double.c++");
    let m = [
        "[db executeQuery:@\"SELECT * FROM t\"];",
        "[s rangeOfString:@\"a+\"];",
    ];
    has_in(
        ObjC,
        &m,
        "SELECT",
        "string.quoted.double.objc keyword.other.DML.sql",
    );
    has_in(ObjC, &m, "a+", "string.quoted.double.objc");
    let re = ["[NSRegularExpression regularExpressionWithPattern:@\"^a+$\" options:0 error:nil];"];
    has_in(
        ObjC,
        &re,
        "+",
        "string.quoted.double.objc keyword.operator.quantifier.regexp",
    );
}

/// `text`'s lines lexed one at a time and as one block in `lang` agree, spans and state.
fn block_as_lines(lang: crate::Lang, text: &str) {
    let mut state = State::default();
    let mut one = Vec::new();
    let mut spans = Vec::new();
    for l in text.lines() {
        spans.clear();
        crate::lex(lang, l, &mut state, &mut spans);
        one.push((spans.clone(), state));
    }
    let mut state = State::default();
    let mut all = Vec::new();
    let mut v = Vec::new();
    let block = match lang {
        crate::Lang::ObjC => block_objc,
        crate::Lang::ObjCpp => block_objcpp,
        crate::Lang::C => block_c,
        _ => block,
    };
    block(&mut state, &mut v, &mut |c| {
        for l in text.lines() {
            all.push(c.next(l.as_bytes()).to_vec());
        }
        None
    });
    for (i, (a, (o, _))) in all.iter().zip(&one).enumerate() {
        assert_eq!(a, o, "line {} of {lang:?}", i + 1);
    }
    assert_eq!(state, one.last().unwrap().1, "{lang:?}");
}

#[test]
fn objc_blocks_lex_as_their_lines() {
    // frames past the kept three (a send in a block in a send in a block…), a head over lines,
    // SQL over lines, a raw SQL string over lines, braces past Objective-C's 52 levels
    let deep = "{".repeat(60);
    let text = format!(
        "- (void)a:(int)x\n  b:(int)y {{\n  [a b:^{{ [c d:^{{ [e f:^{{ [g h:1\n  i:2]; }}]; }}]; }}];\n\
         [x y:[z w:[q r:[s t\n   u:1]]]];\n  sqlite3_exec(db, \"SELECT a \"\n   \"FROM t\", 0, 0, 0);\n\
         [db executeQuery:\n  @\"SELECT 1\"];\n}}\n{deep}\nx;\n"
    );
    for lang in [crate::Lang::ObjC, crate::Lang::ObjCpp] {
        block_as_lines(lang, &text);
    }
    block_as_lines(
        crate::Lang::ObjCpp,
        "q.exec(R\"x(\n SELECT -- a\n FROM t)x\");\n",
    );
    block_as_lines(
        crate::Lang::C,
        "sqlite3_prepare_v2(db,\n \"SELECT\"\n \" 1\", -1, &s, 0);\n",
    );
}
