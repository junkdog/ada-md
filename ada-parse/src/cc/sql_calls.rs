//! Which of C's, C++'s and Objective-C's strings are SQL (X-134), by the call they are an
//! argument of, as phase 4 reads SQL outside Python: call sites and markers, not a string's text.
//! A string is SQL where the call it is an argument of, looked back on its line from the string
//! ([`sql_at`]), takes SQL in that argument:
//!
//! | API | call | SQL |
//! |---|---|---|
//! | SQLite (C) | `sqlite3_prepare(`, `sqlite3_prepare_v2(`, `sqlite3_prepare_v3(`, `sqlite3_exec(` | the second argument |
//! | libpq (C) | `PQexec(`, `PQexecParams(`, `PQsendQuery(`, `PQsendQueryParams(` | the second |
//! | libpq (C) | `PQprepare(`, `PQsendPrepare(` | the third |
//! | Qt, SQLiteCpp, libpqxx (C++) | a member's `.exec(`, `->exec(`, `.prepare(`, pqxx's `exec0(`, `exec1(`, `exec_params(`… | the first |
//! | Qt (C++) | `QSqlQuery q("…")`, `QSqlQuery("…")` | the first |
//! | SQLiteCpp (C++) | `SQLite::Statement s(db, "…")` | the second |
//! | FMDB (Objective-C) | `executeQuery:`, `executeUpdate:`, `executeStatements:`, `…WithFormat:` | the string after the part ([`super::objc`]) |
//!
//! A SQL string's adjacent literals are SQL too (`"SELECT a "` `"FROM t"`), on its line and the
//! next; so is a string that starts the line after a call's `(` or `,` where its SQL argument
//! starts. Not followed: a string passed in a variable; a call whose name is on a line before
//! the argument's; `std::format`'s and `sqlite3_mprintf`'s strings.

/// The SQL argument (from 0) of a call named `w`: a function's, or a member's (`member`, after
/// `.` or `->`) or a type's in a declaration (`cpp`: C++'s only).
pub(super) fn sql_argument(w: &[u8], member: bool, cpp: bool) -> Option<u8> {
    if member {
        return match w {
            b"exec" | b"prepare" | b"exec0" | b"exec1" | b"exec_n" | b"exec_params"
            | b"exec_params0" | b"exec_params1" | b"exec_params_n"
                if cpp =>
            {
                Some(0)
            }
            _ => None,
        };
    }
    match w {
        b"sqlite3_prepare"
        | b"sqlite3_prepare_v2"
        | b"sqlite3_prepare_v3"
        | b"sqlite3_exec"
        | b"PQexec"
        | b"PQexecParams"
        | b"PQsendQuery"
        | b"PQsendQueryParams" => Some(1),
        b"PQprepare" | b"PQsendPrepare" => Some(2),
        b"QSqlQuery" if cpp => Some(0),
        b"Statement" if cpp => Some(1),
        _ => None,
    }
}

/// The names [`sql_argument`] knows, for the word table: a line with one is checked.
pub(super) const SQL_NAMES: [&[u8]; 21] = [
    b"sqlite3_prepare",
    b"sqlite3_prepare_v2",
    b"sqlite3_prepare_v3",
    b"sqlite3_exec",
    b"PQexec",
    b"PQexecParams",
    b"PQsendQuery",
    b"PQsendQueryParams",
    b"PQprepare",
    b"PQsendPrepare",
    b"exec",
    b"prepare",
    b"exec0",
    b"exec1",
    b"exec_n",
    b"exec_params",
    b"exec_params0",
    b"exec_params1",
    b"exec_params_n",
    b"QSqlQuery",
    b"Statement",
];

fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Where the blanks before `p` start.
fn blank_back(b: &[u8], mut p: usize) -> usize {
    while p > 0 && matches!(b[p - 1], b' ' | b'\t') {
        p -= 1;
    }
    p
}

/// Where the word ending at `w` starts.
fn word_back(b: &[u8], mut w: usize) -> usize {
    while w > 0 && word_byte(b[w - 1]) {
        w -= 1;
    }
    w
}

/// Whether a string whose argument starts after `o` (a `(` or a `,`) is a SQL call's SQL
/// argument, the call looked back for on the line `b`: past the arguments before (their
/// brackets and strings), its name before its `(`.
pub(super) fn sql_at(b: &[u8], o: usize, cpp: bool) -> bool {
    let mut index = 0u8;
    let mut depth = 0u32;
    let mut i = o;
    // the call's `(`
    let open = loop {
        match b[i] {
            b'(' if depth == 0 => break i,
            b',' if depth == 0 => index = index.saturating_add(1),
            b')' | b']' | b'}' => depth += 1,
            // an argument inside a brace's or a bracket's list: no call's
            b'[' | b'{' if depth == 0 => return false,
            b'(' | b'[' | b'{' => depth -= 1,
            q @ (b'"' | b'\'') => {
                // a string or a character before: to its opening quote
                let mut k = i;
                loop {
                    if k == 0 {
                        return false;
                    }
                    k -= 1;
                    if b[k] == q && (k == 0 || b[k - 1] != b'\\') {
                        break;
                    }
                }
                i = k;
            }
            _ => {}
        }
        if i == 0 {
            return false;
        }
        i -= 1;
    };
    let e = blank_back(b, open);
    let s = word_back(b, e);
    if s == e {
        return false;
    }
    let name = &b[s..e];
    let before = blank_back(b, s);
    let member =
        before > 0 && b[before - 1] == b'.' || before > 1 && &b[before - 2..before] == b"->";
    if let Some(n) = sql_argument(name, member, cpp) {
        return n == index;
    }
    // a declaration: `QSqlQuery q("…")`, `SQLite::Statement s(db, "…")`
    if !cpp || member || before == 0 || !word_byte(b[before - 1]) {
        return false;
    }
    let t = word_back(b, before);
    sql_argument(&b[t..before], false, true).is_some_and(|n| n == index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: &str, cpp: bool) -> bool {
        let b = line.as_bytes();
        let q = line.rfind('"').unwrap();
        // the string's opening quote: the last string's
        let s = line[..q].rfind('"').unwrap();
        let o = blank_back(b, s) - 1;
        sql_at(b, o, cpp)
    }

    #[test]
    fn calls_by_their_sql_argument() {
        assert!(at(r#"rc = sqlite3_prepare_v2(db, "SELECT 1""#, false));
        assert!(!at(r#"rc = sqlite3_prepare_v2("x""#, false));
        assert!(at(r#"PQprepare(conn, "stmt", "SELECT $1""#, false));
        assert!(at(r#"sqlite3_exec(get(db, "a,b"), "DELETE FROM t""#, false));
        assert!(at(r#"query.exec("SELECT 1""#, true));
        assert!(!at(r#"query.exec("SELECT 1""#, false));
        assert!(at(r#"q->prepare("SELECT 1""#, true));
        assert!(!at(r#"exec("ls""#, true));
        assert!(at(r#"QSqlQuery q("SELECT 1""#, true));
        assert!(at(r#"QSqlQuery("SELECT 1""#, true));
        assert!(at(r#"SQLite::Statement query(db, "SELECT 1""#, true));
        assert!(!at(r#"SQLite::Statement query("SELECT 1""#, true));
        assert!(!at(r#"printf("%s", "SELECT""#, false));
    }
}
