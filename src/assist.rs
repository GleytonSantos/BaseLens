//! Editor helpers: context-aware autocomplete (tables after FROM/JOIN,
//! columns of an alias after "alias.", columns of the tables already in the
//! query, keywords and functions with fuzzy matching), a small SQL formatter
//! and line commenting.

use crate::db::DbKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct SchemaInfo {
    pub kind: DbKind,
    /// (table name, is view, columns as (name, type))
    pub tables: Vec<(String, bool, Vec<(String, String)>)>,
    /// identifiers the user typed in past queries, with how often
    pub learned: Vec<(String, u32)>,
    /// known values per "table.column" (lower case): sampled + remembered
    pub values: HashMap<String, Vec<ValueHint>>,
}

#[derive(Clone, Debug)]
pub struct ValueHint {
    pub text: String,
    /// rows with this value in the sample
    pub count: Option<i64>,
    /// times the user wrote it in a query
    pub used: u32,
}

/// Where a value is being typed: `col = '…`, `t.col IN (…`, `col LIKE …`.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueTarget {
    pub table: String,
    pub column: String,
    pub col_type: String,
    /// "table.column" in lower case
    pub key: String,
    /// char index where the replacement starts (the opening quote, if any)
    pub start: usize,
    pub partial: String,
    pub quoted: bool,
}

pub fn value_key(table: &str, column: &str) -> String {
    format!("{}.{}", table.to_lowercase(), column.to_lowercase())
}

/// SQLite column affinity from a declared type, as the original's SQLite
/// computes it (inlined in `valueFromExpr`, function 0x68a7000): the type
/// name is read letter by letter keeping the last four letters, lower-cased
/// through the table at 0x20fd090, in a 32-bit value `h = (h << 8) | c`:
/// "char" / "clob" / "text" → TEXT; "blob" → BLOB while still NUMERIC or
/// REAL; "real" / "floa" / "doub" → REAL while still NUMERIC; the last three
/// letters "int" → INTEGER at once; anything else stays NUMERIC. A column
/// declared without a type has BLOB affinity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Affinity {
    Blob,
    Text,
    Numeric,
    Integer,
    Real,
}

pub const H_CHAR: u32 = 0x63686172;
pub const H_CLOB: u32 = 0x636c6f62;
pub const H_TEXT: u32 = 0x74657874;
pub const H_BLOB: u32 = 0x626c6f62;
pub const H_REAL: u32 = 0x7265616c;
pub const H_FLOA: u32 = 0x666c6f61;
pub const H_DOUB: u32 = 0x646f7562;
pub const H_INT: u32 = 0x696e74;

pub fn sqlite_affinity(decl: &str) -> Affinity {
    if decl.trim().is_empty() {
        return Affinity::Blob;
    }
    let mut aff = Affinity::Numeric;
    let mut h: u32 = 0;
    for b in decl.bytes() {
        h = (h << 8) | b.to_ascii_lowercase() as u32;
        if h == H_CHAR || h == H_CLOB || h == H_TEXT {
            aff = Affinity::Text;
        } else if h == H_BLOB && (aff == Affinity::Numeric || aff == Affinity::Real) {
            aff = Affinity::Blob;
        } else if (h == H_REAL || h == H_FLOA || h == H_DOUB) && aff == Affinity::Numeric {
            aff = Affinity::Real;
        } else if h & 0x00ff_ffff == H_INT {
            return Affinity::Integer;
        }
    }
    aff
}

fn numeric_type(t: &str) -> bool {
    let t = t.to_lowercase();
    ["int", "real", "num", "dec", "float", "double", "serial", "money"].iter().any(|k| t.contains(k)) && !t.contains("point")
}

/// SQL text for a suggested value: numbers bare for numeric columns,
/// everything else as a quoted string.
pub fn value_literal(kind: DbKind, v: &str, col_type: &str) -> String {
    let numeric_col = if kind == DbKind::Sqlite {
        matches!(sqlite_affinity(col_type), Affinity::Integer | Affinity::Real | Affinity::Numeric)
    } else {
        numeric_type(col_type)
    };
    if numeric_col && v.parse::<f64>().is_ok() {
        v.to_string()
    } else {
        format!("'{}'", v.replace('\'', "''"))
    }
}

/// Detects that the cursor is on the value side of a comparison.
pub fn value_target(text: &str, cursor: usize, schema: &SchemaInfo) -> Option<ValueTarget> {
    let all: Vec<char> = text.chars().collect();
    let cur = cursor.min(all.len());
    let chars = &all[..cur];
    // current statement only
    let stmt_start = chars.iter().rposition(|c| *c == ';').map(|i| i + 1).unwrap_or(0);
    let quotes = chars[stmt_start..].iter().filter(|c| **c == '\'').count();
    let (start, partial, quoted) = if quotes % 2 == 1 {
        let q = chars.iter().rposition(|c| *c == '\'')?;
        (q, chars[q + 1..].iter().collect::<String>(), true)
    } else {
        let mut st = cur;
        while st > stmt_start && (chars[st - 1].is_alphanumeric() || matches!(chars[st - 1], '_' | '-' | '.')) {
            st -= 1;
        }
        (st, chars[st..].iter().collect::<String>(), false)
    };
    let mut i = start;
    let skip_ws = |i: &mut usize| {
        while *i > stmt_start && chars[*i - 1].is_whitespace() {
            *i -= 1;
        }
    };
    skip_ws(&mut i);
    if i == stmt_start {
        return None;
    }
    let word_before = |end: usize| -> (usize, String) {
        let mut b = end;
        while b > stmt_start && (chars[b - 1].is_alphanumeric() || chars[b - 1] == '_') {
            b -= 1;
        }
        (b, chars[b..end].iter().collect::<String>())
    };
    // operator
    let c = chars[i - 1];
    if c == '(' || c == ',' {
        // inside IN ( … ): find the "(" and the IN before it
        let open = chars[stmt_start..i].iter().rposition(|x| *x == '(').map(|p| p + stmt_start)?;
        if chars[open..i].iter().any(|x| *x == ')') {
            return None;
        }
        let mut j = open;
        skip_ws(&mut j);
        let (b, w) = word_before(j);
        if !w.eq_ignore_ascii_case("in") {
            return None;
        }
        i = b;
    } else if matches!(c, '=' | '<' | '>') {
        while i > stmt_start && matches!(chars[i - 1], '=' | '<' | '>' | '!') {
            i -= 1;
        }
    } else {
        let (b, w) = word_before(i);
        if !(w.eq_ignore_ascii_case("like") || w.eq_ignore_ascii_case("ilike")) {
            return None;
        }
        i = b;
        let mut j = i;
        skip_ws(&mut j);
        let (b2, w2) = word_before(j);
        if w2.eq_ignore_ascii_case("not") {
            i = b2;
        }
    }
    skip_ws(&mut i);
    // column, maybe qualified / quoted
    let mut b = i;
    while b > stmt_start && (chars[b - 1].is_alphanumeric() || matches!(chars[b - 1], '_' | '.' | '"' | '`')) {
        b -= 1;
    }
    let ident: String = chars[b..i].iter().filter(|c| **c != '"' && **c != '`').collect();
    if ident.is_empty() {
        return None;
    }
    let (qual, col) = match ident.rfind('.') {
        Some(p) => (Some(ident[..p].to_string()), ident[p + 1..].to_string()),
        None => (None, ident.clone()),
    };
    let al = aliases(text);
    let has_col = |t: &str| schema.tables.iter().find(|x| x.0.eq_ignore_ascii_case(t)).and_then(|x| x.2.iter().find(|c| c.0.eq_ignore_ascii_case(&col)).map(|c| (x.0.clone(), c.0.clone(), c.1.clone())));
    let found = match &qual {
        Some(q) => {
            let q = q.rsplit('.').next().unwrap_or(q).to_lowercase();
            let t = al.get(&q).cloned().unwrap_or(q);
            has_col(&t)
        }
        None => al.values().find_map(|t| has_col(t)).or_else(|| schema.tables.iter().find_map(|t| has_col(&t.0))),
    }?;
    Some(ValueTarget { key: value_key(&found.0, &found.1), table: found.0, column: found.1, col_type: found.2, start, partial, quoted })
}

// ------------------------------------------------------------ learning

/// What the user wrote in past queries, per connection: identifiers and the
/// values used in comparisons. Fed back into autocomplete.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Learned {
    pub words: HashMap<String, u32>,
    pub values: HashMap<String, Vec<(String, u32)>>,
}

pub const LEARN_MAX_WORDS: usize = 500;
pub const LEARN_MAX_KEYS: usize = 300;
pub const LEARN_MAX_VALUES: usize = 30;
pub const VALUE_MAX_CHARS: usize = 80;

impl Learned {
    fn bump_value(&mut self, key: String, v: String) {
        if v.is_empty() || v.chars().count() > VALUE_MAX_CHARS {
            return;
        }
        let list = self.values.entry(key).or_default();
        match list.iter_mut().find(|x| x.0 == v) {
            Some(x) => x.1 += 1,
            None => list.push((v, 1)),
        }
        list.sort_by(|a, b| b.1.cmp(&a.1));
        list.truncate(LEARN_MAX_VALUES);
    }

    pub fn learn(&mut self, sql: &str, result_cols: &[String], schema: &SchemaInfo) {
        let known: std::collections::HashSet<String> = schema
            .tables
            .iter()
            .flat_map(|t| std::iter::once(t.0.to_lowercase()).chain(t.2.iter().map(|c| c.0.to_lowercase())))
            .collect();
        let toks = tokenize(sql);
        for t in &toks {
            if let Tok::Word(w) = t {
                for part in w.split('.') {
                    let ok = part.len() >= 2
                        && part.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false)
                        && part.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !is_keyword(part);
                    // schema names are suggested anyway; remember the rest
                    // (aliases, computed names) and count everything used
                    if ok {
                        *self.words.entry(part.to_string()).or_insert(0) += if known.contains(&part.to_lowercase()) { 1 } else { 2 };
                    }
                }
            }
        }
        for c in result_cols {
            if c.len() >= 2 && c.chars().all(|x| x.is_alphanumeric() || x == '_') && !is_keyword(c) {
                *self.words.entry(c.clone()).or_insert(0) += 1;
            }
        }
        // values: <ident> (=|<>|!=|LIKE) <literal>  and  <ident> IN ( <literal>, … )
        let al = aliases(sql);
        let resolve = |ident: &str| -> Option<String> {
            let (q, col) = match ident.rfind('.') {
                Some(p) => (Some(&ident[..p]), &ident[p + 1..]),
                None => (None, ident),
            };
            let has = |t: &str| schema.tables.iter().find(|x| x.0.eq_ignore_ascii_case(t)).filter(|x| x.2.iter().any(|c| c.0.eq_ignore_ascii_case(col))).map(|x| value_key(&x.0, col));
            match q {
                Some(q) => has(&al.get(&q.to_lowercase()).cloned().unwrap_or_else(|| q.to_string())),
                None => al.values().find_map(|t| has(t)),
            }
        };
        let lit = |t: &Tok| -> Option<String> {
            match t {
                Tok::Str(s) if s.starts_with('\'') && s.len() >= 2 => Some(s[1..s.len() - 1].replace("''", "'")),
                Tok::Word(w) if w.parse::<f64>().is_ok() => Some(w.clone()),
                _ => None,
            }
        };
        let mut i = 0;
        while i < toks.len() {
            if let Tok::Word(id) = &toks[i] {
                if !is_keyword(id) {
                    let mut j = i + 1;
                    let mut op = String::new();
                    while let Some(Tok::Punct(c)) = toks.get(j) {
                        if matches!(c, '=' | '<' | '>' | '!') {
                            op.push(*c);
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    let is_in = op.is_empty() && matches!(toks.get(j), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("in"));
                    let is_like = op.is_empty() && matches!(toks.get(j), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("like") || w.eq_ignore_ascii_case("ilike"));
                    if matches!(op.as_str(), "=" | "<>" | "!=") || is_like {
                        if is_like {
                            j += 1;
                        }
                        if let (Some(key), Some(v)) = (resolve(id), toks.get(j).and_then(lit)) {
                            let v = if is_like { v.trim_matches('%').to_string() } else { v };
                            self.bump_value(key, v);
                        }
                    } else if is_in && matches!(toks.get(j + 1), Some(Tok::Punct('('))) {
                        if let Some(key) = resolve(id) {
                            let mut k = j + 2;
                            while let Some(t) = toks.get(k) {
                                match t {
                                    Tok::Punct(')') => break,
                                    Tok::Punct(',') => {}
                                    other => {
                                        if let Some(v) = lit(other) {
                                            self.bump_value(key.clone(), v);
                                        }
                                    }
                                }
                                k += 1;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        // keep the files small: drop the least used
        if self.words.len() > LEARN_MAX_WORDS {
            let mut v: Vec<(String, u32)> = self.words.drain().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            v.truncate(LEARN_MAX_WORDS);
            self.words = v.into_iter().collect();
        }
        if self.values.len() > LEARN_MAX_KEYS {
            let mut v: Vec<(String, Vec<(String, u32)>)> = self.values.drain().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1.iter().map(|y| y.1).sum::<u32>()));
            v.truncate(LEARN_MAX_KEYS);
            self.values = v.into_iter().collect();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SugKind {
    Column,
    Table,
    View,
    Keyword,
    Function,
    Snippet,
    Value,
    Learned,
}

impl SugKind {
    pub fn badge(self) -> &'static str {
        match self {
            SugKind::Column => "col",
            SugKind::Table => "tbl",
            SugKind::View => "view",
            SugKind::Keyword => "kw",
            SugKind::Function => "fn",
            SugKind::Snippet => "snip",
            SugKind::Value => "val",
            SugKind::Learned => "used",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    pub kind: SugKind,
    pub detail: String,
}

pub const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL", "AS", "ON", "JOIN", "LEFT JOIN", "RIGHT JOIN",
    "INNER JOIN", "FULL JOIN", "CROSS JOIN", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "OFFSET", "INSERT INTO", "VALUES",
    "UPDATE", "SET", "DELETE FROM", "CREATE TABLE", "CREATE VIEW", "CREATE INDEX", "DROP TABLE", "ALTER TABLE",
    "DISTINCT", "UNION", "UNION ALL", "CASE", "WHEN", "THEN", "ELSE", "END", "WITH", "LIKE", "ILIKE", "BETWEEN", "EXISTS",
    "ASC", "DESC", "DEFAULT", "BEGIN", "COMMIT", "ROLLBACK", "TRUE", "FALSE", "RETURNING", "EXPLAIN", "PRIMARY KEY",
    "FOREIGN KEY", "REFERENCES",
];

pub const FUNCTIONS: &[&str] = &[
    "COUNT", "SUM", "AVG", "MIN", "MAX", "COALESCE", "NULLIF", "ROUND", "LOWER", "UPPER", "LENGTH", "SUBSTR", "TRIM",
    "CAST", "DATE", "NOW", "CURRENT_DATE", "CURRENT_TIMESTAMP", "ABS", "REPLACE", "CONCAT", "STRFTIME", "JSON_EXTRACT",
    "ROW_NUMBER", "RANK", "DENSE_RANK", "STRING_AGG", "GROUP_CONCAT", "EXTRACT", "DATE_TRUNC",
];

/// Values shown in the popup at once.
pub const VALUE_SUGGESTIONS_SHOWN: usize = 14;

const SNIPPETS: &[(&str, &str, &str)] = &[
    ("sel", "SELECT * FROM ", "SELECT * FROM …"),
    ("selc", "SELECT COUNT(*) FROM ", "SELECT COUNT(*) FROM …"),
    ("ins", "INSERT INTO  () VALUES ()", "INSERT INTO … VALUES"),
];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Prefix being typed before `cursor` (char index): (start, prefix, qualifier
/// before a dot, e.g. "o" in "o.na").
pub fn word_before(text: &str, cursor: usize) -> (usize, String, Option<String>) {
    let chars: Vec<char> = text.chars().collect();
    let cur = cursor.min(chars.len());
    let mut start = cur;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let prefix: String = chars[start..cur].iter().collect();
    let mut qual = None;
    if start > 0 && chars[start - 1] == '.' {
        let mut qs = start - 1;
        let mut qe = qs;
        if qs > 0 && (chars[qs - 1] == '"' || chars[qs - 1] == '`') {
            qe -= 1;
            qs -= 1;
            let qc = chars[qs];
            while qs > 0 && chars[qs - 1] != qc {
                qs -= 1;
            }
            let name: String = chars[qs..qe].iter().collect();
            qual = Some(name);
        } else {
            while qs > 0 && is_word(chars[qs - 1]) {
                qs -= 1;
            }
            let name: String = chars[qs..qe].iter().collect();
            if !name.is_empty() {
                qual = Some(name);
            }
        }
    }
    (start, prefix, qual)
}

fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in text.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
            continue;
        }
        if c == '"' || c == '`' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            quote = Some(c);
        } else if c == '\'' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            quote = Some('\'');
        } else if is_word(c) || c == '.' {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if c == ',' || c == '(' || c == ')' || c == ';' || c == '=' {
                out.push(c.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

const NOT_ALIAS: &[&str] = &[
    "WHERE", "ON", "JOIN", "LEFT", "RIGHT", "INNER", "FULL", "CROSS", "OUTER", "GROUP", "ORDER", "LIMIT", "SET", "VALUES",
    "USING", "HAVING", "UNION", "AS", ",", "(", ")", ";", "OFFSET", "NATURAL",
];

/// alias (lower-case) → table, plus every referenced table → itself.
pub fn aliases(sql: &str) -> HashMap<String, String> {
    let w = words(sql);
    let mut map = HashMap::new();
    let mut i = 0;
    while i < w.len() {
        let u = w[i].to_uppercase();
        if matches!(u.as_str(), "FROM" | "JOIN" | "UPDATE" | "INTO") || (u == "," && i > 0) {
            if let Some(t) = w.get(i + 1) {
                if !NOT_ALIAS.contains(&t.to_uppercase().as_str()) && t != "(" {
                    let table = t.rsplit('.').next().unwrap_or(t).to_string();
                    map.insert(table.to_lowercase(), table.clone());
                    let mut j = i + 2;
                    if w.get(j).map(|x| x.eq_ignore_ascii_case("AS")).unwrap_or(false) {
                        j += 1;
                    }
                    if let Some(a) = w.get(j) {
                        if !NOT_ALIAS.contains(&a.to_uppercase().as_str()) && !KEYWORDS.contains(&a.to_uppercase().as_str()) {
                            map.insert(a.to_lowercase(), table.clone());
                        }
                    }
                }
            }
        }
        i += 1;
    }
    map
}

/// Score how well `cand` matches `prefix` (lower is better); None = no match.
fn score(cand: &str, prefix: &str) -> Option<u32> {
    if prefix.is_empty() {
        return Some(5);
    }
    let c = cand.to_lowercase();
    let p = prefix.to_lowercase();
    if c == p {
        return Some(0);
    }
    if c.starts_with(&p) {
        return Some(1);
    }
    if c.split(['_', ' ', '.']).any(|part| part.starts_with(&p)) {
        return Some(2);
    }
    if c.contains(&p) {
        return Some(3);
    }
    // fuzzy: letters in order
    let mut it = c.chars();
    if p.chars().all(|ch| it.any(|x| x == ch)) && p.len() >= 2 {
        return Some(4);
    }
    None
}

/// The 147 words SQLite treats as keywords, read from the original's keyword
/// tables (text 0x20fd320; offsets and lengths at 0x20fd5c0 / 0x20fd6f0,
/// indexed from 1 as the hash table 0x20fdeb0 stores index+1).
const SQLITE_KEYWORDS: &[&str] = &[
    "ABORT", "ACTION", "ADD", "AFTER", "ALL", "ALTER", "ALWAYS", "ANALYZE", "AND", "AS", "ASC", "ATTACH",
    "AUTOINCREMENT", "BEFORE", "BEGIN", "BETWEEN", "BY", "CASCADE", "CASE", "CAST", "CHECK", "COLLATE",
    "COLUMN", "COMMIT", "CONFLICT", "CONSTRAINT", "CREATE", "CROSS", "CURRENT", "CURRENT_DATE",
    "CURRENT_TIME", "CURRENT_TIMESTAMP", "DATABASE", "DEFAULT", "DEFERRABLE", "DEFERRED", "DELETE", "DESC",
    "DETACH", "DISTINCT", "DO", "DROP", "EACH", "ELSE", "END", "ESCAPE", "EXCEPT", "EXCLUDE", "EXCLUSIVE",
    "EXISTS", "EXPLAIN", "FAIL", "FILTER", "FIRST", "FOLLOWING", "FOR", "FOREIGN", "FROM", "FULL",
    "GENERATED", "GLOB", "GROUP", "GROUPS", "HAVING", "IF", "IGNORE", "IMMEDIATE", "IN", "INDEX", "INDEXED",
    "INITIALLY", "INNER", "INSERT", "INSTEAD", "INTERSECT", "INTO", "IS", "ISNULL", "JOIN", "KEY", "LAST",
    "LEFT", "LIKE", "LIMIT", "MATCH", "MATERIALIZED", "NATURAL", "NO", "NOT", "NOTHING", "NOTNULL", "NULL",
    "NULLS", "OF", "OFFSET", "ON", "OR", "ORDER", "OTHERS", "OUTER", "OVER", "PARTITION", "PLAN", "PRAGMA",
    "PRECEDING", "PRIMARY", "QUERY", "RAISE", "RANGE", "RECURSIVE", "REFERENCES", "REGEXP", "REINDEX", "RELEASE",
    "RENAME", "REPLACE", "RESTRICT", "RETURNING", "RIGHT", "ROLLBACK", "ROW", "ROWS", "SAVEPOINT", "SELECT",
    "SET", "TABLE", "TEMP", "TEMPORARY", "THEN", "TIES", "TO", "TRANSACTION", "TRIGGER", "UNBOUNDED",
    "UNION", "UNIQUE", "UPDATE", "USING", "VACUUM", "VALUES", "VIEW", "VIRTUAL", "WHEN", "WHERE", "WINDOW",
    "WITH", "WITHOUT",
];

/// Keyword test as in the original (0x6903ab0): only names of 2+ bytes are
/// looked up, and each byte is folded with `& 0xdf`, so "select", "Select"
/// and "SELECT" are the same keyword.
pub fn is_sql_keyword(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 2
        && SQLITE_KEYWORDS.iter().any(|k| k.len() == b.len() && k.bytes().zip(b).all(|(k, &c)| k == c & 0xdf))
}

/// Quote rule from the original's identifier writer (0x6903ab0): quote when the
/// name is empty, starts with a digit, has a byte that is not a letter, digit or
/// '_' (ctype bits 0x06), or is a keyword in any case. Postgres also needs
/// quotes to keep upper-case letters.
fn needs_quote(kind: DbKind, name: &str) -> bool {
    let first_digit = name.bytes().next().map(|c| c.is_ascii_digit()).unwrap_or(true);
    let all_ok = name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    let mixed = kind == DbKind::Postgres && name.chars().any(|c| c.is_ascii_uppercase());
    first_digit || !all_ok || is_sql_keyword(name) || mixed
}

fn ident(kind: DbKind, name: &str) -> String {
    if needs_quote(kind, name) {
        kind.quote(name)
    } else {
        name.to_string()
    }
}

/// Keyword casing follows the typed prefix (setting keywordCasing = preserve):
/// "sel" → "select", "SEL" → "SELECT", nothing typed → upper case.
fn cased(kw: &str, prefix: &str) -> String {
    let has_lower = prefix.chars().any(|c| c.is_lowercase());
    let has_upper = prefix.chars().any(|c| c.is_uppercase());
    if has_lower && !has_upper {
        kw.to_lowercase()
    } else {
        kw.to_uppercase()
    }
}

pub fn suggest(text: &str, cursor: usize, schema: &SchemaInfo, forced: bool) -> (usize, Vec<Suggestion>) {
    // value side of a comparison: offer the column's values
    if let Some(vt) = value_target(text, cursor, schema) {
        let p = vt.partial.to_lowercase();
        let mut out: Vec<Suggestion> = schema
            .values
            .get(&vt.key)
            .map(|vals| {
                vals.iter()
                    .filter(|v| p.is_empty() || v.text.to_lowercase().contains(&p))
                    .map(|v| Suggestion {
                        label: v.text.clone(),
                        insert: value_literal(schema.kind, &v.text, &vt.col_type),
                        kind: if v.used > 0 { SugKind::Learned } else { SugKind::Value },
                        detail: match (v.count, v.used) {
                            (Some(n), u) if u > 0 => format!("{n} rows · used {u}×"),
                            (Some(n), _) => format!("{n} rows"),
                            (None, u) => format!("used {u}×"),
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        // values starting with what was typed first
        out.sort_by_key(|s| !s.label.to_lowercase().starts_with(&p));
        out.truncate(VALUE_SUGGESTIONS_SHOWN);
        if !out.is_empty() || vt.quoted || !vt.partial.chars().all(|c| c.is_alphanumeric() || c == '_') || vt.partial.is_empty() {
            return (vt.start, out);
        }
    }
    let (start, prefix, qual) = word_before(text, cursor);
    if prefix.is_empty() && qual.is_none() && !forced {
        return (start, vec![]);
    }
    let before: String = text.chars().take(start).collect();
    let prev_words = words(&before);
    let prev = prev_words.iter().rev().find(|w| *w != ",").map(|w| w.to_uppercase()).unwrap_or_default();
    let prev2 = prev_words.iter().rev().nth(1).map(|w| w.to_uppercase()).unwrap_or_default();
    let want_tables = matches!(prev.as_str(), "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE")
        || (prev_words.last().map(|w| w == ",").unwrap_or(false) && matches!(prev2.as_str(), "FROM"));
    let al = aliases(text);
    let mut out: Vec<(u32, u32, Suggestion)> = Vec::new();
    let col_of = |t: &str| schema.tables.iter().find(|x| x.0.eq_ignore_ascii_case(t));

    if let Some(q) = &qual {
        let table = al.get(&q.to_lowercase()).cloned().unwrap_or_else(|| q.clone());
        if let Some((tname, _, cols)) = col_of(&table) {
            for (c, ty) in cols {
                if let Some(s) = score(c, &prefix) {
                    out.push((0, s, Suggestion { label: c.clone(), insert: ident(schema.kind, c), kind: SugKind::Column, detail: format!("{tname} · {}", ty.to_lowercase()) }));
                }
            }
        }
    } else if want_tables {
        for (t, view, cols) in &schema.tables {
            if let Some(s) = score(t, &prefix) {
                out.push((0, s, Suggestion {
                    label: t.clone(),
                    insert: ident(schema.kind, t),
                    kind: if *view { SugKind::View } else { SugKind::Table },
                    detail: format!("{} columns", cols.len()),
                }));
            }
        }
    } else {
        let in_query: Vec<String> = al.values().cloned().collect();
        for (t, _, cols) in &schema.tables {
            let referenced = in_query.iter().any(|x| x.eq_ignore_ascii_case(t));
            if !referenced && prefix.is_empty() {
                continue;
            }
            for (c, ty) in cols {
                if let Some(s) = score(c, &prefix) {
                    out.push((if referenced { 0 } else { 3 }, s, Suggestion { label: c.clone(), insert: ident(schema.kind, c), kind: SugKind::Column, detail: format!("{t} · {}", ty.to_lowercase()) }));
                }
            }
        }
        if !prefix.is_empty() {
            for (t, view, cols) in &schema.tables {
                if let Some(s) = score(t, &prefix) {
                    out.push((2, s, Suggestion { label: t.clone(), insert: ident(schema.kind, t), kind: if *view { SugKind::View } else { SugKind::Table }, detail: format!("{} columns", cols.len()) }));
                }
            }
        }
        for k in KEYWORDS {
            if let Some(s) = score(k, &prefix) {
                if s <= 2 {
                    out.push((1, s, Suggestion { label: k.to_string(), insert: cased(k, &prefix), kind: SugKind::Keyword, detail: "keyword".into() }));
                }
            }
        }
        for f in FUNCTIONS {
            if let Some(s) = score(f, &prefix) {
                if s <= 2 {
                    out.push((1, s, Suggestion { label: format!("{f}()"), insert: format!("{}(", cased(f, &prefix)), kind: SugKind::Function, detail: "function".into() }));
                }
            }
        }
        if !prefix.is_empty() {
            for (w, n) in &schema.learned {
                if let Some(sc) = score(w, &prefix) {
                    if sc <= 2 {
                        out.push((1, sc, Suggestion { label: w.clone(), insert: ident(schema.kind, w), kind: SugKind::Learned, detail: format!("used {n}×") }));
                    }
                }
            }
        }
        for (trig, ins, label) in SNIPPETS {
            if !prefix.is_empty() && trig.starts_with(&prefix.to_lowercase()) {
                // after the keywords, and in the same case as what was typed
                let lower = prefix.chars().any(|c| c.is_lowercase()) && !prefix.chars().any(|c| c.is_uppercase());
                let (label, ins) = if lower { (label.to_lowercase(), ins.to_lowercase()) } else { (label.to_string(), ins.to_string()) };
                out.push((1, 3, Suggestion { label, insert: ins, kind: SugKind::Snippet, detail: "snippet".into() }));
            }
        }
    }
    // exact-prefix matches first, then group (context), then fuzziness, then length
    out.sort_by(|a, b| {
        let ea = (a.1 > 1) as u32;
        let eb = (b.1 > 1) as u32;
        (ea, a.0, a.1, a.2.label.len()).cmp(&(eb, b.0, b.1, b.2.label.len()))
    });
    let mut seen = std::collections::HashSet::new();
    let list: Vec<Suggestion> = out
        .into_iter()
        .map(|x| x.2)
        // one entry per name: the best-ranked kind wins
        .filter(|s| seen.insert(s.label.to_lowercase()))
        // don't offer exactly what is already typed
        .filter(|s| !(s.insert == prefix && s.kind != SugKind::Function))
        .take(14)
        .collect();
    (start, list)
}

// ------------------------------------------------------------ formatting

const CLAUSES: &[&str] = &[
    "SELECT", "FROM", "WHERE", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "OFFSET", "VALUES", "SET", "RETURNING", "UNION",
    "UNION ALL", "JOIN", "LEFT JOIN", "RIGHT JOIN", "INNER JOIN", "FULL JOIN", "CROSS JOIN", "LEFT OUTER JOIN",
    "RIGHT OUTER JOIN", "FULL OUTER JOIN", "INSERT INTO", "UPDATE", "DELETE FROM", "WITH",
];

#[derive(Debug)]
enum Tok {
    Word(String),
    Str(String),
    Comment(String),
    Punct(char),
}

fn tokenize(sql: &str) -> Vec<Tok> {
    let cs: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && cs.get(i + 1) == Some(&'-') {
            let s = i;
            while i < cs.len() && cs[i] != '\n' {
                i += 1;
            }
            out.push(Tok::Comment(cs[s..i].iter().collect()));
        } else if c == '\'' || c == '"' || c == '`' {
            let s = i;
            i += 1;
            while i < cs.len() && cs[i] != c {
                i += 1;
            }
            i = (i + 1).min(cs.len());
            out.push(Tok::Str(cs[s..i].iter().collect()));
        } else if is_word(c) || c == '.' || c == '*' || c == '$' || c == ':' || c == '@' {
            let s = i;
            while i < cs.len() && (is_word(cs[i]) || cs[i] == '.' || cs[i] == '*' || cs[i] == '$' || cs[i] == ':' || cs[i] == '@') {
                i += 1;
            }
            out.push(Tok::Word(cs[s..i].iter().collect()));
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    out
}

fn is_keyword(w: &str) -> bool {
    let u = w.to_uppercase();
    crate::widgets::is_sql_keyword(&u) || FUNCTIONS.contains(&u.as_str())
}

/// Readable layout: one clause per line, select columns on their own lines,
/// AND/OR indented, keywords upper-case. Strings and comments are untouched.
/// Keyword case for the formatter, following the query itself (setting
/// keywordCasing = preserve): lower case when most keywords were typed in
/// lower case, otherwise UPPER case.
fn keyword_case(toks: &[Tok]) -> bool {
    let (mut lower, mut upper) = (0, 0);
    for t in toks {
        if let Tok::Word(w) = t {
            if is_keyword(w) && !w.contains('.') {
                if w.chars().any(|c| c.is_lowercase()) {
                    lower += 1;
                } else {
                    upper += 1;
                }
            }
        }
    }
    lower > upper
}

pub fn format_sql(sql: &str) -> String {
    let toks = tokenize(sql);
    let lower_kw = keyword_case(&toks);
    let kw = |k: &str| if lower_kw { k.to_lowercase() } else { k.to_uppercase() };
    let mut out = String::new();
    let mut depth: i32 = 0;
    let mut clause = String::new();
    let mut i = 0;
    let indent = |d: i32| "  ".repeat(d.max(0) as usize);
    let need_space = |out: &str| !(out.is_empty() || out.ends_with(' ') || out.ends_with('\n') || out.ends_with('('));
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) => {
                // try 3-, 2- and 1-word clause names
                let mut matched = None;
                for n in [3usize, 2, 1] {
                    if i + n <= toks.len() {
                        let parts: Option<Vec<String>> = toks[i..i + n].iter().map(|t| if let Tok::Word(x) = t { Some(x.to_uppercase()) } else { None }).collect();
                        if let Some(p) = parts {
                            let j = p.join(" ");
                            if CLAUSES.contains(&j.as_str()) {
                                matched = Some((j, n));
                                break;
                            }
                        }
                    }
                }
                if let Some((cl, n)) = matched {
                    let trimmed = out.trim_end().to_string();
                    out = trimmed;
                    if !out.is_empty() && !out.ends_with('(') {
                        out.push('\n');
                    }
                    out.push_str(&indent(depth));
                    out.push_str(&kw(&cl));
                    if cl == "SELECT" {
                        out.push('\n');
                        out.push_str(&indent(depth + 1));
                    } else {
                        out.push(' ');
                    }
                    clause = cl;
                    i += n;
                    continue;
                }
                let u = w.to_uppercase();
                if (u == "AND" || u == "OR") && clause != "SELECT" {
                    out = out.trim_end().to_string();
                    out.push('\n');
                    out.push_str(&indent(depth + 1));
                    out.push_str(&kw(&u));
                    out.push(' ');
                } else {
                    if need_space(&out) {
                        out.push(' ');
                    }
                    out.push_str(&if is_keyword(w) && !w.contains('.') { kw(&u) } else { w.clone() });
                }
            }
            Tok::Str(s) => {
                if need_space(&out) {
                    out.push(' ');
                }
                out.push_str(s);
            }
            Tok::Comment(c) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(c);
                out.push('\n');
                out.push_str(&indent(depth));
            }
            Tok::Punct(c) => match c {
                ',' => {
                    out = out.trim_end().to_string();
                    out.push(',');
                    if clause == "SELECT" && depth == 0 {
                        out.push('\n');
                        out.push_str(&indent(1));
                    } else {
                        out.push(' ');
                    }
                }
                '(' => {
                    if need_space(&out) && !out.ends_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
                        out.push(' ');
                    }
                    out.push('(');
                    depth += 1;
                }
                ')' => {
                    out = out.trim_end().to_string();
                    out.push(')');
                    depth -= 1;
                }
                ';' => {
                    out = out.trim_end().to_string();
                    out.push_str(";\n\n");
                    depth = 0;
                    clause.clear();
                }
                '.' => {
                    out = out.trim_end().to_string();
                    out.push('.');
                }
                other => {
                    let after_op = out.ends_with(['>', '<', '!', '=', '|']);
                    if need_space(&out) && !after_op {
                        out.push(' ');
                    }
                    out.push(*other);
                    // keep two-char operators together
                    let next_is_op = matches!(toks.get(i + 1), Some(Tok::Punct(n)) if matches!(n, '=' | '>' | '<' | '|'));
                    if !next_is_op {
                        out.push(' ');
                    }
                }
            },
        }
        i += 1;
    }
    out.trim().to_string() + "\n"
}

/// Adds or removes "-- " on every line touched by the char range.
/// Returns the new text and the new cursor (char index).
pub fn toggle_comment(text: &str, a: usize, b: usize) -> (String, usize) {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut pos = 0;
    let mut first = None;
    let mut last = 0;
    for (i, l) in lines.iter().enumerate() {
        let len = l.chars().count();
        let (ls, le) = (pos, pos + len);
        if (a <= le && b >= ls) || (a == b && a >= ls && a <= le) {
            if first.is_none() {
                first = Some(i);
            }
            last = i;
        }
        pos = le + 1;
    }
    let first = first.unwrap_or(lines.len().saturating_sub(1));
    let all_commented = lines[first..=last].iter().filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with("--"));
    let mut delta: isize = 0;
    let new: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i < first || i > last || l.trim().is_empty() {
                return l.to_string();
            }
            if all_commented {
                let ws: String = l.chars().take_while(|c| c.is_whitespace()).collect();
                let rest = l.trim_start();
                let rest = rest.strip_prefix("-- ").or_else(|| rest.strip_prefix("--")).unwrap_or(rest);
                let s = format!("{ws}{rest}");
                delta -= (l.chars().count() - s.chars().count()) as isize;
                s
            } else {
                delta += 3;
                format!("-- {l}")
            }
        })
        .collect();
    let cursor = (b.max(a) as isize + delta).max(0) as usize;
    (new.join("\n"), cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> SchemaInfo {
        SchemaInfo {
            kind: DbKind::Sqlite,
            tables: vec![
                ("orders".into(), false, vec![("id".into(), "INTEGER".into()), ("customer_id".into(), "INTEGER".into()), ("total".into(), "REAL".into())]),
                ("customers".into(), false, vec![("id".into(), "INTEGER".into()), ("name".into(), "TEXT".into())]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn alias_columns() {
        let sql = "SELECT o.to FROM orders o";
        let (_, s) = suggest(sql, 11, &schema(), false);
        assert_eq!(s[0].label, "total");
    }

    #[test]
    fn tables_after_from() {
        let sql = "select * from cu";
        let (_, s) = suggest(sql, sql.len(), &schema(), false);
        assert_eq!(s[0].label, "customers");
    }

    #[test]
    fn keyword_casing() {
        let (_, s) = suggest("sel", 3, &schema(), false);
        assert!(s.iter().any(|x| x.insert == "select"));
    }

    #[test]
    fn values_after_operator() {
        let mut sc = schema();
        sc.values.insert("orders.total".into(), vec![ValueHint { text: "10.5".into(), count: Some(3), used: 0 }]);
        sc.tables.push(("customers2".into(), false, vec![("status".into(), "TEXT".into())]));
        sc.values.insert("customers2.status".into(), vec![ValueHint { text: "paid".into(), count: Some(9), used: 0 }, ValueHint { text: "it's".into(), count: Some(1), used: 0 }]);
        let sql = "SELECT * FROM customers2 c WHERE c.status = '";
        let vt = value_target(sql, sql.chars().count(), &sc).unwrap();
        assert_eq!(vt.key, "customers2.status");
        assert!(vt.quoted);
        let (start, s) = suggest(sql, sql.chars().count(), &sc, false);
        assert_eq!(start, sql.chars().count() - 1);
        assert_eq!(s[0].insert, "'paid'");
        assert_eq!(s[1].insert, "'it''s'");
        let sql = "select * from orders o where o.total >= ";
        let (_, s) = suggest(sql, sql.len(), &sc, false);
        assert_eq!(s[0].insert, "10.5");
        let sql = "select * from customers2 where status in ('paid', ";
        assert!(value_target(sql, sql.len(), &sc).is_some());
        let sql = "select * from customers2 where status like 'pa";
        assert_eq!(value_target(sql, sql.len(), &sc).unwrap().partial, "pa");
        assert!(value_target("select status from customers2", 13, &sc).is_none());
    }

    #[test]
    fn sqlite_affinity_rules() {
        use Affinity::*;
        for (t, a) in [
            ("INTEGER", Integer),
            ("BIGINT", Integer),
            ("FLOATING POINT", Integer),
            ("VARCHAR(20)", Text),
            ("NCHAR", Text),
            ("TEXT", Text),
            ("BLOB", Blob),
            ("", Blob),
            ("REAL", Real),
            ("DOUBLE PRECISION", Real),
            ("FLOAT", Real),
            ("NUMERIC", Numeric),
            ("DECIMAL(10,5)", Numeric),
            ("JSON", Numeric),
            ("CHARINT", Integer),
        ] {
            assert_eq!(sqlite_affinity(t), a, "{t}");
        }
        // checked against SQLite itself: typeof(CAST('12' AS <type>))
        let c = rusqlite::Connection::open_in_memory().unwrap();
        for (t, expect) in [("FLOATING POINT", "integer"), ("VARCHAR", "text"), ("DOUBLE", "real"), ("JSON", "integer"), ("BLOB", "blob")] {
            let got: String = c.query_row(&format!("SELECT typeof(CAST('12' AS {t}))"), [], |r| r.get(0)).unwrap();
            assert_eq!(got, expect, "{t}");
        }
        assert_eq!(value_literal(DbKind::Sqlite, "12", "VARCHAR"), "'12'");
        assert_eq!(value_literal(DbKind::Sqlite, "12", "FLOATING POINT"), "12");
    }

    #[test]
    fn learns_from_queries() {
        let sc = schema();
        let mut l = Learned::default();
        l.learn("SELECT o.total AS revenue FROM orders o WHERE o.customer_id = 7 AND o.id IN (1, 2)", &["revenue".into()], &sc);
        assert!(l.words.contains_key("revenue"));
        assert_eq!(l.values.get("orders.customer_id").unwrap()[0].0, "7");
        assert_eq!(l.values.get("orders.id").unwrap().len(), 2);
        let mut sc2 = sc.clone();
        sc2.learned = l.words.iter().map(|(a, b)| (a.clone(), *b)).collect();
        let (_, s) = suggest("select rev", 10, &sc2, false);
        assert_eq!(s[0].label, "revenue");
        assert_eq!(s[0].kind, SugKind::Learned);
    }

    #[test]
    fn keywords_match_in_any_case_and_get_quoted() {
        assert!(is_sql_keyword("select") && is_sql_keyword("SELECT") && is_sql_keyword("Order"));
        assert!(!is_sql_keyword("orders") && !is_sql_keyword("a"));
        assert_eq!(SQLITE_KEYWORDS.len(), 147);
        assert_eq!(ident(DbKind::Sqlite, "order"), "\"order\"");
        assert_eq!(ident(DbKind::Mysql, "Group"), "`Group`");
        assert_eq!(ident(DbKind::Sqlite, "orders"), "orders");
        assert_eq!(ident(DbKind::Sqlite, "2024_sales"), "\"2024_sales\"");
        // the editor colours them the same in any case, including words only SQLite lists
        for w in ["select", "SELECT", "Select", "window", "OVER", "partition"] {
            assert!(crate::widgets::is_sql_keyword(&w.to_ascii_uppercase()), "{w}");
        }
    }

    #[test]
    fn lower_and_upper_case_behave_the_same() {
        let sc = schema();
        for (typed, want) in [("sel", "select"), ("SEL", "SELECT"), ("fr", "from"), ("FR", "FROM")] {
            let (_, s) = suggest(typed, typed.len(), &sc, false);
            assert_eq!(s[0].insert, want, "{typed}");
        }
        for sql in ["select * from cu", "SELECT * FROM CU", "Select * From Cu"] {
            let (_, s) = suggest(sql, sql.len(), &sc, false);
            assert_eq!(s[0].label, "customers", "{sql}");
        }
        for sql in ["select o.to from orders o", "SELECT O.TO FROM ORDERS O"] {
            let (_, s) = suggest(sql, 11, &sc, false);
            assert_eq!(s[0].label, "total", "{sql}");
        }
        let lower = format_sql("select a from t where x = 1 and y = 2");
        assert!(lower.starts_with("select\n") && lower.contains("\nfrom t") && lower.contains("and y"), "{lower}");
        let upper = format_sql("SELECT a FROM t WHERE x = 1");
        assert!(upper.starts_with("SELECT\n") && upper.contains("\nFROM t"), "{upper}");
    }

    #[test]
    fn formats() {
        let f = format_sql("select a, b from t where x = 1 and y >= 2 order by a");
        assert!(f.starts_with("select\n  a,\n  b\nfrom t\nwhere x = 1\n  and y >= 2\norder by a"), "{f}");
    }

    #[test]
    fn comments() {
        let (t, _) = toggle_comment("a\nb", 0, 0);
        assert_eq!(t, "-- a\nb");
        let (t, _) = toggle_comment(&t, 0, 0);
        assert_eq!(t, "a\nb");
    }
}
