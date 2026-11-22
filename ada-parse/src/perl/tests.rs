//! Perl's scanner's tests (X-170): its readings, each departure from bat's grammar among them, the
//! state across lines, a block against its lines, linear time on long lines.

use super::*;
use crate::Lang;
use crate::tests::marked;

/// Each line of `text` lexed alone from the state the one before left: its spans (checked in
/// order, on the line) and the state after it.
fn lines(text: &str) -> Vec<(Vec<crate::Span>, State)> {
    let mut state = State::default();
    let mut out = Vec::new();
    for l in text.lines() {
        let mut spans = Vec::new();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        let mut at = 0;
        for s in &spans {
            assert!(
                s.start as usize >= at && s.end >= s.start && s.end as usize <= l.len(),
                "{l:?}: {spans:?}"
            );
            at = s.end as usize;
        }
        out.push((spans, state));
    }
    out
}

fn of(lines: &[&str]) -> Vec<(String, String)> {
    marked(Lang::Perl, lines)
        .into_iter()
        .map(|(t, s)| (t.to_owned(), s.to_owned()))
        .collect()
}

fn has(lines: &[&str], text: &str, scopes: &str) {
    let got = of(lines);
    assert!(
        got.iter().any(|(t, s)| t == text && s == scopes),
        "{text:?} as {scopes:?} in {got:#?}"
    );
}

/// `text` has no span of a scope holding `scope`.
fn lacks(lines: &[&str], text: &str, scope: &str) {
    let got = of(lines);
    assert!(
        !got.iter().any(|(t, s)| t == text && s.contains(scope)),
        "{text:?} not {scope:?} in {got:#?}"
    );
}

/// `text` is plain: no span covers it.
fn plain(lines: &[&str], text: &str) {
    let got = of(lines);
    assert!(
        !got.iter().any(|(t, _)| t == text),
        "{text:?} plain in {got:#?}"
    );
}

const SQ: &str = "string.quoted.single.perl";
const DQ: &str = "string.quoted.double.perl";
const RE: &str = "string.regexp.perl";
const UNQ: &str = "string.unquoted.perl";
const HD: &str = "string.unquoted.heredoc.perl";
const VAR: &str = "variable.other.readwrite.perl";
const KEY: &str = "constant.other.key.perl";

/// `/` a regex where a term starts, a divide after a value; bat reads a regex after any
/// bareword and runs it on across lines.
#[test]
fn slash_by_what_came_before() {
    let l = [
        "my $x = $y / 2; $h{key} / 2; ($a + 1) / 3; foo / 2;",
        "split /,/, $line; grep { /x/ } @l; return /y/ ? 1 : 0;",
        "if ($s =~ /z/i) { print \"ok\" }",
        "my $d = shift // 'none'; $n //= 1; $m /= 2;",
    ];
    for d in ["/", "/="] {
        has(
            &l,
            d,
            if d == "/" {
                "keyword.operator.arithmetic.perl"
            } else {
                "keyword.operator.assignment.perl"
            },
        );
    }
    has(&l, ",", RE);
    has(&l, "x", RE);
    has(&l, "y", RE);
    has(&l, "z", RE);
    has(&l, "i", "constant.language.flags.regexp.perl");
    has(&l, "//", "keyword.operator.logical.perl");
    has(&l, "//=", "keyword.operator.assignment.perl");
}

/// A quote-like operator's word read as a quote only where Perl reads one: not a hash's key,
/// a method, a variable or a subscript's word.
#[test]
fn quote_like_words_as_names() {
    let l = [
        "my %h = (q => 1, s => 2, y => 3); $h{s}{y} = 1; $h{ q } = 2;",
        "$obj->s(1); $obj->y; my $s = $q; sub tr { }",
        "print -s $file; $x = -e $f;",
    ];
    has(&l, "q", KEY);
    has(&l, "s", KEY);
    has(&l, "y", KEY);
    has(&l, "s", "variable.function.member.perl");
    has(&l, "y", "variable.function.member.perl");
    has(&l, "tr", "entity.name.function.perl");
    has(&l, "-s", "keyword.operator.filetest.perl");
    has(&l, "-e", "keyword.operator.filetest.perl");
}

/// Quote-like constructs: their delimiters, brackets nesting, across lines, `s` and `tr`'s two
/// parts, a comment between a bracketed `s`'s parts, the flags, `s///e`'s code.
#[test]
fn quote_like_constructs() {
    let l = [
        "my @w = qw(a b",
        "  c d);",
        "my $q = q{a {nested} b}; my $qq = qq[x $y];",
        "s{foo}  # a comment",
        " {bar}gx;",
        "tr/a-z/A-Z/; y/a/b/r;",
        "$n =~ s/(\\d+)/$1 + 1/ge;",
        "my $re = qr#^a+#i; m'no $interp';",
        "q #not a delimiter after a blank",
        "(x);",
    ];
    has(&l, "a b", UNQ);
    has(&l, "  c d", UNQ);
    has(&l, "a {nested} b", UNQ);
    has(&l, " a comment", "comment.line.number-sign.perl");
    has(&l, "bar", UNQ);
    has(&l, "gx", "constant.language.flags.regexp.perl");
    has(&l, "a-z", UNQ);
    has(&l, "+", "keyword.operator.arithmetic.perl");
    has(&l, "ge", "constant.language.flags.regexp.perl");
    has(&l, "#", "punctuation.section.generic.begin.perl");
    // a `'`-delimited regex doesn't interpolate: `$` an anchor
    lacks(&l, "interp", VAR);
    has(
        &l,
        "not a delimiter after a blank",
        "comment.line.number-sign.perl",
    );
    has(&l, "x", UNQ);
}

/// Strings: a literal one's only escapes are `\\` and `\'`; an interpolating one's variables,
/// subscripts (their code lexed), `${…}`, `@{[…]}`, `->[…]`; placeholders; no hash interpolated.
#[test]
fn strings() {
    let l = [
        r"my $t = 'It\'s \n';",
        r#"my $s = "a $h{key}[0]->{x} @arr ${name} @{[ $n + 1 ]} $obj->method %h %s %.2f\n";"#,
        r#"print "$1 $& $$ $x::y";"#,
    ];
    has(&l, r"\'", &format!("{SQ} constant.character.escape.perl"));
    has(&l, r"s \n", SQ);
    has(&l, "key", KEY);
    has(&l, "0", "constant.numeric.integer.decimal.perl");
    has(&l, "x", KEY);
    has(&l, "arr", VAR);
    has(&l, "name", UNQ);
    has(&l, "n", VAR);
    has(&l, "+", "keyword.operator.arithmetic.perl");
    has(&l, "->method %h ", DQ);
    has(
        &l,
        "%",
        &format!("{DQ} constant.other.placeholder.perl punctuation.definition.placeholder.perl"),
    );
    has(&l, "s", &format!("{DQ} constant.other.placeholder.perl"));
    has(
        &l,
        ".",
        &format!("{DQ} constant.other.placeholder.perl punctuation.separator.decimal.perl"),
    );
    has(&l, r"\n", &format!("{DQ} constant.character.escape.perl"));
    has(&l, "1", "variable.language.regexp.match-group.perl");
    has(&l, "&", "variable.language.regexp.match.perl");
    has(&l, "y", VAR);
}

/// Here-documents: `<<"EOT"`, `<<'EOT'`, `<<EOT`, `<<~EOT` with an indented closer, two on a
/// line, labels Perl takes that bat doesn't; a shift after a value; SQL's and HTML's.
#[test]
fn here_documents() {
    let l = [
        "print <<~EOT;",
        "    indented $x",
        "    EOT",
        "print <<\"A\", <<'B';",
        "a $y",
        "A",
        "b $z",
        "B",
        "my $v = 1 <<EOT;",
        "my $t = <<'#3...';",
        "text",
        "#3...",
        "$dbh->do(<<SQL);",
        "SELECT a FROM t -- c",
        "SQL",
        "my $h = <<HTML;",
        "<b>$name</b>",
        "HTML",
        "print OUT2 <<EOM; my $half = PI / 2;",
        "a mail",
        "EOM",
        "print $fh <<\"EOH\";",
        "to a handle",
        "EOH",
        "print {$out} <<EOB;",
        "braced handle",
        "EOB",
        "my $s = $x << 2; $s = $y <<= 1;",
        "$s->name(\\<<'');",
        "SELECT empty label",
        "",
        "1;",
    ];
    has(&l, "<<~", "keyword.operator.heredoc.perl");
    has(&l, "    indented ", HD);
    has(&l, "x", VAR);
    has(
        &l,
        "EOT",
        "meta.tag.heredoc.perl entity.name.tag.heredoc.plain.perl",
    );
    has(&l, "y", VAR);
    lacks(&l, "z", "variable");
    has(&l, "b $z", HD);
    has(&l, "<<", "keyword.operator.bitwise.perl");
    has(&l, "text", HD);
    has(
        &l,
        "#3...",
        "meta.tag.heredoc.perl entity.name.tag.heredoc.plain.perl",
    );
    has(&l, "SELECT", "meta.embedded.perl keyword.other.DML.sql");
    has(
        &l,
        "SQL",
        "meta.tag.heredoc.perl entity.name.tag.heredoc.sql.perl",
    );
    has(&l, "name", VAR);
    has(&l, "<b>", "meta.embedded.perl");
    has(&l, "1", "constant.numeric.integer.decimal.perl");
    has(&l, "a mail", HD);
    // `print $fh <<"EOH"`: the scalar is the filehandle, `<<` a here-document; after any other
    // value a shift
    has(&l, "to a handle", HD);
    has(&l, "braced handle", HD);
    has(&l, "<<=", "keyword.operator.assignment.perl");
    has(&l, "<<", "keyword.operator.bitwise.perl");
    has(&l, "/", "keyword.operator.arithmetic.perl");
    has(&l, "SELECT empty label", HD);
}

/// SQL in DBI's calls' first argument, its strings each SQL; not past the first `,`.
#[test]
fn sql_in_dbi_calls() {
    let l = [
        "my $sth = $dbh->prepare(\"SELECT id FROM t WHERE x = ?\" . 'AND y = ?', 'SELECT not');",
        "$dbh->selectall_arrayref(q{UPDATE t SET a = 1});",
        "$obj->other('SELECT plain');",
    ];
    has(&l, "SELECT", &format!("{DQ} keyword.other.DML.sql"));
    has(&l, "AND", &format!("{SQ} keyword.operator.logical.sql"));
    has(&l, "UPDATE", &format!("{UNQ} keyword.other.DML.sql"));
    has(&l, "SELECT not", SQ);
    has(&l, "SELECT plain", SQ);
}

/// Regexes on the regex scanner, Perl's variables interpolated (`\$` the regex's), `/x`'s
/// comments, a regex across lines extended.
#[test]
fn regexes() {
    let l = [
        r"$s =~ m/^a\$b$c\d+$/;",
        "my $re = qr{",
        "  (\\w+)  # a word",
        "  \\s*",
        "}x;",
        "$s =~ /a#b/;",
    ];
    has(
        &l,
        r"\$",
        &format!("{RE} constant.character.escape.backslash.regexp"),
    );
    has(&l, "c", VAR);
    has(
        &l,
        "# a word",
        &format!("{RE} comment.line.number-sign.regexp"),
    );
    lacks(&l, "#b", "comment");
}

/// Variables: special ones, package paths, dereferences, `$#array`, `${^NAME}`, postfix
/// dereferences; `@a` not `$a`'s special.
#[test]
fn variables() {
    let l = [
        "my @l = @$r; my %h = %{$r}; my $n = $#array + $#{$r} + ${^MATCH};",
        "local $_ = $ENV{HOME}; @ARGV; $0; $a <=> $b; @a; $Foo::Bar::x; $::y;",
        "my @all = $r->@*; my $last = $r->$#*; &foo(1); \\&bar; *STDOUT;",
    ];
    has(&l, "_", "variable.language.perl");
    has(&l, "ENV", "variable.language.perl");
    has(&l, "ARGV", "variable.language.perl");
    has(&l, "{^MATCH}", "variable.language.perl");
    has(&l, "a", "variable.language.perl");
    // `@a` isn't `$a`'s special variable
    has(&l, "a", VAR);
    has(&l, "Bar", "variable.namespace.perl");
    has(&l, "array", VAR);
    has(&l, "@*", "keyword.operator.dereference.perl");
    has(&l, "$#*", "keyword.operator.dereference.perl");
    has(&l, "foo", "variable.function.perl");
    has(&l, "STDOUT", VAR);
}

/// Subs: names, callbacks, phases, signatures' parameters (bat's invalid), prototypes,
/// attributes with their arguments as code; packages, `use`'s meta scope, versions.
#[test]
fn declarations() {
    let l = [
        "package My::Class 1.02;",
        "use strict; use v5.36; use Foo::Bar qw(a b);",
        "sub new { } sub DESTROY { } BEGIN { }",
        "sub f($self, $x = 1, @rest) { }",
        "sub g($$;@) { }",
        "sub h :lvalue :Path('a') { } sub p :prototype($;@) { }",
        "package # hide from PAUSE",
        "  Hidden::Pkg;",
    ];
    has(&l, "Class", "entity.name.namespace.perl");
    has(&l, "1", "constant.numeric.version.perl");
    has(&l, " ", "meta.preprocessor.use.perl");
    has(
        &l,
        "strict",
        "meta.preprocessor.use.perl entity.name.namespace.perl",
    );
    has(
        &l,
        "v",
        "constant.numeric.version.perl punctuation.definition.version.perl",
    );
    has(&l, "new", "entity.name.function.perl");
    has(&l, "DESTROY", "entity.name.function.callback.perl");
    has(&l, "BEGIN", "entity.name.function.prepocessor.perl");
    has(&l, "$self", "variable.parameter.perl");
    has(&l, "@rest", "variable.parameter.perl");
    has(&l, "$$", "variable.parameter.perl");
    has(&l, ";", "punctuation.separator.sequence.perl");
    has(&l, "lvalue", "variable.annotation.attribute.perl");
    has(&l, "a", "string.quoted.single.perl");
    has(&l, "$;@", "variable.parameter.perl");
    has(&l, "Pkg", "entity.name.namespace.perl");
}

/// Keywords, calls, constants, labels, a block's or a hash's `{`, word operators.
#[test]
fn statements() {
    let l = [
        "LINE: while (my $l = <$fh>) { next LINE if $l eq ''; last }",
        "foreach my $x (@l) { print STDERR $x x 2 } goto DONE;",
        "croak \"bad\"; explain \\@x; my $t = ArrayRef[Int]; return SUCCESS;",
        "my $h = { a => 1 }; my @v = (HashRef, 1);",
        "Foo::Bar->new; Foo::bar(); CORE::say 'x'; __PACKAGE__->m;",
    ];
    has(&l, "LINE", "entity.name.label.perl");
    has(&l, "LINE", "variable.label.perl");
    has(&l, "DONE", "variable.label.perl");
    has(&l, "fh", VAR);
    has(
        &l,
        "<",
        "string.quoted.angle.perl punctuation.definition.string.begin.perl",
    );
    has(&l, "eq", "keyword.operator.logical.perl");
    has(&l, "STDERR", "constant.language.filehandle.perl");
    has(&l, "x", "keyword.operator.arithmetic.perl");
    has(&l, "croak", "variable.function.perl");
    has(&l, "explain", "variable.function.perl");
    has(&l, "ArrayRef", "variable.function.perl");
    has(&l, "SUCCESS", "constant.other.perl");
    has(&l, "HashRef", "constant.other.perl");
    has(&l, "a", KEY);
    has(&l, "Bar", "variable.namespace.perl");
    has(&l, "bar", "variable.function.perl");
    has(&l, "say", "support.function.perl");
    has(&l, "__PACKAGE__", "constant.language.perl");
}

/// POD: commands, formatting codes (nested, `C<< … >>`), verbatim lines without them, `=cut`;
/// `__END__`'s text, POD in it.
#[test]
fn pod_and_data() {
    let l = [
        "=head1 NAME",
        "",
        "Foo - B<bold I<italic>> C<< $a <=> $b >> L<Foo::Bar> E<gt> F<file>",
        "",
        "    verbatim B<not bold>",
        "",
        "so C<4",
        "* 5> wraps, and",
        "  an indented line B<goes on>",
        "",
        "=cut",
        "",
        "1;",
        "__END__",
        "data # not a comment",
        "=pod",
        "",
        "X<index>",
        "",
        "=cut",
        "more data",
    ];
    has(
        &l,
        "=head1",
        "comment.block.documentation.perl entity.name.tag.pod.perl",
    );
    has(
        &l,
        " NAME",
        "comment.block.documentation.perl markup.heading.perl",
    );
    has(
        &l,
        "bold ",
        "comment.block.documentation.perl markup.bold.perl",
    );
    has(
        &l,
        "italic",
        "comment.block.documentation.perl markup.italic.perl",
    );
    has(
        &l,
        " $a <=> $b ",
        "comment.block.documentation.perl markup.quote.perl",
    );
    has(
        &l,
        "Foo::Bar",
        "comment.block.documentation.perl markup.underline.link.perl",
    );
    has(
        &l,
        "gt",
        "comment.block.documentation.perl constant.character.escape.perl",
    );
    has(
        &l,
        "file",
        "comment.block.documentation.perl string.unquoted.perl",
    );
    has(
        &l,
        "    verbatim B<not bold>",
        "comment.block.documentation.perl",
    );
    has(
        &l,
        "* 5",
        "comment.block.documentation.perl markup.quote.perl",
    );
    has(&l, " wraps, and", "comment.block.documentation.perl");
    has(
        &l,
        "goes on",
        "comment.block.documentation.perl markup.bold.perl",
    );
    has(&l, "__END__", "constant.language.perl");
    plain(&l, "data # not a comment");
    has(
        &l,
        "index",
        "comment.block.documentation.perl entity.name.label.perl",
    );
    plain(&l, "more data");
}

/// `format`'s pictures to a `.` line.
#[test]
fn formats() {
    let l = [
        "format STDOUT =",
        "@<<<<<< @>>>>>",
        "$name, $value",
        ".",
        "print 1;",
    ];
    has(&l, "format", "storage.type.format.perl");
    has(
        &l,
        "<<<<<<",
        "string.unquoted.perl constant.other.placeholder.text.perl",
    );
    has(&l, "name", VAR);
    has(&l, ".", "punctuation.terminator.format.perl");
    has(&l, "print", "support.function.perl");
}

/// Openers at a line's end: a quote-like operator's delimiter on the next line, a string, a
/// regex, POD at a line's start only.
#[test]
fn openers_at_a_lines_end() {
    let l = [
        "my @w = qw",
        "  # a comment between",
        "  (a b);",
        "my $s = \"open",
        "still $x\";",
        "my $z = $x",
        "  = 1;",
        "my $r = m{",
        "}",
        ";",
    ];
    has(&l, " a comment between", "comment.line.number-sign.perl");
    has(&l, "a b", UNQ);
    has(&l, "still ", DQ);
    has(&l, "x", VAR);
    has(&l, "=", "keyword.operator.assignment.perl");
}

/// Frames past the levels kept, of every kind, and back.
#[test]
fn deep_frames() {
    let open = "{ ( [ $h{ @{ sub { map { (";
    let close = ") } } } } ] ) }";
    let mut text = String::new();
    for _ in 0..8 {
        text.push_str(open);
        text.push('\n');
    }
    text.push_str("print 'x';\n");
    for _ in 0..8 {
        text.push_str(close);
        text.push('\n');
    }
    let out = lines(&text);
    assert!(out.iter().any(|(_, s)| s.depth > LEVELS));
    assert_eq!(out.last().unwrap().1.depth, 0);
}

/// A block lexes exactly as its lines one at a time, with every field the state keeps set
/// across lines past its caps: frames, modes, a construct open, two here-documents waiting, a
/// `use` statement, a DBI call's argument.
#[test]
fn a_block_lexes_as_its_lines() {
    let mut text =
        String::from("use constant {\n  A => 1,\n};\n$dbh->prepare(\n'SELECT a\n FROM t');\n");
    // guests' bodies (X-191): their state saved beside Perl's
    text.push_str("{ print <<HTML;\n<b a=\"$x\">\n<!-- c\nHTML\n}\n=for html <i>\nj</i>\n\n=cut\n");
    for i in 0..40 {
        text.push_str(["f({\n", "@{[ (\n", "x = [\n", "${ \\ (\n"][i % 4]);
    }
    text.push_str("my @w = qw(a\nb); s{a}\n# c\n{b}; m{\na # b\n}x;\n");
    text.push_str("print <<A, <<\"B\";\na $x\nA\nb\nB\nmy $q = q\n(x);\nfor my $i\n(@l) { }\n");
    text.push_str("sub f\n{ }\nformat =\n@<<\n.\n=pod\n\nB<x>\n=cut\n");
    for _ in 0..40 {
        text.push_str(")]}\n");
    }
    text.push_str("__END__\ndata\n=head1 X\n=cut\nmore\n");
    let one = lines(&text);
    let mut state = State::default();
    let mut all = Vec::new();
    let mut v = Vec::new();
    block(&mut state, &mut v, &mut |c| {
        for l in text.lines() {
            all.push(c.next(l.as_bytes()).to_vec());
        }
        None
    });
    for (i, (l, (s, _))) in text.lines().zip(&one).enumerate() {
        assert_eq!(&all[i], s, "line {} {l:?}", i + 1);
    }
    assert_eq!(state, one.last().unwrap().1);
    let mode = |s: &State| (s.arg & ARG_MODE) as u8;
    assert!(
        one.iter().any(|(_, s)| s.depth > LEVELS),
        "past the levels kept"
    );
    for m in [M_POD, M_DATA, M_QUOTE, M_HEREDOC, M_FORMAT, M_QPEND] {
        assert!(one.iter().any(|(_, s)| mode(s) == m), "mode {m}");
    }
    assert!(
        one.iter().any(|(_, s)| (s.arg >> ARG_QLEN) & 3 == 2),
        "two here-documents"
    );
    assert!(one.iter().any(|(_, s)| s.withs[11] != 0), "a use statement");
    assert!(
        one.iter().any(|(_, s)| s.withs[12] & 0x80 != 0),
        "a guest's body"
    );
    assert!(
        one.iter().any(|(_, s)| s.arg & ARG_FOR != 0),
        "for's condition"
    );
}

/// The seeding's findings (X-170): a bracketed `s`'s replacement opener past ASCII (the scan
/// stood inside a character), a `use` statement's fill behind a string's interpolated code that
/// closed below it, a `use` before `::` or before a word and `=>` (the loop stood still).
#[test]
fn seeding_findings() {
    for l in [
        "  s[  0.000000] \u{fe}cnt (A:12",
        "use x { ( \"${ }) }\" ); $y;",
        "use constant ( \"@{[ ) ]}\" ) . 1",
        "use ::-moz-ce and binary forms",
        "use Comp=>er\\Config;",
        "require ::x; no Foo=>1;",
        "$dbh->do(q{SELECT \\é});",
    ] {
        let _ = lines(l);
        let _ = of(&[l]);
    }
}

/// Each construct repeated on one long line lexes in linear time (the guide's mistake 17).
#[test]
fn long_lines_are_linear() {
    for unit in [
        "a, ",
        "f(a, ",
        "$x / ",
        "/a/ ",
        "< x ",
        "<<\"",
        "<< ",
        "\"$x{$x{",
        "\"$x[$x[",
        "\"${\\ \"${",
        "\"@{[",
        "qw(",
        "s{a}{",
        "q{{",
        "$h{s",
        "-e ",
        "B<",
        "%-5.2f %",
        "$dbh->do('SELECT ",
        "foo bar ",
        "sub f :a(",
        "#",
        "} <<x ",
        "{ } <<x ",
        "print $f <<x ",
    ] {
        let line = unit.repeat(20_000);
        let t = std::time::Instant::now();
        let _ = of(&[&line]);
        assert!(t.elapsed().as_secs() < 2, "{unit:?}: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let _ = of(&["=pod", &line]);
        assert!(t.elapsed().as_secs() < 2, "POD {unit:?}: {:?}", t.elapsed());
    }
    // a guest's body's line (X-191), on both paths: its interpolations, escapes, the guest's
    // own constructs
    for unit in ["$x{$y} <b a=\"", "\\\" @{[ ", "<!-- ", "`${", "${x}{"] {
        let line = unit.repeat(20_000);
        for head in ["my $h = <<HTML;", "print <<JS;", "=for html"] {
            let t = std::time::Instant::now();
            let _ = of(&[head, &line]);
            let _ = guests(&[head, &line]);
            assert!(
                t.elapsed().as_secs() < 4,
                "{head} {unit:?}: {:?}",
                t.elapsed()
            );
        }
    }
}

/// `lines` on the run path with a theme that keeps guests (X-191): each colored run's text, its
/// language and its scopes' last; the span path beside it leaves the same state.
fn guests(lines: &[&str]) -> Vec<(String, Lang, String)> {
    crate::tests::guest_runs(Lang::Perl, lines)
}

/// Whether `text` is a run of `lang`'s, its last scope `leaf` where one is given.
fn run_in(got: &[(String, Lang, String)], text: &str, lang: Lang, leaf: Option<&str>) {
    assert!(
        got.iter()
            .any(|(t, l, f)| t == text && *l == lang && leaf.is_none_or(|x| f == x)),
        "{text:?} in {lang:?} {leaf:?}: {got:#?}"
    );
}

/// Here-documents whose labels name HTML, CSS, JavaScript, JSON or XML, and POD's `=begin`
/// regions and `=for` paragraphs, on their own scanners on the run path (X-191); an
/// interpolating body's escapes and variables Perl's among the guest's, which reads them
/// blanked (`<body$id` HTML's `body` tag); a `%` the guest's; Perl again after the closer.
#[test]
fn guests_in_their_languages() {
    let l = [
        "my $h = <<HTML;",
        "<body$id class=\"a\">",
        "<p>$x->{k} \\$5 100%</p>",
        "HTML",
        "my $c = <<'CSS';",
        "a { color: red; }",
        "CSS",
        "print <<EOJS . 1;",
        "var a = 1; /* open",
        "x */ var b = $y;",
        "EOJS",
        "my $j = <<~JSON_TXT;",
        "    {\"key\": 1}",
        "    JSON_TXT",
        "my $m = <<XML;",
        "<doc a=\"1\"/>",
        "XML",
        "=begin html",
        "",
        "<i>pod</i>",
        "",
        "=end html",
        "",
        "=for html <img src=\"a.png\">",
        "<br>",
        "",
        "Text B<bold>.",
        "",
        "=cut",
        "1;",
    ];
    let got = guests(&l);
    let (html, perl) = (Lang::Html, Lang::Perl);
    run_in(&got, "body", html, None);
    run_in(&got, "id", perl, Some("variable.other.readwrite.perl"));
    run_in(&got, "class", html, None);
    run_in(&got, "p", html, None);
    run_in(&got, "x", perl, Some("variable.other.readwrite.perl"));
    run_in(&got, "->", perl, None);
    run_in(&got, "\\$", perl, Some("constant.character.escape.perl"));
    assert!(
        !got.iter().any(|(t, l, _)| t.contains('%') && *l == perl),
        "a guest's % its own: {got:#?}"
    );
    run_in(
        &got,
        "HTML",
        perl,
        Some("entity.name.tag.heredoc.html.perl"),
    );
    run_in(&got, "color", Lang::Css, None);
    run_in(&got, "CSS", perl, Some("entity.name.tag.heredoc.css.perl"));
    run_in(&got, "var", Lang::JavaScript, None);
    // the comment across the body's lines: the guest's state carried
    run_in(&got, "x ", Lang::JavaScript, Some("comment.block.js"));
    run_in(&got, "y", perl, Some("variable.other.readwrite.perl"));
    run_in(&got, "1", perl, None);
    run_in(&got, "key", Lang::Json, None);
    run_in(&got, "doc", Lang::Xml, None);
    run_in(&got, "=begin", perl, Some("entity.name.tag.pod.perl"));
    run_in(
        &got,
        "html",
        perl,
        Some("constant.other.language-name.perl"),
    );
    run_in(&got, "i", html, None);
    run_in(&got, "=end", perl, Some("entity.name.tag.pod.perl"));
    run_in(&got, "img", html, None);
    run_in(&got, "br", html, None);
    // `=for`'s paragraph over at the blank line: POD again
    run_in(&got, "bold", perl, Some("markup.bold.perl"));
    run_in(&got, "=cut", perl, None);
    // the span path: Perl's text, its interpolations Perl's, no placeholder in a guest's text
    has(&l, "<body", "meta.embedded.perl");
    has(&l, "id", VAR);
    lacks(&l, "%<", "placeholder");
    has(
        &l,
        "html",
        "comment.block.documentation.perl constant.other.language-name.perl",
    );
    has(&l, "<i>pod</i>", "comment.block.documentation.perl");
}

/// Where Perl's fields don't fit beside a guest's state, the body is Perl's text (X-191): six
/// frames open, a second here-document after it, a DBI call's argument; five frames fit. A
/// guest's line that sets Perl's bytes of the state loses the guest's state, not Perl's: the
/// body's lines each from a fresh one, its closer found, Perl's frames back after it.
#[test]
fn guests_that_do_not_fit() {
    let deep = [
        "{ { { { { { my $h = <<HTML;",
        "<b>six</b>",
        "HTML",
        "} } } } } }",
        "{ { { { { my $h = <<HTML;",
        "<i>five</i>",
        "HTML",
        "} } } } } $x;",
        "print <<HTML, <<EOT;",
        "<u>two</u>",
        "HTML",
        "t",
        "EOT",
    ];
    let got = guests(&deep);
    run_in(&got, "<b>six</b>", Lang::Perl, Some("meta.embedded.perl"));
    run_in(&got, "i", Lang::Html, None);
    run_in(
        &got,
        "}",
        Lang::Perl,
        Some("punctuation.section.block.end.perl"),
    );
    run_in(&got, "<u>two</u>", Lang::Perl, Some("meta.embedded.perl"));
    let nested = "[".repeat(25);
    let lost = [
        "{ { sub f { return <<'JS'; }",
        nested.as_str(),
        "var a = 1;",
        "JS",
        "} } $z;",
    ];
    let got = guests(&lost);
    run_in(&got, "var", Lang::JavaScript, None);
    run_in(&got, "JS", Lang::Perl, None);
    run_in(&got, "z", Lang::Perl, Some("variable.other.readwrite.perl"));
    let st = lines(&lost.join("\n"));
    assert_eq!(st[1].1.withs[12] & 0x80, 0x80, "a body's layout");
    let v = u64::from_le_bytes(st[1].1.withs[6..14].try_into().unwrap());
    assert_ne!(v & G_LOST, 0, "the guest's state lost");
    assert_eq!(st[4].1.depth, 0, "Perl's frames back");
}

/// A block on the run path, cut into blocks of a few lines (a body's state saved and loaded
/// between them), lexes as its lines do one at a time: runs and states (X-191).
#[test]
fn guest_blocks_lex_as_their_lines() {
    let text = "{ my $h = <<\"HTML\";\n<p a=\"$x\">\n<!-- c\n-->$y</p>\nHTML\n}\n\
        =for html <b>\nx</b>\n\n=begin json\n{\"a\":\n1}\n=end json\n\n=cut\n\
        print <<'EOJS';\nvar s = `a\n${b}`;\nEOJS\n1;";
    use std::cell::RefCell;
    use std::rc::Rc;
    let names = Rc::new(RefCell::new(Vec::new()));
    let mut rec = crate::tests::Rec::new(Lang::Perl, Rc::clone(&names));
    let (mut want, mut st, mut spans, mut runs) =
        (Vec::new(), State::default(), Vec::new(), Vec::new());
    for l in text.lines() {
        runs.clear();
        crate::lex_runs(Lang::Perl, l, &mut st, &mut spans, &mut rec, &mut runs);
        want.push((runs.clone(), st));
    }
    assert!(want.iter().any(|(_, s)| s.withs[12] & 0x80 != 0));
    for max in [1, 2, 3, 5, usize::MAX] {
        let (mut st, mut buf, mut ends, mut runs) =
            (State::default(), String::new(), Vec::new(), Vec::new());
        let mut rest = Some(text);
        let mut n = 0;
        while let Some(t) = rest {
            runs.clear();
            ends.clear();
            rest = crate::lex_lines_runs(
                Lang::Perl,
                t,
                max,
                &mut st,
                (&mut spans, &mut buf),
                None,
                &mut rec,
                &mut runs,
                &mut ends,
            )
            .map(|p| &t[p..]);
            let mut from = 0;
            for &e in &ends {
                let e = e as usize;
                assert_eq!(runs[from..e], want[n].0, "block of {max}: line {}", n + 1);
                (from, n) = (e, n + 1);
            }
            assert_eq!(st, want[n - 1].1, "block of {max}: state after line {n}");
        }
        assert_eq!(n, want.len());
    }
}
