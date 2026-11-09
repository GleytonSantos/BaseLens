//! Reusable pieces: SQL colouring, the data grid, the JSON tree, charts and
//! the ER diagram.

use crate::db::{Cell, ColumnInfo, ForeignKey, QueryResult};
use crate::theme::{chart_colors, Palette};
use eframe::egui::{self, text::LayoutJob, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextFormat, Vec2};
use std::collections::{HashMap, HashSet};

// Grid numbers from MECHANICS.md
pub const DEFAULT_COL_W: f32 = 125.0;
pub const MIN_COL_W: f32 = 100.0;
pub const MAX_INITIAL_COL_W: f32 = 500.0;
pub const MAX_COL_W: f32 = 1000.0;
pub const ROW_H: f32 = 26.0;

const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL", "AS", "ON", "JOIN", "LEFT", "RIGHT", "INNER",
    "OUTER", "FULL", "CROSS", "GROUP", "BY", "ORDER", "HAVING", "LIMIT", "OFFSET", "INSERT", "INTO", "VALUES", "UPDATE",
    "SET", "DELETE", "CREATE", "TABLE", "VIEW", "INDEX", "DROP", "ALTER", "ADD", "COLUMN", "PRIMARY", "KEY", "FOREIGN",
    "REFERENCES", "DISTINCT", "UNION", "ALL", "CASE", "WHEN", "THEN", "ELSE", "END", "WITH", "LIKE", "ILIKE", "BETWEEN",
    "EXISTS", "ASC", "DESC", "COUNT", "SUM", "AVG", "MIN", "MAX", "DEFAULT", "BEGIN", "COMMIT", "ROLLBACK", "TRUE",
    "FALSE", "RETURNING", "EXPLAIN", "PRAGMA", "SHOW", "USE", "CAST", "COALESCE", "ROUND", "IF", "REPLACE", "TRUNCATE",
    "UNIQUE", "CHECK", "CONSTRAINT", "INTEGER", "TEXT", "REAL", "VARCHAR", "BOOLEAN", "JSON", "DATE", "TIMESTAMP",
];

/// Editor keywords plus the original's 147 SQLite keywords (matched in any
/// case with the `& 0xdf` fold from 0x6903ab0).
pub fn is_sql_keyword(upper: &str) -> bool {
    KEYWORDS.contains(&upper) || crate::assist::is_sql_keyword(upper)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlTok {
    Space,
    Comment,
    Str,
    QuotedId,
    Number,
    Blob,
    Param,
    Word,
    Keyword,
    Other,
}

/// Character classes from the original's tokenizer table (sqlite3CtypeMap at
/// 0x20fd210): 0x01 space, 0x02 letter, 0x04 digit, 0x08 hex digit, 0x40 the
/// extra identifier bytes '$', '_' and every byte >= 0x80. An identifier byte
/// is `ctype & 0x46` (read in 0x685a620).
fn ct_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0c | b'\r')
}
fn ct_id(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}
fn ct_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// Splits SQL the way the original's tokenizer (sqlite3GetToken, 0x685a620)
/// does, for colouring: `--` to end of line and `/* */` comments, '…' strings
/// and "…"/`…`/[…] names where a doubled quote stays inside, x'…' blobs,
/// numbers with 0x hex, '.', e±exponent and '_' separators, ?NNN :name @name
/// #name $name parameters, and keywords matched in any case.
pub fn sql_tokens(text: &str) -> Vec<(std::ops::Range<usize>, SqlTok)> {
    let z = text.as_bytes();
    let n = z.len();
    let at = |k: usize| if k < n { z[k] } else { 0 };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let s = i;
        let c = z[i];
        let kind;
        if ct_space(c) {
            while i < n && ct_space(z[i]) {
                i += 1;
            }
            kind = SqlTok::Space;
        } else if c == b'-' && at(i + 1) == b'-' {
            while i < n && z[i] != b'\n' {
                i += 1;
            }
            kind = SqlTok::Comment;
        } else if c == b'/' && at(i + 1) == b'*' {
            i += 2;
            while i < n && !(z[i] == b'*' && at(i + 1) == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            kind = SqlTok::Comment;
        } else if (c == b'x' || c == b'X') && at(i + 1) == b'\'' {
            i += 2;
            while i < n && z[i] != b'\'' {
                i += 1;
            }
            i = (i + 1).min(n);
            kind = SqlTok::Blob;
        } else if c == b'\'' || c == b'"' || c == b'`' {
            i += 1;
            while i < n {
                if z[i] == c {
                    if at(i + 1) == c {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            kind = if c == b'\'' { SqlTok::Str } else { SqlTok::QuotedId };
        } else if c == b'[' {
            while i < n && z[i] != b']' {
                i += 1;
            }
            i = (i + 1).min(n);
            kind = SqlTok::QuotedId;
        } else if c.is_ascii_digit() || (c == b'.' && at(i + 1).is_ascii_digit()) {
            if c == b'0' && (at(i + 1) | 0x20) == b'x' && ct_hex(at(i + 2)) {
                i += 2;
                while i < n && (ct_hex(z[i]) || z[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < n && (z[i].is_ascii_digit() || z[i] == b'_') {
                    i += 1;
                }
                if at(i) == b'.' {
                    i += 1;
                    while i < n && (z[i].is_ascii_digit() || z[i] == b'_') {
                        i += 1;
                    }
                }
                let e = at(i) | 0x20 == b'e';
                if e && (at(i + 1).is_ascii_digit() || (matches!(at(i + 1), b'+' | b'-') && at(i + 2).is_ascii_digit())) {
                    i += 2;
                    while i < n && (z[i].is_ascii_digit() || z[i] == b'_') {
                        i += 1;
                    }
                }
            }
            kind = SqlTok::Number;
        } else if c == b'?' {
            i += 1;
            while i < n && z[i].is_ascii_digit() {
                i += 1;
            }
            kind = SqlTok::Param;
        } else if matches!(c, b':' | b'@' | b'#') && ct_id(at(i + 1)) && !(c == b':' && i > 0 && z[i - 1] == b':') {
            // (a Postgres cast `x::int` is not a :name parameter)
            i += 1;
            while i < n && ct_id(z[i]) {
                i += 1;
            }
            kind = SqlTok::Param;
        } else if c == b'$' && ct_id(at(i + 1)) {
            i += 1;
            while i < n && ct_id(z[i]) {
                i += 1;
            }
            kind = SqlTok::Param;
        } else if ct_id(c) {
            while i < n && ct_id(z[i]) {
                i += 1;
            }
            let w = &text[s..i];
            kind = if is_sql_keyword(&w.to_ascii_uppercase()) { SqlTok::Keyword } else { SqlTok::Word };
        } else {
            i += 1;
            kind = SqlTok::Other;
        }
        // never split a UTF-8 character
        while i < n && !text.is_char_boundary(i) {
            i += 1;
        }
        out.push((s..i, kind));
    }
    out
}

pub fn sql_job(text: &str, p: &Palette, font: FontId, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let fmt = |c: Color32| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
    for (r, k) in sql_tokens(text) {
        let color = match k {
            SqlTok::Comment => p.text_lighter,
            SqlTok::Str | SqlTok::Blob => p.success,
            SqlTok::QuotedId | SqlTok::Param => p.secondary,
            SqlTok::Number => p.pink,
            SqlTok::Keyword => p.primary,
            SqlTok::Word => p.text_dark,
            SqlTok::Space | SqlTok::Other => p.text,
        };
        job.append(&text[r], 0.0, fmt(color));
    }
    job
}

#[cfg(test)]
mod sql_token_tests {
    use super::*;

    fn kinds(s: &str) -> Vec<(String, SqlTok)> {
        sql_tokens(s).into_iter().filter(|(_, k)| *k != SqlTok::Space).map(|(r, k)| (s[r].to_string(), k)).collect()
    }

    #[test]
    fn tokens_follow_the_original_tokenizer() {
        let t = kinds("select 'it''s', \"a\"\"b\", [x y], 0x1F, 1.5e-3, 1_000, x'ab', ?1, :n, @m, $v /* c */ -- end");
        let want = [
            ("select", SqlTok::Keyword),
            ("'it''s'", SqlTok::Str),
            ("\"a\"\"b\"", SqlTok::QuotedId),
            ("[x y]", SqlTok::QuotedId),
            ("0x1F", SqlTok::Number),
            ("1.5e-3", SqlTok::Number),
            ("1_000", SqlTok::Number),
            ("x'ab'", SqlTok::Blob),
            ("?1", SqlTok::Param),
            (":n", SqlTok::Param),
            ("@m", SqlTok::Param),
            ("$v", SqlTok::Param),
            ("/* c */", SqlTok::Comment),
            ("-- end", SqlTok::Comment),
        ];
        let got: Vec<_> = t.iter().filter(|(w, _)| w != ",").cloned().collect();
        assert_eq!(got, want.iter().map(|(a, b)| (a.to_string(), *b)).collect::<Vec<_>>());
        assert_eq!(kinds("SELECT ação FROM t")[1], ("ação".to_string(), SqlTok::Word));
        assert_eq!(kinds("Select")[0].1, SqlTok::Keyword);
        assert_eq!(kinds("a::int")[3], ("int".to_string(), SqlTok::Word));
        // the whole text is covered, nothing lost
        let s = "select ç /* open";
        assert_eq!(sql_tokens(s).iter().map(|(r, _)| &s[r.clone()]).collect::<String>(), s);
    }
}

// ------------------------------------------------------------------ grid

#[derive(Default, Clone)]
pub struct GridState {
    pub selected: Option<(usize, usize)>,
    pub editing: Option<(usize, usize, String)>,
    pub focus_edit: bool,
    /// column to point out (set by the Ask bot)
    pub highlight_col: Option<String>,
}

pub struct GridEdits<'a> {
    pub edits: &'a mut HashMap<(usize, usize), Cell>,
    pub deletes: &'a mut HashSet<usize>,
    pub editable: bool,
}

#[derive(Default)]
pub struct GridOut {
    pub sort_clicked: Option<usize>,
    /// explicit sort from the header menu: (column, ascending)
    pub sort_set: Option<(usize, bool)>,
    pub open_json: bool,
    /// quick filter picked from a cell or header: (column name, operator, value)
    pub filter: Option<(String, crate::filter::Op, String)>,
    /// Argon2 tool asked from a cell: (hash to check, cell to fill)
    pub argon2: Option<(Option<String>, Option<(usize, usize)>)>,
}

fn col_width(res: &QueryResult, c: usize) -> f32 {
    let mut chars = res.columns[c].chars().count() + 3;
    for row in res.rows.iter().take(50) {
        chars = chars.max(row[c].display().chars().count().min(80));
    }
    let w = chars as f32 * 7.2 + 16.0;
    if res.rows.is_empty() {
        DEFAULT_COL_W
    } else {
        w.clamp(MIN_COL_W, MAX_INITIAL_COL_W)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn grid(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    res: &QueryResult,
    st: &mut GridState,
    mut edits: Option<GridEdits>,
    p: &Palette,
    row_offset: usize,
    sort: Option<(usize, bool)>,
    pk_cols: &[usize],
) -> GridOut {
    use egui_extras::{Column, TableBuilder};
    let mut out = GridOut::default();
    if res.columns.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(format!("✔ Query ran. {} row(s) affected.", res.affected.unwrap_or(0))).color(p.success).size(15.0));
            ui.label(RichText::new(&res.sql).monospace().color(p.text_light));
        });
        return out;
    }
    let editable = edits.as_ref().map(|e| e.editable).unwrap_or(false);
    // wide results scroll sideways inside the card instead of stretching it
    let id_salt = egui::Id::new(&id);
    egui::ScrollArea::horizontal().id_salt(id_salt.with("hscroll")).auto_shrink([false, false]).show(ui, |ui| ui.push_id(id_salt, |ui| {
        let mut tb = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .max_scroll_height(f32::INFINITY)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(44.0));
        for c in 0..res.columns.len() {
            tb = tb.column(Column::initial(col_width(res, c)).at_least(40.0).at_most(MAX_COL_W).clip(true));
        }
        let head_bg = crate::theme::mix(p.panel_alt, p.text_dark, 0.045);
        let hl = st.highlight_col.clone();
        tb.header(30.0, |mut h| {
            h.col(|ui| {
                ui.painter().rect_filled(ui.max_rect(), 0.0, head_bg);
                ui.label(RichText::new("#").color(p.text_lighter).small());
            });
            for (c, name) in res.columns.iter().enumerate() {
                h.col(|ui| {
                    let rect = ui.max_rect();
                    let is_hl = hl.as_deref() == Some(name.as_str());
                    ui.painter().rect_filled(rect, 0.0, if is_hl { crate::theme::mix(head_bg, p.primary, 0.25) } else { head_bg });
                    if is_hl {
                        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.5_f32, p.primary));
                    }
                    let mut t = name.clone();
                    if pk_cols.contains(&c) {
                        t = format!("🔑 {t}");
                    }
                    if let Some((sc, asc)) = sort {
                        if sc == c {
                            t.push_str(if asc { "  ⬆" } else { "  ⬇" });
                        }
                    }
                    let r = ui.add(egui::Label::new(RichText::new(t).strong().color(if is_hl { p.primary } else { p.text_dark })).sense(Sense::click()).truncate());
                    if r.clicked() {
                        out.sort_clicked = Some(c);
                    }
                    let r = r.on_hover_text("Click to sort · right-click for more");
                    r.context_menu(|ui| {
                        if ui.button("⬆  Sort ascending").clicked() {
                            out.sort_set = Some((c, true));
                            ui.close_menu();
                        }
                        if ui.button("⬇  Sort descending").clicked() {
                            out.sort_set = Some((c, false));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("🔍  Filter this column…").clicked() {
                            out.filter = Some((name.clone(), crate::filter::Op::Contains, String::new()));
                            ui.close_menu();
                        }
                        if ui.button("Only rows with a value").clicked() {
                            out.filter = Some((name.clone(), crate::filter::Op::NotNull, String::new()));
                            ui.close_menu();
                        }
                        if ui.button("Copy column name").clicked() {
                            ui.ctx().copy_text(name.clone());
                            ui.close_menu();
                        }
                    });
                });
            }
        })
        .body(|body| {
            body.rows(ROW_H, res.rows.len(), |mut row| {
                let r = row.index();
                let deleted = edits.as_ref().map(|e| e.deletes.contains(&r)).unwrap_or(false);
                let row_sel = st.selected.map(|s| s.0 == r).unwrap_or(false);
                row.col(|ui| {
                    if deleted {
                        ui.painter().rect_filled(ui.max_rect(), 0.0, p.row_delete);
                    } else if row_sel {
                        ui.painter().rect_filled(ui.max_rect(), 0.0, p.selection);
                    }
                    let resp = ui.add(egui::Label::new(RichText::new((row_offset + r + 1).to_string()).color(p.text_lighter).small()).sense(Sense::click()));
                    if resp.clicked() {
                        st.selected = Some((r, 0));
                    }
                });
                for c in 0..res.columns.len() {
                    row.col(|ui| {
                        let rect = ui.max_rect();
                        let edited = edits.as_ref().and_then(|e| e.edits.get(&(r, c)).cloned());
                        let cell = edited.clone().unwrap_or_else(|| res.rows[r][c].clone());
                        let is_sel = st.selected == Some((r, c));
                        if deleted {
                            ui.painter().rect_filled(rect, 0.0, p.row_delete);
                        } else if edited.is_some() {
                            ui.painter().rect_filled(rect, 0.0, p.cell_edited);
                        } else if row_sel {
                            ui.painter().rect_filled(rect, 0.0, p.selection.gamma_multiply(0.6));
                        }
                        if hl.as_deref() == Some(res.columns[c].as_str()) && !deleted && edited.is_none() {
                            ui.painter().rect_filled(rect, 0.0, p.primary.gamma_multiply(0.07));
                        }
                        if is_sel {
                            ui.painter().rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(1.5_f32, p.primary), egui::StrokeKind::Inside);
                        }
                        // in-place editor
                        if let Some((er, ec, buf)) = st.editing.as_mut() {
                            if *er == r && *ec == c {
                                let te = ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY).frame(false));
                                if st.focus_edit {
                                    te.request_focus();
                                    st.focus_edit = false;
                                }
                                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                if esc {
                                    st.editing = None;
                                } else if te.lost_focus() || enter {
                                    let new = Cell::Text(buf.clone());
                                    if let Some(e) = edits.as_mut() {
                                        if res.rows[r][c].display() != *buf {
                                            e.edits.insert((r, c), new);
                                        }
                                    }
                                    st.editing = None;
                                }
                                return;
                            }
                        }
                        let text = match &cell {
                            Cell::Null => RichText::new("(NULL)").italics().color(p.text_lighter),
                            Cell::Int(_) | Cell::Float(_) => RichText::new(cell.display()).color(p.text_dark),
                            Cell::Text(s) if cell.json().is_some() => RichText::new(s.replace('\n', " ")).color(p.info),
                            Cell::Text(s) => RichText::new(s.replace('\n', "⏎")).color(p.text_dark),
                            Cell::Bytes(_) => RichText::new(cell.display()).color(p.purple).monospace(),
                        };
                        let numeric = matches!(cell, Cell::Int(_) | Cell::Float(_)) || (matches!(cell, Cell::Text(_)) && cell.as_f64().is_some() && cell.json().is_none());
                        let resp = if cell.is_null() {
                            // NULL as a small pill
                            egui::Frame::new()
                                .fill(crate::theme::mix(p.panel_alt, p.text_dark, 0.06))
                                .corner_radius(5)
                                .inner_margin(egui::Margin::symmetric(5, 0))
                                .show(ui, |ui| ui.add(egui::Label::new(RichText::new("NULL").small().color(p.text_lighter)).sense(Sense::click())))
                                .inner
                        } else if numeric {
                            let t = match &cell {
                                Cell::Text(s) => RichText::new(s.clone()).color(p.text_dark),
                                _ => text,
                            };
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::Label::new(t.monospace()).truncate().sense(Sense::click()))).inner
                        } else {
                            ui.add(egui::Label::new(text).truncate().sense(Sense::click()))
                        };
                        let resp = ui.interact(rect, resp.id.with("cell"), Sense::click()).union(resp);
                        if resp.clicked() {
                            st.selected = Some((r, c));
                        }
                        // MECHANICS: editTrigger = doubleclick
                        if resp.double_clicked() {
                            st.selected = Some((r, c));
                            if editable && !deleted {
                                let start = if cell.is_null() { String::new() } else { cell.display() };
                                st.editing = Some((r, c, start));
                                st.focus_edit = true;
                            } else if cell.json().is_some() {
                                out.open_json = true;
                            }
                        }
                        resp.context_menu(|ui| {
                            st.selected = Some((r, c));
                            if ui.button("Copy value").clicked() {
                                ui.ctx().copy_text(cell.display());
                                ui.close_menu();
                            }
                            if ui.button("Copy row as JSON").clicked() {
                                ui.ctx().copy_text(serde_json::to_string_pretty(&row_json(res, r)).unwrap_or_default());
                                ui.close_menu();
                            }
                            if ui.button("Copy as INSERT").clicked() {
                                let vals: Vec<String> = res.rows[r].iter().map(|v| v.sql_literal()).collect();
                                ui.ctx().copy_text(format!("INSERT INTO t ({}) VALUES ({});", res.columns.join(", "), vals.join(", ")));
                                ui.close_menu();
                            }
                            if ui.button("Show in JSON inspector").clicked() {
                                out.open_json = true;
                                ui.close_menu();
                            }
                            if crate::argon2tool::looks_like_hash(&cell.display()) && ui.button("🔑  Check a password against this hash…").clicked() {
                                out.argon2 = Some((Some(cell.display()), None));
                                ui.close_menu();
                            }
                            if editable && !deleted && ui.button("🔑  Put a new password hash here…").clicked() {
                                out.argon2 = Some((None, Some((r, c))));
                                ui.close_menu();
                            }
                            ui.separator();
                            let colname = res.columns[c].clone();
                            if cell.is_null() {
                                if ui.button("🔍  Only rows where this is empty").clicked() {
                                    out.filter = Some((colname.clone(), crate::filter::Op::IsNull, String::new()));
                                    ui.close_menu();
                                }
                            } else {
                                let v = cell.display();
                                let short: String = v.chars().take(24).collect();
                                if ui.button(format!("🔍  Only rows where {colname} = {short}")).clicked() {
                                    out.filter = Some((colname.clone(), crate::filter::Op::Equals, v.clone()));
                                    ui.close_menu();
                                }
                                if ui.button(format!("🚫  Hide rows where {colname} = {short}")).clicked() {
                                    out.filter = Some((colname.clone(), crate::filter::Op::NotEquals, v.clone()));
                                    ui.close_menu();
                                }
                                if cell.as_f64().is_some() {
                                    if ui.button(format!("Only rows where {colname} > {short}")).clicked() {
                                        out.filter = Some((colname.clone(), crate::filter::Op::Greater, v.clone()));
                                        ui.close_menu();
                                    }
                                    if ui.button(format!("Only rows where {colname} < {short}")).clicked() {
                                        out.filter = Some((colname.clone(), crate::filter::Op::Less, v.clone()));
                                        ui.close_menu();
                                    }
                                }
                            }
                            if let Some(e) = edits.as_mut() {
                                if e.editable {
                                    ui.separator();
                                    if ui.button("Edit cell").clicked() {
                                        st.editing = Some((r, c, if cell.is_null() { String::new() } else { cell.display() }));
                                        st.focus_edit = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Set NULL").clicked() {
                                        e.edits.insert((r, c), Cell::Null);
                                        ui.close_menu();
                                    }
                                    if ui.button("Revert cell").clicked() {
                                        e.edits.remove(&(r, c));
                                        ui.close_menu();
                                    }
                                    let del = e.deletes.contains(&r);
                                    if ui.button(if del { "Undo delete row" } else { "Delete row" }).clicked() {
                                        if del {
                                            e.deletes.remove(&r);
                                        } else {
                                            e.deletes.insert(r);
                                        }
                                        ui.close_menu();
                                    }
                                }
                            }
                        });
                    });
                }
            });
        });
    }));
    out
}

pub fn row_json(res: &QueryResult, r: usize) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    if let Some(row) = res.rows.get(r) {
        for (c, name) in res.columns.iter().enumerate() {
            m.insert(name.clone(), row[c].to_json());
        }
    }
    serde_json::Value::Object(m)
}

pub fn result_json(res: &QueryResult, limit: usize) -> serde_json::Value {
    serde_json::Value::Array((0..res.rows.len().min(limit)).map(|r| row_json(res, r)).collect())
}

// ------------------------------------------------------------------ JSON tree

pub fn json_tree(ui: &mut egui::Ui, key: &str, v: &serde_json::Value, path: &str, depth: usize, p: &Palette, filter: &str) {
    use serde_json::Value;
    let key_txt = RichText::new(key).color(p.secondary).monospace();
    match v {
        Value::Object(map) => {
            egui::CollapsingHeader::new(RichText::new(format!("{key}  {{{}}}", map.len())).color(p.secondary).monospace())
                .id_salt(path)
                .default_open(depth < 3)
                .show(ui, |ui| {
                    for (k, val) in map {
                        if filter.is_empty() || contains_text(k, val, filter) {
                            json_tree(ui, k, val, &format!("{path}.{k}"), depth + 1, p, filter);
                        }
                    }
                });
        }
        Value::Array(a) => {
            egui::CollapsingHeader::new(RichText::new(format!("{key}  [{}]", a.len())).color(p.secondary).monospace())
                .id_salt(path)
                .default_open(depth < 3)
                .show(ui, |ui| {
                    for (i, val) in a.iter().enumerate() {
                        let k = i.to_string();
                        if filter.is_empty() || contains_text(&k, val, filter) {
                            json_tree(ui, &k, val, &format!("{path}[{i}]"), depth + 1, p, filter);
                        }
                    }
                });
        }
        leaf => {
            let (txt, col) = match leaf {
                Value::String(s) => (format!("\"{s}\""), p.success),
                Value::Number(n) => (n.to_string(), p.pink),
                Value::Bool(b) => (b.to_string(), p.purple),
                _ => ("null".to_string(), p.text_lighter),
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(14.0);
                ui.label(key_txt);
                ui.label(RichText::new(":").color(p.text_lighter));
                let r = ui.add(egui::Label::new(RichText::new(txt).color(col).monospace()).sense(Sense::click()));
                r.context_menu(|ui| {
                    if ui.button("Copy value").clicked() {
                        ui.ctx().copy_text(match leaf {
                            Value::String(s) => s.clone(),
                            o => o.to_string(),
                        });
                        ui.close_menu();
                    }
                    if ui.button("Copy path").clicked() {
                        ui.ctx().copy_text(path.trim_start_matches('$').trim_start_matches('.').to_string());
                        ui.close_menu();
                    }
                });
            });
        }
    }
}

fn contains_text(k: &str, v: &serde_json::Value, f: &str) -> bool {
    let f = f.to_lowercase();
    k.to_lowercase().contains(&f) || v.to_string().to_lowercase().contains(&f)
}

// ------------------------------------------------------------------ charts

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ChartKind {
    #[default]
    Bar,
    Line,
    Pie,
}

/// How rows that share a label are combined.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Agg {
    #[default]
    Sum,
    Avg,
    Count,
    Max,
    Min,
    /// one bar per row
    None,
}

impl Agg {
    pub const ALL: [Agg; 6] = [Agg::Sum, Agg::Avg, Agg::Count, Agg::Max, Agg::Min, Agg::None];
    pub fn label(self) -> &'static str {
        match self {
            Agg::Sum => "Sum",
            Agg::Avg => "Average",
            Agg::Count => "Count rows",
            Agg::Max => "Highest",
            Agg::Min => "Lowest",
            Agg::None => "Each row",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ChartSort {
    /// biggest first
    #[default]
    ValueDesc,
    /// A→Z / oldest first (good for dates)
    LabelAsc,
    /// as the query returned it
    AsIs,
}

impl ChartSort {
    pub const ALL: [ChartSort; 3] = [ChartSort::ValueDesc, ChartSort::LabelAsc, ChartSort::AsIs];
    pub fn label(self) -> &'static str {
        match self {
            ChartSort::ValueDesc => "Biggest first",
            ChartSort::LabelAsc => "By label (A→Z, dates in order)",
            ChartSort::AsIs => "As returned",
        }
    }
}

pub const CHART_TOP_DEFAULT: usize = 20;
pub const CHART_TOP_MAX: usize = 200;
pub const CHART_LABEL_CHARS: usize = 14;

#[derive(Clone, Default, Debug)]
pub struct ChartCfg {
    pub kind: ChartKind,
    pub x: usize,
    /// None = count rows per label
    pub y: Option<usize>,
    pub agg: Agg,
    pub sort: ChartSort,
    pub top: usize,
    pub init: bool,
    /// bumps when the data or settings change so the plot zoom resets
    pub version: u64,
    /// the columns the settings were made for
    pub cols_sig: u64,
}

fn hash_of<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// Columns whose non-empty values (first 50 rows) are all numbers.
pub fn numeric_columns(res: &QueryResult) -> Vec<usize> {
    (0..res.columns.len())
        .filter(|&c| {
            let mut seen = 0;
            for r in res.rows.iter().take(50) {
                if r[c].is_null() {
                    continue;
                }
                if r[c].as_f64().is_none() {
                    return false;
                }
                seen += 1;
            }
            seen > 0
        })
        .collect()
}

fn looks_like_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 7 && b[..4].iter().all(|c| c.is_ascii_digit()) && b[4] == b'-' && b[5].is_ascii_digit() && b[6].is_ascii_digit()
}

fn id_like(name: &str) -> bool {
    let n = name.to_lowercase();
    n == "id" || n.ends_with("_id") || n.ends_with("id") && n.len() <= 3
}

/// Sensible first settings for a result.
pub fn auto_config(res: &QueryResult) -> ChartCfg {
    let nums = numeric_columns(res);
    let ncols = res.columns.len();
    let x = (0..ncols).find(|c| !nums.contains(c)).unwrap_or(0);
    // value: the last numeric column that isn't the label or an id
    let y = nums.iter().rev().copied().find(|&c| c != x && !id_like(&res.columns[c])).or_else(|| nums.iter().rev().copied().find(|&c| c != x));
    let labels: Vec<String> = res.rows.iter().map(|r| r[x].display()).collect();
    let unique: std::collections::HashSet<&String> = labels.iter().collect();
    let repeats = unique.len() < labels.len();
    let dates = labels.iter().take(20).filter(|l| looks_like_date(l)).count() * 2 > labels.len().min(20);
    let agg = if y.is_none() {
        Agg::Count
    } else if repeats {
        Agg::Sum
    } else {
        Agg::None
    };
    ChartCfg {
        kind: if dates { ChartKind::Line } else { ChartKind::Bar },
        x,
        y,
        agg,
        sort: if dates { ChartSort::LabelAsc } else { ChartSort::ValueDesc },
        top: CHART_TOP_DEFAULT,
        init: true,
        version: 1,
        cols_sig: 0,
    }
}

/// Running sum with the same rules as the SQLite built into the original
/// (`sumStep` at 0x693d4b0, `sumFinalize` at 0x693da70):
/// - while every value is an integer the sum is kept exactly as an integer;
/// - after the first non-integer (or an integer overflow) it switches to a
///   Kahan-Babuska-Neumaier compensated sum: `s = sum + x`, and the lost
///   part is kept in `err`: `err += (sum - s) + x` when `|sum| > |x|`,
///   else `err += (x - s) + sum`; the result is `sum + err`;
/// - integers of magnitude >= 2^52 (4503599627370496) are added in two
///   parts, `x % 16384` and the rest, so no bits are lost.
#[derive(Clone, Copy, Default, Debug)]
pub struct SqlSum {
    isum: i64,
    sum: f64,
    err: f64,
    approx: bool,
}

pub const KBN_SPLIT_LIMIT: i64 = 4503599627370496; // 2^52
pub const KBN_SPLIT_MOD: i64 = 16384;

impl SqlSum {
    fn kbn(&mut self, x: f64) {
        let s = self.sum + x;
        if self.sum.abs() > x.abs() {
            self.err += (self.sum - s) + x;
        } else {
            self.err += (x - s) + self.sum;
        }
        self.sum = s;
    }
    fn kbn_int(&mut self, i: i64) {
        if i <= -KBN_SPLIT_LIMIT || i >= KBN_SPLIT_LIMIT {
            let small = i % KBN_SPLIT_MOD;
            self.kbn((i - small) as f64);
            self.kbn(small as f64);
        } else {
            self.kbn(i as f64);
        }
    }
    pub fn add_int(&mut self, i: i64) {
        if self.approx {
            self.kbn_int(i);
            return;
        }
        match self.isum.checked_add(i) {
            Some(v) => self.isum = v,
            None => {
                // overflow: continue in floating point
                let prev = self.isum;
                self.approx = true;
                self.sum = 0.0;
                self.err = 0.0;
                self.kbn_int(prev);
                self.kbn_int(i);
            }
        }
    }
    pub fn add_float(&mut self, x: f64) {
        if !self.approx {
            let prev = self.isum;
            self.approx = true;
            self.sum = 0.0;
            self.err = 0.0;
            self.kbn_int(prev);
        }
        self.kbn(x);
    }
    pub fn value(&self) -> f64 {
        if self.approx {
            self.sum + self.err
        } else {
            self.isum as f64
        }
    }
}

/// The (label, value) pairs to draw, plus how many rows went into them.
pub fn chart_series(res: &QueryResult, cfg: &ChartCfg) -> (Vec<(String, f64)>, usize) {
    let mut used = 0;
    let mut out: Vec<(String, f64)> = Vec::new();
    let value_of = |r: &Vec<Cell>| -> Option<f64> {
        match cfg.y {
            Some(y) => r.get(y).and_then(|c| c.as_f64()),
            None => Some(1.0),
        }
    };
    if cfg.agg == Agg::None {
        for r in &res.rows {
            if let (Some(l), Some(v)) = (r.get(cfg.x), value_of(r)) {
                out.push((l.display(), v));
                used += 1;
            }
        }
    } else {
        // keep first-seen order for "as returned"
        let mut order: Vec<String> = Vec::new();
        let mut acc: HashMap<String, (SqlSum, usize, f64, f64)> = HashMap::new();
        for r in &res.rows {
            let Some(l) = r.get(cfg.x).map(|c| c.display()) else { continue };
            let v = if cfg.agg == Agg::Count { Some(1.0) } else { value_of(r) };
            let Some(v) = v else { continue };
            used += 1;
            let e = acc.entry(l.clone()).or_insert_with(|| {
                order.push(l.clone());
                (SqlSum::default(), 0, f64::INFINITY, f64::NEG_INFINITY)
            });
            // integers stay exact, like SQLite's sum()
            match cfg.y.and_then(|y| r.get(y)) {
                Some(Cell::Int(i)) => e.0.add_int(*i),
                _ => e.0.add_float(v),
            }
            e.1 += 1;
            e.2 = e.2.min(v);
            e.3 = e.3.max(v);
        }
        for l in order {
            let (sum, n, mn, mx) = acc[&l];
            let sum = sum.value();
            let v = match cfg.agg {
                Agg::Sum => sum,
                Agg::Avg => sum / n as f64,
                Agg::Count => n as f64,
                Agg::Max => mx,
                Agg::Min => mn,
                Agg::None => sum,
            };
            out.push((l, v));
        }
    }
    match cfg.sort {
        ChartSort::ValueDesc => out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)),
        ChartSort::LabelAsc => out.sort_by(|a, b| match (a.0.parse::<f64>(), b.0.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            _ => a.0.cmp(&b.0),
        }),
        ChartSort::AsIs => {}
    }
    // keep the chart readable: top N, the rest folded into "Other"
    let top = cfg.top.clamp(1, CHART_TOP_MAX);
    if out.len() > top {
        let rest: f64 = out[top..].iter().map(|x| x.1).sum();
        let n_rest = out.len() - top;
        out.truncate(top);
        if matches!(cfg.agg, Agg::Sum | Agg::Count) && cfg.kind != ChartKind::Line {
            out.push((format!("Other ({n_rest})"), rest));
        }
    }
    (out, used)
}

fn short_label(s: &str) -> String {
    if s.chars().count() > CHART_LABEL_CHARS {
        let t: String = s.chars().take(CHART_LABEL_CHARS - 1).collect();
        format!("{t}…")
    } else {
        s.to_string()
    }
}

pub fn chart(ui: &mut egui::Ui, id: &str, res: &QueryResult, cfg: &mut ChartCfg, p: &Palette) {
    if res.columns.is_empty() || res.rows.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Run a query that returns rows to draw a chart.").color(p.text_light)));
        return;
    }
    // keep the user's settings while the columns stay the same
    let sig = hash_of(&res.columns);
    if !cfg.init || cfg.cols_sig != sig || cfg.x >= res.columns.len() || cfg.y.map(|y| y >= res.columns.len()).unwrap_or(false) {
        let v = cfg.version;
        *cfg = auto_config(res);
        cfg.version = v + 1;
        cfg.cols_sig = sig;
    }
    let nums = numeric_columns(res);
    let before = (cfg.kind, cfg.x, cfg.y, cfg.agg, cfg.sort, cfg.top);
    // never draw wider than the card
    let full_w = ui.available_width();
    ui.set_max_width(full_w);
    // ---- settings: what to draw (row 1) and how (row 2)
    egui::Frame::new().fill(crate::theme::mix(p.panel_alt, p.text_dark, 0.04)).corner_radius(10).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(full_w - 20.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (k, label) in [(ChartKind::Bar, "📊 Bars"), (ChartKind::Line, "📈 Line"), (ChartKind::Pie, "🍩 Pie")] {
                ui.selectable_value(&mut cfg.kind, k, label);
            }
            ui.separator();
            ui.label(RichText::new("Label").color(p.text_light));
            egui::ComboBox::from_id_salt((id, "x")).selected_text(&res.columns[cfg.x]).width(130.0).show_ui(ui, |ui| {
                for (i, c) in res.columns.iter().enumerate() {
                    ui.selectable_value(&mut cfg.x, i, c);
                }
            });
            ui.label(RichText::new("Value").color(p.text_light));
            let ytext = cfg.y.map(|y| res.columns[y].clone()).unwrap_or_else(|| "Number of rows".into());
            egui::ComboBox::from_id_salt((id, "y")).selected_text(ytext).width(130.0).show_ui(ui, |ui| {
                if ui.selectable_label(cfg.y.is_none(), "Number of rows").clicked() {
                    cfg.y = None;
                    cfg.agg = Agg::Count;
                }
                for &i in &nums {
                    if ui.selectable_label(cfg.y == Some(i), &res.columns[i]).clicked() {
                        cfg.y = Some(i);
                        if cfg.agg == Agg::Count {
                            cfg.agg = Agg::Sum;
                        }
                    }
                }
                if nums.is_empty() {
                    ui.label(RichText::new("No number columns in this result").small().color(p.text_lighter));
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new("Combine").color(p.text_light));
            egui::ComboBox::from_id_salt((id, "agg")).selected_text(cfg.agg.label()).width(100.0).show_ui(ui, |ui| {
                for a in Agg::ALL {
                    if cfg.y.is_none() && !matches!(a, Agg::Count | Agg::None) {
                        continue;
                    }
                    ui.selectable_value(&mut cfg.agg, a, a.label());
                }
            });
            ui.label(RichText::new("Order").color(p.text_light));
            egui::ComboBox::from_id_salt((id, "sort")).selected_text(cfg.sort.label()).width(120.0).show_ui(ui, |ui| {
                for s in ChartSort::ALL {
                    ui.selectable_value(&mut cfg.sort, s, s.label());
                }
            });
            ui.label(RichText::new("Show").color(p.text_light));
            ui.add(egui::DragValue::new(&mut cfg.top).range(1..=CHART_TOP_MAX).suffix(" max"));
            if ui.button("↺ Auto").on_hover_text("Pick good settings for this result").clicked() {
                cfg.init = false;
            }
        });
    });
    if before != (cfg.kind, cfg.x, cfg.y, cfg.agg, cfg.sort, cfg.top) {
        cfg.version += 1;
    }
    let (series, used) = chart_series(res, cfg);
    let what = match (cfg.y, cfg.agg) {
        (None, _) | (_, Agg::Count) => "number of rows".to_string(),
        (Some(y), Agg::None) => res.columns[y].clone(),
        (Some(y), a) => format!("{} of {}", a.label().to_lowercase(), res.columns[y]),
    };
    ui.label(RichText::new(format!("{} {} by {} · from {used} of {} rows", series.len(), if cfg.kind == ChartKind::Pie { "slices" } else { "points" }, res.columns[cfg.x], res.rows.len())).small().color(p.text_lighter));
    if series.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Nothing to draw: the value column has no numbers.").color(p.text_light)));
        return;
    }
    let labels: Vec<String> = series.iter().map(|s| s.0.clone()).collect();
    let values: Vec<f64> = series.iter().map(|s| s.1).collect();
    let colors = chart_colors(p);
    match cfg.kind {
        ChartKind::Bar | ChartKind::Line => {
            use egui_plot::{Bar, BarChart, Line, Plot, PlotPoints, Points};
            let l2 = labels.clone();
            let l3 = labels.clone();
            let what2 = what.clone();
            // a new id per version: zoom/pan reset when the data or settings change
            let data_sig = hash_of(&series.iter().map(|s| (s.0.clone(), s.1.to_bits())).collect::<Vec<_>>());
            let plot_h = ui.available_height().max(120.0);
            let plot = Plot::new((id, "plot", cfg.version, data_sig))
                .width(full_w)
                .height(plot_h)
                .allow_scroll(false)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_boxed_zoom(false)
                .show_grid([false, true])
                .y_axis_label(what.clone())
                .include_y(0.0)
                .x_axis_formatter(move |m, _r| {
                    let i = m.value.round();
                    if (m.value - i).abs() < 1e-6 && i >= 0.0 {
                        l2.get(i as usize).map(|s| short_label(s)).unwrap_or_default()
                    } else {
                        String::new()
                    }
                })
                .label_formatter(move |_name, pt| {
                    let i = pt.x.round();
                    match l3.get(i.max(0.0) as usize) {
                        Some(l) if (pt.x - i).abs() < 0.5 => format!("{l}\n{what2}: {}", format_num(pt.y)),
                        _ => String::new(),
                    }
                });
            plot.show(ui, |pu| {
                if cfg.kind == ChartKind::Bar {
                    let bars: Vec<Bar> = values
                        .iter()
                        .enumerate()
                        .map(|(i, v)| Bar::new(i as f64, *v).width(0.7).fill(colors[i % colors.len()].gamma_multiply(0.9)).name(&labels[i]))
                        .collect();
                    pu.bar_chart(BarChart::new(bars));
                } else {
                    let pts: Vec<[f64; 2]> = values.iter().enumerate().map(|(i, v)| [i as f64, *v]).collect();
                    pu.line(Line::new(PlotPoints::from(pts.clone())).color(p.primary).width(2.5).fill(0.0));
                    pu.points(Points::new(PlotPoints::from(pts)).color(p.secondary).radius(4.0));
                }
            });
        }
        ChartKind::Pie => pie(ui, id, &labels, &values, p),
    }
}

fn pie(ui: &mut egui::Ui, id: &str, labels: &[String], values: &[f64], p: &Palette) {
    let colors = chart_colors(p);
    let total: f64 = values.iter().filter(|v| **v > 0.0).sum();
    if total <= 0.0 {
        ui.label("A pie needs positive values.");
        return;
    }
    let avail = ui.available_size();
    ui.horizontal(|ui| {
        let size = (avail.y - 10.0).min(avail.x * 0.55).max(120.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        let painter = ui.painter_at(rect);
        let c = rect.center();
        let radius = size * 0.45;
        let mut a0 = -std::f64::consts::FRAC_PI_2;
        let hover = resp.hover_pos();
        let mut hovered: Option<usize> = None;
        for (i, v) in values.iter().enumerate() {
            if *v <= 0.0 {
                continue;
            }
            let sweep = v / total * std::f64::consts::TAU;
            if let Some(h) = hover {
                let d = h - c;
                let r = d.length();
                let mut ang = (d.y as f64).atan2(d.x as f64);
                while ang < a0 {
                    ang += std::f64::consts::TAU;
                }
                if r <= radius && r >= radius * 0.55 && ang <= a0 + sweep {
                    hovered = Some(i);
                }
            }
            let steps = ((sweep / 0.05).ceil() as usize).max(1);
            let col = colors[i % colors.len()];
            let rad = if hovered == Some(i) { radius * 1.04 } else { radius };
            for s in 0..steps {
                let t0 = a0 + sweep * s as f64 / steps as f64;
                let t1 = a0 + sweep * (s + 1) as f64 / steps as f64;
                let pt = |t: f64| Pos2::new(c.x + rad * t.cos() as f32, c.y + rad * t.sin() as f32);
                painter.add(egui::Shape::convex_polygon(vec![c, pt(t0), pt(t1)], col, Stroke::NONE));
            }
            a0 += sweep;
        }
        painter.circle_filled(c, radius * 0.55, p.panel_alt);
        let center_text = match hovered {
            Some(i) => format!("{}\n{} ({:.1}%)", short_label(&labels[i]), format_num(values[i]), values[i] / total * 100.0),
            None => format!("Total\n{}", format_num(total)),
        };
        painter.text(c, egui::Align2::CENTER_CENTER, center_text, FontId::proportional(15.0), p.text_dark);
        ui.vertical(|ui| {
            egui::ScrollArea::vertical().id_salt((id, "pie_legend")).show(ui, |ui| {
                for (i, l) in labels.iter().enumerate() {
                    if values[i] <= 0.0 {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, colors[i % colors.len()]);
                        ui.label(RichText::new(l).color(if hovered == Some(i) { p.primary } else { p.text_dark }));
                        ui.label(RichText::new(format!("{}  ({:.1}%)", format_num(values[i]), values[i] / total * 100.0)).color(p.text_light));
                    });
                }
            });
        });
    });
}

pub fn format_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

// ------------------------------------------------------------------ ER diagram

pub struct ErTable {
    pub name: String,
    pub cols: Vec<ColumnInfo>,
    pub pos: Pos2,
}

#[derive(Default)]
pub struct ErState {
    pub tables: Vec<ErTable>,
    pub fks: Vec<ForeignKey>,
    pub pan: Vec2,
    pub loaded: bool,
    pub zoom: f32,
}

const ER_W: f32 = 210.0;
const ER_HEAD: f32 = 28.0;
const ER_ROW: f32 = 19.0;

impl ErState {
    pub fn set(&mut self, tables: Vec<(String, Vec<ColumnInfo>)>, fks: Vec<ForeignKey>) {
        let n = tables.len().max(1);
        let per_row = (n as f32).sqrt().ceil() as usize;
        let mut y = 20.0;
        let mut row_h: f32 = 0.0;
        self.tables.clear();
        for (i, (name, cols)) in tables.into_iter().enumerate() {
            if i > 0 && i % per_row == 0 {
                y += row_h + 50.0;
                row_h = 0.0;
            }
            let x = 20.0 + (i % per_row) as f32 * (ER_W + 70.0);
            row_h = row_h.max(ER_HEAD + cols.len() as f32 * ER_ROW + 8.0);
            self.tables.push(ErTable { name, cols, pos: Pos2::new(x, y) });
        }
        self.fks = fks;
        self.loaded = true;
        self.zoom = 1.0;
    }
}

pub fn er_diagram(ui: &mut egui::Ui, st: &mut ErState, p: &Palette) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::drag());
    let area = resp.rect;
    painter.rect_filled(area, 0.0, p.editor_bg);
    // dotted background
    let step = 24.0;
    let mut gx = area.left() + (st.pan.x % step);
    while gx < area.right() {
        let mut gy = area.top() + (st.pan.y % step);
        while gy < area.bottom() {
            painter.circle_filled(Pos2::new(gx, gy), 1.0, p.border);
            gy += step;
        }
        gx += step;
    }
    if resp.dragged() {
        st.pan += resp.drag_delta();
    }
    let origin = area.min.to_vec2() + st.pan;
    let rect_of = |t: &ErTable| Rect::from_min_size(t.pos + origin, Vec2::new(ER_W, ER_HEAD + t.cols.len() as f32 * ER_ROW + 6.0));
    // relationships
    let colors = chart_colors(p);
    for (k, fk) in st.fks.iter().enumerate() {
        let a = st.tables.iter().find(|t| t.name == fk.from_table);
        let b = st.tables.iter().find(|t| t.name == fk.to_table);
        if let (Some(a), Some(b)) = (a, b) {
            let ra = rect_of(a);
            let rb = rect_of(b);
            let ia = a.cols.iter().position(|c| c.name == fk.from_col).unwrap_or(0);
            let ib = b.cols.iter().position(|c| c.name == fk.to_col).unwrap_or(0);
            let ya = ra.top() + ER_HEAD + ia as f32 * ER_ROW + ER_ROW / 2.0;
            let yb = rb.top() + ER_HEAD + ib as f32 * ER_ROW + ER_ROW / 2.0;
            let (pa, pb, da, db) = if (ra.center().x - rb.center().x).abs() < ER_W {
                (Pos2::new(ra.left(), ya), Pos2::new(rb.left(), yb), -70.0, -70.0)
            } else if ra.center().x <= rb.center().x {
                (Pos2::new(ra.right(), ya), Pos2::new(rb.left(), yb), 60.0, -60.0)
            } else {
                (Pos2::new(ra.left(), ya), Pos2::new(rb.right(), yb), -60.0, 60.0)
            };
            let col = colors[k % colors.len()];
            let curve = egui::epaint::CubicBezierShape::from_points_stroke(
                [pa, pa + Vec2::new(da, 0.0), pb + Vec2::new(db, 0.0), pb],
                false,
                Color32::TRANSPARENT,
                Stroke::new(1.8_f32, col),
            );
            painter.add(curve);
            painter.circle_filled(pa, 3.5, col);
            painter.circle_stroke(pb, 4.0, Stroke::new(1.8_f32, col));
        }
    }
    // tables
    for (i, t) in st.tables.iter_mut().enumerate() {
        let r = Rect::from_min_size(t.pos + origin, Vec2::new(ER_W, ER_HEAD + t.cols.len() as f32 * ER_ROW + 6.0));
        if !r.intersects(area) {
            let head = ui.interact(Rect::from_min_size(r.min, Vec2::new(ER_W, ER_HEAD)), ui.id().with(("er", i)), Sense::drag());
            if head.dragged() {
                t.pos += head.drag_delta();
            }
            continue;
        }
        painter.rect_filled(r.translate(Vec2::new(0.0, 3.0)), 8.0, Color32::from_black_alpha(if p.dark { 90 } else { 25 }));
        painter.rect_filled(r, 8.0, if p.dark { p.panel_alt } else { Color32::WHITE });
        painter.rect_stroke(r, 8.0, Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
        let head_r = Rect::from_min_size(r.min, Vec2::new(ER_W, ER_HEAD));
        painter.rect_filled(head_r, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.primary.gamma_multiply(0.9));
        painter.text(head_r.left_center() + Vec2::new(10.0, 0.0), egui::Align2::LEFT_CENTER, &t.name, FontId::proportional(13.5), Color32::from_rgb(20, 20, 20));
        for (k, c) in t.cols.iter().enumerate() {
            let y = r.top() + ER_HEAD + k as f32 * ER_ROW + ER_ROW / 2.0 + 2.0;
            let name = if c.pk { format!("🔑 {}", c.name) } else { c.name.clone() };
            painter.text(Pos2::new(r.left() + 10.0, y), egui::Align2::LEFT_CENTER, name, FontId::proportional(12.0), if c.pk { p.primary } else { p.text_dark });
            painter.text(Pos2::new(r.right() - 10.0, y), egui::Align2::RIGHT_CENTER, c.data_type.to_lowercase(), FontId::monospace(10.5), p.text_light);
        }
        let head = ui.interact(head_r, ui.id().with(("er", i)), Sense::drag());
        if head.dragged() {
            t.pos += head.drag_delta();
        }
        if head.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    if st.tables.is_empty() {
        painter.text(area.center(), egui::Align2::CENTER_CENTER, if st.loaded { "No tables found" } else { "Loading schema…" }, FontId::proportional(16.0), p.text_light);
    }
}

/// Small drawn icon for tables/views in the sidebar.
pub fn table_icon(ui: &mut egui::Ui, color: Color32, view: bool) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
    let r = r.shrink(1.5);
    let pt = ui.painter();
    pt.rect_stroke(r, 2.0, Stroke::new(1.3_f32, color), egui::StrokeKind::Inside);
    if view {
        pt.circle_stroke(r.center(), 2.5, Stroke::new(1.2_f32, color));
    } else {
        let y = r.top() + r.height() * 0.38;
        pt.line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], Stroke::new(1.2_f32, color));
        pt.line_segment([Pos2::new(r.center().x, y), Pos2::new(r.center().x, r.bottom())], Stroke::new(1.0_f32, color));
    }
}

/// The BaseLens mark (see brand.rs).
pub fn logo(ui: &mut egui::Ui, p: &Palette, size: f32) {
    crate::brand::mark(ui, p, size);
}

#[cfg(test)]
mod sum_tests {
    use super::*;

    #[test]
    fn sums_like_sqlite() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t (g TEXT, v);").unwrap();
        let mut vals: Vec<(String, Cell)> = Vec::new();
        for i in 0..10 {
            vals.push(("a".into(), Cell::Float(0.1)));
            vals.push(("b".into(), Cell::Float(1e16)));
            vals.push(("b".into(), Cell::Float(1.0)));
            vals.push(("b".into(), Cell::Float(-1e16)));
            vals.push(("c".into(), Cell::Int(9007199254740993 + i)));
            vals.push(("d".into(), Cell::Int(3)));
            vals.push(("d".into(), Cell::Float(0.25)));
        }
        for (g, v) in &vals {
            let val = match v {
                Cell::Int(i) => rusqlite::types::Value::Integer(*i),
                Cell::Float(f) => rusqlite::types::Value::Real(*f),
                _ => rusqlite::types::Value::Null,
            };
            c.execute("INSERT INTO t VALUES (?1, ?2)", rusqlite::params![g, val]).unwrap();
        }
        let res = QueryResult { columns: vec!["g".into(), "v".into()], rows: vals.iter().map(|(g, v)| vec![Cell::Text(g.clone()), v.clone()]).collect(), ..Default::default() };
        let cfg = ChartCfg { x: 0, y: Some(1), agg: Agg::Sum, sort: ChartSort::LabelAsc, top: 50, init: true, ..Default::default() };
        let (series, _) = chart_series(&res, &cfg);
        assert_eq!(series.len(), 4);
        for (g, ours) in series {
            let theirs: f64 = c.query_row("SELECT CAST(sum(v) AS REAL) FROM t WHERE g = ?1", [&g], |r| r.get(0)).unwrap();
            assert_eq!(ours, theirs, "group {g}");
        }
        // a plain left-to-right sum is off for these groups
        let naive_a: f64 = (0..10).map(|_| 0.1).sum();
        assert_ne!(naive_a, 1.0);
        let mut naive_b = 0.0f64;
        for _ in 0..10 {
            naive_b += 1e16;
            naive_b += 1.0;
            naive_b += -1e16;
        }
        assert_ne!(naive_b, 10.0);
    }
}
