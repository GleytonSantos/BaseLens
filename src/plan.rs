//! Visual EXPLAIN: asks the database for its query plan and draws it as a
//! tree, each step with its cost, estimated rows and (with ANALYZE) the
//! real time, plus a bar showing its share of the total cost.
//!
//! - PostgreSQL: `EXPLAIN (FORMAT JSON[, ANALYZE, BUFFERS]) <query>`
//! - MySQL 8: `EXPLAIN FORMAT=TREE <query>` (`EXPLAIN ANALYZE` with timing)
//! - SQLite: `EXPLAIN QUERY PLAN <query>` (no costs: SQLite doesn't give them)

use crate::db::{DbKind, QueryResult};
use crate::theme::Palette;
use eframe::egui::{self, Color32, RichText, Sense, Vec2};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanNode {
    /// "Seq Scan", "Index Scan", "Hash Join", "SCAN orders"…
    pub label: String,
    /// table, index, condition…
    pub detail: String,
    /// total cost of this step (including its children), in the database's units
    pub cost: Option<f64>,
    pub rows: Option<f64>,
    /// real milliseconds (ANALYZE only)
    pub actual_ms: Option<f64>,
    pub actual_rows: Option<f64>,
    pub children: Vec<PlanNode>,
}

impl PlanNode {
    fn max_cost(&self) -> f64 {
        self.children.iter().map(|c| c.max_cost()).fold(self.cost.unwrap_or(0.0), f64::max)
    }
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(|c| c.count()).sum::<usize>()
    }
}

/// The EXPLAIN statement for `sql` on this database.
pub fn explain_sql(kind: DbKind, sql: &str, analyze: bool) -> String {
    let q = sql.trim().trim_end_matches(';');
    match kind {
        DbKind::Postgres => {
            if analyze {
                format!("EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS) {q}")
            } else {
                format!("EXPLAIN (FORMAT JSON) {q}")
            }
        }
        DbKind::Mysql => {
            if analyze {
                format!("EXPLAIN ANALYZE {q}")
            } else {
                format!("EXPLAIN FORMAT=TREE {q}")
            }
        }
        DbKind::Sqlite => format!("EXPLAIN QUERY PLAN {q}"),
    }
}

/// Turns the database's answer into a tree.
pub fn parse(kind: DbKind, r: &QueryResult) -> Result<PlanNode, String> {
    match kind {
        DbKind::Postgres => {
            let text = r.rows.first().and_then(|row| row.first()).map(|c| c.display()).ok_or("The server returned no plan.")?;
            parse_pg_json(&text)
        }
        DbKind::Mysql => {
            let text = r.rows.iter().filter_map(|row| row.first()).map(|c| c.display()).collect::<Vec<_>>().join("\n");
            parse_mysql_tree(&text)
        }
        DbKind::Sqlite => parse_sqlite(r),
    }
}

pub fn parse_pg_json(text: &str) -> Result<PlanNode, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("Couldn't read the plan: {e}"))?;
    let top = v.as_array().and_then(|a| a.first()).ok_or("Unexpected plan format.")?;
    let plan = top.get("Plan").ok_or("No \"Plan\" in the answer.")?;
    fn node(p: &serde_json::Value) -> PlanNode {
        let s = |k: &str| p.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let f = |k: &str| p.get(k).and_then(|x| x.as_f64());
        let mut label = s("Node Type");
        let join = s("Join Type");
        if !join.is_empty() && label.contains("Join") || label == "Nested Loop" && !join.is_empty() {
            label = format!("{label} ({join})");
        }
        let mut detail = Vec::new();
        let rel = s("Relation Name");
        if !rel.is_empty() {
            let alias = s("Alias");
            detail.push(if !alias.is_empty() && alias != rel { format!("on {rel} {alias}") } else { format!("on {rel}") });
        }
        let idx = s("Index Name");
        if !idx.is_empty() {
            detail.push(format!("using {idx}"));
        }
        for k in ["Index Cond", "Hash Cond", "Merge Cond", "Join Filter", "Filter", "Recheck Cond"] {
            let c = s(k);
            if !c.is_empty() {
                detail.push(format!("{}: {c}", k.to_lowercase()));
            }
        }
        if let Some(keys) = p.get("Sort Key").and_then(|x| x.as_array()) {
            let ks: Vec<String> = keys.iter().filter_map(|k| k.as_str().map(str::to_string)).collect();
            detail.push(format!("sort by {}", ks.join(", ")));
        }
        let loops = f("Actual Loops").unwrap_or(1.0).max(1.0);
        PlanNode {
            label,
            detail: detail.join(" · "),
            cost: f("Total Cost"),
            rows: f("Plan Rows"),
            // per-loop time × loops = time this step took in total
            actual_ms: f("Actual Total Time").map(|t| t * loops),
            actual_rows: f("Actual Rows").map(|r| r * loops),
            children: p.get("Plans").and_then(|x| x.as_array()).map(|a| a.iter().map(node).collect()).unwrap_or_default(),
        }
    }
    Ok(node(plan))
}

/// Number after `key=` in MySQL's "(cost=1.25 rows=3)" or
/// "(actual time=0.1..0.2 rows=3 loops=1)".
fn num_after(s: &str, key: &str) -> Option<f64> {
    let i = s.find(key)? + key.len();
    let t: String = s[i..].chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == '+' || *c == '-').collect();
    t.parse().ok()
}

pub fn parse_mysql_tree(text: &str) -> Result<PlanNode, String> {
    // each step is a line "-> what  (cost=… rows=…) (actual time=a..b rows=… loops=…)",
    // indented 4 spaces per level
    let mut stack: Vec<(usize, PlanNode)> = Vec::new();
    let mut roots: Vec<PlanNode> = Vec::new();
    for line in text.lines() {
        let Some(pos) = line.find("-> ") else { continue };
        let depth = line[..pos].chars().count() / 4;
        let body = &line[pos + 3..];
        let (what, paren) = match body.find("  (") {
            Some(i) => (&body[..i], &body[i..]),
            None => (body, ""),
        };
        let (label, detail) = match what.split_once(": ") {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (what.to_string(), String::new()),
        };
        let est = paren.split("(actual").next().unwrap_or("");
        let act = paren.split("(actual").nth(1).unwrap_or("");
        let loops = num_after(act, "loops=").unwrap_or(1.0).max(1.0);
        let node = PlanNode {
            label,
            detail,
            cost: num_after(est, "cost="),
            rows: num_after(est, "rows="),
            actual_ms: act.find("..").and_then(|i| act[i + 2..].split_whitespace().next()).and_then(|t| t.parse::<f64>().ok()).map(|t| t * loops),
            actual_rows: num_after(act, "rows=").map(|r| r * loops),
            children: vec![],
        };
        while let Some((d, _)) = stack.last() {
            if *d >= depth {
                let (_, done) = stack.pop().unwrap();
                match stack.last_mut() {
                    Some((_, parent)) => parent.children.push(done),
                    None => roots.push(done),
                }
            } else {
                break;
            }
        }
        stack.push((depth, node));
    }
    while let Some((_, done)) = stack.pop() {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(done),
            None => roots.push(done),
        }
    }
    match roots.len() {
        0 => Err("The server returned no plan (EXPLAIN FORMAT=TREE needs MySQL 8.0.16 or newer).".into()),
        1 => Ok(roots.remove(0)),
        _ => Ok(PlanNode { label: "Query".into(), children: roots, ..Default::default() }),
    }
}

pub fn parse_sqlite(r: &QueryResult) -> Result<PlanNode, String> {
    // columns: id, parent, notused, detail
    let col = |name: &str| r.columns.iter().position(|c| c.eq_ignore_ascii_case(name));
    let (Some(ci), Some(cp), Some(cd)) = (col("id"), col("parent"), col("detail")) else {
        return Err("Unexpected EXPLAIN QUERY PLAN answer.".into());
    };
    let num = |c: &crate::db::Cell| c.display().parse::<i64>().unwrap_or(0);
    let items: Vec<(i64, i64, String)> = r.rows.iter().map(|row| (num(&row[ci]), num(&row[cp]), row[cd].display())).collect();
    fn build(items: &[(i64, i64, String)], parent: i64) -> Vec<PlanNode> {
        items
            .iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, d)| {
                let (label, detail) = match d.split_once(' ') {
                    Some((a, b)) if a.chars().all(|c| c.is_ascii_uppercase()) => (a.to_string(), b.to_string()),
                    _ => (d.clone(), String::new()),
                };
                PlanNode { label, detail, children: build(items, *id), ..Default::default() }
            })
            .collect()
    }
    let mut roots = build(&items, 0);
    match roots.len() {
        0 => Err("SQLite returned no plan.".into()),
        1 => Ok(roots.remove(0)),
        _ => Ok(PlanNode { label: "QUERY PLAN".into(), children: roots, ..Default::default() }),
    }
}

fn fmt_num(v: f64) -> String {
    if v >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if v >= 1e4 {
        format!("{:.1}k", v / 1e3)
    } else if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// Draws the plan; the costliest steps are highlighted.
pub fn show(ui: &mut egui::Ui, root: &PlanNode, p: &Palette, id: u64) {
    let max = root.max_cost().max(f64::MIN_POSITIVE);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} step(s)", root.count())).small().color(p.text_light));
        if let Some(c) = root.cost {
            ui.label(RichText::new(format!("· total cost {}", fmt_num(c))).small().color(p.text_light));
        }
        if let Some(t) = root.actual_ms {
            ui.label(RichText::new(format!("· took {t:.1} ms")).small().color(p.text_light));
        }
    });
    egui::ScrollArea::both().id_salt(("plan", id)).auto_shrink([false, false]).show(ui, |ui| {
        node_ui(ui, root, p, max, 0, &mut vec![id as usize]);
    });
}

fn node_ui(ui: &mut egui::Ui, n: &PlanNode, p: &Palette, max: f64, depth: usize, path: &mut Vec<usize>) {
    let share = n.cost.map(|c| (c / max).clamp(0.0, 1.0)).unwrap_or(0.0) as f32;
    // the share of the cost spent in this step itself, not its children
    let own = n.cost.map(|c| c - n.children.iter().filter_map(|k| k.cost).sum::<f64>()).unwrap_or(0.0);
    let hot = n.cost.is_some() && own / max > 0.5;
    let col: Color32 = if hot { p.danger } else if share > 0.25 { p.warning } else { p.primary };
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 18.0);
        ui.label(RichText::new(if n.children.is_empty() { "•" } else { "▾" }).color(p.text_lighter));
        let (r, _) = ui.allocate_exact_size(Vec2::new(60.0, 8.0), Sense::hover());
        ui.painter().rect_filled(r, 3.0, p.border);
        if share > 0.0 {
            let mut f = r;
            f.set_width((r.width() * share).max(2.0));
            ui.painter().rect_filled(f, 3.0, col);
        }
        ui.label(RichText::new(&n.label).strong().color(if hot { p.danger } else { p.text_dark }));
        if !n.detail.is_empty() {
            ui.label(RichText::new(&n.detail).small().monospace().color(p.text_light));
        }
        let mut facts = Vec::new();
        if let Some(c) = n.cost {
            facts.push(format!("cost {}", fmt_num(c)));
        }
        if let Some(r) = n.rows {
            facts.push(format!("~{} rows", fmt_num(r)));
        }
        if let Some(t) = n.actual_ms {
            facts.push(format!("{t:.2} ms"));
        }
        if let Some(r) = n.actual_rows {
            facts.push(format!("{} rows real", fmt_num(r)));
        }
        if !facts.is_empty() {
            ui.label(RichText::new(facts.join(" · ")).small().color(p.text_lighter));
        }
    });
    for (i, c) in n.children.iter().enumerate() {
        path.push(i);
        node_ui(ui, c, p, max, depth + 1, path);
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Cell;

    #[test]
    fn reads_a_postgres_plan() {
        let j = r#"[{"Plan":{"Node Type":"Hash Join","Join Type":"Inner","Total Cost":40.5,"Plan Rows":10,"Hash Cond":"(o.customer_id = c.id)",
            "Plans":[{"Node Type":"Seq Scan","Relation Name":"orders","Alias":"o","Total Cost":20.0,"Plan Rows":100,"Filter":"(total > 10)"},
                     {"Node Type":"Hash","Total Cost":15.0,"Plan Rows":50,"Plans":[{"Node Type":"Index Scan","Relation Name":"customers","Alias":"c","Index Name":"customers_pkey","Total Cost":14.0,"Plan Rows":50,"Actual Total Time":0.5,"Actual Loops":2}]}]}}]"#;
        let n = parse_pg_json(j).unwrap();
        assert_eq!(n.label, "Hash Join (Inner)");
        assert_eq!(n.cost, Some(40.5));
        assert!(n.detail.contains("hash cond: (o.customer_id = c.id)"));
        assert_eq!(n.children.len(), 2);
        assert_eq!(n.children[0].detail, "on orders o · filter: (total > 10)");
        let idx = &n.children[1].children[0];
        assert!(idx.detail.contains("using customers_pkey"));
        assert_eq!(idx.actual_ms, Some(1.0)); // 0.5 ms × 2 loops
        assert_eq!(n.count(), 4);
    }

    #[test]
    fn reads_a_mysql_tree() {
        let t = "-> Nested loop inner join  (cost=4.5 rows=3)\n    -> Filter: (o.total > 10)  (cost=1.25 rows=3)\n        -> Table scan on o  (cost=1.25 rows=10)\n    -> Single-row index lookup on c using PRIMARY (id=o.customer_id)  (cost=0.25 rows=1) (actual time=0.010..0.012 rows=1 loops=3)";
        let n = parse_mysql_tree(t).unwrap();
        assert_eq!(n.label, "Nested loop inner join");
        assert_eq!(n.cost, Some(4.5));
        assert_eq!(n.children.len(), 2);
        assert_eq!(n.children[0].label, "Filter");
        assert_eq!(n.children[0].detail, "(o.total > 10)");
        assert_eq!(n.children[0].children[0].label, "Table scan on o");
        let l = &n.children[1];
        assert_eq!(l.rows, Some(1.0));
        assert!((l.actual_ms.unwrap() - 0.036).abs() < 1e-9);
        assert_eq!(l.actual_rows, Some(3.0));
    }

    #[test]
    fn reads_a_sqlite_plan() {
        let r = QueryResult {
            columns: vec!["id".into(), "parent".into(), "notused".into(), "detail".into()],
            rows: vec![
                vec![Cell::Int(2), Cell::Int(0), Cell::Int(0), Cell::Text("SCAN o".into())],
                vec![Cell::Int(5), Cell::Int(0), Cell::Int(0), Cell::Text("SEARCH c USING INTEGER PRIMARY KEY (rowid=?)".into())],
                vec![Cell::Int(9), Cell::Int(0), Cell::Int(0), Cell::Text("USE TEMP B-TREE FOR ORDER BY".into())],
            ],
            ..Default::default()
        };
        let n = parse_sqlite(&r).unwrap();
        assert_eq!(n.label, "QUERY PLAN");
        assert_eq!(n.children.len(), 3);
        assert_eq!(n.children[1].label, "SEARCH");
        assert_eq!(n.children[1].detail, "c USING INTEGER PRIMARY KEY (rowid=?)");
    }

    #[test]
    fn explain_statements() {
        assert_eq!(explain_sql(DbKind::Postgres, "select 1;", false), "EXPLAIN (FORMAT JSON) select 1");
        assert_eq!(explain_sql(DbKind::Postgres, "select 1", true), "EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS) select 1");
        assert_eq!(explain_sql(DbKind::Mysql, "select 1", false), "EXPLAIN FORMAT=TREE select 1");
        assert_eq!(explain_sql(DbKind::Sqlite, "select 1", false), "EXPLAIN QUERY PLAN select 1");
    }
}
