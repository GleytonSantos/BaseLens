//! Point-and-click filters: turn "column / operator / value" rows and a
//! search-everywhere box into a safe WHERE clause, so simple lookups never
//! need hand-written SQL.

use crate::db::{sql_string, DbKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    StartsWith,
    EndsWith,
    Greater,
    GreaterEq,
    Less,
    LessEq,
    Between,
    InList,
    IsNull,
    NotNull,
}

impl Op {
    pub const ALL: [Op; 14] = [
        Op::Contains,
        Op::NotContains,
        Op::Equals,
        Op::NotEquals,
        Op::StartsWith,
        Op::EndsWith,
        Op::Greater,
        Op::GreaterEq,
        Op::Less,
        Op::LessEq,
        Op::Between,
        Op::InList,
        Op::IsNull,
        Op::NotNull,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Op::Contains => "contains",
            Op::NotContains => "doesn't contain",
            Op::Equals => "=",
            Op::NotEquals => "≠",
            Op::StartsWith => "starts with",
            Op::EndsWith => "ends with",
            Op::Greater => ">",
            Op::GreaterEq => "≥",
            Op::Less => "<",
            Op::LessEq => "≤",
            Op::Between => "between",
            Op::InList => "is one of",
            Op::IsNull => "is empty (NULL)",
            Op::NotNull => "has a value",
        }
    }
    pub fn needs_value(self) -> bool {
        !matches!(self, Op::IsNull | Op::NotNull)
    }
    pub fn hint(self) -> &'static str {
        match self {
            Op::Between => "from, to",
            Op::InList => "a, b, c",
            _ => "value",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub column: String,
    pub op: Op,
    pub value: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FilterSet {
    pub search: String,
    pub rules: Vec<Rule>,
    pub match_any: bool,
    pub raw: String,
    pub raw_mode: bool,
}

impl FilterSet {
    pub fn active_count(&self) -> usize {
        let mut n = self.rules.iter().filter(|r| r.enabled && (!r.op.needs_value() || !r.value.trim().is_empty())).count();
        if !self.search.trim().is_empty() {
            n += 1;
        }
        if self.raw_mode && !self.raw.trim().is_empty() {
            n += 1;
        }
        n
    }
}

/// Column as text, so "contains" also works on numbers and dates.
fn as_text(kind: DbKind, col: &str) -> String {
    let q = kind.quote(col);
    match kind {
        DbKind::Postgres => format!("CAST({q} AS TEXT)"),
        DbKind::Mysql => format!("CAST({q} AS CHAR)"),
        DbKind::Sqlite => q,
    }
}

fn like(kind: DbKind) -> &'static str {
    // case-insensitive everywhere
    if kind == DbKind::Postgres {
        "ILIKE"
    } else {
        "LIKE"
    }
}

// Limits from the SQLite built into the original (hard-limit table at
// 0x20fd990, checked by the LIKE function at 0x6934ee0):
/// SQLITE_LIMIT_LIKE_PATTERN_LENGTH: longer patterns fail with
/// "LIKE or GLOB pattern too complex".
pub const LIKE_PATTERN_MAX_BYTES: usize = 50000;
/// SQLITE_LIMIT_EXPR_DEPTH: an OR chain over N columns nests N deep.
pub const EXPR_DEPTH_MAX: usize = 1000;
/// Columns searched at once, kept well under the expression-depth limit
/// (the rest of the WHERE clause also adds depth).
pub const SEARCH_COLUMNS_MAX: usize = EXPR_DEPTH_MAX * 9 / 10;
/// The ESCAPE must be exactly one character and must not be '%' or '_'
/// (0x6934ee0 turns that wildcard off when they are equal).
pub const LIKE_ESCAPE: char = '!';

fn like_escape(v: &str) -> String {
    // '!' as the LIKE escape works the same on SQLite, PostgreSQL and MySQL
    // (a backslash would be read as a string escape by MySQL)
    let e = v.replace(LIKE_ESCAPE, "!!").replace('%', "!%").replace('_', "!_");
    // leave room for the two '%' around it
    if e.len() + 2 > LIKE_PATTERN_MAX_BYTES {
        let mut cut = LIKE_PATTERN_MAX_BYTES - 2;
        while !e.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut t = e[..cut].to_string();
        // never end on a lone escape character
        let trailing = t.chars().rev().take_while(|c| *c == LIKE_ESCAPE).count();
        if trailing % 2 == 1 {
            t.pop();
        }
        t
    } else {
        e
    }
}

/// A value literal: numbers stay numbers, everything else is a quoted string.
fn literal(v: &str) -> String {
    let t = v.trim();
    if !t.is_empty() && t.parse::<f64>().is_ok() && !t.starts_with('+') && !(t.len() > 1 && t.starts_with('0') && !t.starts_with("0.")) {
        t.to_string()
    } else {
        sql_string(t)
    }
}

/// SQLite's LIKE folds case only for ASCII letters: in `patternCompare`
/// (0x68580d0) two characters are compared through the A–Z→a–z table at
/// 0x20fd090 only when `(c1 | c2) <= 0x7f`; any other pair must be equal.
/// So "ação" would never match "AÇÃO". For SQLite and text with non-ASCII
/// letters we also try the lower-case, upper-case and Capitalised forms.
pub fn case_variants(kind: DbKind, v: &str) -> Vec<String> {
    let mut out = vec![v.to_string()];
    if kind != DbKind::Sqlite || v.is_ascii() || !v.chars().any(|c| !c.is_ascii() && c.is_alphabetic()) {
        return out;
    }
    let cap: String = v
        .split(' ')
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    for x in [v.to_lowercase(), v.to_uppercase(), cap] {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// `expr LIKE pattern` for every case variant, OR-ed together.
/// `wrap` turns the escaped text into the pattern (adds the % signs).
fn like_any(kind: DbKind, expr: &str, value: &str, negate: bool, wrap: impl Fn(&str) -> String) -> String {
    let lk = like(kind);
    let not = if negate { "NOT " } else { "" };
    let parts: Vec<String> = case_variants(kind, value).iter().map(|v| format!("{expr} {not}{lk} {} ESCAPE '!'", sql_string(&wrap(&like_escape(v))))).collect();
    if parts.len() == 1 {
        parts.into_iter().next().unwrap_or_default()
    } else {
        format!("({})", parts.join(if negate { " AND " } else { " OR " }))
    }
}

fn rule_sql(kind: DbKind, r: &Rule) -> Option<String> {
    let q = kind.quote(&r.column);
    let v = r.value.trim();
    if r.op.needs_value() && v.is_empty() {
        return None;
    }
    let e = as_text(kind, &r.column);
    Some(match r.op {
        Op::Contains => like_any(kind, &e, v, false, |x| format!("%{x}%")),
        Op::NotContains => format!("({q} IS NULL OR {})", like_any(kind, &e, v, true, |x| format!("%{x}%"))),
        Op::StartsWith => like_any(kind, &e, v, false, |x| format!("{x}%")),
        Op::EndsWith => like_any(kind, &e, v, false, |x| format!("%{x}")),
        Op::Equals => format!("{q} = {}", literal(v)),
        Op::NotEquals => format!("({q} IS NULL OR {q} <> {})", literal(v)),
        Op::Greater => format!("{q} > {}", literal(v)),
        Op::GreaterEq => format!("{q} >= {}", literal(v)),
        Op::Less => format!("{q} < {}", literal(v)),
        Op::LessEq => format!("{q} <= {}", literal(v)),
        Op::Between => {
            let mut it = v.splitn(2, [',', ';']);
            let a = it.next().unwrap_or("").trim();
            let b = it.next().unwrap_or("").trim();
            if a.is_empty() || b.is_empty() {
                return None;
            }
            format!("{q} BETWEEN {} AND {}", literal(a), literal(b))
        }
        Op::InList => {
            let items: Vec<String> = v.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).map(literal).collect();
            if items.is_empty() {
                return None;
            }
            format!("{q} IN ({})", items.join(", "))
        }
        Op::IsNull => format!("{q} IS NULL"),
        Op::NotNull => format!("{q} IS NOT NULL"),
    })
}

/// The WHERE clause body (without "WHERE"), or empty when nothing filters.
pub fn where_clause(kind: DbKind, f: &FilterSet, columns: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let s = f.search.trim();
    if !s.is_empty() && !columns.is_empty() {
        // each column adds one OR per case variant: keep the whole chain
        // under the expression-depth limit
        let n_var = case_variants(kind, s).len().max(1);
        let max_cols = (SEARCH_COLUMNS_MAX / n_var).max(1);
        let pats: Vec<String> = case_variants(kind, s).iter().map(|v| sql_string(&format!("%{}%", like_escape(v)))).collect();
        let ors: Vec<String> = columns
            .iter()
            .take(max_cols)
            .flat_map(|c| pats.iter().map(move |pat| format!("{} {} {pat} ESCAPE '!'", as_text(kind, c), like(kind))))
            .collect();
        parts.push(format!("({})", ors.join(" OR ")));
    }
    let rules: Vec<String> = f.rules.iter().filter(|r| r.enabled).filter_map(|r| rule_sql(kind, r)).collect();
    if !rules.is_empty() {
        let joiner = if f.match_any { " OR " } else { " AND " };
        parts.push(format!("({})", rules.join(joiner)));
    }
    if f.raw_mode && !f.raw.trim().is_empty() {
        parts.push(format!("({})", f.raw.trim()));
    }
    parts.join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_where() {
        let f = FilterSet {
            search: "ho".into(),
            rules: vec![
                Rule { column: "price".into(), op: Op::Greater, value: "20".into(), enabled: true },
                Rule { column: "name".into(), op: Op::Contains, value: "it's".into(), enabled: true },
                Rule { column: "x".into(), op: Op::Equals, value: "".into(), enabled: true },
            ],
            ..Default::default()
        };
        let w = where_clause(DbKind::Sqlite, &f, &["name".into(), "category".into()]);
        assert!(w.contains("\"price\" > 20"));
        assert!(w.contains("'%it''s%'"));
        assert!(w.contains("\"category\" LIKE '%ho%'"));
        assert!(!w.contains("\"x\""));
        let pg = where_clause(DbKind::Postgres, &f, &["name".into()]);
        assert!(pg.contains("ILIKE"));
        assert_eq!(literal("007"), "'007'");
        assert_eq!(literal("12.5"), "12.5");
    }

    #[test]
    fn non_ascii_case_on_sqlite() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t (name TEXT); INSERT INTO t VALUES ('AÇÃO RÁPIDA'), ('Ação'), ('outra');").unwrap();
        // plain LIKE misses the upper-case row (ASCII-only folding)
        let plain: i64 = c.query_row("SELECT COUNT(*) FROM t WHERE name LIKE '%ação%'", [], |r| r.get(0)).unwrap();
        assert_eq!(plain, 1);
        let f = FilterSet { search: "ação".into(), ..Default::default() };
        let w = where_clause(DbKind::Sqlite, &f, &["name".into()]);
        let n: i64 = c.query_row(&format!("SELECT COUNT(*) FROM t WHERE {w}"), [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        let rule = FilterSet { rules: vec![Rule { column: "name".into(), op: Op::NotContains, value: "ação".into(), enabled: true }], ..Default::default() };
        let w = where_clause(DbKind::Sqlite, &rule, &["name".into()]);
        let n: i64 = c.query_row(&format!("SELECT COUNT(*) FROM t WHERE {w}"), [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        assert_eq!(case_variants(DbKind::Postgres, "ação").len(), 1);
        assert_eq!(case_variants(DbKind::Sqlite, "honey").len(), 1);
    }

    #[test]
    fn respects_sqlite_limits() {
        assert!(LIKE_ESCAPE != '%' && LIKE_ESCAPE != '_');
        let long = "x".repeat(60000);
        assert!(like_escape(&long).len() + 2 <= LIKE_PATTERN_MAX_BYTES);
        let odd = "!".repeat(30000);
        let e = like_escape(&odd);
        assert!(e.len() + 2 <= LIKE_PATTERN_MAX_BYTES && e.len() % 2 == 0);
        let cols: Vec<String> = (0..1500).map(|i| format!("c{i}")).collect();
        let f = FilterSet { search: "a".into(), ..Default::default() };
        let w = where_clause(DbKind::Sqlite, &f, &cols);
        assert_eq!(w.matches(" OR ").count() + 1, SEARCH_COLUMNS_MAX);
        // a real SQLite accepts the search over a very wide table
        let c = rusqlite::Connection::open_in_memory().unwrap();
        let defs: Vec<String> = cols.iter().take(1500).map(|x| format!("{x} TEXT")).collect();
        c.execute_batch(&format!("CREATE TABLE w ({});", defs.join(","))).unwrap();
        c.prepare(&format!("SELECT * FROM w WHERE {w}")).unwrap();
        let big = FilterSet { search: long, ..Default::default() };
        let w2 = where_clause(DbKind::Sqlite, &big, &["c0".into()]);
        let n: i64 = c.query_row(&format!("SELECT COUNT(*) FROM w WHERE {w2}"), [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
