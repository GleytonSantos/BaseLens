//! SQL checking, linting and formatting, in one place.
//!
//! - `validate`: offline syntax check with sqlparser, per dialect. Only a
//!   warning in the editor: the server stays the judge (sqlparser doesn't
//!   know every extension).
//! - `lint` / `fix`: sqruff's core rules (spacing, keyword case, aliases…).
//! - `format`: BaseLens's own layout (one clause per line, case kept) and
//!   then sqruff's fixes on top; the own layout alone when sqruff can't parse
//!   the SQL or the `lint` feature is off.

use crate::db::DbKind;
use std::ops::Range;

/// Longest SQL checked while typing (bigger scripts are checked on demand).
pub const VALIDATE_MAX_BYTES: usize = 200_000;
/// Pause after the last key before the editor checks the SQL.
pub const VALIDATE_DEBOUNCE_MS: u64 = 600;
/// Longest text Format sends through sqruff (longer: BaseLens's layout only).
pub const FORMAT_SQRUFF_MAX_BYTES: usize = 20_000;
/// Longest SQL linted automatically while typing (longer: Ctrl+Shift+I).
pub const LINT_LIVE_MAX_BYTES: usize = 20_000;
/// Most lines compared when the fixed text is split into one fix per place
/// (longer texts get a single fix for everything).
pub const FIX_DIFF_MAX_LINES: usize = 2_000;

/// One place sqruff's fix changes: these lines (0-based start) become those.
#[derive(Clone, Debug, PartialEq)]
pub struct FixHunk {
    pub old_start: usize,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
}

/// The lint done while typing: findings, the fixed text, and that fix cut
/// into places so each can be applied alone.
#[derive(Clone, Debug, Default)]
pub struct LiveLint {
    pub findings: Vec<Violation>,
    pub fixed: Option<String>,
    pub hunks: Vec<FixHunk>,
}

impl LiveLint {
    /// The fix that changes the finding's line (1-based), if any.
    pub fn hunk_for(&self, line: usize) -> Option<usize> {
        let l = line.saturating_sub(1);
        self.hunks
            .iter()
            .position(|h| if h.old_lines.is_empty() { h.old_start == l } else { h.old_start <= l && l < h.old_start + h.old_lines.len() })
    }
}

/// Lines that differ between two texts, grouped into places (longest common
/// subsequence of lines).
pub fn fix_hunks(old: &str, new: &str) -> Vec<FixHunk> {
    if old == new {
        return Vec::new();
    }
    let a: Vec<&str> = old.split('\n').collect();
    let b: Vec<&str> = new.split('\n').collect();
    let whole = || vec![FixHunk { old_start: 0, old_lines: a.iter().map(|s| s.to_string()).collect(), new_lines: b.iter().map(|s| s.to_string()).collect() }];
    if a.len() > FIX_DIFF_MAX_LINES || b.len() > FIX_DIFF_MAX_LINES {
        return whole();
    }
    let (n, m) = (a.len(), b.len());
    // lcs[i][j] = common lines of a[i..] and b[j..]
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if a[i] == b[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
        }
    }
    let mut out: Vec<FixHunk> = Vec::new();
    let mut cur: Option<FixHunk> = None;
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            if let Some(h) = cur.take() {
                out.push(h);
            }
            i += 1;
            j += 1;
            continue;
        }
        let h = cur.get_or_insert_with(|| FixHunk { old_start: i, old_lines: Vec::new(), new_lines: Vec::new() });
        if j < m && (i == n || lcs[at(i, j + 1)] >= lcs[at(i + 1, j)]) {
            h.new_lines.push(b[j].to_string());
            j += 1;
        } else {
            h.old_lines.push(a[i].to_string());
            i += 1;
        }
    }
    if let Some(h) = cur {
        out.push(h);
    }
    // lines changed one for one: one fix per line, so each can be applied alone
    out.into_iter()
        .flat_map(|h| {
            if h.old_lines.len() == h.new_lines.len() && h.old_lines.len() > 1 {
                (0..h.old_lines.len())
                    .map(|k| FixHunk { old_start: h.old_start + k, old_lines: vec![h.old_lines[k].clone()], new_lines: vec![h.new_lines[k].clone()] })
                    .collect::<Vec<_>>()
            } else {
                vec![h]
            }
        })
        .collect()
}

/// The text with one fix applied, if its lines are still where they were.
pub fn apply_hunk(text: &str, h: &FixHunk) -> Option<String> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    let end = h.old_start + h.old_lines.len();
    if end > lines.len() || lines[h.old_start..end].iter().zip(&h.old_lines).any(|(x, y)| x != y) {
        return None;
    }
    lines.splice(h.old_start..end, h.new_lines.iter().map(|s| s.as_str()));
    Some(lines.join("\n"))
}

/// Lint plus the fix cut into places, for the editor while typing.
pub fn live_lint(sql: &str, kind: DbKind) -> Result<LiveLint, String> {
    let findings = lint(sql, kind)?;
    let fixed = if findings.iter().any(|f| f.fixable) { fix(sql, kind).ok().filter(|f| f != sql) } else { None };
    let hunks = fixed.as_deref().map(|f| fix_hunks(sql, f)).unwrap_or_default();
    Ok(LiveLint { findings, fixed, hunks })
}

#[derive(Clone, Debug, PartialEq)]
pub struct SqlError {
    /// 1-based, as the parser reports them (0 = unknown)
    pub line: usize,
    pub col: usize,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    pub line: usize,
    pub col: usize,
    pub code: String,
    pub name: String,
    pub desc: String,
    pub fixable: bool,
}

/// sqruff's dialect name for each database.
pub fn sqruff_dialect(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => "postgres",
        DbKind::Sqlite => "sqlite",
        DbKind::Mysql => "mysql",
    }
}

fn parser_dialect(kind: DbKind) -> Box<dyn sqlparser::dialect::Dialect> {
    use sqlparser::dialect::*;
    match kind {
        DbKind::Postgres => Box::new(PostgreSqlDialect {}),
        DbKind::Sqlite => Box::new(SQLiteDialect {}),
        DbKind::Mysql => Box::new(MySqlDialect {}),
    }
}

/// "… at Line: 2, Column: 7" → (2, 7)
fn position_in(msg: &str) -> (usize, usize) {
    let num_after = |key: &str| -> usize {
        msg.rfind(key)
            .map(|i| msg[i + key.len()..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };
    (num_after("Line: "), num_after("Column: "))
}

/// SQLite's only three syntax-error messages, as `sqlite3RunParser`
/// (0x685aca0 in the original) writes them: a token the tokenizer can't read
/// (TK_ILLEGAL) → `unrecognized token: "%T"`; a parse error at the end of the
/// text → `incomplete input`; any other parse error → `near "%T": syntax
/// error`. Every other error (no such table, no such column…) comes later,
/// from name resolution, and isn't a syntax error.
pub const SQLITE_SYNTAX_MESSAGES: [&str; 3] = ["unrecognized token: \"", "incomplete input", "near \""];

fn is_sqlite_syntax_error(msg: &str) -> bool {
    SQLITE_SYNTAX_MESSAGES.iter().any(|p| msg.starts_with(p)) && (!msg.starts_with("near \"") || msg.ends_with(": syntax error"))
}

/// Byte offset → (line, column), both 1-based, column in characters.
fn line_col(text: &str, byte: usize) -> (usize, usize) {
    let byte = byte.min(text.len());
    let before = &text[..text.floor_char_boundary(byte)];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, col)
}

/// SQLite: the engine's own parser (the bundled SQLite, the same code the
/// original ships). Each statement is prepared on an empty in-memory
/// database — preparing never runs anything — and only syntax errors count.
fn validate_sqlite(sql: &str) -> Result<(), SqlError> {
    let Ok(c) = rusqlite::Connection::open_in_memory() else { return Ok(()) };
    let mut from = 0;
    for stmt in crate::db::split_statements(sql) {
        let start = sql[from..].find(&stmt).map(|i| from + i).unwrap_or(from);
        from = start + stmt.len();
        let err = match c.prepare(&stmt) {
            Ok(_) => continue,
            Err(e) => e,
        };
        let (msg, offset) = match &err {
            rusqlite::Error::SqlInputError { msg, offset, .. } => (msg.clone(), (*offset >= 0).then_some(*offset as usize)),
            rusqlite::Error::SqliteFailure(_, Some(m)) => (m.clone(), None),
            other => (other.to_string(), None),
        };
        if !is_sqlite_syntax_error(&msg) {
            continue;
        }
        // "incomplete input" points at the end of the statement
        let at = match offset {
            Some(o) => start + o,
            None if msg == "incomplete input" => start + stmt.trim_end().len(),
            None => start,
        };
        let (line, col) = line_col(sql, at);
        return Err(SqlError { line, col, message: msg });
    }
    Ok(())
}

/// Offline syntax check. Ok for empty text.
pub fn validate(sql: &str, kind: DbKind) -> Result<(), SqlError> {
    if crate::db::split_statements(sql).is_empty() {
        return Ok(());
    }
    if kind == DbKind::Sqlite {
        return validate_sqlite(sql);
    }
    let d = parser_dialect(kind);
    match sqlparser::parser::Parser::parse_sql(d.as_ref(), sql) {
        Ok(_) => Ok(()),
        Err(e) => {
            let raw = e.to_string();
            let (line, col) = position_in(&raw);
            let message = raw
                .trim_start_matches("sql parser error: ")
                .split(" at Line: ")
                .next()
                .unwrap_or(&raw)
                .to_string();
            Err(SqlError { line, col, message })
        }
    }
}

/// Byte range of the word at (line, col) in `text`, to underline it.
pub fn error_range(text: &str, line: usize, col: usize) -> Option<Range<usize>> {
    if line == 0 || col == 0 {
        return None;
    }
    let mut start = 0;
    for (i, l) in text.split('\n').enumerate() {
        if i + 1 == line {
            let off: usize = l.chars().take(col - 1).map(char::len_utf8).sum();
            let s = start + off.min(l.len());
            let rest = &text[s..];
            let len = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').map(char::len_utf8).sum::<usize>();
            let len = if len == 0 { rest.chars().next().map(char::len_utf8).unwrap_or(0) } else { len };
            return (len > 0).then_some(s..s + len);
        }
        start += l.len() + 1;
    }
    None
}

#[cfg(feature = "lint")]
fn linter(kind: DbKind) -> Result<sqruff_lib::core::linter::core::Linter, String> {
    use sqruff_lib::core::config::FluffConfig;
    // core rules, without "file must end with a newline" (an editor buffer
    // isn't a file) and without line-length reflow
    let cfg = format!("[sqruff]\ndialect = {}\nrules = core\nexclude_rules = LT12,LT05\n", sqruff_dialect(kind));
    let cfg = FluffConfig::try_from_source(&cfg, None).map_err(|e| e.to_string())?;
    sqruff_lib::core::linter::core::Linter::new(cfg, None, None, true)
}

/// sqruff's findings. Parse problems come back as code "PRS".
#[cfg(feature = "lint")]
pub fn lint(sql: &str, kind: DbKind) -> Result<Vec<Violation>, String> {
    let l = linter(kind)?;
    let f = l.lint_string(sql, None, false).map_err(|e| e.to_string())?;
    let mut v: Vec<Violation> = f
        .violations()
        .iter()
        .map(|e| Violation {
            line: e.line_no,
            col: e.line_pos,
            code: if e.rule.is_some() { e.rule_code().to_string() } else { "PRS".into() },
            name: if e.rule.is_some() { e.rule_name().to_string() } else { "parse".into() },
            desc: e.desc().to_string(),
            fixable: e.fixable,
        })
        .collect();
    v.sort_by_key(|x| (x.line, x.col));
    Ok(v)
}

#[cfg(not(feature = "lint"))]
pub fn lint(_sql: &str, _kind: DbKind) -> Result<Vec<Violation>, String> {
    Err("This build of BaseLens was made without the SQL linter.".into())
}

/// sqruff's automatic fixes applied to the whole text.
#[cfg(feature = "lint")]
pub fn fix(sql: &str, kind: DbKind) -> Result<String, String> {
    let l = linter(kind)?;
    let f = l.lint_string(sql, None, true).map_err(|e| e.to_string())?;
    if f.has_parse_or_templating_errors() {
        return Err("sqruff couldn't read all of this SQL, so nothing was changed.".into());
    }
    Ok(f.fix_string())
}

#[cfg(not(feature = "lint"))]
pub fn fix(_sql: &str, _kind: DbKind) -> Result<String, String> {
    Err("This build of BaseLens was made without the SQL linter.".into())
}

/// Readable SQL: BaseLens's layout, then sqruff's fixes when it can read it.
pub fn format(sql: &str, kind: DbKind) -> String {
    let own = crate::assist::format_sql(sql);
    // sqruff runs on the window's thread here: keep it to normal-sized text
    if own.len() > FORMAT_SQRUFF_MAX_BYTES {
        return own;
    }
    let trailing_nl = own.ends_with('\n');
    match fix(&own, kind) {
        Ok(f) => {
            let f = f.trim_end_matches('\n').to_string();
            if trailing_nl {
                f + "\n"
            } else {
                f
            }
        }
        Err(_) => own,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_per_dialect() {
        assert_eq!(validate("select id, name from users where id = 1", DbKind::Postgres), Ok(()));
        assert_eq!(validate("", DbKind::Postgres), Ok(()));
        assert_eq!(validate("-- just a comment", DbKind::Sqlite), Ok(()));
        let e = validate("select id from users wher id = 1", DbKind::Postgres).unwrap_err();
        assert_eq!(e.line, 1);
        assert!(e.col > 0 && !e.message.contains("Line:"), "{e:?}");
        let e = validate("select 1;\nselect * frm t", DbKind::Sqlite).unwrap_err();
        assert_eq!(e.line, 2, "{e:?}");
        // dialect-specific syntax
        assert_eq!(validate("select '{\"a\":1}'::jsonb ->> 'a'", DbKind::Postgres), Ok(()));
        assert_eq!(validate("select `id` from `users` limit 1, 10", DbKind::Mysql), Ok(()));
        // dollar-quoted bodies
        assert_eq!(validate("create function f() returns int as $$ select 1 $$ language sql", DbKind::Postgres), Ok(()));
    }

    #[test]
    fn sqlite_uses_its_own_parser_and_words() {
        let e = validate("select 1;\nselect * frm t", DbKind::Sqlite).unwrap_err();
        assert_eq!(e.message, "near \"frm\": syntax error");
        assert_eq!((e.line, e.col), (2, 10));
        let e = validate("select (1", DbKind::Sqlite).unwrap_err();
        assert_eq!(e.message, "incomplete input");
        let e = validate("select x'zz'", DbKind::Sqlite).unwrap_err();
        assert!(e.message.starts_with("unrecognized token: \""), "{e:?}");
        // not syntax: unknown tables are fine offline
        assert_eq!(validate("select * from nope; select missing_col from t2", DbKind::Sqlite), Ok(()));
        // SQLite-only syntax sqlparser may not know
        assert_eq!(validate("select * from t where rowid = 1 indexed by i", DbKind::Sqlite).map_err(|e| e.message.contains("syntax")), Err(true));
        assert_eq!(validate("insert or replace into t(a) values (1) returning *", DbKind::Sqlite), Ok(()));
        assert_eq!(validate("pragma table_info('t')", DbKind::Sqlite), Ok(()));
        // nothing is run while checking
        assert_eq!(validate("create table z(a); drop table z", DbKind::Sqlite), Ok(()));
    }

    #[test]
    fn underlines_the_word_at_the_error() {
        let t = "select 1;\nselect * frm t";
        let r = error_range(t, 2, 10).unwrap();
        assert_eq!(&t[r], "frm");
        assert_eq!(error_range(t, 0, 0), None);
        assert_eq!(error_range("ação x", 1, 6), Some(7..8)); // "x" after 4 letters (6 bytes) and a space
    }

    #[cfg(feature = "lint")]
    #[test]
    fn lints_and_fixes() {
        let v = lint("select a,b from t where x=1", DbKind::Postgres).unwrap();
        assert!(v.iter().any(|x| x.code == "LT01" && x.fixable), "{v:?}");
        assert!(!v.iter().any(|x| x.code == "LT12"), "no trailing-newline rule for editor text");
        assert_eq!(fix("select a,b from t where x=1", DbKind::Postgres).unwrap(), "select a, b from t where x = 1");
        // unreadable SQL is never changed
        assert!(fix("select * frm t", DbKind::Postgres).is_err());
        assert!(lint("select * frm t", DbKind::Postgres).unwrap().iter().any(|x| x.code == "PRS"));
    }

    #[test]
    fn format_is_stable() {
        for kind in [DbKind::Postgres, DbKind::Sqlite, DbKind::Mysql] {
            let once = format("select a,b from orders o join items i on i.order_id=o.id where o.total>10 and i.qty>=2 order by a", kind);
            assert_eq!(format(&once, kind), once, "{kind:?}");
            assert!(once.contains("\nfrom") && once.contains("o.total > 10"), "{once}");
        }
        // unreadable SQL keeps BaseLens's own layout
        let bad = format("select a,b frm t", DbKind::Postgres);
        assert_eq!(bad, crate::assist::format_sql("select a,b frm t"));
    }
}
