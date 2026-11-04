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
//! lines is that context, a byte: a block comment, one of the five strings, or
//! `identifier_create`. `\w` past ASCII is a letter or a decimal digit, `\s` Unicode's spaces,
//! `\d` Unicode's decimal digits; `(?i)` folds ASCII case only (Oniguruma's `ſ` for `s` isn't
//! followed).

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

/// How many kinds there are.
pub(crate) const KINDS: usize = 77;

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
];

/// The scopes under `source.sql` kind `kind` carries.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// Lex `b`, a line of SQL without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        out,
        mark: 0,
        is_end: usize::MAX,
        from: 0,
        embed: None,
        at: 0,
    };
    let mut ctx = state.word() as u8;
    // a context's lexing up to the line's end, or to where it hands over to another
    let mut p = 0;
    loop {
        let next = match ctx {
            MAIN => s.main(p),
            BLOCK => s.block(p),
            SINGLE..=MODR => s.string(ctx, p),
            CREATE => s.create(p),
            _ => None,
        };
        match next {
            Some((q, c)) => (p, ctx) = (q, c),
            None => break,
        }
    }
    state.set_word(u64::from(ctx));
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
    if !b[q..].starts_with(b"language=") {
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
];

const SIZE: usize = 512;

/// FNV-1a over the word's bytes, ASCII case folded.
const fn hash(w: &[u8]) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < w.len() {
        h ^= w[i].to_ascii_lowercase() as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    (h as usize) & (SIZE - 1)
}

/// The table: each slot an entry's index plus one, 0 for none; open addressing.
static SLOTS: [u8; SIZE] = {
    let mut t = [0u8; SIZE];
    let mut i = 0;
    while i < ENTRIES.len() {
        let mut h = hash(ENTRIES[i].0);
        while t[h] != 0 {
            h = (h + 1) & (SIZE - 1);
        }
        t[h] = i as u8 + 1;
        i += 1;
    }
    t
};

/// The longest word in the table: longer ones are none of them.
const LONGEST: usize = 17;

/// The pattern word `w` starts, ASCII case ignored.
fn lookup(w: &[u8]) -> u8 {
    if w.len() > LONGEST {
        return W_NONE;
    }
    let mut h = hash(w);
    loop {
        let i = SLOTS[h];
        if i == 0 {
            return W_NONE;
        }
        let (e, k) = ENTRIES[usize::from(i - 1)];
        if e.eq_ignore_ascii_case(w) {
            return k;
        }
        h = (h + 1) & (SIZE - 1);
    }
}

/// The objects `drop` and `alter` take, in their order: no `\b` after them.
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
];

/// `create`'s, in extended mode (`operator class` is `operatorclass`), each followed by
/// `\b`; `(?:unique\s+)?index` is tried apart.
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

/// `[A-Za-z0-9_]`.
static WORD: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = (c as u8).is_ascii_alphanumeric() || c == b'_' as usize;
        c += 1;
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
}

impl Scan<'_, '_, '_, '_> {
    fn tok(&mut self, start: usize, end: usize, kind: u16) {
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
                p = self.rest_of_word(p, word_end(b, p));
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
                _ => p += char_len(c),
            }
        }
        // a token took the line's `\n`: the host gets no turn at it
        if p > n && self.embed.is_some() {
            return Some((p, MAIN));
        }
        None
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
        if id != W_NONE
            && let Some(r) = self.keyword(id, p, e)
        {
            return r;
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
        Ok(self.rest_of_word(p, e))
    }

    /// The rest of a word from `p` to its end `e`, where no pattern starts but
    /// `\s*(drop)\s+(table)\s+(\w+)(\s+cascade)?\b`, which has no `\b` before `drop`: where
    /// the scan goes on.
    fn rest_of_word(&mut self, p: usize, e: usize) -> usize {
        if e >= p + 4
            && at_ci(self.b, e - 4, b"drop")
            && let Some(q) = self.drop_table(e - 4)
        {
            return q;
        }
        e
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
                let k = self.then_word(e, b"join")?;
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
                let k = self.then_word(e, b"work").unwrap_or(e);
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
            W_AS => ALIAS,
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

    /// `\b(create(?:\s+or\s+replace)?)\s+(objects)\b\s*`, `create` at `p` to `e`: where
    /// `identifier_create` is pushed.
    fn create_statement(&mut self, p: usize, e: usize) -> Option<usize> {
        let replace = self
            .then_word(e, b"or")
            .and_then(|q| self.then_word(q, b"replace"));
        for c1 in [replace, Some(e)].into_iter().flatten() {
            let Some(s) = self.spaces1(c1) else {
                continue;
            };
            let unique = self
                .keyword_at(s, b"unique")
                .and_then(|q| self.then_word(q, b"index"));
            let found =
                unique.or_else(|| CREATE_OBJECTS.iter().find_map(|o| self.keyword_at(s, o)));
            if let Some(o) = found {
                self.tok(p, c1, CREATE_KEYWORD);
                self.tok(s, o, CREATE_OBJECT);
                return Some(self.spaces(o));
            }
        }
        None
    }

    /// `\s*\b(drop)\s+(objects)`, `drop` at `p` to `e`: where the scan goes on. The objects
    /// have no `\b` after them, so the rest of a longer word is passed over (`tables`).
    fn drop(&mut self, p: usize, e: usize) -> Option<usize> {
        let s = self.spaces1(e)?;
        let o = OBJECTS
            .iter()
            .find(|o| at_ci(&self.b[..self.b.len()], s, o))
            .map(|o| s + o.len())?;
        self.tok(p, e, DROP_KEYWORD);
        self.tok(s, o, DROP_OBJECT);
        let end = word_end(self.b, o);
        Some(if end > o {
            self.rest_of_word(o, end)
        } else {
            o
        })
    }

    /// `\s*(drop)\s+(table)\s+(\w+)(\s+cascade)?\b` with `drop` at `p`: where it ends.
    fn drop_table(&mut self, p: usize) -> Option<usize> {
        let t = self.spaces1(p + 4)?;
        if !at_ci(&self.b[..self.b.len()], t, b"table") {
            return None;
        }
        let nm = self.spaces1(t + 5)?;
        if !is_word(&self.b[..self.b.len()], nm) {
            return None;
        }
        let ne = word_end(self.b, nm);
        let cascade = self.then_word(ne, b"cascade");
        self.tok(p, p + 4, DROP_KEYWORD);
        self.tok(t, t + 5, DROP_TABLE);
        self.tok(nm, ne, DROP_NAME);
        if let Some(c) = cascade {
            self.tok(ne, c, DROP_CASCADE);
            return Some(c);
        }
        Some(ne)
    }

    /// `\s*\b(alter)\s+(objects)\s+`, `alter` at `p` to `e`: where it ends.
    fn alter(&mut self, p: usize, e: usize) -> Option<usize> {
        let s = self.spaces1(e)?;
        let (o, end) = OBJECTS.iter().find_map(|o| {
            if !at_ci(&self.b[..self.b.len()], s, o) {
                return None;
            }
            let q = s + o.len();
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
            while p < lim && b[p] != quote && b[p] != other {
                p += 1;
            }
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
            if let Some((s, e, end)) = self.create_name(p) {
                // `(?:(\w+)|'([^']+)'|"([^"]+)"|`([^`]+)`){{end_identifier}}`
                if self.end_identifier(end) {
                    self.tok(s, e, CREATE_NAME);
                    return Some((end, MAIN));
                }
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

/// The bytes of the UTF-8 character that starts with `c`.
fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// The character that starts at byte `i` of `b`, past ASCII.
#[cold]
fn wide_char(b: &[u8], i: usize) -> Option<char> {
    let e = (i + char_len(b[i])).min(b.len());
    std::str::from_utf8(&b[i..e]).ok()?.chars().next()
}

/// Whether `c` is one of Unicode's decimal digits, `\p{Nd}`.
fn is_nd(c: char) -> bool {
    let u = c as u32;
    c.is_ascii_digit() || crate::csharp::ND.iter().any(|&s| u >= s && u - s < 10)
}

/// Whether the character at byte `i` of `b` is a word's, `\w`: past ASCII, a letter or a
/// decimal digit. `i` is a character's first byte.
fn is_word(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => WORD[usize::from(c)],
        Some(_) => wide_char(b, i).is_some_and(|c| c.is_alphabetic() || is_nd(c)),
        None => false,
    }
}

/// Whether the character before byte `p` of `b` is a word's.
fn is_word_before(b: &[u8], p: usize) -> bool {
    let mut s = p - 1;
    while s > 0 && b[s] & 0xc0 == 0x80 {
        s -= 1;
    }
    is_word(b, s)
}

/// Where the word at `p` ends.
fn word_end(b: &[u8], mut p: usize) -> usize {
    loop {
        while p < b.len() && WORD[usize::from(b[p])] {
            p += 1;
        }
        if p < b.len() && b[p] >= 0x80 && is_word(b, p) {
            p += char_len(b[p]);
        } else {
            return p.min(b.len());
        }
    }
}

/// `\s` at `p` of the line (not its `\n`): how many bytes the space takes.
fn space_len(b: &[u8], p: usize) -> Option<usize> {
    let &c = b.get(p)?;
    if c < 0x80 {
        return matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r').then_some(1);
    }
    wide_space(b, p)
}

#[cold]
fn wide_space(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    matches!(
        c,
        '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
    .then(|| c.len_utf8())
}

/// `\d` at `p`, Unicode's decimal digits: where it ends.
fn digit(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        c if c.is_ascii_digit() => Some(p + 1),
        c if c < 0x80 => None,
        _ => wide_char(b, p)
            .filter(|&c| is_nd(c))
            .map(|c| p + c.len_utf8()),
    }
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
                ("drop", "meta.drop.sql keyword.other.create.sql"),
                ("table", "meta.drop.sql keyword.other.sql"),
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
        assert_eq!(
            kinds(&["backdrop table x cascade"]),
            [
                ("drop", "meta.drop.sql keyword.other.create.sql"),
                ("table", "meta.drop.sql keyword.other.table.sql"),
                ("x", "meta.drop.sql entity.name.function.sql"),
                (" cascade", "meta.drop.sql keyword.other.cascade.sql"),
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
