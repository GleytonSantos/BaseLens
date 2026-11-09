//! Export a result in many formats: CSV, JSON, SQL INSERTs, Markdown, HTML,
//! XML and Excel (XLSX). Pure functions from a result to bytes, so they're
//! easy to test; the app only picks the file.

use crate::db::{Cell, DbKind, QueryResult};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Csv,
    Json,
    SqlInsert,
    Markdown,
    Html,
    Xml,
    Xlsx,
}

impl Format {
    pub const ALL: [Format; 7] = [Format::Csv, Format::Json, Format::Xlsx, Format::SqlInsert, Format::Markdown, Format::Html, Format::Xml];
    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "JSON",
            Format::SqlInsert => "SQL INSERT statements",
            Format::Markdown => "Markdown table",
            Format::Html => "HTML table",
            Format::Xml => "XML",
            Format::Xlsx => "Excel (XLSX)",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::SqlInsert => "sql",
            Format::Markdown => "md",
            Format::Html => "html",
            Format::Xml => "xml",
            Format::Xlsx => "xlsx",
        }
    }
    pub fn shortcut(self) -> &'static str {
        match self {
            Format::Csv => "Ctrl+E",
            Format::Json => "Ctrl+Shift+J",
            _ => "",
        }
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Text of a cell for text formats (NULL → empty).
fn txt(c: &Cell) -> String {
    if c.is_null() {
        String::new()
    } else {
        c.display()
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// XML element names: letters, digits, `_`, `-`, `.`; must not start with a digit.
fn xml_name(s: &str) -> String {
    let mut n: String = s.chars().map(|c| if c.is_alphanumeric() || "_-.".contains(c) { c } else { '_' }).collect();
    if n.is_empty() || n.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') || n.to_ascii_lowercase().starts_with("xml") {
        n.insert(0, '_');
    }
    n
}

fn md_cell(s: &str) -> String {
    s.replace('\\', "\\\\").replace('|', "\\|").replace('\n', "<br>")
}

/// The result as a file in `fmt`. `table` names the target of SQL INSERTs.
pub fn render(r: &QueryResult, fmt: Format, table: &str, kind: DbKind) -> Result<Vec<u8>, String> {
    Ok(match fmt {
        Format::Csv => {
            let mut out = r.columns.iter().map(|c| csv_field(c)).collect::<Vec<_>>().join(",") + "\n";
            for row in &r.rows {
                out.push_str(&row.iter().map(|c| csv_field(&txt(c))).collect::<Vec<_>>().join(","));
                out.push('\n');
            }
            out.into_bytes()
        }
        Format::Json => serde_json::to_string_pretty(&crate::widgets::result_json(r, usize::MAX)).map_err(|e| e.to_string())?.into_bytes(),
        Format::SqlInsert => {
            let cols = r.columns.iter().map(|c| kind.quote(c)).collect::<Vec<_>>().join(", ");
            let t = table.split('.').map(|p| kind.quote(p)).collect::<Vec<_>>().join(".");
            let mut out = String::new();
            for row in &r.rows {
                let vals = row.iter().map(|c| c.sql_literal_for(kind)).collect::<Vec<_>>().join(", ");
                out.push_str(&format!("INSERT INTO {t} ({cols}) VALUES ({vals});\n"));
            }
            out.into_bytes()
        }
        Format::Markdown => {
            let mut out = format!("| {} |\n", r.columns.iter().map(|c| md_cell(c)).collect::<Vec<_>>().join(" | "));
            out.push_str(&format!("|{}\n", " --- |".repeat(r.columns.len())));
            for row in &r.rows {
                out.push_str(&format!("| {} |\n", row.iter().map(|c| md_cell(&txt(c))).collect::<Vec<_>>().join(" | ")));
            }
            out.into_bytes()
        }
        Format::Html => {
            let mut out = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>BaseLens export</title>\n<style>table{border-collapse:collapse;font-family:sans-serif;font-size:14px}th,td{border:1px solid #ccc;padding:4px 8px;text-align:left}th{background:#f2f2f7}td.null{color:#999;font-style:italic}</style></head><body>\n<table>\n<thead><tr>");
            for c in &r.columns {
                out.push_str(&format!("<th>{}</th>", html_escape(c)));
            }
            out.push_str("</tr></thead>\n<tbody>\n");
            for row in &r.rows {
                out.push_str("<tr>");
                for c in row {
                    if c.is_null() {
                        out.push_str("<td class=\"null\">NULL</td>");
                    } else {
                        out.push_str(&format!("<td>{}</td>", html_escape(&c.display())));
                    }
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody>\n</table>\n</body></html>\n");
            out.into_bytes()
        }
        Format::Xml => {
            let names: Vec<String> = r.columns.iter().map(|c| xml_name(c)).collect();
            let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rows>\n");
            for row in &r.rows {
                out.push_str("  <row>");
                for (c, n) in row.iter().zip(&names) {
                    if c.is_null() {
                        out.push_str(&format!("<{n} null=\"true\"/>"));
                    } else {
                        out.push_str(&format!("<{n}>{}</{n}>", html_escape(&c.display())));
                    }
                }
                out.push_str("</row>\n");
            }
            out.push_str("</rows>\n");
            out.into_bytes()
        }
        Format::Xlsx => {
            use rust_xlsxwriter::{Format as XFormat, Workbook};
            let mut wb = Workbook::new();
            let ws = wb.add_worksheet();
            let bold = XFormat::new().set_bold();
            let err = |e: rust_xlsxwriter::XlsxError| e.to_string();
            for (i, c) in r.columns.iter().enumerate() {
                ws.write_string_with_format(0, i as u16, c, &bold).map_err(err)?;
            }
            // Excel's limits (rows include the header)
            const XLSX_MAX_ROWS: usize = 1048576;
            const XLSX_MAX_COLS: usize = 16384;
            for (ri, row) in r.rows.iter().take(XLSX_MAX_ROWS - 1).enumerate() {
                let y = ri as u32 + 1;
                for (ci, c) in row.iter().take(XLSX_MAX_COLS).enumerate() {
                    let x = ci as u16;
                    match c {
                        Cell::Null => {}
                        Cell::Int(i) if i.unsigned_abs() < (1u64 << 53) => {
                            ws.write_number(y, x, *i as f64).map_err(err)?;
                        }
                        Cell::Float(f) if f.is_finite() => {
                            ws.write_number(y, x, *f).map_err(err)?;
                        }
                        other => {
                            const XLSX_MAX_CELL_CHARS: usize = 32767;
                            let s: String = other.display().chars().take(XLSX_MAX_CELL_CHARS).collect();
                            ws.write_string(y, x, s).map_err(err)?;
                        }
                    }
                }
            }
            ws.set_freeze_panes(1, 0).map_err(err)?;
            wb.save_to_buffer().map_err(err)?
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QueryResult {
        QueryResult {
            columns: vec!["id".into(), "name".into(), "note".into()],
            rows: vec![
                vec![Cell::Int(1), Cell::Text("Ann, \"A\"".into()), Cell::Null],
                vec![Cell::Int(2), Cell::Text("<b>|x".into()), Cell::Text("it's".into())],
            ],
            ..Default::default()
        }
    }

    fn s(f: Format) -> String {
        String::from_utf8(render(&sample(), f, "public.people", DbKind::Postgres).unwrap()).unwrap()
    }

    #[test]
    fn text_formats_escape_correctly() {
        assert_eq!(s(Format::Csv), "id,name,note\n1,\"Ann, \"\"A\"\"\",\n2,<b>|x,it's\n");
        let sql = s(Format::SqlInsert);
        assert!(sql.starts_with("INSERT INTO \"public\".\"people\" (\"id\", \"name\", \"note\") VALUES (1, 'Ann, \"A\"', NULL);"), "{sql}");
        assert!(sql.contains("'it''s'"));
        let md = s(Format::Markdown);
        assert!(md.starts_with("| id | name | note |\n| --- | --- | --- |\n"));
        assert!(md.contains("<b>\\|x"));
        let html = s(Format::Html);
        assert!(html.contains("<td>&lt;b&gt;|x</td>") && html.contains("<td class=\"null\">NULL</td>"));
        let xml = s(Format::Xml);
        assert!(xml.contains("<name>&lt;b&gt;|x</name>") && xml.contains("<note null=\"true\"/>"));
        assert!(s(Format::Json).contains("\"name\": \"Ann, \\\"A\\\"\""));
    }

    #[test]
    fn xlsx_is_a_real_workbook() {
        let b = render(&sample(), Format::Xlsx, "t", DbKind::Sqlite).unwrap();
        assert_eq!(&b[..2], b"PK"); // a zip file, as .xlsx is
        assert!(b.len() > 1000);
    }

    #[test]
    fn xml_names_are_valid() {
        assert_eq!(xml_name("1st col"), "_1st_col");
        assert_eq!(xml_name("xmlThing"), "_xmlThing");
        assert_eq!(xml_name("ok_name"), "ok_name");
    }
}
