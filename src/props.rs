//! Table properties (like DBeaver's Properties panel): columns,
//! constraints, foreign keys, references, indexes, triggers, partitions,
//! rules, policies, dependencies, permissions, size and DDL.
//!
//! Every section is one read-only catalog query per database; a section that
//! fails (old server, missing privilege) shows its error and the others still
//! load. The DDL is put together from the pieces where the database has no
//! single "give me the CREATE" (PostgreSQL).

use crate::db::{sql_string, Cell, DbKind, QueryResult};

#[derive(Clone, Debug)]
pub enum Body {
    Table(QueryResult),
    Text(String),
    Note(String),
}

#[derive(Clone, Debug)]
pub struct Section {
    pub name: &'static str,
    pub body: Body,
}

impl Section {
    /// Rows in a table section (for the tab's count).
    pub fn count(&self) -> Option<usize> {
        match &self.body {
            Body::Table(r) => Some(r.rows.len()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TableProps {
    pub sections: Vec<Section>,
}

/// Sections that are only used to build others start with '_' and aren't shown.
pub type Query = (&'static str, String);

fn pg_rel(schema: &str, table: &str) -> String {
    format!(
        "(SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = {} AND c.relname = {})",
        sql_string(schema),
        sql_string(table)
    )
}

/// The catalog queries for one table, in the order the tabs appear.
pub fn queries(kind: DbKind, schema: Option<&str>, table: &str, is_view: bool) -> Vec<Query> {
    let t = sql_string(table);
    match kind {
        DbKind::Postgres => {
            let s = schema.unwrap_or("public");
            let r = pg_rel(s, table);
            let mut q: Vec<Query> = vec![
                ("Columns", format!(
                    "SELECT a.attnum AS \"#\", a.attname AS \"column\", format_type(a.atttypid, a.atttypmod) AS \"type\", NOT a.attnotnull AS \"nullable\", \
                     pg_get_expr(d.adbin, d.adrelid) AS \"default\", col_description(a.attrelid, a.attnum) AS \"comment\" \
                     FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                     WHERE a.attrelid = {r} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum")),
                ("Constraints", format!(
                    "SELECT conname AS \"name\", CASE contype WHEN 'p' THEN 'primary key' WHEN 'u' THEN 'unique' WHEN 'c' THEN 'check' WHEN 'f' THEN 'foreign key' \
                     WHEN 'x' THEN 'exclusion' WHEN 'n' THEN 'not null' ELSE contype::text END AS \"kind\", pg_get_constraintdef(oid) AS \"definition\" \
                     FROM pg_catalog.pg_constraint WHERE conrelid = {r} ORDER BY contype, conname")),
                ("Foreign keys", format!(
                    "SELECT conname AS \"name\", confrelid::regclass::text AS \"references\", pg_get_constraintdef(oid) AS \"definition\" \
                     FROM pg_catalog.pg_constraint WHERE conrelid = {r} AND contype = 'f' ORDER BY conname")),
                ("References", format!(
                    "SELECT conrelid::regclass::text AS \"from table\", conname AS \"constraint\", pg_get_constraintdef(oid) AS \"definition\" \
                     FROM pg_catalog.pg_constraint WHERE confrelid = {r} AND contype = 'f' ORDER BY 1, 2")),
                ("Indexes", format!(
                    "SELECT i.relname AS \"name\", ix.indisunique AS \"unique\", ix.indisprimary AS \"primary\", pg_get_indexdef(ix.indexrelid) AS \"definition\", \
                     pg_size_pretty(pg_relation_size(ix.indexrelid)) AS \"size\", \
                     EXISTS (SELECT 1 FROM pg_catalog.pg_constraint k WHERE k.conindid = ix.indexrelid) AS \"from constraint\" \
                     FROM pg_catalog.pg_index ix JOIN pg_catalog.pg_class i ON i.oid = ix.indexrelid WHERE ix.indrelid = {r} ORDER BY i.relname")),
                ("Triggers", format!(
                    "SELECT tgname AS \"name\", tgenabled <> 'D' AS \"enabled\", pg_get_triggerdef(oid) AS \"definition\" \
                     FROM pg_catalog.pg_trigger WHERE tgrelid = {r} AND NOT tgisinternal ORDER BY tgname")),
                ("Partitions", format!(
                    "SELECT c.relname AS \"partition\", pg_get_expr(c.relpartbound, c.oid) AS \"bound\", pg_size_pretty(pg_total_relation_size(c.oid)) AS \"size\" \
                     FROM pg_catalog.pg_inherits i JOIN pg_catalog.pg_class c ON c.oid = i.inhrelid WHERE i.inhparent = {r} ORDER BY 1")),
                ("Rules", format!(
                    "SELECT rulename AS \"name\", definition FROM pg_catalog.pg_rules WHERE schemaname = {} AND tablename = {t} ORDER BY 1", sql_string(s))),
                ("Policies", format!(
                    "SELECT polname AS \"name\", CASE polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END AS \"command\", \
                     polpermissive AS \"permissive\", pg_get_expr(polqual, polrelid) AS \"using\", pg_get_expr(polwithcheck, polrelid) AS \"with check\" \
                     FROM pg_catalog.pg_policy WHERE polrelid = {r} ORDER BY 1")),
                ("Dependencies", format!(
                    "SELECT DISTINCT dn.nspname || '.' || dv.relname AS \"view\", CASE dv.relkind WHEN 'm' THEN 'materialized view' ELSE 'view' END AS \"kind\" \
                     FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_rewrite rw ON d.objid = rw.oid JOIN pg_catalog.pg_class dv ON rw.ev_class = dv.oid \
                     JOIN pg_catalog.pg_namespace dn ON dn.oid = dv.relnamespace WHERE d.refobjid = {r} AND dv.oid <> {r} ORDER BY 1")),
                ("Permissions", format!(
                    "SELECT grantee, string_agg(privilege_type, ', ' ORDER BY privilege_type) AS \"privileges\", bool_or(is_grantable = 'YES') AS \"can grant\" \
                     FROM information_schema.role_table_grants WHERE table_schema = {} AND table_name = {t} GROUP BY grantee ORDER BY grantee", sql_string(s))),
                ("Info", format!(
                    "SELECT c.reltuples::bigint AS \"rows (estimate)\", pg_size_pretty(pg_total_relation_size(c.oid)) AS \"total size\", \
                     pg_size_pretty(pg_relation_size(c.oid)) AS \"table size\", pg_size_pretty(pg_indexes_size(c.oid)) AS \"indexes size\", \
                     pg_get_userbyid(c.relowner) AS \"owner\", obj_description(c.oid, 'pg_class') AS \"comment\", c.relrowsecurity AS \"row security\" \
                     FROM pg_catalog.pg_class c WHERE c.oid = {r}")),
            ];
            if is_view {
                q.push(("_viewdef", format!("SELECT pg_get_viewdef({r}, true)")));
            }
            q
        }
        DbKind::Sqlite => vec![
            ("Columns", format!(
                "SELECT cid AS \"#\", name AS \"column\", type, NOT \"notnull\" AS \"nullable\", dflt_value AS \"default\", pk AS \"primary key #\", \
                 CASE hidden WHEN 0 THEN '' WHEN 1 THEN 'hidden' WHEN 2 THEN 'generated (virtual)' ELSE 'generated (stored)' END AS \"extra\" \
                 FROM pragma_table_xinfo({t})")),
            ("Indexes", format!(
                "SELECT il.name, il.\"unique\", CASE il.origin WHEN 'pk' THEN 'primary key' WHEN 'u' THEN 'unique constraint' ELSE 'CREATE INDEX' END AS \"from\", \
                 il.partial, m.sql AS \"definition\" FROM pragma_index_list({t}) il LEFT JOIN sqlite_master m ON m.type = 'index' AND m.name = il.name ORDER BY il.name")),
            ("Foreign keys", format!(
                "SELECT id, seq, \"table\" AS \"references\", \"from\", \"to\", on_update, on_delete, \"match\" FROM pragma_foreign_key_list({t}) ORDER BY id, seq")),
            ("References", format!(
                "SELECT m.name AS \"from table\", f.\"from\", f.\"to\", f.on_delete FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) f \
                 WHERE m.type = 'table' AND f.\"table\" = {t} COLLATE NOCASE ORDER BY 1")),
            ("Triggers", format!("SELECT name, sql AS \"definition\" FROM sqlite_master WHERE type = 'trigger' AND tbl_name = {t} ORDER BY name")),
            ("Dependencies", format!(
                "SELECT name AS \"view\", 'view (mentions it)' AS \"kind\" FROM sqlite_master WHERE type = 'view' AND name <> {t} AND sql LIKE {} ORDER BY name",
                sql_string(&format!("%{table}%")))),
            ("Info", format!("SELECT (SELECT count(*) FROM {}) AS \"rows\", (SELECT type FROM sqlite_master WHERE name = {t}) AS \"type\"", DbKind::Sqlite.quote(table))),
            ("_ddl", format!(
                "SELECT sql FROM sqlite_master WHERE tbl_name = {t} AND sql IS NOT NULL ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 0 WHEN 'index' THEN 1 ELSE 2 END, name")),
        ],
        DbKind::Mysql => {
            let qt = DbKind::Mysql.quote(table);
            vec![
                ("Columns", format!(
                    "SELECT ordinal_position AS `#`, column_name AS `column`, column_type AS `type`, is_nullable = 'YES' AS `nullable`, column_default AS `default`, \
                     extra, column_comment AS `comment` FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = {t} ORDER BY ordinal_position")),
                ("Constraints", format!(
                    "SELECT constraint_name AS `name`, lower(constraint_type) AS `kind` FROM information_schema.table_constraints \
                     WHERE table_schema = DATABASE() AND table_name = {t} ORDER BY constraint_type, constraint_name")),
                ("Foreign keys", format!(
                    "SELECT k.constraint_name AS `name`, k.column_name AS `column`, k.referenced_table_name AS `references`, k.referenced_column_name AS `to`, \
                     r.update_rule AS `on update`, r.delete_rule AS `on delete` FROM information_schema.key_column_usage k \
                     JOIN information_schema.referential_constraints r ON r.constraint_schema = k.constraint_schema AND r.constraint_name = k.constraint_name \
                     WHERE k.table_schema = DATABASE() AND k.table_name = {t} ORDER BY k.constraint_name, k.ordinal_position")),
                ("References", format!(
                    "SELECT table_name AS `from table`, column_name AS `column`, constraint_name AS `constraint` FROM information_schema.key_column_usage \
                     WHERE referenced_table_schema = DATABASE() AND referenced_table_name = {t} ORDER BY 1, 3")),
                ("Indexes", format!(
                    "SELECT index_name AS `name`, NOT non_unique AS `unique`, group_concat(column_name ORDER BY seq_in_index) AS `columns`, index_type AS `type` \
                     FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = {t} GROUP BY index_name, non_unique, index_type ORDER BY index_name")),
                ("Triggers", format!(
                    "SELECT trigger_name AS `name`, action_timing AS `when`, event_manipulation AS `event`, action_statement AS `definition` \
                     FROM information_schema.triggers WHERE event_object_schema = DATABASE() AND event_object_table = {t} ORDER BY 1")),
                ("Partitions", format!(
                    "SELECT partition_name AS `partition`, partition_method AS `method`, partition_description AS `bound`, table_rows AS `rows` \
                     FROM information_schema.partitions WHERE table_schema = DATABASE() AND table_name = {t} AND partition_name IS NOT NULL ORDER BY partition_ordinal_position")),
                ("Permissions", format!(
                    "SELECT grantee, group_concat(privilege_type ORDER BY privilege_type) AS `privileges`, max(is_grantable) AS `can grant` \
                     FROM information_schema.table_privileges WHERE table_schema = DATABASE() AND table_name = {t} GROUP BY grantee ORDER BY grantee")),
                ("Info", format!(
                    "SELECT table_rows AS `rows (estimate)`, engine, round(data_length / 1048576, 2) AS `data MiB`, round(index_length / 1048576, 2) AS `indexes MiB`, \
                     table_collation AS `collation`, create_time AS `created`, table_comment AS `comment` FROM information_schema.tables \
                     WHERE table_schema = DATABASE() AND table_name = {t}")),
                ("_ddl", if is_view { format!("SHOW CREATE VIEW {qt}") } else { format!("SHOW CREATE TABLE {qt}") }),
            ]
        }
    }
}

fn text(c: &Cell) -> String {
    if c.is_null() {
        String::new()
    } else {
        c.display()
    }
}

fn truthy(c: &Cell) -> bool {
    matches!(c, Cell::Int(1)) || matches!(text(c).as_str(), "t" | "true" | "1")
}

fn col<'a>(r: &'a QueryResult, name: &str) -> Option<usize> {
    r.columns.iter().position(|c| c == name)
}

/// PostgreSQL has no single CREATE TABLE source: build it from the columns,
/// constraints, standalone indexes, triggers and comments.
fn pg_ddl(schema: &str, table: &str, is_view: bool, get: &dyn Fn(&str) -> Option<QueryResult>) -> Result<String, String> {
    let name = format!("{}.{}", DbKind::Postgres.quote(schema), DbKind::Postgres.quote(table));
    if is_view {
        let def = get("_viewdef").and_then(|r| r.rows.first().map(|row| text(&row[0]))).ok_or("The view's definition couldn't be read.")?;
        return Ok(format!("CREATE OR REPLACE VIEW {name} AS\n{}", def.trim_end()));
    }
    let cols = get("Columns").ok_or("The columns couldn't be read.")?;
    let (ci, ti, ni, di) = (col(&cols, "column"), col(&cols, "type"), col(&cols, "nullable"), col(&cols, "default"));
    let mut lines: Vec<String> = cols
        .rows
        .iter()
        .map(|r| {
            let mut l = format!("    {} {}", DbKind::Postgres.quote(&text(&r[ci.unwrap_or(1)])), text(&r[ti.unwrap_or(2)]));
            let def = di.map(|i| text(&r[i])).unwrap_or_default();
            if !def.is_empty() {
                l.push_str(&format!(" DEFAULT {def}"));
            }
            if ni.map(|i| !truthy(&r[i])).unwrap_or(false) {
                l.push_str(" NOT NULL");
            }
            l
        })
        .collect();
    if let Some(cons) = get("Constraints") {
        let (nm, kd, df) = (col(&cons, "name"), col(&cons, "kind"), col(&cons, "definition"));
        for r in &cons.rows {
            // NOT NULL constraints are already on the columns (PostgreSQL 18 lists them too)
            if kd.map(|i| text(&r[i]) == "not null").unwrap_or(false) {
                continue;
            }
            if let (Some(nm), Some(df)) = (nm, df) {
                lines.push(format!("    CONSTRAINT {} {}", DbKind::Postgres.quote(&text(&r[nm])), text(&r[df])));
            }
        }
    }
    let mut out = format!("CREATE TABLE {name} (\n{}\n);", lines.join(",\n"));
    if let Some(ix) = get("Indexes") {
        let (df, fc) = (col(&ix, "definition"), col(&ix, "from constraint"));
        for r in &ix.rows {
            if fc.map(|i| !truthy(&r[i])).unwrap_or(true) {
                if let Some(df) = df {
                    out.push_str(&format!("\n{};", text(&r[df])));
                }
            }
        }
    }
    if let Some(tr) = get("Triggers") {
        if let Some(df) = col(&tr, "definition") {
            for r in &tr.rows {
                out.push_str(&format!("\n{};", text(&r[df])));
            }
        }
    }
    if let Some(info) = get("Info") {
        if let (Some(i), Some(row)) = (col(&info, "comment"), info.rows.first()) {
            let c = text(&row[i]);
            if !c.is_empty() {
                out.push_str(&format!("\nCOMMENT ON TABLE {name} IS {};", sql_string(&c)));
            }
        }
    }
    if let Some(cols) = get("Columns") {
        if let (Some(ci), Some(mi)) = (col(&cols, "column"), col(&cols, "comment")) {
            for r in &cols.rows {
                let c = text(&r[mi]);
                if !c.is_empty() {
                    out.push_str(&format!("\nCOMMENT ON COLUMN {name}.{} IS {};", DbKind::Postgres.quote(&text(&r[ci])), sql_string(&c)));
                }
            }
        }
    }
    Ok(out)
}

/// Turns the answers into tabs (hidden "_" queries feed the DDL tab).
pub fn assemble(kind: DbKind, schema: Option<&str>, table: &str, is_view: bool, answers: Vec<(&'static str, Result<QueryResult, String>)>) -> TableProps {
    let get = |n: &str| answers.iter().find(|(k, _)| *k == n).and_then(|(_, r)| r.as_ref().ok().cloned());
    let mut sections: Vec<Section> = answers
        .iter()
        .filter(|(n, _)| !n.starts_with('_'))
        .map(|(n, r)| Section {
            name: n,
            body: match r {
                Ok(r) => Body::Table(r.clone()),
                Err(e) => Body::Note(format!("Couldn't read this: {e}")),
            },
        })
        .collect();
    let ddl = match kind {
        DbKind::Postgres => pg_ddl(schema.unwrap_or("public"), table, is_view, &get),
        DbKind::Sqlite => get("_ddl")
            .map(|r| r.rows.iter().map(|row| format!("{};", text(&row[0]).trim_end_matches(';'))).collect::<Vec<_>>().join("\n\n"))
            .filter(|s| !s.is_empty())
            .ok_or("SQLite has no stored definition for this object.".to_string()),
        DbKind::Mysql => get("_ddl")
            .and_then(|r| r.rows.first().and_then(|row| row.get(1).map(|c| format!("{};", text(c)))))
            .ok_or("The definition couldn't be read (SHOW CREATE needs some privilege on the table).".to_string()),
    };
    sections.push(Section {
        name: "DDL",
        body: match ddl {
            Ok(s) => Body::Text(s),
            Err(e) => Body::Note(e),
        },
    });
    TableProps { sections }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qr(cols: &[&str], rows: Vec<Vec<Cell>>) -> QueryResult {
        QueryResult { columns: cols.iter().map(|c| c.to_string()).collect(), rows, ..Default::default() }
    }

    #[test]
    fn names_are_quoted_safely() {
        for kind in [DbKind::Postgres, DbKind::Sqlite, DbKind::Mysql] {
            for (_, sql) in queries(kind, Some("pub'lic"), "o'rders", false) {
                assert!(!sql.contains("'o'rders'") && !sql.contains("'pub'lic'"), "{sql}");
                let first = sql.split_whitespace().next().unwrap_or("");
                assert!(first == "SELECT" || first == "SHOW", "{sql}");
            }
        }
    }

    #[test]
    fn builds_postgres_ddl() {
        let answers = vec![
            ("Columns", Ok(qr(&["#", "column", "type", "nullable", "default", "comment"], vec![
                vec![Cell::Int(1), Cell::Text("id".into()), Cell::Text("integer".into()), Cell::Text("false".into()), Cell::Text("nextval('o_id_seq'::regclass)".into()), Cell::Null],
                vec![Cell::Int(2), Cell::Text("note".into()), Cell::Text("text".into()), Cell::Text("true".into()), Cell::Null, Cell::Text("free text".into())],
            ]))),
            ("Constraints", Ok(qr(&["name", "kind", "definition"], vec![vec![Cell::Text("o_pkey".into()), Cell::Text("primary key".into()), Cell::Text("PRIMARY KEY (id)".into())]]))),
            ("Indexes", Ok(qr(&["name", "unique", "primary", "definition", "size", "from constraint"], vec![
                vec![Cell::Text("o_pkey".into()), Cell::Text("true".into()), Cell::Text("true".into()), Cell::Text("CREATE UNIQUE INDEX o_pkey ON public.o USING btree (id)".into()), Cell::Text("16 kB".into()), Cell::Text("true".into())],
                vec![Cell::Text("o_note".into()), Cell::Text("false".into()), Cell::Text("false".into()), Cell::Text("CREATE INDEX o_note ON public.o USING btree (note)".into()), Cell::Text("8 kB".into()), Cell::Text("false".into())],
            ]))),
            ("Triggers", Ok(qr(&["name", "enabled", "definition"], vec![]))),
            ("Info", Ok(qr(&["comment"], vec![vec![Cell::Null]]))),
        ];
        let p = assemble(DbKind::Postgres, Some("public"), "o", false, answers);
        let Body::Text(ddl) = &p.sections.last().unwrap().body else { panic!() };
        assert!(ddl.starts_with("CREATE TABLE \"public\".\"o\" (\n    \"id\" integer DEFAULT nextval('o_id_seq'::regclass) NOT NULL,\n    \"note\" text,\n    CONSTRAINT \"o_pkey\" PRIMARY KEY (id)\n);"), "{ddl}");
        assert!(ddl.contains("CREATE INDEX o_note") && !ddl.contains("CREATE UNIQUE INDEX o_pkey"), "{ddl}");
        assert!(ddl.contains("COMMENT ON COLUMN \"public\".\"o\".\"note\" IS 'free text';"));
        assert_eq!(p.sections.iter().map(|s| s.name).collect::<Vec<_>>(), ["Columns", "Constraints", "Indexes", "Triggers", "Info", "DDL"]);
    }

    #[test]
    fn a_failed_section_does_not_hide_the_rest() {
        let p = assemble(DbKind::Mysql, None, "t", false, vec![("Columns", Ok(qr(&["#"], vec![]))), ("Permissions", Err("denied".into())), ("_ddl", Err("denied".into()))]);
        assert!(matches!(p.sections[1].body, Body::Note(ref s) if s.contains("denied")));
        assert!(matches!(p.sections.last().unwrap().body, Body::Note(_)));
    }
}
