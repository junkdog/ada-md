//! SQL by a scanner of ada's own (ADR-40, speed first), emitting bat's scopes, with bat's
//! `SQL` grammar as the reference it is checked against.
//!
//! bat's grammar (18 contexts, 54 patterns) is a list of case-insensitive keyword patterns
//! tried in order at each place, then comments and strings. A list's words all start at a
//! `\b` and end at one, so the scanner reads a word once, looks it up in one table
//! ([`lookup`]), and tries only the one pattern that word can start, then the patterns
//! any word can (a number, `db.table`). The few patterns that don't start at a word start on
//! a byte of their own (`--`, `#`, `/*`, quotes, operators). What the order of bat's patterns
//! decides is kept: a keyword pattern beats `db.table`, `drop`'s objects match as prefixes
//! (`drop tables` colors `table`), and `\s*(drop)\s+(table)` has no `\b`, so it matches at the
//! end of a word (`backdrop table x`).
//!
//! `\s` takes the line's `\n`, so `CREATE TABLE` at a line's end leaves `identifier_create`
//! open for the name on the next line, and `ALTER TABLE` and `COMMENT ON … IS` end there.
//! The stack never holds more than `main` and one context above it, so the state between
//! lines is that context: a block comment, one of the five strings, a dollar-quoted string, or
//! `identifier_create`; beside it, a dollar-quoted body's tag ([`line`]). `\w` past ASCII is a
//! letter or a decimal digit, `\s` Unicode's spaces, `\d` Unicode's decimal digits; `(?i)`
//! folds ASCII case only (Oniguruma's `ſ` for `s` isn't followed).
//!
//! **The dialects' words (X-124).** bat's lists are an old grammar's: most of what PostgreSQL,
//! MySQL, SQLite and T-SQL add stayed plain. The table holds them too, in the kinds bat's
//! scopes already give a word of their sort (a type `storage.type`, a function
//! `support.function`, a DML word `keyword.other.DML`), and two of ada's own:
//! `keyword.other.sql` for the statements' and clauses' words, `keyword.control.sql` for the
//! procedural ones (`if`, `while`, `return`, `raise`, `throw`). A word that is often a
//! column's name (`name`, `type`, `key`, `value`, `user`, `level`) stays plain; a function
//! is one only before `(`. A word that starts one of bat's compound patterns keeps a color
//! where the pattern doesn't follow (`on` without `delete`, `is` without `null`, `float`
//! without a size, `create` before an object bat doesn't list). Departures from bat's
//! reading, each a fix: `drop`'s and `create`'s objects are whole words (`drop tables` no
//! longer colors `table`, `backdrop table` no `drop`); `create`'s modifiers (`temp`,
//! `materialized`, `unique`…) come before its object; `if not exists`, `concurrently` and `on`
//! after it aren't the name; T-SQL's `GO` alone on its line; `N'…'`, `E'…'`, `B'…'`, `X'…'`
//! strings; PostgreSQL's dollar quotes (`$$…$$`, `$tag$…$tag$`): after `as` or `do` a
//! function's body, read as SQL to its closing tag, otherwise a string.

use crate::heredoc::Label;
use crate::prim::{WORD, char_len, digit, is_word_starting as is_word, space_len, word_end};
use crate::words::{Words, text_len};
use crate::{Out, State, is_space, skip_spaces, trim_end};

// The contexts above `main`, by what they read; the line comments end with their line.
pub(crate) const MAIN: u8 = 0;
/// `/* … */`.
pub(crate) const BLOCK: u8 = 1;
/// `'…'`.
pub(crate) const SINGLE: u8 = 2;
/// `"…"`.
pub(crate) const DOUBLE: u8 = 3;
/// `` `…` ``.
pub(crate) const BACKTICK: u8 = 4;
/// `%{…}`.
pub(crate) const BRACKETS: u8 = 5;
/// `%r{…}`.
pub(crate) const MODR: u8 = 6;
/// `identifier_create`: the name after `create table`.
pub(crate) const CREATE: u8 = 7;
/// Embedded only: `--` and `#` to the line's end, where a host's pattern can still end them
/// (on its own, a line comment is one token to the line's end).
pub(crate) const DASHES: u8 = 8;
pub(crate) const HASHES: u8 = 9;
/// On its own only: a dollar-quoted string, `$tag$…$tag$` ([`Scan::label`] its tag).
const DOLLAR: u8 = 10;
/// Not a context: what `main` returns where a dollar-quoted body opens ([`Scan::opened`]), so
/// [`line`] finds its closing tag before the body is read.
const OPEN_BODY: u8 = 11;

// The kinds: an index into [`LEAVES`].
const DASH: u16 = 1;
const DASH_MARK: u16 = 2;
const HASH: u16 = 3;
const HASH_MARK: u16 = 4;
const COMMENT: u16 = 5;
const COMMENT_MARK: u16 = 6;
/// The first of the strings' kinds: seven a string, by context from [`SINGLE`], each at
/// these offsets.
const STRINGS: u16 = 7;
const TEXT: u16 = 0;
const BEGIN: u16 = 1;
const END: u16 = 2;
const ESCAPE: u16 = 3;
const INTERPOLATION_BEGIN: u16 = 4;
const INTERPOLATION: u16 = 5;
const INTERPOLATION_END: u16 = 6;
const CREATE_KEYWORD: u16 = 42;
const CREATE_OBJECT: u16 = 43;
const CREATE_NAME: u16 = 44;
const CREATE_DOT: u16 = 45;
const DROP_KEYWORD: u16 = 46;
const DROP_OBJECT: u16 = 47;
const DROP_TABLE: u16 = 48;
const DROP_NAME: u16 = 49;
const DROP_CASCADE: u16 = 50;
const ALTER_KEYWORD: u16 = 51;
const ALTER_OBJECT: u16 = 52;
const TYPE: u16 = 53;
const NUMBER: u16 = 54;
const MODIFIER: u16 = 55;
const BOOLEAN: u16 = 56;
const DML: u16 = 57;
const LOGICAL: u16 = 58;
const NULL: u16 = 59;
const DML2: u16 = 60;
const LUW: u16 = 61;
const AUTHORIZATION: u16 = 62;
const OBJECT_COMMENTS: u16 = 63;
const ALIAS: u16 = 64;
const ORDER: u16 = 65;
const STAR: u16 = 66;
const COMPARISON: u16 = 67;
const MATH: u16 = 68;
const CONCATENATOR: u16 = 69;
const SCALAR: u16 = 70;
const AGGREGATE: u16 = 71;
const STRING_FUNCTION: u16 = 72;
const DATABASE: u16 = 73;
const TABLE: u16 = 74;
const SCOPE_BEGIN: u16 = 75;
const SCOPE_END: u16 = 76;
/// The dialects' words (X-124): a statement's or a clause's.
const KEYWORD: u16 = 77;
/// The dialects' procedural words.
const CONTROL: u16 = 78;
const DOLLAR_TEXT: u16 = 79;
const DOLLAR_BEGIN: u16 = 80;
const DOLLAR_END: u16 = 81;

/// How many kinds there are.
pub(crate) const KINDS: usize = 82;

/// Each kind's scopes under `source.sql`: bat's for the token, with the meta scope of the
/// pattern that matched it.
static LEAVES: [&str; KINDS] = [
    "",
    "comment.line.double-dash.sql",
    "comment.line.double-dash.sql punctuation.definition.comment.sql",
    "comment.line.number-sign.sql",
    "comment.line.number-sign.sql punctuation.definition.comment.sql",
    "comment.block.c",
    "comment.block.c punctuation.definition.comment.sql",
    // `'…'`
    "string.quoted.single.sql",
    "string.quoted.single.sql punctuation.definition.string.begin.sql",
    "string.quoted.single.sql punctuation.definition.string.end.sql",
    "string.quoted.single.sql constant.character.escape.sql",
    "",
    "",
    "",
    // `"…"`
    "string.quoted.double.sql",
    "string.quoted.double.sql punctuation.definition.string.begin.sql",
    "string.quoted.double.sql punctuation.definition.string.end.sql",
    "string.quoted.double.sql constant.character.escape.sql",
    "string.quoted.double.sql string.interpolated.sql punctuation.definition.string.begin.sql",
    "string.quoted.double.sql string.interpolated.sql",
    "string.quoted.double.sql string.interpolated.sql punctuation.definition.string.end.sql",
    // `` `…` ``
    "string.quoted.other.backtick.sql",
    "string.quoted.other.backtick.sql punctuation.definition.string.begin.sql",
    "string.quoted.other.backtick.sql punctuation.definition.string.end.sql",
    "string.quoted.other.backtick.sql constant.character.escape.sql",
    "",
    "",
    "",
    // `%{…}`
    "string.other.quoted.brackets.sql",
    "string.other.quoted.brackets.sql punctuation.definition.string.begin.sql",
    "string.other.quoted.brackets.sql punctuation.definition.string.end.sql",
    "",
    "string.other.quoted.brackets.sql string.interpolated.sql punctuation.definition.string.begin.sql",
    "string.other.quoted.brackets.sql string.interpolated.sql",
    "string.other.quoted.brackets.sql string.interpolated.sql punctuation.definition.string.end.sql",
    // `%r{…}`
    "string.regexp.modr.sql",
    "string.regexp.modr.sql punctuation.definition.string.begin.sql",
    "string.regexp.modr.sql punctuation.definition.string.end.sql",
    "",
    "string.regexp.modr.sql string.interpolated.sql punctuation.definition.string.begin.sql",
    "string.regexp.modr.sql string.interpolated.sql",
    "string.regexp.modr.sql string.interpolated.sql punctuation.definition.string.end.sql",
    // `main`
    "meta.create.sql keyword.other.create.sql",
    "meta.create.sql keyword.other.sql",
    "meta.toc-list.full-identifier.sql entity.name.function.sql",
    "meta.toc-list.full-identifier.sql punctuation.accessor.dot.sql",
    "meta.drop.sql keyword.other.create.sql",
    "meta.drop.sql keyword.other.sql",
    "meta.drop.sql keyword.other.table.sql",
    "meta.drop.sql entity.name.function.sql",
    "meta.drop.sql keyword.other.cascade.sql",
    "meta.alter.sql keyword.other.create.sql",
    "meta.alter.sql keyword.other.table.sql",
    "storage.type.sql",
    "constant.numeric.sql",
    "storage.modifier.sql",
    "constant.boolean.sql",
    "keyword.other.DML.sql",
    "keyword.operator.logical.sql",
    "constant.language.sql",
    "keyword.other.DML.II.sql",
    "keyword.other.LUW.sql",
    "keyword.other.authorization.sql",
    "keyword.other.object-comments.sql",
    "keyword.operator.assignment.alias.sql",
    "keyword.other.order.sql",
    "variable.language.star.sql",
    "keyword.operator.comparison.sql",
    "keyword.operator.math.sql",
    "keyword.operator.concatenator.sql",
    "support.function.scalar.sql",
    "support.function.aggregate.sql",
    "support.function.string.sql",
    "constant.other.database-name.sql",
    "constant.other.table-name.sql",
    "meta.block.sql punctuation.section.scope.begin.sql",
    "meta.block.sql punctuation.section.scope.end.sql",
    "keyword.other.sql",
    "keyword.control.sql",
    "string.quoted.dollar.sql",
    "string.quoted.dollar.sql punctuation.definition.string.begin.sql",
    "string.quoted.dollar.sql punctuation.definition.string.end.sql",
];

/// The scopes under `source.sql` kind `kind` carries.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

// The state between lines on its own, a word: the context in the low bits; a dollar-quoted
// body open and its tag's label; `as` or `do` ending the line (a dollar quote opening the next
// is a body); a dollar-quoted string's tag's label.
const CTX_BITS: u64 = 0xf;
const BODY_OPEN: u64 = 1 << 4;
const AFTER_AS: u64 = 1 << 5;
const BODY_AT: u32 = 6;
/// A MySQL client's `DELIMITER` line seen: `$` quotes nothing (`DELIMITER $$`, `END $$`).
const MYSQL: u64 = 1 << 62;
const STRING_AT: u32 = BODY_AT + Label::BITS;
const _: () = assert!(STRING_AT + Label::BITS <= 62 && (OPEN_BODY as u64) <= CTX_BITS);

/// Lex `b`, a line of SQL without its `\n`, from `state`.
///
/// In a dollar-quoted body the closing tag is found first, wherever it stands (PostgreSQL's
/// lexer reads the body as one string before its text is SQL): the body's text is lexed up to
/// it, whatever context it leaves open there ends with it, and `main` goes on past it.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let w = state.word();
    let mut ctx = (w & CTX_BITS) as u8;
    let mut body = (w & BODY_OPEN != 0).then(|| Label::from_bits((w >> BODY_AT) as u32));
    let mut label = Label::from_bits((w >> STRING_AT) as u32);
    let mut after_as = w & AFTER_AS != 0;
    let mut mysql = w & MYSQL != 0;
    let mut p = 0;
    loop {
        let close = body.and_then(|l| find_tag(b, p, l));
        let end = close.map_or(b.len(), |c| c.0);
        let mut s = Scan {
            b: &b[..end],
            out,
            mark: p,
            is_end: usize::MAX,
            from: 0,
            embed: None,
            at: 0,
            label,
            in_body: body.is_some(),
            after_as,
            opened: None,
            mysql,
        };
        // a context's lexing up to the region's end, or to where it hands over to another
        loop {
            let next = match ctx {
                MAIN => s.main(p),
                BLOCK => s.block(p),
                SINGLE..=MODR => s.string(ctx, p),
                CREATE => s.create(p),
                DOLLAR => s.dollar(p),
                _ => None,
            };
            match next {
                Some((q, c)) => (p, ctx) = (q, c),
                None => break,
            }
            if ctx == OPEN_BODY {
                break;
            }
        }
        label = s.label;
        after_as = s.as_before(end);
        mysql = s.mysql;
        if ctx == OPEN_BODY {
            (body, ctx, after_as) = (s.opened, MAIN, false);
            continue;
        }
        let Some((start, e)) = close else {
            break;
        };
        out.push(start, e, DOLLAR_END);
        (body, ctx, p, after_as) = (None, MAIN, e, false);
    }
    let mut w = u64::from(ctx);
    if let Some(l) = body {
        w |= BODY_OPEN | u64::from(l.bits()) << BODY_AT;
    }
    if after_as {
        w |= AFTER_AS;
    }
    if mysql {
        w |= MYSQL;
    }
    if ctx == DOLLAR {
        w |= u64::from(label.bits()) << STRING_AT;
    }
    state.set_word(w);
}

/// A dollar quote's tag at `p`, a `$`: `$$` or `$` and a name (not starting with a digit) and
/// `$`. Where it ends.
fn tag_end(b: &[u8], p: usize) -> Option<usize> {
    let mut q = p + 1;
    match *b.get(q)? {
        b'$' => return Some(q + 1),
        c if c >= 0x80 || c == b'_' || c.is_ascii_alphabetic() => {}
        _ => return None,
    }
    while q < b.len() && (b[q] >= 0x80 || WORD[usize::from(b[q])]) {
        q += 1;
    }
    (b.get(q) == Some(&b'$')).then_some(q + 1)
}

/// The first `$tag$` from `from` whose tag has `label`: where it starts and ends.
fn find_tag(b: &[u8], from: usize, label: Label) -> Option<(usize, usize)> {
    let mut q = from;
    while let Some(i) = b.get(q..)?.iter().position(|&c| c == b'$') {
        let s = q + i;
        if let Some(e) = tag_end(b, s)
            && Label::of(&b[s + 1..e - 1]) == label
        {
            return Some((s, e));
        }
        q = s + 1;
    }
    None
}

/// SQL in a host language's string (phase 4): what the host gives the scan.
///
/// The host's patterns (the string's end, its escapes and placeholders) come before SQL's at
/// every place, as syntect's `with_prototype` and `embed … escape` have them, so the scan hands
/// each place one of them can start back to the host ([`embedded`]). A SQL token that starts
/// before such a place may run past it, as syntect's leftmost match does.
pub(crate) struct Embed<'t> {
    /// The bytes a pattern of the host's can start on; the line's end is always one.
    pub yields: &'t [bool; 256],
    /// Added to each of SQL's kinds: where they sit among the host's.
    pub base: u16,
    /// The kind of the text no SQL pattern takes in `main`: the host string's.
    pub text: u16,
}

/// What the scan of a line keeps between the host's turns.
pub(crate) struct Embedded {
    /// [`Scan::is_end`]: `usize::MAX` at a line's start.
    pub is_end: usize,
    /// Where the spans so far end; the text before a SQL token, past it, is
    /// [`Embed::text`].
    pub at: usize,
}

/// Lex SQL embedded in a host's string, in context `ctx`, from `p`, where the host's patterns
/// didn't match: up to the next place they could (a byte of [`Embed::yields`] or the line's
/// end), or past the line. Where it stopped and the context there; `None` from past the line.
pub(crate) fn embedded(
    b: &[u8],
    p: usize,
    ctx: u8,
    host: &Embed,
    keep: &mut Embedded,
    out: &mut Out,
) -> Option<(usize, u8)> {
    let n = b.len();
    if p > n {
        return None;
    }
    let mut s = Scan {
        b,
        out,
        mark: p,
        is_end: keep.is_end,
        from: p,
        embed: Some(host),
        at: keep.at,
        label: Label::from_bits(0),
        in_body: false,
        after_as: false,
        opened: None,
        mysql: false,
    };
    let (mut p, mut ctx) = (p, ctx);
    loop {
        let next = match ctx {
            MAIN => s.main(p),
            BLOCK => s.block(p),
            SINGLE..=MODR => s.string(ctx, p),
            CREATE => s.create(p),
            DASHES | HASHES => s.line_comment(ctx, p),
            _ => None,
        };
        match next {
            Some((q, c)) => {
                (p, ctx) = (q, c);
                if q > n {
                    // past the `\n` a match took, `{{end_identifier}}`'s `$` holds, and
                    // `identifier_create` pops (as `create` does past the line on its own)
                    if ctx == CREATE {
                        ctx = MAIN;
                    }
                    break;
                }
                if q > s.from && (q == n || host.yields[usize::from(b[q])]) {
                    break;
                }
            }
            // the context ran to the line's end: the host's turn there, then past it
            None => {
                p = if s.from < n { n } else { n + 1 };
                break;
            }
        }
    }
    keep.is_end = s.is_end;
    keep.at = s.at;
    Some((p, ctx))
}

// Where a host's string is SQL (phase 4, SQL by call sites and markers, as IntelliJ has it):
// the JVM's calls and annotations, and IntelliJ's markers.

/// Whether the call `b[p..e]` names takes SQL as its first argument: JDBC's
/// (`prepareStatement`, `executeQuery`…), JPA's and Hibernate's (`createQuery`,
/// `createNativeQuery`…), Spring's (`queryForObject`, `batchUpdate`, `JdbcClient.sql`), JDBI's
/// (`createUpdate`, `createScript`), Android's (`execSQL`, `rawQuery`). `query`, `update` and
/// `select` only on a receiver named as a database's (`jdbcTemplate.query`, `handle.select`):
/// jsoup's `select("css")` and the like aren't SQL.
pub(crate) fn jvm_call(b: &[u8], p: usize, e: usize) -> bool {
    let name = &b[p..e];
    // the first byte first: most calls are none of these
    match name.first() {
        Some(b'p') => matches!(name, b"prepareStatement" | b"prepareCall"),
        Some(b'e') => matches!(
            name,
            b"executeQuery" | b"executeUpdate" | b"executeLargeUpdate" | b"execute" | b"execSQL"
        ),
        Some(b'a') => name == b"addBatch",
        Some(b'c') => matches!(
            name,
            b"createQuery"
                | b"createNativeQuery"
                | b"createSQLQuery"
                | b"createNativeMutationQuery"
                | b"createMutationQuery"
                | b"createSelectionQuery"
                | b"createUpdate"
                | b"createCall"
                | b"createScript"
                | b"compileStatement"
        ),
        Some(b'q') => {
            matches!(
                name,
                b"queryForObject"
                    | b"queryForList"
                    | b"queryForMap"
                    | b"queryForRowSet"
                    | b"queryForStream"
                    | b"queryForLong"
                    | b"queryForInt"
            ) || (name == b"query" && db_receiver(b, p))
        }
        Some(b'b') => name == b"batchUpdate",
        Some(b's') => name == b"sql" || (name == b"select" && db_receiver(b, p)),
        Some(b'r') => name == b"rawQuery",
        Some(b'u') => name == b"update" && db_receiver(b, p),
        _ => false,
    }
}

/// Whether the call at `p` is on a receiver named as a database's: `….name(` with the name
/// ending in `template` or `handle`, or holding `jdbc` (ASCII case ignored).
pub(crate) fn db_receiver(b: &[u8], p: usize) -> bool {
    let q = trim_end(b, 0, p);
    if q == 0 || b[q - 1] != b'.' {
        return false;
    }
    let e = trim_end(b, 0, q - 1);
    let mut s = e;
    while s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_') {
        s -= 1;
    }
    let r = b[s..e].to_ascii_lowercase();
    r.ends_with(b"template") || r.ends_with(b"handle") || r.windows(4).any(|w| w == b"jdbc")
}

/// Whether an annotation named `name` takes SQL: its `value` (a string alone in its
/// parentheses), and the arguments [`jvm_argument`] names. Spring Data's and Micronaut's
/// `@Query` (JPQL by default, SQL with `nativeQuery`), `@NativeQuery`, JDBI's `@SqlQuery`…,
/// MyBatis' `@Select`…, JPA's `@NamedQuery` and `@NamedNativeQuery`, Hibernate's
/// `@Formula`, `@Subselect`, `@SQLRestriction`, `@SQLInsert`…, Room's `@Query`.
pub(crate) fn jvm_annotation(name: &[u8]) -> bool {
    matches!(
        name,
        b"Query"
            | b"NativeQuery"
            | b"SqlQuery"
            | b"SqlUpdate"
            | b"SqlBatch"
            | b"SqlCall"
            | b"SqlScript"
            | b"Select"
            | b"Insert"
            | b"Update"
            | b"Delete"
            | b"NamedQuery"
            | b"NamedNativeQuery"
            | b"Formula"
            | b"Subselect"
            | b"Where"
            | b"SQLRestriction"
            | b"SQLSelect"
            | b"SQLInsert"
            | b"SQLUpdate"
            | b"SQLDelete"
            | b"SQLDeleteAll"
    )
}

/// Whether a SQL annotation's argument named `name` is SQL: `value`, `query` (`@NamedQuery`),
/// `countQuery` (`@Query`), `sql` (`@SQLInsert`), `clause` (`@Where`).
pub(crate) fn jvm_argument(name: &[u8]) -> bool {
    matches!(
        name,
        b"value" | b"query" | b"countQuery" | b"sql" | b"clause"
    )
}

/// Whether IntelliJ's language id `id` (`@Language("…")`, `// language=…`) is SQL or one of its
/// dialects: one holding `sql` (`PostgreSQL`, `SQLite`, `TSQL`…), or a database's name.
pub(crate) fn is_sql_language(id: &[u8]) -> bool {
    let id = id.to_ascii_lowercase();
    id.windows(3).any(|w| w == b"sql")
        || matches!(
            &id[..],
            b"oracle"
                | b"h2"
                | b"hsqldb"
                | b"derby"
                | b"db2"
                | b"sybase"
                | b"redshift"
                | b"snowflake"
                | b"bigquery"
                | b"clickhouse"
                | b"cockroachdb"
                | b"exasol"
                | b"greenplum"
                | b"vertica"
        )
}

/// A `language=ID` comment's text from `p` (after `//` or `/*`): whether it marks SQL.
pub(crate) fn marker_comment(b: &[u8], p: usize) -> bool {
    let q = skip_spaces(b, p);
    // the first byte first: most comments aren't markers, and the compare is a call
    if b.get(q) != Some(&b'l') || !b[q..].starts_with(b"language=") {
        return false;
    }
    let s = q + 9;
    let mut e = s;
    while e < b.len() && !is_space(b[e]) && b[e] != b'*' {
        e += 1;
    }
    is_sql_language(&b[s..e])
}

/// Whether a SQL marker at `from` still holds at `to`: no `;`, `,`, `)`, `{` or `}` between (the
/// declaration or argument it marks ended there; a marked parameter's default stays plain).
pub(crate) fn marker_holds(b: &[u8], from: usize, to: usize) -> bool {
    !b[from.min(to)..to]
        .iter()
        .any(|&c| matches!(c, b';' | b',' | b')' | b'{' | b'}'))
}

/// `@Language("ID")`'s `("ID")`, from `p` past the name: where it ends, if `ID` is SQL.
pub(crate) fn language_annotation(b: &[u8], p: usize) -> Option<usize> {
    let q = skip_spaces(b, p);
    if b.get(q) != Some(&b'(') {
        return None;
    }
    let s = skip_spaces(b, q + 1);
    if b.get(s) != Some(&b'"') {
        return None;
    }
    let e = s + 1 + b[s + 1..].iter().position(|&c| c == b'"')?;
    let r = skip_spaces(b, e + 1);
    (b.get(r) == Some(&b')') && is_sql_language(&b[s + 1..e])).then_some(r + 1)
}

// The words a pattern of `main` starts with, by the pattern (or the part of one) they start.
const W_NONE: u8 = 0;
const W_CREATE: u8 = 1;
const W_DROP: u8 = 2;
const W_ALTER: u8 = 3;
/// The types' first form: a word alone.
const W_TYPE: u8 = 4;
/// `double\sprecision`.
const W_DOUBLE: u8 = 5;
/// The types' second form, a size required: `tinyint`, `float`, `interval`.
const W_SIZED: u8 = 6;
/// `character\s(?:varying)?` and a size.
const W_CHARACTER: u8 = 7;
/// `var\schar` and a size.
const W_VAR: u8 = 8;
/// The third form, a size allowed: `char`, `number`, `nvarchar`, `varbinary`, `varchar\d?`.
const W_CHAR: u8 = 9;
/// `numeric`, `decimal`, and two sizes allowed.
const W_NUMERIC: u8 = 10;
/// `times?`.
const W_TIME: u8 = 11;
/// `timestamp(?:(s|tz))?`.
const W_TIMESTAMP: u8 = 12;
/// `(?:primary|foreign)\s+key`.
const W_KEY: u8 = 13;
/// `references`, `check`, `constraint`, `default`.
const W_MODIFIER: u8 = 14;
/// `on\sdelete`, `on\supdate`.
const W_ON: u8 = 15;
const W_BOOLEAN: u8 = 16;
/// The DML words alone.
const W_DML: u8 = 17;
/// `select(\s+(distinct|top))?`.
const W_SELECT: u8 = 18;
/// `insert(\s+(ignore\s+)?into)?`.
const W_INSERT: u8 = 19;
/// `group\s+by`, `order\s+by`.
const W_BY: u8 = 20;
/// `union(\s+all)?`.
const W_UNION: u8 = 21;
/// `(inner|cross)\s+join`.
const W_INNER: u8 = 22;
/// `(left|right)(\s+outer)?\s+join`.
const W_SIDE: u8 = 23;
/// `natural(\s+(left|right)(\s+outer)?)?\s+join`.
const W_NATURAL: u8 = 24;
const W_IS: u8 = 25;
const W_NOT: u8 = 26;
const W_NULL: u8 = 27;
const W_LOGICAL: u8 = 28;
const W_VALUES: u8 = 29;
/// `begin`, `commit`, `rollback`, each `(\s+work)?`.
const W_WORK: u8 = 30;
/// `start\s+transaction`.
const W_START: u8 = 31;
/// `grant(\swith\sgrant\soption)?`.
const W_GRANT: u8 = 32;
const W_REVOKE: u8 = 33;
/// `comment\s+on\s+(…)\s+.*?\s+(is)\s+`.
const W_COMMENT: u8 = 34;
const W_AS: u8 = 35;
const W_ORDER: u8 = 36;
const W_SCALAR: u8 = 37;
/// `(AVG|COUNT|MIN|MAX|SUM)(?=\s*\()`.
const W_AGGREGATE: u8 = 38;
const W_STRING_FUNCTION: u8 = 39;
// The dialects' words (X-124).
/// A statement's or a clause's word: [`KEYWORD`].
const W_KEYWORD: u8 = 40;
/// A procedural word: [`CONTROL`].
const W_CONTROL: u8 = 41;
/// A function, `(?=\s*\()`: [`SCALAR`].
const W_FUNCTION: u8 = 42;
/// `do`: [`KEYWORD`], and a dollar quote after it a body.
const W_DO: u8 = 43;
/// T-SQL's `GO` (`go(\s+\d+)?`), alone on its line.
const W_GO: u8 = 44;
/// `each(\s+(row|statement))?`.
const W_EACH: u8 = 45;
/// `nulls(\s+(first|last|distinct))?`.
const W_NULLS: u8 = 46;
/// `isnull`: a function before `(` (T-SQL), else an operator (PostgreSQL, SQLite).
const W_ISNULL: u8 = 47;
/// `replace`: a function before `(`, else MySQL's and SQLite's statement or resolution.
const W_REPLACE: u8 = 48;
/// `repeat`: a function before `(`, else MySQL's loop.
const W_REPEAT: u8 = 49;
/// `savepoint`, `transaction`, `tran`: [`LUW`].
const W_LUW: u8 = 50;
/// `language`: [`KEYWORD`]; after `do` its name too keeps a dollar quote next a body
/// (`DO LANGUAGE plpgsql $$`).
const W_LANGUAGE: u8 = 51;
/// A MySQL client's `DELIMITER` line: [`KEYWORD`], its delimiter plain, and `$` quotes nothing
/// after it ([`MYSQL`]).
const W_DELIMITER: u8 = 52;

/// Every word that starts a pattern of `main`, lowercase, with the pattern.
const ENTRIES: &[(&[u8], u8)] = &[
    (b"create", W_CREATE),
    (b"drop", W_DROP),
    (b"alter", W_ALTER),
    (b"bigint", W_TYPE),
    (b"bigserial", W_TYPE),
    (b"bit", W_TYPE),
    (b"boolean", W_TYPE),
    (b"box", W_TYPE),
    (b"bytea", W_TYPE),
    (b"cidr", W_TYPE),
    (b"circle", W_TYPE),
    (b"date", W_TYPE),
    (b"datetime", W_TYPE),
    (b"inet", W_TYPE),
    (b"int", W_TYPE),
    (b"integer", W_TYPE),
    (b"line", W_TYPE),
    (b"lseg", W_TYPE),
    (b"macaddr", W_TYPE),
    (b"money", W_TYPE),
    (b"ntext", W_TYPE),
    (b"oid", W_TYPE),
    (b"path", W_TYPE),
    (b"point", W_TYPE),
    (b"polygon", W_TYPE),
    (b"real", W_TYPE),
    (b"serial", W_TYPE),
    (b"smallint", W_TYPE),
    (b"sysdate", W_TYPE),
    (b"sysname", W_TYPE),
    (b"text", W_TYPE),
    (b"double", W_DOUBLE),
    (b"tinyint", W_SIZED),
    (b"float", W_SIZED),
    (b"interval", W_SIZED),
    (b"character", W_CHARACTER),
    (b"var", W_VAR),
    (b"char", W_CHAR),
    (b"number", W_CHAR),
    (b"nvarchar", W_CHAR),
    (b"varbinary", W_CHAR),
    (b"varchar", W_CHAR),
    (b"varchar0", W_CHAR),
    (b"varchar1", W_CHAR),
    (b"varchar2", W_CHAR),
    (b"varchar3", W_CHAR),
    (b"varchar4", W_CHAR),
    (b"varchar5", W_CHAR),
    (b"varchar6", W_CHAR),
    (b"varchar7", W_CHAR),
    (b"varchar8", W_CHAR),
    (b"varchar9", W_CHAR),
    (b"numeric", W_NUMERIC),
    (b"decimal", W_NUMERIC),
    (b"time", W_TIME),
    (b"times", W_TIME),
    (b"timestamp", W_TIMESTAMP),
    (b"timestamps", W_TIMESTAMP),
    (b"timestamptz", W_TIMESTAMP),
    (b"primary", W_KEY),
    (b"foreign", W_KEY),
    (b"references", W_MODIFIER),
    (b"check", W_MODIFIER),
    (b"constraint", W_MODIFIER),
    (b"default", W_MODIFIER),
    (b"on", W_ON),
    (b"true", W_BOOLEAN),
    (b"false", W_BOOLEAN),
    (b"update", W_DML),
    (b"delete", W_DML),
    (b"truncate", W_DML),
    (b"from", W_DML),
    (b"set", W_DML),
    (b"where", W_DML),
    (b"with", W_DML),
    (b"case", W_DML),
    (b"when", W_DML),
    (b"then", W_DML),
    (b"else", W_DML),
    (b"end", W_DML),
    (b"using", W_DML),
    (b"limit", W_DML),
    (b"join", W_DML),
    (b"straight_join", W_DML),
    (b"select", W_SELECT),
    (b"insert", W_INSERT),
    (b"group", W_BY),
    (b"order", W_BY),
    (b"union", W_UNION),
    (b"inner", W_INNER),
    (b"cross", W_INNER),
    (b"left", W_SIDE),
    (b"right", W_SIDE),
    (b"natural", W_NATURAL),
    (b"is", W_IS),
    (b"not", W_NOT),
    (b"null", W_NULL),
    (b"and", W_LOGICAL),
    (b"or", W_LOGICAL),
    (b"like", W_LOGICAL),
    (b"having", W_LOGICAL),
    (b"exists", W_LOGICAL),
    (b"between", W_LOGICAL),
    (b"in", W_LOGICAL),
    (b"values", W_VALUES),
    (b"begin", W_WORK),
    (b"commit", W_WORK),
    (b"rollback", W_WORK),
    (b"start", W_START),
    (b"grant", W_GRANT),
    (b"revoke", W_REVOKE),
    (b"comment", W_COMMENT),
    (b"as", W_AS),
    (b"asc", W_ORDER),
    (b"desc", W_ORDER),
    (b"current_date", W_SCALAR),
    (b"current_time", W_SCALAR),
    (b"current_timestamp", W_SCALAR),
    (b"current_user", W_SCALAR),
    (b"session_user", W_SCALAR),
    (b"system_user", W_SCALAR),
    (b"avg", W_AGGREGATE),
    (b"count", W_AGGREGATE),
    (b"min", W_AGGREGATE),
    (b"max", W_AGGREGATE),
    (b"sum", W_AGGREGATE),
    (b"concatenate", W_STRING_FUNCTION),
    (b"convert", W_STRING_FUNCTION),
    (b"lower", W_STRING_FUNCTION),
    (b"substring", W_STRING_FUNCTION),
    (b"translate", W_STRING_FUNCTION),
    (b"trim", W_STRING_FUNCTION),
    (b"upper", W_STRING_FUNCTION),
    // The dialects' words (X-124), from the survey of the corpus and the hosts' SQL: PostgreSQL,
    // MySQL, SQLite and T-SQL first. Not here: words that are often a column's name (`name`,
    // `type`, `key`, `value`, `user`, `role`, `level`, `owner`, `row`, `rows`, `first`, `next`,
    // `option`, `open`, `close`, `filter`, `source`, `status`, `version`).
    (b"add", W_KEYWORD),
    (b"after", W_KEYWORD),
    (b"always", W_KEYWORD),
    (b"analyse", W_KEYWORD),
    (b"analyze", W_KEYWORD),
    (b"ansi_nulls", W_KEYWORD),
    (b"array", W_KEYWORD),
    (b"attach", W_KEYWORD),
    (b"authorization", W_KEYWORD),
    (b"before", W_KEYWORD),
    (b"call", W_KEYWORD),
    (b"cascade", W_KEYWORD),
    (b"charset", W_KEYWORD),
    (b"checkpoint", W_KEYWORD),
    (b"clustered", W_KEYWORD),
    (b"collate", W_KEYWORD),
    (b"column", W_KEYWORD),
    (b"concurrently", W_KEYWORD),
    (b"conflict", W_KEYWORD),
    (b"copy", W_KEYWORD),
    (b"cursor", W_KEYWORD),
    (b"database", W_KEYWORD),
    (b"deallocate", W_KEYWORD),
    (b"declare", W_KEYWORD),
    (b"deferrable", W_KEYWORD),
    (b"deferred", W_KEYWORD),
    (b"definer", W_KEYWORD),
    (b"detach", W_KEYWORD),
    (b"deterministic", W_KEYWORD),
    (b"diagnostics", W_KEYWORD),
    (b"duplicate", W_KEYWORD),
    (b"engine", W_KEYWORD),
    (b"escape", W_KEYWORD),
    (b"exclude", W_KEYWORD),
    (b"exec", W_KEYWORD),
    (b"execute", W_KEYWORD),
    (b"explain", W_KEYWORD),
    (b"extension", W_KEYWORD),
    (b"following", W_KEYWORD),
    (b"for", W_KEYWORD),
    (b"function", W_KEYWORD),
    (b"generated", W_KEYWORD),
    (b"ignore", W_KEYWORD),
    (b"immediate", W_KEYWORD),
    (b"immutable", W_KEYWORD),
    (b"include", W_KEYWORD),
    (b"increment", W_KEYWORD),
    (b"index", W_KEYWORD),
    (b"inherits", W_KEYWORD),
    (b"initially", W_KEYWORD),
    (b"inout", W_KEYWORD),
    (b"instead", W_KEYWORD),
    (b"invoker", W_KEYWORD),
    (b"leakproof", W_KEYWORD),
    (b"listen", W_KEYWORD),
    (b"lock", W_KEYWORD),
    (b"materialized", W_KEYWORD),
    (b"matched", W_KEYWORD),
    (b"modifies", W_KEYWORD),
    (b"nocount", W_KEYWORD),
    (b"nolock", W_KEYWORD),
    (b"nonclustered", W_KEYWORD),
    (b"nothing", W_KEYWORD),
    (b"notify", W_KEYWORD),
    (b"nowait", W_KEYWORD),
    (b"of", W_KEYWORD),
    (b"off", W_KEYWORD),
    (b"only", W_KEYWORD),
    (b"partition", W_KEYWORD),
    (b"pragma", W_KEYWORD),
    (b"preceding", W_KEYWORD),
    (b"prepare", W_KEYWORD),
    (b"print", W_KEYWORD),
    (b"proc", W_KEYWORD),
    (b"procedure", W_KEYWORD),
    (b"quoted_identifier", W_KEYWORD),
    (b"reads", W_KEYWORD),
    (b"readonly", W_KEYWORD),
    (b"recompile", W_KEYWORD),
    (b"recursive", W_KEYWORD),
    (b"refresh", W_KEYWORD),
    (b"reindex", W_KEYWORD),
    (b"rename", W_KEYWORD),
    (b"reset", W_KEYWORD),
    (b"restrict", W_KEYWORD),
    (b"returns", W_KEYWORD),
    (b"schema", W_KEYWORD),
    (b"schemabinding", W_KEYWORD),
    (b"sequence", W_KEYWORD),
    (b"setof", W_KEYWORD),
    (b"show", W_KEYWORD),
    (b"sqlexception", W_KEYWORD),
    (b"sqlstate", W_KEYWORD),
    (b"sqlwarning", W_KEYWORD),
    (b"stable", W_KEYWORD),
    (b"strict", W_KEYWORD),
    (b"table", W_KEYWORD),
    (b"tablespace", W_KEYWORD),
    (b"temp", W_KEYWORD),
    (b"temporary", W_KEYWORD),
    (b"to", W_KEYWORD),
    (b"trigger", W_KEYWORD),
    (b"unbounded", W_KEYWORD),
    (b"unlisten", W_KEYWORD),
    (b"unlock", W_KEYWORD),
    (b"unlogged", W_KEYWORD),
    (b"use", W_KEYWORD),
    (b"vacuum", W_KEYWORD),
    (b"variadic", W_KEYWORD),
    (b"verbose", W_KEYWORD),
    (b"view", W_KEYWORD),
    (b"virtual", W_KEYWORD),
    (b"volatile", W_KEYWORD),
    (b"without", W_KEYWORD),
    (b"xact_abort", W_KEYWORD),
    (b"do", W_DO),
    (b"language", W_LANGUAGE),
    (b"delimiter", W_DELIMITER),
    (b"go", W_GO),
    (b"each", W_EACH),
    (b"nulls", W_NULLS),
    (b"apply", W_DML),
    (b"distinct", W_DML),
    (b"except", W_DML),
    (b"fetch", W_DML),
    (b"intersect", W_DML),
    (b"into", W_DML),
    (b"lateral", W_DML),
    (b"merge", W_DML),
    (b"offset", W_DML),
    (b"output", W_DML),
    (b"over", W_DML),
    (b"pivot", W_DML),
    (b"qualify", W_DML),
    (b"returning", W_DML),
    (b"tablesample", W_DML),
    (b"top", W_DML),
    (b"unpivot", W_DML),
    (b"window", W_DML),
    (b"within", W_DML),
    (b"by", W_DML),
    (b"outer", W_INNER),
    (b"full", W_SIDE),
    (b"all", W_LOGICAL),
    (b"any", W_LOGICAL),
    (b"some", W_LOGICAL),
    (b"ilike", W_LOGICAL),
    (b"similar", W_LOGICAL),
    (b"regexp", W_LOGICAL),
    (b"rlike", W_LOGICAL),
    (b"glob", W_LOGICAL),
    (b"xor", W_LOGICAL),
    (b"notnull", W_LOGICAL),
    (b"isnull", W_ISNULL),
    (b"replace", W_REPLACE),
    (b"repeat", W_REPEAT),
    (b"savepoint", W_LUW),
    (b"transaction", W_LUW),
    (b"tran", W_LUW),
    (b"if", W_CONTROL),
    (b"elsif", W_CONTROL),
    (b"elseif", W_CONTROL),
    (b"while", W_CONTROL),
    (b"loop", W_CONTROL),
    (b"return", W_CONTROL),
    (b"exit", W_CONTROL),
    (b"continue", W_CONTROL),
    (b"leave", W_CONTROL),
    (b"iterate", W_CONTROL),
    (b"until", W_CONTROL),
    (b"goto", W_CONTROL),
    (b"raise", W_CONTROL),
    (b"raiserror", W_CONTROL),
    (b"throw", W_CONTROL),
    (b"try", W_CONTROL),
    (b"catch", W_CONTROL),
    (b"break", W_CONTROL),
    (b"foreach", W_CONTROL),
    (b"waitfor", W_CONTROL),
    (b"perform", W_CONTROL),
    (b"signal", W_CONTROL),
    (b"resignal", W_CONTROL),
    (b"exception", W_CONTROL),
    (b"unique", W_MODIFIER),
    (b"unsigned", W_MODIFIER),
    (b"zerofill", W_MODIFIER),
    (b"auto_increment", W_MODIFIER),
    (b"autoincrement", W_MODIFIER),
    (b"identity", W_MODIFIER),
    (b"current_role", W_SCALAR),
    (b"current_schema", W_SCALAR),
    (b"current_catalog", W_SCALAR),
    (b"localtime", W_SCALAR),
    (b"localtimestamp", W_SCALAR),
    // PostgreSQL's types
    (b"anyarray", W_TYPE),
    (b"anyelement", W_TYPE),
    (b"bool", W_TYPE),
    (b"bpchar", W_TYPE),
    (b"citext", W_TYPE),
    (b"cstring", W_TYPE),
    (b"daterange", W_TYPE),
    (b"float4", W_TYPE),
    (b"float8", W_TYPE),
    (b"geography", W_TYPE),
    (b"geometry", W_TYPE),
    (b"hstore", W_TYPE),
    (b"int2", W_TYPE),
    (b"int4", W_TYPE),
    (b"int8", W_TYPE),
    (b"int4range", W_TYPE),
    (b"int8range", W_TYPE),
    (b"json", W_TYPE),
    (b"jsonb", W_TYPE),
    (b"macaddr8", W_TYPE),
    (b"numrange", W_TYPE),
    (b"record", W_TYPE),
    (b"regclass", W_TYPE),
    (b"regconfig", W_TYPE),
    (b"regproc", W_TYPE),
    (b"regtype", W_TYPE),
    (b"serial2", W_TYPE),
    (b"serial4", W_TYPE),
    (b"serial8", W_TYPE),
    (b"smallserial", W_TYPE),
    (b"timetz", W_TYPE),
    (b"tsquery", W_TYPE),
    (b"tsrange", W_TYPE),
    (b"tstzrange", W_TYPE),
    (b"tsvector", W_TYPE),
    (b"uuid", W_TYPE),
    (b"varbit", W_TYPE),
    (b"void", W_TYPE),
    (b"xml", W_TYPE),
    // MySQL's and SQLite's
    (b"blob", W_TYPE),
    (b"enum", W_TYPE),
    (b"longblob", W_TYPE),
    (b"longtext", W_TYPE),
    (b"mediumblob", W_TYPE),
    (b"mediumint", W_TYPE),
    (b"mediumtext", W_TYPE),
    (b"tinyblob", W_TYPE),
    (b"tinytext", W_TYPE),
    (b"binary", W_CHAR),
    // T-SQL's, and the standard's large objects
    (b"clob", W_TYPE),
    (b"datetime2", W_TYPE),
    (b"datetimeoffset", W_TYPE),
    (b"hierarchyid", W_TYPE),
    (b"nclob", W_TYPE),
    (b"rowversion", W_TYPE),
    (b"smalldatetime", W_TYPE),
    (b"smallmoney", W_TYPE),
    (b"sql_variant", W_TYPE),
    (b"uniqueidentifier", W_TYPE),
    (b"nchar", W_CHAR),
    // functions, before `(`
    (b"abs", W_FUNCTION),
    (b"age", W_FUNCTION),
    (b"array_length", W_FUNCTION),
    (b"array_to_string", W_FUNCTION),
    (b"btrim", W_FUNCTION),
    (b"cast", W_FUNCTION),
    (b"ceil", W_FUNCTION),
    (b"ceiling", W_FUNCTION),
    (b"char_length", W_FUNCTION),
    (b"character_length", W_FUNCTION),
    (b"charindex", W_FUNCTION),
    (b"choose", W_FUNCTION),
    (b"coalesce", W_FUNCTION),
    (b"concat", W_FUNCTION),
    (b"concat_ws", W_FUNCTION),
    (b"curdate", W_FUNCTION),
    (b"currval", W_FUNCTION),
    (b"current_setting", W_FUNCTION),
    (b"curtime", W_FUNCTION),
    (b"date_format", W_FUNCTION),
    (b"date_part", W_FUNCTION),
    (b"date_trunc", W_FUNCTION),
    (b"dateadd", W_FUNCTION),
    (b"datediff", W_FUNCTION),
    (b"datename", W_FUNCTION),
    (b"datepart", W_FUNCTION),
    (b"eomonth", W_FUNCTION),
    (b"exp", W_FUNCTION),
    (b"extract", W_FUNCTION),
    (b"floor", W_FUNCTION),
    (b"format", W_FUNCTION),
    (b"format_type", W_FUNCTION),
    (b"from_unixtime", W_FUNCTION),
    (b"gen_random_uuid", W_FUNCTION),
    (b"generate_series", W_FUNCTION),
    (b"getdate", W_FUNCTION),
    (b"getutcdate", W_FUNCTION),
    (b"greatest", W_FUNCTION),
    (b"ifnull", W_FUNCTION),
    (b"iif", W_FUNCTION),
    (b"initcap", W_FUNCTION),
    (b"instr", W_FUNCTION),
    (b"isnumeric", W_FUNCTION),
    (b"json_array", W_FUNCTION),
    (b"json_build_array", W_FUNCTION),
    (b"json_build_object", W_FUNCTION),
    (b"json_extract", W_FUNCTION),
    (b"json_object", W_FUNCTION),
    (b"json_query", W_FUNCTION),
    (b"json_typeof", W_FUNCTION),
    (b"json_value", W_FUNCTION),
    (b"jsonb_build_array", W_FUNCTION),
    (b"jsonb_build_object", W_FUNCTION),
    (b"jsonb_set", W_FUNCTION),
    (b"jsonb_typeof", W_FUNCTION),
    (b"last_insert_id", W_FUNCTION),
    (b"lastval", W_FUNCTION),
    (b"least", W_FUNCTION),
    (b"len", W_FUNCTION),
    (b"length", W_FUNCTION),
    (b"ln", W_FUNCTION),
    (b"log", W_FUNCTION),
    (b"lpad", W_FUNCTION),
    (b"ltrim", W_FUNCTION),
    (b"md5", W_FUNCTION),
    (b"mod", W_FUNCTION),
    (b"newid", W_FUNCTION),
    (b"nextval", W_FUNCTION),
    (b"now", W_FUNCTION),
    (b"nullif", W_FUNCTION),
    (b"nvl", W_FUNCTION),
    (b"object_id", W_FUNCTION),
    (b"octet_length", W_FUNCTION),
    (b"patindex", W_FUNCTION),
    (b"pg_sleep", W_FUNCTION),
    (b"pg_typeof", W_FUNCTION),
    (b"position", W_FUNCTION),
    (b"power", W_FUNCTION),
    (b"quote_ident", W_FUNCTION),
    (b"quote_literal", W_FUNCTION),
    (b"quote_nullable", W_FUNCTION),
    (b"random", W_FUNCTION),
    (b"regexp_like", W_FUNCTION),
    (b"regexp_matches", W_FUNCTION),
    (b"regexp_replace", W_FUNCTION),
    (b"regexp_substr", W_FUNCTION),
    (b"reverse", W_FUNCTION),
    (b"round", W_FUNCTION),
    (b"row_to_json", W_FUNCTION),
    (b"rpad", W_FUNCTION),
    (b"rtrim", W_FUNCTION),
    (b"scope_identity", W_FUNCTION),
    (b"set_config", W_FUNCTION),
    (b"setval", W_FUNCTION),
    (b"sign", W_FUNCTION),
    (b"sleep", W_FUNCTION),
    (b"split_part", W_FUNCTION),
    (b"sqrt", W_FUNCTION),
    (b"str_to_date", W_FUNCTION),
    (b"string_to_array", W_FUNCTION),
    (b"strpos", W_FUNCTION),
    (b"stuff", W_FUNCTION),
    (b"substr", W_FUNCTION),
    (b"sysdatetime", W_FUNCTION),
    (b"sysutcdatetime", W_FUNCTION),
    (b"to_char", W_FUNCTION),
    (b"to_date", W_FUNCTION),
    (b"to_json", W_FUNCTION),
    (b"to_jsonb", W_FUNCTION),
    (b"to_number", W_FUNCTION),
    (b"to_timestamp", W_FUNCTION),
    (b"trunc", W_FUNCTION),
    (b"try_cast", W_FUNCTION),
    (b"try_convert", W_FUNCTION),
    (b"unix_timestamp", W_FUNCTION),
    (b"unnest", W_FUNCTION),
    (b"uuid_generate_v4", W_FUNCTION),
    // aggregates and window functions, before `(`
    (b"array_agg", W_AGGREGATE),
    (b"bool_and", W_AGGREGATE),
    (b"bool_or", W_AGGREGATE),
    (b"count_big", W_AGGREGATE),
    (b"cume_dist", W_AGGREGATE),
    (b"dense_rank", W_AGGREGATE),
    (b"first_value", W_AGGREGATE),
    (b"group_concat", W_AGGREGATE),
    (b"json_agg", W_AGGREGATE),
    (b"json_object_agg", W_AGGREGATE),
    (b"jsonb_agg", W_AGGREGATE),
    (b"jsonb_object_agg", W_AGGREGATE),
    (b"lag", W_AGGREGATE),
    (b"last_value", W_AGGREGATE),
    (b"lead", W_AGGREGATE),
    (b"listagg", W_AGGREGATE),
    (b"nth_value", W_AGGREGATE),
    (b"ntile", W_AGGREGATE),
    (b"percent_rank", W_AGGREGATE),
    (b"percentile_cont", W_AGGREGATE),
    (b"percentile_disc", W_AGGREGATE),
    (b"rank", W_AGGREGATE),
    (b"row_number", W_AGGREGATE),
    (b"stddev", W_AGGREGATE),
    (b"stddev_pop", W_AGGREGATE),
    (b"stddev_samp", W_AGGREGATE),
    (b"string_agg", W_AGGREGATE),
    (b"var_pop", W_AGGREGATE),
    (b"var_samp", W_AGGREGATE),
    (b"variance", W_AGGREGATE),
];

/// The table's list: [`ENTRIES`]' words.
const LIST: [&[u8]; ENTRIES.len()] = {
    let mut l: [&[u8]; ENTRIES.len()] = [b""; ENTRIES.len()];
    let mut i = 0;
    while i < l.len() {
        l[i] = ENTRIES[i].0;
        i += 1;
    }
    l
};

/// [`ENTRIES`]' words, ASCII case ignored: a word's id is its index plus one.
static WORDS: Words<2048, { ENTRIES.len() }, { text_len(&LIST) }, true> = Words::new(&LIST);

/// Each word's pattern word by its id, none at 0.
static INFO: [u8; ENTRIES.len() + 1] = {
    let mut t = [W_NONE; ENTRIES.len() + 1];
    let mut i = 0;
    while i < ENTRIES.len() {
        t[i + 1] = ENTRIES[i].1;
        i += 1;
    }
    t
};

/// The pattern word `w` starts, ASCII case ignored.
fn lookup(w: &[u8]) -> u8 {
    WORDS.value(w, &INFO)
}

/// The objects `drop` and `alter` take, each a whole word (bat's had no `\b` after them, so
/// `drop tables` colored `table`); bat's, then the dialects' (X-124).
const OBJECTS: &[&[u8]] = &[
    b"aggregate",
    b"conversion",
    b"database",
    b"domain",
    b"function",
    b"group",
    b"index",
    b"language",
    b"operator class",
    b"operator",
    b"procedure",
    b"rule",
    b"schema",
    b"sequence",
    b"table",
    b"tablespace",
    b"trigger",
    b"type",
    b"user",
    b"view",
    b"collation",
    b"event",
    b"extension",
    b"login",
    b"macro",
    b"policy",
    b"proc",
    b"publication",
    b"role",
    b"server",
    b"statistics",
    b"subscription",
    b"synonym",
];

/// What may stand between `drop` or `alter` and its object (X-124): `materialized view`,
/// `foreign table`, MySQL's `temporary table`.
const OBJECT_MODIFIERS: &[&[u8]] = &[b"materialized", b"foreign", b"temporary", b"temp"];

/// `create`'s, in extended mode (`operator class` is `operatorclass`), each followed by
/// `\b`; bat's, then the dialects' (X-124).
const CREATE_OBJECTS: &[&[u8]] = &[
    b"aggregate",
    b"conversion",
    b"database",
    b"domain",
    b"function",
    b"group",
    b"index",
    b"language",
    b"operatorclass",
    b"operator",
    b"procedure",
    b"rule",
    b"schema",
    b"sequence",
    b"tablespace",
    b"table",
    b"trigger",
    b"type",
    b"user",
    b"view",
    b"collation",
    b"event",
    b"extension",
    b"login",
    b"macro",
    b"policy",
    b"proc",
    b"publication",
    b"role",
    b"server",
    b"statistics",
    b"subscription",
    b"synonym",
];

/// What may stand between `create` (or `create or replace`) and its object, any number, each
/// a whole word (X-124; bat's took only `unique` before `index`).
const CREATE_MODIFIERS: &[&[u8]] = &[
    b"temp",
    b"temporary",
    b"unlogged",
    b"global",
    b"local",
    b"materialized",
    b"recursive",
    b"unique",
    b"clustered",
    b"nonclustered",
    b"virtual",
    b"external",
    b"foreign",
    b"trusted",
    b"procedural",
    b"constraint",
    b"fulltext",
];

/// `comment on`'s, each followed by `\s+`.
const COMMENT_OBJECTS: &[&[u8]] = &[
    b"table",
    b"column",
    b"aggregate",
    b"constraint",
    b"database",
    b"domain",
    b"function",
    b"index",
    b"operator",
    b"rule",
    b"schema",
    b"sequence",
    b"trigger",
    b"type",
    b"view",
];

/// The bytes `main`'s scan stops at: a word's, past ASCII, or one a pattern starts on.
static STOP: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric()
            || b >= 0x80
            || matches!(
                b,
                b'_' | b'-'
                    | b'#'
                    | b'/'
                    | b'*'
                    | b'<'
                    | b'>'
                    | b'!'
                    | b'='
                    | b'+'
                    | b'|'
                    | b'\''
                    | b'"'
                    | b'`'
                    | b'%'
                    | b'('
                    | b'$'
            );
        c += 1;
    }
    t
};

/// [`STOP`] and the bytes a host's pattern can start on, for an embedded scan.
static EMBED_STOP: [bool; 256] = {
    let mut t = STOP;
    let mut i = 0;
    let host = b"\\{}$\"'`%";
    while i < host.len() {
        t[host[i] as usize] = true;
        i += 1;
    }
    t
};

struct Scan<'b, 'o, 's, 'e> {
    b: &'b [u8],
    out: &'o mut Out<'s>,
    /// Where the last token ended, or where `main` took over: where syntect's search for
    /// the next match starts.
    mark: usize,
    /// Where the line's last `\s+(is)\s+` has its `is`, for `comment on`: 0 for none,
    /// `usize::MAX` until asked.
    is_end: usize,
    /// Embedded: where the scan took over from the host, whose patterns had their turn there.
    from: usize,
    /// The host, when embedded.
    embed: Option<&'e Embed<'e>>,
    /// Embedded: where the spans so far end.
    at: usize,
    /// The open dollar-quoted string's tag ([`DOLLAR`]).
    label: Label,
    /// On its own: whether the text is a dollar-quoted body's, so a dollar quote in it is a
    /// string.
    in_body: bool,
    /// Whether the last token was `as` or `do`: a dollar quote right after opens a body.
    after_as: bool,
    /// The tag of the body `main` opened ([`OPEN_BODY`]).
    opened: Option<Label>,
    /// On its own: a `DELIMITER` line seen, so the text is a MySQL client's script, where `$`
    /// quotes nothing.
    mysql: bool,
}

impl Scan<'_, '_, '_, '_> {
    fn tok(&mut self, start: usize, end: usize, kind: u16) {
        self.after_as = false;
        let e = end.min(self.b.len());
        match self.embed {
            None => self.out.push(start, e, kind),
            Some(host) => {
                if start > self.at && host.text != 0 {
                    self.out.push_merged(self.at, start, host.text);
                }
                if e > start {
                    self.out.push(start, e, host.base + kind);
                    self.at = e;
                } else {
                    self.at = self.at.max(start);
                }
            }
        }
        self.mark = end;
    }

    /// Whether the host's patterns get the place `p`, past where the scan took over.
    fn yields(&self, p: usize) -> bool {
        match self.embed {
            None => false,
            Some(host) => p > self.from && p < self.b.len() && host.yields[usize::from(self.b[p])],
        }
    }

    /// Where the `\s` before `p` starts, not before [`Scan::mark`]: a pattern's `\s*` takes
    /// them from the first place the search tries.
    fn spaces_before(&self, mut p: usize) -> usize {
        let b = self.b;
        while p > self.mark {
            let mut s = p - 1;
            while s > 0 && b[s] & 0xc0 == 0x80 {
                s -= 1;
            }
            match space_len(b, s) {
                Some(k) if s + k == p => p = s,
                _ => break,
            }
        }
        p
    }

    /// `main` from `p`: `None` at the line's end, or the context it pushes and where.
    fn main(&mut self, mut p: usize) -> Option<(usize, u8)> {
        let (b, n) = (self.b, self.b.len());
        self.mark = p;
        let stop = if self.embed.is_some() {
            // a word the host's token ended in the middle of: no `\b` before its rest
            if p > 0 && p < n && is_word(b, p) && is_word_before(b, p) {
                p = word_end(b, p);
            }
            &EMBED_STOP
        } else {
            &STOP
        };
        while p < n {
            if !stop[usize::from(b[p])] {
                p += 1;
                continue;
            }
            let c = b[p];
            if WORD[usize::from(c)] || (c >= 0x80 && is_word(b, p)) {
                // `N'…'`, `E'…'`, `B'…'`, `X'…'`: the prefix is the string's (X-124)
                if b.get(p + 1) == Some(&b'\'')
                    && matches!(c | 0x20, b'n' | b'e' | b'b' | b'x')
                    && !self.yields(p + 1)
                    && (p == 0 || !is_word_before(b, p))
                {
                    self.tok(p, p + 2, string_kind(SINGLE, BEGIN));
                    return Some((p + 2, SINGLE));
                }
                match self.word(p) {
                    Ok(q) => p = q,
                    Err(q) => return Some((q, CREATE)),
                }
                continue;
            }
            if self.yields(p) {
                return Some((p, MAIN));
            }
            let next = b.get(p + 1).copied();
            match c {
                b'-' if next == Some(b'-') => {
                    self.tok(p, p + 2, DASH_MARK);
                    if self.embed.is_some() {
                        return Some((p + 2, DASHES));
                    }
                    self.tok(p + 2, n, DASH);
                    return None;
                }
                b'#' => {
                    self.tok(p, p + 1, HASH_MARK);
                    if self.embed.is_some() {
                        return Some((p + 1, HASHES));
                    }
                    self.tok(p + 1, n, HASH);
                    return None;
                }
                b'/' if next == Some(b'*') => {
                    self.tok(p, p + 2, COMMENT_MARK);
                    return Some((p + 2, BLOCK));
                }
                b'*' => {
                    self.tok(p, p + 1, STAR);
                    p += 1;
                }
                // `<=>|[!<>]?=|<>|<|>`
                b'<' | b'>' | b'!' | b'=' => {
                    let e = match (c, next) {
                        (b'<', Some(b'=')) if b.get(p + 2) == Some(&b'>') => p + 3,
                        (b'<' | b'>' | b'!', Some(b'=')) => p + 2,
                        (b'<', Some(b'>')) => p + 2,
                        (b'!', _) => p,
                        _ => p + 1,
                    };
                    if e > p {
                        self.tok(p, e, COMPARISON);
                        p = e;
                    } else {
                        p += 1;
                    }
                }
                b'-' | b'+' | b'/' => {
                    self.tok(p, p + 1, MATH);
                    p += 1;
                }
                b'|' if next == Some(b'|') => {
                    self.tok(p, p + 2, CONCATENATOR);
                    p += 2;
                }
                b'\'' | b'"' | b'`' => {
                    let ctx = match c {
                        b'\'' => SINGLE,
                        b'"' => DOUBLE,
                        _ => BACKTICK,
                    };
                    self.tok(p, p + 1, string_kind(ctx, BEGIN));
                    return Some((p + 1, ctx));
                }
                b'%' if next == Some(b'{') => {
                    self.tok(p, p + 2, string_kind(BRACKETS, BEGIN));
                    return Some((p + 2, BRACKETS));
                }
                b'%' if next == Some(b'r') && b.get(p + 2) == Some(&b'{') => {
                    self.tok(p, p + 3, string_kind(MODR, BEGIN));
                    return Some((p + 3, MODR));
                }
                b'(' if next == Some(b')') => {
                    self.tok(p, p + 1, SCOPE_BEGIN);
                    self.tok(p + 1, p + 2, SCOPE_END);
                    p += 2;
                }
                b'$' if !self.mysql && (p == 0 || !(b[p - 1] == b'$' || is_word_before(b, p))) => {
                    match tag_end(b, p) {
                        Some(e) => {
                            if let Some(r) = self.dollar_quote(p, e) {
                                return Some(r);
                            }
                            p = self.mark;
                        }
                        None => p += 1,
                    }
                }
                _ => p += char_len(c),
            }
        }
        // a token took the line's `\n`: the host gets no turn at it
        if p > n && self.embed.is_some() {
            return Some((p, MAIN));
        }
        None
    }

    /// Whether `as` or `do` was the last token and only blanks stand between it and `p`.
    fn as_before(&self, p: usize) -> bool {
        self.after_as && self.b[self.mark.min(p)..p].iter().all(|&c| is_space(c))
    }

    /// The dollar quote `p..e` in `main` (X-124). On its own, a body's opening after `as` or
    /// `do` (outside a body: [`OPEN_BODY`]), else a string's ([`DOLLAR`]). Embedded, with no
    /// state between lines for a tag, a body's delimiter alone, or a string where it closes
    /// before the host's next place: `None`, `main` going on at [`Scan::mark`].
    fn dollar_quote(&mut self, p: usize, e: usize) -> Option<(usize, u8)> {
        let body = self.as_before(p);
        let label = Label::of(&self.b[p + 1..e - 1]);
        if self.embed.is_some() {
            let close = if body {
                None
            } else {
                find_tag(&self.b[..self.host_next(e)], e, label)
            };
            self.tok(p, e, DOLLAR_BEGIN);
            if let Some((s, t)) = close {
                self.tok(e, s, DOLLAR_TEXT);
                self.tok(s, t, DOLLAR_END);
            }
            return None;
        }
        self.tok(p, e, DOLLAR_BEGIN);
        if body && !self.in_body {
            self.opened = Some(label);
            return Some((e, OPEN_BODY));
        }
        self.label = label;
        Some((e, DOLLAR))
    }

    /// A dollar-quoted string from `p`: `None` at the region's end, or `main` past its tag.
    fn dollar(&mut self, p: usize) -> Option<(usize, u8)> {
        match find_tag(self.b, p, self.label) {
            Some((s, e)) => {
                self.tok(p, s, DOLLAR_TEXT);
                self.tok(s, e, DOLLAR_END);
                Some((e, MAIN))
            }
            None => {
                self.tok(p, self.b.len(), DOLLAR_TEXT);
                None
            }
        }
    }

    /// Embedded, a line comment from `p` (its mark's end): its text up to the host's next
    /// place, or to the line's end, where it pops.
    fn line_comment(&mut self, ctx: u8, p: usize) -> Option<(usize, u8)> {
        let n = self.b.len();
        if p >= n {
            return Some((n + 1, MAIN));
        }
        let mut q = p;
        while q < n && !self.yields(q) {
            q += 1;
        }
        self.tok(p, q, if ctx == DASHES { DASH } else { HASH });
        if q < n { Some((q, ctx)) } else { None }
    }

    /// The patterns of `main` at `p`, a word's start: where the scan goes on in `main`, or
    /// (`Err`) where `identifier_create` is pushed.
    fn word(&mut self, p: usize) -> Result<usize, usize> {
        let b = self.b;
        let e = word_end(b, p);
        let id = lookup(&b[p..e]);
        if id != W_NONE {
            if let Some(r) = self.keyword(id, p, e) {
                return r;
            }
            if let Some(kind) = self.alone(id, p, e) {
                self.tok(p, e, kind);
                return Ok(e);
            }
        }
        // `\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b`, a fraction or an exponent: bat's grammar takes
        // only `\b\d+\b`, so `1.5` read as two numbers and `1e3` plain (X-008)
        if let Some(d) = self.decimal(p) {
            self.tok(p, d, NUMBER);
            return Ok(d);
        }
        // `\b\d+\b`
        if self.all_digits(p, e) {
            self.tok(p, e, NUMBER);
            return Ok(e);
        }
        // `\b(\w+?)\.(\w+)\b`
        if b.get(e) == Some(&b'.') && is_word(&b[..self.b.len()], e + 1) {
            let t = word_end(b, e + 1);
            self.tok(p, e, DATABASE);
            self.tok(e + 1, t, TABLE);
            return Ok(t);
        }
        Ok(e)
    }

    /// The kind a word that starts one of bat's compound patterns takes where the pattern
    /// doesn't follow (X-124): `on` without `delete`, `is` without `null`, `float` without a
    /// size, `create` before an object no list has, `left(` a function. `None` for plain.
    fn alone(&self, id: u8, p: usize, e: usize) -> Option<u16> {
        Some(match id {
            W_CREATE | W_DROP | W_ALTER | W_ON => KEYWORD,
            W_DOUBLE | W_SIZED | W_CHARACTER => TYPE,
            W_KEY => MODIFIER,
            W_BY | W_INNER | W_NATURAL | W_REPLACE => DML,
            W_SIDE if self.paren(e) => STRING_FUNCTION,
            W_SIDE if at_ci(self.b, p, b"full") => DML,
            W_IS | W_NOT | W_ISNULL => LOGICAL,
            W_REPEAT => CONTROL,
            _ => return None,
        })
    }

    /// Whether `(` follows `e`, past `\s*`.
    fn paren(&self, e: usize) -> bool {
        self.b.get(self.spaces(e)) == Some(&b'(')
    }

    /// The pattern word `id`, at `p` to `e`, starts: `None` if it doesn't match.
    fn keyword(&mut self, id: u8, p: usize, e: usize) -> Option<Result<usize, usize>> {
        let kind = match id {
            W_CREATE => return self.create_statement(p, e).map(Err),
            W_DROP => return self.drop(p, e).map(Ok),
            W_ALTER => return self.alter(p, e).map(Ok),
            W_TYPE => TYPE,
            W_DOUBLE..=W_TIMESTAMP => return self.sized_type(id, p, e).map(Ok),
            W_KEY => {
                let q = self.spaces1(e)?;
                let k = self.keyword_at(q, b"key")?;
                self.tok(p, k, MODIFIER);
                return Some(Ok(k));
            }
            W_MODIFIER => MODIFIER,
            W_ON => {
                // `on\sdelete(\s+cascade)?\b`, `on\supdate(\s+cascade)?\b`
                let q = self.space(e)?;
                let k = self
                    .keyword_at(q, b"delete")
                    .or_else(|| self.keyword_at(q, b"update"))?;
                let k = self.then_word(k, b"cascade").unwrap_or(k);
                self.tok(p, k, MODIFIER);
                return Some(Ok(k));
            }
            W_BOOLEAN => BOOLEAN,
            W_DML => DML,
            W_SELECT => {
                let k = self
                    .then_word(e, b"distinct")
                    .or_else(|| self.then_word(e, b"top"))
                    .unwrap_or(e);
                self.tok(p, k, DML);
                return Some(Ok(k));
            }
            W_INSERT => {
                let into = |s: &Self, q: usize| s.then_word(q, b"into");
                let k = self
                    .then_word(e, b"ignore")
                    .and_then(|q| into(self, q))
                    .or_else(|| into(self, e))
                    .unwrap_or(e);
                self.tok(p, k, DML);
                return Some(Ok(k));
            }
            W_BY => {
                let k = self.then_word(e, b"by")?;
                self.tok(p, k, DML);
                return Some(Ok(k));
            }
            W_UNION => {
                let k = self.then_word(e, b"all").unwrap_or(e);
                self.tok(p, k, DML);
                return Some(Ok(k));
            }
            W_INNER => {
                // `(inner|cross)\s+join`; T-SQL's `(cross|outer)\s+apply`
                let k = self
                    .then_word(e, b"join")
                    .or_else(|| self.then_word(e, b"apply"))?;
                self.tok(p, k, DML);
                return Some(Ok(k));
            }
            W_SIDE => {
                let k = self.side_join(e)?;
                self.tok(p, k, DML);
                return Some(Ok(k));
            }
            W_NATURAL => {
                // `natural(\s+(left|right)(\s+outer)?)?\s+join`
                let side = self
                    .then_word(e, b"left")
                    .or_else(|| self.then_word(e, b"right"));
                let k = side
                    .and_then(|q| {
                        let o = self.then_word(q, b"outer");
                        o.and_then(|o| self.then_word(o, b"join"))
                            .or_else(|| self.then_word(q, b"join"))
                    })
                    .or_else(|| self.then_word(e, b"join"))?;
                self.tok(p, k, DML);
                return Some(Ok(k));
            }
            W_IS | W_NOT | W_NULL => return self.null(id, p, e).map(Ok),
            W_LOGICAL => LOGICAL,
            W_VALUES => DML2,
            W_WORK => {
                // T-SQL's `tran` and `transaction` too
                let k = self
                    .then_word(e, b"work")
                    .or_else(|| self.then_word(e, b"transaction"))
                    .or_else(|| self.then_word(e, b"tran"))
                    .unwrap_or(e);
                self.tok(p, k, LUW);
                return Some(Ok(k));
            }
            W_START => {
                let k = self.then_word(e, b"transaction")?;
                self.tok(p, k, LUW);
                return Some(Ok(k));
            }
            W_GRANT => {
                // `grant(\swith\sgrant\soption)?\b`
                let k = self
                    .space(e)
                    .and_then(|q| self.keyword_at(q, b"with"))
                    .and_then(|q| self.space(q))
                    .and_then(|q| self.keyword_at(q, b"grant"))
                    .and_then(|q| self.space(q))
                    .and_then(|q| self.keyword_at(q, b"option"))
                    .unwrap_or(e);
                self.tok(p, k, AUTHORIZATION);
                return Some(Ok(k));
            }
            W_REVOKE => AUTHORIZATION,
            W_COMMENT => {
                // `\s*\b(comment…`: its scope takes the spaces before
                let k = self.comment_on(e)?;
                self.tok(self.spaces_before(p), k, OBJECT_COMMENTS);
                return Some(Ok(k));
            }
            W_AS | W_DO => {
                self.tok(p, e, if id == W_AS { ALIAS } else { KEYWORD });
                self.after_as = true;
                return Some(Ok(e));
            }
            W_ORDER => ORDER,
            W_SCALAR => SCALAR,
            W_AGGREGATE => {
                // `(?=\s*\()`
                let q = self.spaces(e);
                if self.b.get(q) != Some(&b'(') {
                    return None;
                }
                AGGREGATE
            }
            W_STRING_FUNCTION => STRING_FUNCTION,
            W_KEYWORD => KEYWORD,
            W_CONTROL => CONTROL,
            W_LUW => LUW,
            W_FUNCTION | W_ISNULL if self.paren(e) => SCALAR,
            W_REPLACE | W_REPEAT if self.paren(e) => STRING_FUNCTION,
            W_GO => {
                // alone on its line, a count after it allowed
                let b = &self.b[..self.b.len()];
                let mut q = e;
                while q < b.len() && (is_space(b[q]) || b[q].is_ascii_digit()) {
                    q += 1;
                }
                if q < b.len() || !b[..p].iter().all(|&c| is_space(c)) {
                    return None;
                }
                KEYWORD
            }
            W_LANGUAGE => {
                let was = self.after_as;
                self.tok(p, e, KEYWORD);
                let q = self
                    .spaces1(e)
                    .filter(|&q| is_word(&self.b[..self.b.len()], q));
                if was && let Some(q) = q {
                    self.mark = word_end(self.b, q);
                    self.after_as = true;
                    return Some(Ok(self.mark));
                }
                return Some(Ok(e));
            }
            W_DELIMITER if self.embed.is_none() && self.b[..p].iter().all(|&c| is_space(c)) => {
                self.tok(p, e, KEYWORD);
                self.mysql = true;
                return Some(Ok(self.b.len()));
            }
            W_EACH => {
                let k = self
                    .then_word(e, b"row")
                    .or_else(|| self.then_word(e, b"statement"))
                    .unwrap_or(e);
                self.tok(p, k, KEYWORD);
                return Some(Ok(k));
            }
            W_NULLS => {
                let k = self
                    .then_word(e, b"first")
                    .or_else(|| self.then_word(e, b"last"))
                    .or_else(|| self.then_word(e, b"distinct"))
                    .unwrap_or(e);
                self.tok(p, k, KEYWORD);
                return Some(Ok(k));
            }
            _ => return None,
        };
        self.tok(p, e, kind);
        Some(Ok(e))
    }

    /// `(left|right)(\s+outer)?\s+join` after `left` or `right` ends at `e`.
    fn side_join(&self, e: usize) -> Option<usize> {
        self.then_word(e, b"outer")
            .and_then(|o| self.then_word(o, b"join"))
            .or_else(|| self.then_word(e, b"join"))
    }

    /// `\b(create(?:\s+or\s+(?:replace|alter))?)\s+(?:(modifiers)\s+)*(objects)\b\s*`, `create`
    /// at `p` to `e`: where `identifier_create` is pushed. T-SQL's `or alter` and the modifiers
    /// are X-124's.
    fn create_statement(&mut self, p: usize, e: usize) -> Option<usize> {
        let replace = self.then_word(e, b"or").and_then(|q| {
            self.then_word(q, b"replace")
                .or_else(|| self.then_word(q, b"alter"))
        });
        for c1 in [replace, Some(e)].into_iter().flatten() {
            let Some(s) = self.spaces1(c1) else {
                continue;
            };
            let m = self.modifiers(s, CREATE_MODIFIERS);
            if let Some(o) = CREATE_OBJECTS.iter().find_map(|o| self.keyword_at(m, o)) {
                self.tok(p, c1, CREATE_KEYWORD);
                self.tok(s, o, CREATE_OBJECT);
                return Some(self.spaces(o));
            }
        }
        None
    }

    /// Past the words of `list` from `p`, each a whole word and `\s+` after: where the next
    /// word starts (`p` for none).
    fn modifiers(&self, mut p: usize, list: &[&[u8]]) -> usize {
        while let Some(q) = list
            .iter()
            .find_map(|w| self.keyword_at(p, w))
            .and_then(|q| self.spaces1(q))
        {
            p = q;
        }
        p
    }

    /// `\s*\b(drop)\s+(?:(modifiers)\s+)*(objects)\b`, `drop` at `p` to `e`: where the scan
    /// goes on. bat's objects had no `\b` after them (`drop tables` colored `table`), and its
    /// `drop table` with a name and `cascade` matched only with no `\b` before `drop`
    /// (`backdrop table x`); here `drop table` reads its name (X-124).
    fn drop(&mut self, p: usize, e: usize) -> Option<usize> {
        let s = self.spaces1(e)?;
        let m = self.modifiers(s, OBJECT_MODIFIERS);
        let o = OBJECTS.iter().find_map(|o| self.keyword_at(m, o))?;
        if m == s
            && o == s + 5
            && at_ci(self.b, s, b"table")
            && let Some(q) = self.drop_table(p, e, s)
        {
            return Some(q);
        }
        self.tok(p, e, DROP_KEYWORD);
        self.tok(s, o, DROP_OBJECT);
        Some(o)
    }

    /// `(drop)\s+(table)\s+(?:(if)\s+(exists)\s+)?(\w+)(\s+cascade)?\b`, `drop` at `p` to
    /// `e` and `table` at `t`: where it ends.
    fn drop_table(&mut self, p: usize, e: usize, t: usize) -> Option<usize> {
        let b = &self.b[..self.b.len()];
        let mut nm = self.spaces1(t + 5)?;
        let exists = self
            .keyword_at(nm, b"if")
            .and_then(|i| self.then_word(i, b"exists"));
        let at = nm;
        if let Some(x) = exists {
            nm = self.spaces1(x)?;
        }
        if !is_word(b, nm) {
            return None;
        }
        let ne = word_end(b, nm);
        let cascade = self.then_word(ne, b"cascade");
        self.tok(p, e, DROP_KEYWORD);
        self.tok(t, t + 5, DROP_TABLE);
        if let Some(x) = exists {
            self.tok(at, at + 2, CONTROL);
            self.tok(x - 6, x, LOGICAL);
        }
        self.tok(nm, ne, DROP_NAME);
        if let Some(c) = cascade {
            self.tok(ne, c, DROP_CASCADE);
            return Some(c);
        }
        Some(ne)
    }

    /// `\s*\b(alter)\s+(?:(modifiers)\s+)*(objects)\s+`, `alter` at `p` to `e`: where it ends.
    fn alter(&mut self, p: usize, e: usize) -> Option<usize> {
        let s = self.spaces1(e)?;
        let m = self.modifiers(s, OBJECT_MODIFIERS);
        let (o, end) = OBJECTS.iter().find_map(|o| {
            if !at_ci(&self.b[..self.b.len()], m, o) {
                return None;
            }
            let q = m + o.len();
            Some((q, self.spaces1(q)?))
        })?;
        self.tok(p, e, ALTER_KEYWORD);
        self.tok(s, o, ALTER_OBJECT);
        // the spaces after are the match's too: the next search starts past them
        self.mark = end;
        Some(end)
    }

    /// The types' forms past the first, the word `id` at `p` to `e`: where the type ends.
    fn sized_type(&mut self, id: u8, p: usize, e: usize) -> Option<usize> {
        match id {
            W_DOUBLE => {
                // `double\sprecision`
                let q = self.space(e)?;
                let k = self.keyword_at(q, b"precision")?;
                self.tok(p, k, TYPE);
                Some(k)
            }
            W_SIZED | W_CHARACTER | W_VAR => {
                // `\b(bit\svarying|character\s(?:varying)?|tinyint|var\schar|float|interval)\((\d+)\)`
                // (`bit` is the first form's)
                let t = match id {
                    W_SIZED => e,
                    W_CHARACTER => {
                        // MySQL's `character set`
                        if let Some(k) = self.then_word(e, b"set") {
                            self.tok(p, k, KEYWORD);
                            return Some(k);
                        }
                        let q = self.space(e)?;
                        let v = q + b"varying".len();
                        if at_ci(&self.b[..self.b.len()], q, b"varying") && self.size(v).is_some() {
                            v
                        } else {
                            q
                        }
                    }
                    _ => {
                        let q = self.space(e)?;
                        at_ci(&self.b[..self.b.len()], q, b"char").then_some(q + 4)?
                    }
                };
                let d = self.size(t)?;
                self.tok(p, t, TYPE);
                self.tok(t + 1, d, NUMBER);
                Some(d + 1)
            }
            W_CHAR => {
                // `(char|number|nvarchar|varbinary|varchar\d?)\b(?:\((\d+)\))?`
                self.tok(p, e, TYPE);
                Some(self.optional_size(e))
            }
            W_NUMERIC => {
                // `(numeric|decimal)\b(?:\((\d+),(\d+)\))?`
                self.tok(p, e, TYPE);
                let b = &self.b[..self.b.len()];
                if b.get(e) == Some(&b'(') {
                    let d1 = digits0(b, e + 1);
                    if d1 > e + 1 && b.get(d1) == Some(&b',') {
                        let d2 = digits0(b, d1 + 1);
                        if d2 > d1 + 1 && b.get(d2) == Some(&b')') {
                            self.tok(e + 1, d1, NUMBER);
                            self.tok(d1 + 1, d2, NUMBER);
                            return Some(d2 + 1);
                        }
                    }
                }
                Some(e)
            }
            W_TIME => {
                // `(times?)\b(?:\((\d+)\))?(\swith(?:out)?\stime\szone\b)?`
                self.tok(p, e, TYPE);
                let q = self.optional_size(e);
                Some(self.time_zone(q))
            }
            _ => {
                // `(timestamp)(?:(s|tz))?\b(?:\((\d+)\))?(\s(with|without)\stime\szone\b)?`
                self.tok(p, e, TYPE);
                let q = self.optional_size(e);
                Some(self.time_zone(q))
            }
        }
    }

    /// `\((\d+)\)` at `p`: where its digits end.
    fn size(&self, p: usize) -> Option<usize> {
        let b = &self.b[..self.b.len()];
        if b.get(p) != Some(&b'(') {
            return None;
        }
        let d = digits0(b, p + 1);
        (d > p + 1 && b.get(d) == Some(&b')')).then_some(d)
    }

    /// `(?:\((\d+)\))?` at `p`: where it ends.
    fn optional_size(&mut self, p: usize) -> usize {
        match self.size(p) {
            Some(d) => {
                self.tok(p + 1, d, NUMBER);
                d + 1
            }
            None => p,
        }
    }

    /// `(\swith(?:out)?\stime\szone\b)?` at `p`, a type: where it ends.
    fn time_zone(&mut self, p: usize) -> usize {
        let z = self
            .space(p)
            .and_then(|q| {
                self.keyword_at(q, b"with")
                    .or_else(|| self.keyword_at(q, b"without"))
            })
            .and_then(|q| self.space(q))
            .and_then(|q| self.keyword_at(q, b"time"))
            .and_then(|q| self.space(q))
            .and_then(|q| self.keyword_at(q, b"zone"));
        match z {
            Some(z) => {
                self.tok(p, z, TYPE);
                z
            }
            None => p,
        }
    }

    /// `\b(?:(is)\s+)?(?:(not)\s+)?(null)\b`, the word `id` at `p` to `e`.
    fn null(&mut self, id: u8, p: usize, e: usize) -> Option<usize> {
        // `(?:(not)\s+)?(null)\b` at `q`: `not`'s end if it's there, and `null`'s place
        let tail = |s: &Self, q: usize| -> Option<(Option<usize>, usize, usize)> {
            if let Some(nq) = s.keyword_at(q, b"not")
                && let Some(a) = s.spaces1(nq)
                && let Some(z) = s.keyword_at(a, b"null")
            {
                return Some((Some(nq), a, z));
            }
            s.keyword_at(q, b"null").map(|z| (None, q, z))
        };
        let (not_at, (not, a, z)) = match id {
            W_IS => {
                let q = self.spaces1(e)?;
                let t = tail(self, q)?;
                self.tok(p, e, LOGICAL);
                (q, t)
            }
            _ => (p, tail(self, p)?),
        };
        if let Some(nq) = not {
            self.tok(not_at, nq, LOGICAL);
        }
        self.tok(a, z, NULL);
        Some(z)
    }

    /// `\s+.*?\s+(is)\s+` after `comment`'s `\s+on\s+(objects)`, `comment` ending at `e`:
    /// where it ends. The first `\s+` is greedy and `.*?` lazy, short of the `\n`.
    fn comment_on(&mut self, e: usize) -> Option<usize> {
        let q = self.then_word(e, b"on")?;
        let s = self.spaces1(q)?;
        let o = COMMENT_OBJECTS
            .iter()
            .find(|o| at_ci(&self.b[..self.b.len()], s, o))
            .map(|o| s + o.len())?;
        let mut starts = Vec::new();
        let mut q = o;
        while let Some(e) = self.space(q) {
            starts.push(e);
            q = e;
        }
        // syntect's search fails once for the line where no `\s+is\s+` follows; here the lazy
        // search runs only where one does, so it stops at the first, and the scan stays
        // linear on a line of `comment on`s without one
        let last = self.last_is();
        for &start in starts.iter().rev() {
            if start >= last {
                continue;
            }
            let mut q = start;
            loop {
                if let Some(e) = self
                    .spaces1(q)
                    .filter(|&a| at_ci(&self.b[..self.b.len()], a, b"is"))
                    .and_then(|a| self.spaces1(a + 2))
                {
                    return Some(e);
                }
                if q >= self.b.len() {
                    break;
                }
                q += char_len(self.b[q]);
            }
        }
        None
    }

    /// Where the line's last `is` with a `\s` before it and a `\s` (or the line's `\n`) after
    /// it starts: 0 for none. Found once a line.
    fn last_is(&mut self) -> usize {
        if self.is_end == usize::MAX {
            let b = &self.b[..self.b.len()];
            self.is_end = (1..self.b.len().saturating_sub(1))
                .rev()
                .find(|&r| {
                    at_ci(b, r, b"is")
                        && self.space(r + 2).is_some()
                        && (r.saturating_sub(3)..r).any(|s| space_len(b, s) == Some(r - s))
                })
                .unwrap_or(0);
        }
        self.is_end
    }

    /// `\s+` and then the word `w` (ASCII case ignored) and `\b`, after `p`: where it ends.
    fn then_word(&self, p: usize, w: &[u8]) -> Option<usize> {
        let q = self.spaces1(p)?;
        self.keyword_at(q, w)
    }

    /// The word `w` at `p` (ASCII case ignored), then `\b`: where it ends.
    fn keyword_at(&self, p: usize, w: &[u8]) -> Option<usize> {
        let b = &self.b[..self.b.len()];
        let e = p + w.len();
        (at_ci(b, p, w) && !is_word(b, e)).then_some(e)
    }

    /// `\s` at `p`, the line's `\n` one: where it ends.
    fn space(&self, p: usize) -> Option<usize> {
        if p == self.b.len() {
            return Some(p + 1);
        }
        space_len(&self.b[..self.b.len()], p).map(|k| p + k)
    }

    /// `\s*` at `p`, the line's `\n` taken: where it ends.
    fn spaces(&self, mut p: usize) -> usize {
        while let Some(e) = self.space(p) {
            p = e;
        }
        p
    }

    /// `\s+` at `p`, the line's `\n` taken: where it ends.
    fn spaces1(&self, p: usize) -> Option<usize> {
        self.space(p).map(|e| self.spaces(e))
    }

    /// An ASCII number with a fraction or an exponent at `p`, a word's start, up to a word's
    /// end and the host's next place: where it ends. `None` for digits alone, or anything else.
    fn decimal(&self, p: usize) -> Option<usize> {
        // the byte at `q`, unless it is past the line or the host's
        let at = |q: usize| (q < self.b.len() && !self.yields(q)).then(|| self.b[q]);
        let digits = |mut q: usize| {
            while at(q).is_some_and(|c| c.is_ascii_digit()) {
                q += 1;
            }
            q
        };
        let i = digits(p);
        if i == p {
            return None;
        }
        let mut end = i;
        if at(i) == Some(b'.') {
            let f = digits(i + 1);
            if f > i + 1 {
                end = f;
            }
        }
        if matches!(at(end), Some(b'e' | b'E')) {
            let s = end + 1 + usize::from(matches!(at(end + 1), Some(b'+' | b'-')));
            let x = digits(s);
            if x > s {
                end = x;
            }
        }
        let boundary = at(end).is_none() || !is_word(&self.b[..self.b.len()], end);
        (end > i && boundary).then_some(end)
    }

    /// Whether `p..e`, a word, is all `\d`.
    fn all_digits(&self, p: usize, e: usize) -> bool {
        let w = &self.b[p..e];
        if w.iter().all(u8::is_ascii_digit) {
            return true;
        }
        w.iter().any(|&c| c >= 0x80) && digits0(&self.b[..e], p) == e
    }

    /// A block comment from `p`: `None` at the line's end, or `main` past its `*/`.
    fn block(&mut self, mut p: usize) -> Option<(usize, u8)> {
        let (b, n) = (self.b, self.b.len());
        if p == 0 {
            // `^\s*(\*)(?!/)`
            let mut q = 0;
            while let Some(k) = space_len(b, q) {
                q += k;
            }
            if b.get(q) == Some(&b'*') && b.get(q + 1) != Some(&b'/') {
                self.tok(0, q, COMMENT);
                self.tok(q, q + 1, COMMENT_MARK);
                p = q + 1;
            }
        }
        // the host's next place: its patterns come first there
        let lim = self.host_next(p);
        let mut q = p;
        while let Some(i) = b[q.min(lim)..lim].iter().position(|&c| c == b'*') {
            let star = q + i;
            if b.get(star + 1) == Some(&b'/') {
                self.tok(p, star + 2, COMMENT);
                return Some((star + 2, MAIN));
            }
            q = star + 1;
        }
        self.tok(p, lim, COMMENT);
        if lim < n { Some((lim, BLOCK)) } else { None }
    }

    /// The first place from `p` the host's patterns get, or the line's end.
    fn host_next(&self, p: usize) -> usize {
        if self.embed.is_none() {
            return self.b.len();
        }
        let mut q = p;
        while q < self.b.len() && !self.yields(q) {
            q += 1;
        }
        q
    }

    /// String `ctx` from `p`: `None` at the line's end, or `main` past its end.
    fn string(&mut self, ctx: u8, mut p: usize) -> Option<(usize, u8)> {
        let (b, n) = (self.b, self.b.len());
        let quote = match ctx {
            SINGLE => b'\'',
            DOUBLE => b'"',
            BACKTICK => b'`',
            _ => b'}',
        };
        // `\\.` in `'…'` and `` `…` ``; `#{…}` in the rest
        let other = match ctx {
            SINGLE | BACKTICK => b'\\',
            _ => b'#',
        };
        let mut text = p;
        // the host's next place (the line's end on its own), where its patterns come first
        let mut lim = self.host_next(p);
        loop {
            p = crate::prim::find_in(b, p, lim, [quote, other]);
            if p >= lim {
                if p >= n {
                    break;
                }
                if p == lim {
                    self.tok(text, p, string_kind(ctx, TEXT));
                    return Some((p, ctx));
                }
                // an escape took the host's place: the next one from here
                lim = self.host_next(p);
                continue;
            }
            let c = b[p];
            if c == quote {
                if matches!(ctx, SINGLE | DOUBLE) && b.get(p + 1) == Some(&quote) {
                    self.tok(text, p, string_kind(ctx, TEXT));
                    self.tok(p, p + 2, string_kind(ctx, ESCAPE));
                    p += 2;
                    text = p;
                    continue;
                }
                self.tok(text, p, string_kind(ctx, TEXT));
                self.tok(p, p + 1, string_kind(ctx, END));
                return Some((p + 1, MAIN));
            }
            if c == b'\\' {
                if p + 1 < n {
                    let e = p + 1 + char_len(b[p + 1]);
                    self.tok(text, p, string_kind(ctx, TEXT));
                    self.tok(p, e, string_kind(ctx, ESCAPE));
                    p = e;
                    text = p;
                } else {
                    p += 1;
                }
                continue;
            }
            // `(#\{)([^\}]*)(\})`
            if b.get(p + 1) == Some(&b'{')
                && let Some(i) = b[p + 2..].iter().position(|&c| c == b'}')
            {
                let e = p + 2 + i;
                self.tok(text, p, string_kind(ctx, TEXT));
                self.tok(p, p + 2, string_kind(ctx, INTERPOLATION_BEGIN));
                self.tok(p + 2, e, string_kind(ctx, INTERPOLATION));
                self.tok(e, e + 1, string_kind(ctx, INTERPOLATION_END));
                p = e + 1;
                text = p;
                continue;
            }
            p += 1;
        }
        self.tok(text, n, string_kind(ctx, TEXT));
        None
    }

    /// `identifier_create` from `p`: `main` where it pops. It pops by the line's end at the
    /// latest: past a `\n` that `create`'s `\s*` took, `{{end_identifier}}`'s `$` holds.
    fn create(&mut self, mut p: usize) -> Option<(usize, u8)> {
        let (b, n) = (self.b, self.b.len());
        loop {
            if p > n {
                return Some((p, MAIN));
            }
            if self.yields(p) {
                return Some((p, CREATE));
            }
            // X-124: `if not exists` and `concurrently` aren't the name, and `on` ends it (an
            // index's `ON t` with no name before)
            if let Some(q) = self.create_keywords(p) {
                p = q;
                continue;
            }
            if self.keyword_at(p, b"on").is_some() {
                return Some((p, MAIN));
            }
            if let Some((s, e, end)) = self.create_name(p) {
                // `(?:\w+|'[^']+'|"[^"]+"|`[^`]+`)\s*(\.)`
                let mut d = end;
                while let Some(k) = space_len(b, d) {
                    d += k;
                }
                if b.get(d) == Some(&b'.') {
                    self.tok(d, d + 1, CREATE_DOT);
                    p = d + 1;
                    continue;
                }
                // the first name is the object's (X-124): bat's took a name only before
                // `{{end_identifier}}`, so the last word before a `(` (`CREATE TYPE x AS ENUM (`
                // named `ENUM`, `CREATE INDEX i ON t (` named `t`)
                self.tok(s, e, CREATE_NAME);
                return Some((end, MAIN));
            }
            if self.end_identifier(p) {
                return Some((p, MAIN));
            }
            // nothing matches here: a character on, or past a run of ` ` and `\t`, where
            // nothing matches either and `{{end_identifier}}` reads the same
            if matches!(b[p], b' ' | b'\t') {
                while matches!(b.get(p), Some(b' ' | b'\t')) {
                    p += 1;
                }
            } else {
                p += char_len(b[p]);
            }
        }
    }

    /// `if\s+not\s+exists` or `concurrently` at `p` in `identifier_create`, colored: where it
    /// ends.
    fn create_keywords(&mut self, p: usize) -> Option<usize> {
        if let Some(i) = self.keyword_at(p, b"if") {
            let s = self.spaces1(i)?;
            let x = self
                .keyword_at(s, b"not")
                .and_then(|q| self.then_word(q, b"exists"))?;
            self.tok(p, i, CONTROL);
            self.tok(s, s + 3, LOGICAL);
            self.tok(x - 6, x, LOGICAL);
            return Some(x);
        }
        let c = self.keyword_at(p, b"concurrently")?;
        self.tok(p, c, KEYWORD);
        Some(c)
    }

    /// A name at `p`, `\w+` or one in quotes: its text's start and end, and its end.
    fn create_name(&self, p: usize) -> Option<(usize, usize, usize)> {
        let b = &self.b[..self.b.len()];
        match *b.get(p)? {
            quote @ (b'\'' | b'"' | b'`') => {
                let i = b[p + 1..].iter().position(|&c| c == quote)?;
                (i > 0).then_some((p + 1, p + 1 + i, p + 2 + i))
            }
            _ if is_word(b, p) => {
                let e = word_end(b, p);
                Some((p, e, e))
            }
            _ => None,
        }
    }

    /// `{{end_identifier}}` at `p`: `(?=[ \t]*(?:[^\w'"`. \t]|$))`.
    fn end_identifier(&self, p: usize) -> bool {
        let b = &self.b[..self.b.len()];
        let mut r = p;
        while matches!(b.get(r), Some(b' ' | b'\t')) {
            r += 1;
        }
        r >= self.b.len() || !(is_word(b, r) || matches!(b[r], b'\'' | b'"' | b'`' | b'.'))
    }
}

/// String `ctx`'s kind at `offset` ([`TEXT`] to [`INTERPOLATION_END`]).
fn string_kind(ctx: u8, offset: u16) -> u16 {
    STRINGS + 7 * u16::from(ctx - SINGLE) + offset
}

/// Whether the character before byte `p` of `b` is a word's.
fn is_word_before(b: &[u8], p: usize) -> bool {
    let mut s = p - 1;
    while s > 0 && b[s] & 0xc0 == 0x80 {
        s -= 1;
    }
    is_word(b, s)
}

/// `\d*` at `p`: where it ends.
fn digits0(b: &[u8], mut p: usize) -> usize {
    while let Some(e) = digit(b, p) {
        p = e;
    }
    p
}

/// Whether `w` starts the bytes at `p`, ignoring ASCII case.
fn at_ci(b: &[u8], p: usize, w: &[u8]) -> bool {
    b.len() >= p + w.len() && b[p..p + w.len()].eq_ignore_ascii_case(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn kinds(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::Sql, lines)
    }

    /// The fuzzer's timeout (a 64 KB line of `comment on table`s with no `is`, each searching
    /// the rest of the line from every start) and `identifier_create` stepping a long run of
    /// spaces: both linear now, so these take microseconds, not minutes.
    #[test]
    fn long_lines_stay_linear() {
        let comments = "COMMENT ON TABLE\x0c\x0c\x0c\x0c t x ".repeat(3000);
        let create = format!("CREATE TABLE 'a{}x", " ".repeat(60000));
        let t = std::time::Instant::now();
        let (mut state, mut spans) = (State::default(), Vec::new());
        for line in [&comments, &create] {
            crate::lex(Lang::Sql, line, &mut state, &mut spans);
        }
        assert!(t.elapsed().as_secs() < 2, "{:?}", t.elapsed());
        assert_eq!(
            kinds(&["comment on table t  is x is 'y'"]),
            [
                (
                    "comment on table t  is ",
                    "keyword.other.object-comments.sql"
                ),
                ("is", "keyword.operator.logical.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.begin.sql"
                ),
                ("y", "string.quoted.single.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.end.sql"
                ),
            ]
        );
    }

    #[test]
    fn the_table_finds_its_words() {
        for &(w, k) in ENTRIES {
            assert_eq!(lookup(w), k, "{}", String::from_utf8_lossy(w));
            assert_eq!(lookup(&w.to_ascii_uppercase()), k);
        }
        assert_eq!(lookup(b"selects"), W_NONE);
        assert_eq!(lookup(b"current_timestamps"), W_NONE);
    }

    #[test]
    fn keywords_types_and_names() {
        assert_eq!(
            kinds(&["SELECT DISTINCT a.b, count(*) FROM t WHERE x IS NOT NULL AND y = 1;"]),
            [
                ("SELECT DISTINCT", "keyword.other.DML.sql"),
                ("a", "constant.other.database-name.sql"),
                ("b", "constant.other.table-name.sql"),
                ("count", "support.function.aggregate.sql"),
                ("*", "variable.language.star.sql"),
                ("FROM", "keyword.other.DML.sql"),
                ("WHERE", "keyword.other.DML.sql"),
                ("IS", "keyword.operator.logical.sql"),
                ("NOT", "keyword.operator.logical.sql"),
                ("NULL", "constant.language.sql"),
                ("AND", "keyword.operator.logical.sql"),
                ("=", "keyword.operator.comparison.sql"),
                ("1", "constant.numeric.sql"),
            ]
        );
        assert_eq!(
            kinds(&["x varchar(20) numeric(5,2) timestamp(3) with time zone double  precision"]),
            [
                ("varchar", "storage.type.sql"),
                ("20", "constant.numeric.sql"),
                ("numeric", "storage.type.sql"),
                ("5", "constant.numeric.sql"),
                ("2", "constant.numeric.sql"),
                ("timestamp", "storage.type.sql"),
                ("3", "constant.numeric.sql"),
                (" with time zone", "storage.type.sql"),
                // bat's `double\sprecision` takes one `\s`; alone, `double` is MySQL's (X-124)
                ("double", "storage.type.sql"),
            ]
        );
    }

    #[test]
    fn create_names_on_its_line_only() {
        assert_eq!(
            kinds(&[
                "CREATE OR REPLACE TABLE s.\"t\" (",
                "CREATE TABLE",
                "  s.t (",
                "drop tables",
                "  COMMENT ON TABLE t IS 'x'",
            ]),
            [
                (
                    "CREATE OR REPLACE",
                    "meta.create.sql keyword.other.create.sql"
                ),
                ("TABLE", "meta.create.sql keyword.other.sql"),
                (
                    ".",
                    "meta.toc-list.full-identifier.sql punctuation.accessor.dot.sql"
                ),
                (
                    "t",
                    "meta.toc-list.full-identifier.sql entity.name.function.sql"
                ),
                ("CREATE", "meta.create.sql keyword.other.create.sql"),
                ("TABLE", "meta.create.sql keyword.other.sql"),
                ("s", "constant.other.database-name.sql"),
                ("t", "constant.other.table-name.sql"),
                // a whole word only (bat's `drop` took `table` in `tables`)
                ("drop", "keyword.other.sql"),
                (
                    "  COMMENT ON TABLE t IS ",
                    "keyword.other.object-comments.sql"
                ),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.begin.sql"
                ),
                ("x", "string.quoted.single.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.end.sql"
                ),
            ]
        );
        // `drop` only at a word's start (bat's matched `backdrop table x`), and `drop table`
        // with its name and `cascade`, past `if exists`
        assert_eq!(
            kinds(&["backdrop table x cascade; DROP TABLE IF EXISTS x CASCADE"]),
            [
                ("table", "keyword.other.sql"),
                ("cascade", "keyword.other.sql"),
                ("DROP", "meta.drop.sql keyword.other.create.sql"),
                ("TABLE", "meta.drop.sql keyword.other.table.sql"),
                ("IF", "keyword.control.sql"),
                ("EXISTS", "keyword.operator.logical.sql"),
                ("x", "meta.drop.sql entity.name.function.sql"),
                (" CASCADE", "meta.drop.sql keyword.other.cascade.sql"),
            ]
        );
    }

    /// The dialects' words (X-124): `create`'s modifiers and `if not exists`, PostgreSQL's
    /// types, a word of bat's compound patterns alone, a function only before `(`.
    #[test]
    fn the_dialects_words() {
        assert_eq!(
            kinds(&["CREATE TEMP TABLE IF NOT EXISTS t (id int4 GENERATED ALWAYS AS IDENTITY,"]),
            [
                ("CREATE", "meta.create.sql keyword.other.create.sql"),
                ("TEMP TABLE", "meta.create.sql keyword.other.sql"),
                ("IF", "keyword.control.sql"),
                ("NOT", "keyword.operator.logical.sql"),
                ("EXISTS", "keyword.operator.logical.sql"),
                (
                    "t",
                    "meta.toc-list.full-identifier.sql entity.name.function.sql"
                ),
                ("int4", "storage.type.sql"),
                ("GENERATED", "keyword.other.sql"),
                ("ALWAYS", "keyword.other.sql"),
                ("AS", "keyword.operator.assignment.alias.sql"),
                ("IDENTITY", "storage.modifier.sql"),
            ]
        );
        assert_eq!(
            kinds(&["create index concurrently on t using gin (j) where j is not distinct from x"]),
            [
                ("create", "meta.create.sql keyword.other.create.sql"),
                ("index", "meta.create.sql keyword.other.sql"),
                ("concurrently", "keyword.other.sql"),
                ("on", "keyword.other.sql"),
                ("using", "keyword.other.DML.sql"),
                ("where", "keyword.other.DML.sql"),
                ("is", "keyword.operator.logical.sql"),
                ("not", "keyword.operator.logical.sql"),
                ("distinct", "keyword.other.DML.sql"),
                ("from", "keyword.other.DML.sql"),
            ]
        );
        assert_eq!(
            kinds(&["select coalesce(a, 0), now (), coalesce, left(s, 2), x float, y::jsonb"]),
            [
                ("select", "keyword.other.DML.sql"),
                ("coalesce", "support.function.scalar.sql"),
                ("0", "constant.numeric.sql"),
                ("now", "support.function.scalar.sql"),
                ("(", "meta.block.sql punctuation.section.scope.begin.sql"),
                (")", "meta.block.sql punctuation.section.scope.end.sql"),
                ("left", "support.function.string.sql"),
                ("2", "constant.numeric.sql"),
                ("float", "storage.type.sql"),
                ("jsonb", "storage.type.sql"),
            ]
        );
        assert_eq!(
            kinds(&["for each row execute function f(); insert into t on conflict do nothing"]),
            [
                ("for", "keyword.other.sql"),
                ("each row", "keyword.other.sql"),
                ("execute", "keyword.other.sql"),
                ("function", "keyword.other.sql"),
                ("(", "meta.block.sql punctuation.section.scope.begin.sql"),
                (")", "meta.block.sql punctuation.section.scope.end.sql"),
                ("insert into", "keyword.other.DML.sql"),
                ("on", "keyword.other.sql"),
                ("conflict", "keyword.other.sql"),
                ("do", "keyword.other.sql"),
                ("nothing", "keyword.other.sql"),
            ]
        );
        // the first name is the object's (bat's took the last word before a `(`)
        let name = "meta.toc-list.full-identifier.sql entity.name.function.sql";
        for (line, want) in [
            ("CREATE TYPE mood AS ENUM ('a')", "mood"),
            ("CREATE DOMAIN year AS integer", "year"),
            ("CREATE INDEX i ON t (a)", "i"),
            ("CREATE OR ALTER PROCEDURE dbo.p AS", "p"),
        ] {
            let got = kinds(&[line]);
            assert_eq!(
                got.iter().filter(|(_, k)| *k == name).collect::<Vec<_>>(),
                [&(want, name)],
                "{line}"
            );
        }
        // a column's name stays plain
        assert_eq!(kinds(&["name, type, key, value, user, level"]), []);
    }

    /// T-SQL (X-124): `GO` alone on its line, `N'…'`, its batch and procedural words.
    #[test]
    fn t_sql() {
        assert_eq!(
            kinds(&[
                "SET NOCOUNT ON; DECLARE x datetime2 = N'a';",
                "GO",
                "BEGIN TRY EXEC p; END TRY BEGIN CATCH THROW; END CATCH go 2",
                "  go 2",
            ]),
            [
                ("SET", "keyword.other.DML.sql"),
                ("NOCOUNT", "keyword.other.sql"),
                ("ON", "keyword.other.sql"),
                ("DECLARE", "keyword.other.sql"),
                ("datetime2", "storage.type.sql"),
                ("=", "keyword.operator.comparison.sql"),
                (
                    "N'",
                    "string.quoted.single.sql punctuation.definition.string.begin.sql"
                ),
                ("a", "string.quoted.single.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.end.sql"
                ),
                ("GO", "keyword.other.sql"),
                ("BEGIN", "keyword.other.LUW.sql"),
                ("TRY", "keyword.control.sql"),
                ("EXEC", "keyword.other.sql"),
                ("END", "keyword.other.DML.sql"),
                ("TRY", "keyword.control.sql"),
                ("BEGIN", "keyword.other.LUW.sql"),
                ("CATCH", "keyword.control.sql"),
                ("THROW", "keyword.control.sql"),
                ("END", "keyword.other.DML.sql"),
                ("CATCH", "keyword.control.sql"),
                ("2", "constant.numeric.sql"),
                ("go", "keyword.other.sql"),
                ("2", "constant.numeric.sql"),
            ]
        );
    }

    /// PostgreSQL's dollar quotes (X-124): after `as` or `do` a body, its text SQL to the
    /// closing tag wherever it stands (an open string inside ends there); else a string.
    #[test]
    fn dollar_quotes() {
        let got = kinds(&[
            "CREATE FUNCTION f() RETURNS int AS",
            "$body$ BEGIN RETURN 'it''s $$ x'; -- $q$",
            "  RAISE NOTICE 'a$body$; SELECT $q$it's$q$, 1 $q$x",
            "y$q$ LANGUAGE plpgsql;",
        ]);
        let begin = "string.quoted.dollar.sql punctuation.definition.string.begin.sql";
        let end = "string.quoted.dollar.sql punctuation.definition.string.end.sql";
        let text = "string.quoted.dollar.sql";
        assert_eq!(
            &got[5..],
            [
                ("RETURNS", "keyword.other.sql"),
                ("int", "storage.type.sql"),
                ("AS", "keyword.operator.assignment.alias.sql"),
                ("$body$", begin),
                ("BEGIN", "keyword.other.LUW.sql"),
                ("RETURN", "keyword.control.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.begin.sql"
                ),
                ("it", "string.quoted.single.sql"),
                (
                    "''",
                    "string.quoted.single.sql constant.character.escape.sql"
                ),
                ("s $$ x", "string.quoted.single.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.end.sql"
                ),
                (
                    "--",
                    "comment.line.double-dash.sql punctuation.definition.comment.sql"
                ),
                (" $q$", "comment.line.double-dash.sql"),
                ("RAISE", "keyword.control.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.begin.sql"
                ),
                ("a", "string.quoted.single.sql"),
                ("$body$", end),
                ("SELECT", "keyword.other.DML.sql"),
                ("$q$", begin),
                ("it's", text),
                ("$q$", end),
                ("1", "constant.numeric.sql"),
                ("$q$", begin),
                ("x", text),
                ("y", text),
                ("$q$", end),
                ("LANGUAGE", "keyword.other.sql"),
            ]
        );
        // `DO LANGUAGE plpgsql` before a body
        assert_eq!(
            kinds(&["DO LANGUAGE plpgsql", "$$ BEGIN"]),
            [
                ("DO", "keyword.other.sql"),
                ("LANGUAGE", "keyword.other.sql"),
                ("$$", begin),
                ("BEGIN", "keyword.other.LUW.sql"),
            ]
        );
        // a MySQL client's `DELIMITER $$`: `$` quotes nothing after it
        assert_eq!(
            kinds(&["DELIMITER $$", "SELECT 1 $$", "DELIMITER ;"]),
            [
                ("DELIMITER", "keyword.other.sql"),
                ("SELECT", "keyword.other.DML.sql"),
                ("1", "constant.numeric.sql"),
                ("DELIMITER", "keyword.other.sql"),
            ]
        );
        // `$1` a parameter, not a tag (its digits a number, as bat's); `do $$ … $$` a body on
        // one line
        assert_eq!(
            kinds(&["do $$ begin perform 1; end $$; select $1"]),
            [
                ("do", "keyword.other.sql"),
                ("$$", begin),
                ("begin", "keyword.other.LUW.sql"),
                ("perform", "keyword.control.sql"),
                ("1", "constant.numeric.sql"),
                ("end", "keyword.other.DML.sql"),
                ("$$", end),
                ("select", "keyword.other.DML.sql"),
                ("1", "constant.numeric.sql"),
            ]
        );
    }

    #[test]
    fn strings_and_comments_across_lines() {
        assert_eq!(
            kinds(&["'it''s \\n", "x' /* a", " * b */ -- c"]),
            [
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.begin.sql"
                ),
                ("it", "string.quoted.single.sql"),
                (
                    "''",
                    "string.quoted.single.sql constant.character.escape.sql"
                ),
                ("s ", "string.quoted.single.sql"),
                (
                    "\\n",
                    "string.quoted.single.sql constant.character.escape.sql"
                ),
                ("x", "string.quoted.single.sql"),
                (
                    "'",
                    "string.quoted.single.sql punctuation.definition.string.end.sql"
                ),
                ("/*", "comment.block.c punctuation.definition.comment.sql"),
                (" a", "comment.block.c"),
                (" ", "comment.block.c"),
                ("*", "comment.block.c punctuation.definition.comment.sql"),
                (" b */", "comment.block.c"),
                (
                    "--",
                    "comment.line.double-dash.sql punctuation.definition.comment.sql"
                ),
                (" c", "comment.line.double-dash.sql"),
            ]
        );
    }

    /// A number with a fraction or an exponent is one number, where bat's grammar reads `1.5`
    /// as two and leaves `1e3` plain (X-008); digits before a letter stay as bat reads them.
    #[test]
    fn decimals_and_exponents() {
        let n = "constant.numeric.sql";
        let got = kinds(&["SELECT 1.5, 1e3, 2E-3, 1.5e+3 FROM t WHERE a > .5 AND b = 12ab"]);
        for want in ["1.5", "1e3", "2E-3", "1.5e+3", "5"] {
            assert!(got.contains(&(want, n)), "{want}: {got:?}");
        }
        assert!(!got.iter().any(|(t, _)| t.contains("12")), "{got:?}");
    }
}
