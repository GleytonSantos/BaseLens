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

/// Offline syntax check. Ok for empty text.
pub fn validate(sql: &str, kind: DbKind) -> Result<(), SqlError> {
    if crate::db::split_statements(sql).is_empty() {
        return Ok(());
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
