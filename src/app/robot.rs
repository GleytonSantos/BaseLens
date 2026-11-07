//! Test robot. Drives the real window with real mouse and keyboard events
//! (injected through `raw_input_hook`) and checks what happened.
//!
//! `--self-test`        scripted checks of the main flows, prints PASS/FAIL
//! `--monkey <frames>`  random clicks, keys, text and scrolling to find
//!                      crashes and broken states
//!
//! Both run with a throw-away data folder, no file dialogs and no AI calls.

use super::*;
use crate::widgets::{self, Agg, ChartKind, ChartSort};
use eframe::egui::{Event, PointerButton, Pos2, RawInput, Vec2};
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq)]
pub enum RobotKind {
    SelfTest,
    Monkey(u64),
}

pub struct Robot {
    pub kind: RobotKind,
    step: usize,
    /// frames spent on the current step
    waited: u32,
    /// event groups, one group per frame
    queue: VecDeque<Vec<Event>>,
    pub log: Vec<String>,
    pub failures: Vec<String>,
    rng: u64,
    done: bool,
    sid: Option<u64>,
    mark: usize,
    frames: u64,
}

const STEP_TIMEOUT: u32 = 400;

impl Robot {
    pub fn new(kind: RobotKind) -> Self {
        // BASELENS_SEED changes the monkey's random path
        let seed = std::env::var("BASELENS_SEED").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0x9e3779b97f4a7c15).max(1);
        Robot { kind, step: 0, waited: 0, queue: VecDeque::new(), log: vec![], failures: vec![], rng: seed, done: false, sid: None, mark: 0, frames: 0 }
    }
    fn rand(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn click(&mut self, pos: Pos2) {
        let m = Modifiers::NONE;
        self.queue.push_back(vec![Event::PointerMoved(pos)]);
        self.queue.push_back(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: m }]);
        self.queue.push_back(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: m }]);
        self.queue.push_back(vec![]);
    }
    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.queue.push_back(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        self.queue.push_back(vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]);
        self.queue.push_back(vec![]);
    }
    fn check(&mut self, ok: bool, what: &str) {
        if ok {
            self.log.push(format!("PASS  {what}"));
        } else {
            self.log.push(format!("FAIL  {what}"));
            self.failures.push(what.to_string());
        }
    }
}

impl App {
    /// Called by eframe before each frame: feed the robot's events.
    pub(super) fn robot_input(&mut self, raw: &mut RawInput) {
        if let Some(r) = &mut self.robot {
            if let Some(group) = r.queue.pop_front() {
                raw.events.extend(group);
            }
        }
    }

    fn tab_ids(&self) -> Vec<u64> {
        self.tabs.iter().map(|t| t.id).collect()
    }

    fn active_query(&mut self) -> Option<&mut QueryTab> {
        let id = self.active_tab?;
        match &mut self.tabs.iter_mut().find(|t| t.id == id)?.kind {
            TabKind::Query(q) => Some(q),
            _ => None,
        }
    }

    fn robot_query(&mut self, sid: u64, sql: &str) -> u64 {
        let id = self.new_query_tab(sid, sql.to_string());
        if let Some(q) = self.active_query() {
            q.view = View::Chart;
        }
        self.run_query_tab(id, false);
        id
    }

    fn query_done(&self, id: u64) -> bool {
        self.tabs.iter().find(|t| t.id == id).map(|t| matches!(&t.kind, TabKind::Query(q) if !q.running && (!q.results.is_empty() || q.error.is_some()))).unwrap_or(false)
    }

    fn query_result(&self, id: u64) -> Option<QueryResult> {
        self.tabs.iter().find(|t| t.id == id).and_then(|t| match &t.kind {
            TabKind::Query(q) => q.results.get(q.active).cloned(),
            _ => None,
        })
    }

    /// Invariants that must hold after every frame.
    fn robot_invariants(&mut self) {
        let bad_tab = self.active_tab.map(|id| !self.tabs.iter().any(|t| t.id == id)).unwrap_or(false);
        let bad_conn = self.active_conn.map(|s| !self.open.iter().any(|o| o.sid == s)).unwrap_or(false);
        let orphan = self.tabs.iter().any(|t| !self.open.iter().any(|o| o.sid == t.conn));
        let frame = self.frame;
        if let Some(r) = &mut self.robot {
            if bad_tab {
                r.failures.push(format!("frame {frame}: active tab points to a closed tab"));
            }
            if bad_conn {
                r.failures.push(format!("frame {frame}: active connection points to a closed connection"));
            }
            if orphan {
                r.failures.push(format!("frame {frame}: a tab belongs to a closed connection"));
            }
        }
    }

    pub(super) fn robot_step(&mut self, ctx: &egui::Context) {
        self.robot_ctx = Some(ctx.clone());
        let Some(mut r) = self.robot.take() else { return };
        r.frames += 1;
        if !r.done {
            match r.kind {
                RobotKind::SelfTest => self.self_test(&mut r),
                RobotKind::Monkey(n) => self.monkey(&mut r, ctx, n),
            }
        }
        if r.queue.is_empty() || r.done {
            ctx.request_repaint();
        }
        self.robot = Some(r);
        self.robot_invariants();
        let finished = self.robot.as_ref().map(|r| r.done && r.queue.is_empty()).unwrap_or(false);
        if finished {
            self.robot_finish();
        }
        ctx.request_repaint();
    }

    fn robot_finish(&mut self) {
        let Some(r) = &self.robot else { return };
        let mut failures = r.failures.clone();
        failures.dedup();
        let mut report = r.log.join("\n");
        report.push_str(&format!("\n\n{} check(s), {} problem(s)\n", r.log.len(), failures.len()));
        for f in failures.iter().take(50) {
            report.push_str(&format!("  - {f}\n"));
        }
        eprintln!("{report}");
        let _ = std::fs::write(store::data_dir().join("robot-report.txt"), &report);
        std::process::exit(if failures.is_empty() { 0 } else { 1 });
    }

    fn self_test(&mut self, r: &mut Robot) {
        if !r.queue.is_empty() {
            return;
        }
        r.waited += 1;
        if r.waited > STEP_TIMEOUT {
            r.check(false, &format!("step {} timed out", r.step));
            r.step += 1;
            r.waited = 0;
            return;
        }
        let next = |r: &mut Robot| {
            r.step += 1;
            r.waited = 0;
        };
        match r.step {
            // 0: connect to the demo database and wait for the schema
            0 => {
                if r.waited == 1 {
                    let cfg = self.connections[0].clone();
                    self.connect(cfg, false);
                }
                let ready = self.open.first().map(|o| o.status == ConnStatus::Ready && !o.tables.is_empty() && o.columns.len() >= 4).unwrap_or(false);
                if ready {
                    r.sid = self.open.first().map(|o| o.sid);
                    r.check(true, "connects to the demo database and loads tables + columns");
                    next(r);
                }
            }
            // 1: Ctrl+T opens a query tab
            1 => {
                if r.waited == 1 {
                    r.mark = self.tabs.len();
                    r.key(Key::T, Modifiers::COMMAND);
                } else if r.waited > 3 {
                    r.check(self.tabs.len() == r.mark + 1, "Ctrl+T opens a new query tab");
                    next(r);
                }
            }
            // 2: the × on a tab closes it (reported bug)
            2 => {
                if r.waited == 1 {
                    r.mark = self.tabs.len();
                    let last = self.tabs.last().map(|t| t.id);
                    match last.and_then(|id| self.test_rects.get(&format!("tabclose:{id}")).copied()) {
                        Some(rect) => r.click(rect.center()),
                        None => r.check(false, "tab close button is on screen"),
                    }
                } else if r.waited > 3 {
                    r.check(self.tabs.len() + 1 == r.mark, "clicking × on a tab closes the tab");
                    next(r);
                }
            }
            // 3: clicking the tab body activates it (and does not close it)
            3 => {
                if r.waited == 1 {
                    if let Some(s) = r.sid {
                        self.new_query_tab(s, String::new());
                        self.new_query_tab(s, String::new());
                    }
                    r.mark = self.tabs.len();
                } else if r.waited == 3 {
                    let first = self.tabs.first().map(|t| t.id);
                    if let Some(rect) = first.and_then(|id| self.test_rects.get(&format!("tab:{id}")).copied()) {
                        r.click(rect.left_center() + Vec2::new(14.0, 0.0));
                    }
                } else if r.waited > 6 {
                    r.check(self.tabs.len() == r.mark && self.active_tab == self.tabs.first().map(|t| t.id), "clicking a tab selects it without closing it");
                    next(r);
                }
            }
            // 4: Ctrl+W closes the active tab
            4 => {
                if r.waited == 1 {
                    r.mark = self.tabs.len();
                    r.key(Key::W, Modifiers::COMMAND);
                } else if r.waited > 3 {
                    r.check(self.tabs.len() + 1 == r.mark, "Ctrl+W closes the active tab");
                    next(r);
                }
            }
            // 5: repeated labels are combined in the chart
            5 => {
                if r.waited == 1 {
                    if let Some(s) = r.sid {
                        r.mark = self.robot_query(s, "SELECT status, total FROM orders") as usize;
                    }
                } else if self.query_done(r.mark as u64) {
                    if let Some(res) = self.query_result(r.mark as u64) {
                        let cfg = widgets::auto_config(&res);
                        let (series, used) = widgets::chart_series(&res, &cfg);
                        r.check(cfg.agg == Agg::Sum && cfg.y == Some(1), "chart picks 'total' as value and sums repeated labels");
                        r.check(series.len() == 5 && used == res.rows.len(), "chart groups 80 orders into 5 status bars");
                        let total: f64 = res.rows.iter().filter_map(|row| row[1].as_f64()).sum();
                        let drawn: f64 = series.iter().map(|s| s.1).sum();
                        r.check((total - drawn).abs() < 1e-6, "chart bars add up to the real total");
                        r.check(series.windows(2).all(|w| w[0].1 >= w[1].1), "bars are sorted biggest first");
                    }
                    next(r);
                }
            }
            // 6: a result without numbers counts rows instead of drawing zeros
            6 => {
                if r.waited == 1 {
                    if let Some(s) = r.sid {
                        r.mark = self.robot_query(s, "SELECT country FROM customers") as usize;
                    }
                } else if self.query_done(r.mark as u64) {
                    if let Some(res) = self.query_result(r.mark as u64) {
                        let cfg = widgets::auto_config(&res);
                        let (series, _) = widgets::chart_series(&res, &cfg);
                        r.check(cfg.y.is_none() && cfg.agg == Agg::Count, "text-only result charts the number of rows");
                        r.check(!series.is_empty() && series.iter().all(|s| s.1 >= 1.0), "row counts are drawn (no all-zero bars)");
                    }
                    next(r);
                }
            }
            // 7: dates become a line in date order; many points are capped
            7 => {
                if r.waited == 1 {
                    if let Some(s) = r.sid {
                        r.mark = self.robot_query(s, "SELECT created_at, total FROM orders") as usize;
                    }
                } else if self.query_done(r.mark as u64) {
                    if let Some(res) = self.query_result(r.mark as u64) {
                        let cfg = widgets::auto_config(&res);
                        let (series, _) = widgets::chart_series(&res, &cfg);
                        r.check(cfg.kind == ChartKind::Line && cfg.sort == ChartSort::LabelAsc, "dates are drawn as a line, in date order");
                        r.check(series.windows(2).all(|w| w[0].0 <= w[1].0), "line points are in date order");
                        r.check(series.len() <= cfg.top, "chart keeps at most the chosen number of points");
                    }
                    next(r);
                }
            }
            // 8: changing chart settings sticks after running the query again
            8 => {
                let id = r.mark as u64;
                if r.waited == 1 {
                    if let Some(q) = self.active_query() {
                        q.chart.kind = ChartKind::Pie;
                    }
                    self.run_query_tab(id, false);
                } else if r.waited > 3 && self.query_done(id) {
                    let kind = self.active_query().map(|q| q.chart.kind);
                    r.check(kind == Some(ChartKind::Pie), "chart settings survive re-running the same query");
                    next(r);
                }
            }
            // 9: Ctrl+K opens the command palette and Esc closes it
            9 => {
                if r.waited == 1 {
                    r.key(Key::K, Modifiers::COMMAND);
                } else if r.waited == 4 {
                    r.check(self.quick.is_some(), "Ctrl+K opens the command palette");
                    r.key(Key::Escape, Modifiers::NONE);
                } else if r.waited > 7 {
                    r.check(self.quick.is_none(), "Esc closes the command palette");
                    next(r);
                }
            }
            // 10: search in a table without SQL
            10 => {
                if r.waited == 1 {
                    if let Some((s, t)) = r.sid.and_then(|s| self.conn(s).and_then(|o| o.tables.iter().find(|t| t.name == "products").cloned()).map(|t| (s, t))) {
                        self.ask_open(s, &t, None, Some("honey".into()));
                    }
                } else if r.waited > 5 {
                    let rows = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id)).and_then(|t| match &t.kind {
                        TabKind::Table(tt) if !tt.loading => tt.data.as_ref().map(|d| d.rows.len()),
                        _ => None,
                    });
                    if let Some(n) = rows {
                        r.check(n == 1, "typing 'honey' in a table search finds the one matching row");
                        next(r);
                    }
                }
            }
            // 11: the Lens bot answers about columns
            11 => {
                self.ask.scope = AskScope::Database;
                self.ask.input = "onde fica o preço?".into();
                self.ask_send();
                let ok = self.ask.msgs.last().map(|m| m.hits.iter().any(|h| h.column.as_ref().map(|c| c.0 == "price").unwrap_or(false))).unwrap_or(false);
                r.check(ok, "Lens finds the 'price' column from a Portuguese question");
                next(r);
            }
            // 12: distinct values for WHERE suggestions
            12 => {
                if let Some(s) = r.sid {
                    if r.waited == 1 {
                        self.fetch_values(s, "orders", "status");
                    } else if let Some(v) = self.values_cache.get(&(s, assist::value_key("orders", "status"))) {
                        r.check(v.len() == 5, "value suggestions list the 5 order statuses");
                        next(r);
                    }
                }
            }
            // 13: switching theme and closing every tab leaves a sane state
            13 => {
                if r.waited == 1 {
                    self.run_command(Cmd::ToggleTheme);
                    let ids = self.tab_ids();
                    for id in ids {
                        self.close_tab(id);
                    }
                } else if r.waited > 3 {
                    r.check(self.tabs.is_empty() && self.active_tab.is_none(), "closing every tab leaves no dangling selection");
                    next(r);
                }
            }
            // 14: the ▸ next to a table unfolds its columns (it used to open the table)
            14 => {
                let key = r.sid.map(|s| format!("{s}:products")).unwrap_or_default();
                if r.waited == 1 {
                    // as a user would: open the side panel (also when the window is narrow)
                    self.run_command(Cmd::ShowTables);
                    self.expanded.remove(&key);
                    r.mark = self.tabs.len();
                } else if r.waited == 3 {
                    match self.test_rects.get(&format!("navchev:{key}")).copied() {
                        Some(rect) => r.click(rect.center()),
                        None => r.check(false, "table arrow is on screen in the navigator"),
                    }
                } else if r.waited > 6 {
                    r.check(self.expanded.contains(&key) && self.tabs.len() == r.mark, "clicking ▸ unfolds the columns without opening the table");
                    next(r);
                }
            }
            // 15: clicking the table name opens it
            15 => {
                let key = r.sid.map(|s| format!("{s}:products")).unwrap_or_default();
                if r.waited == 1 {
                    r.mark = self.tabs.len();
                } else if r.waited == 3 {
                    if let Some(rect) = self.test_rects.get(&format!("navrow:{key}")).copied() {
                        r.click(rect.center());
                    }
                } else if r.waited > 6 {
                    let opened = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id)).map(|t| matches!(&t.kind, TabKind::Table(tt) if tt.table == "products")).unwrap_or(false);
                    r.check(opened && self.tabs.len() == r.mark + 1, "clicking a table name in the navigator opens it");
                    next(r);
                }
            }
            // 16/17: typing in the editor opens suggestions the same way in
            // lower case and in UPPER case
            16 | 17 => {
                let upper = r.step == 17;
                let text = if upper { "SELECT * FROM ORD" } else { "select * from ord" };
                if r.waited == 1 {
                    if let Some(s) = r.sid {
                        r.mark = self.new_query_tab(s, String::new()) as usize;
                    }
                } else if r.waited == 4 {
                    let rect = ctx_rect(self, "robot_editor_rect");
                    if let Some(rect) = rect {
                        r.click(rect.left_top() + Vec2::new(30.0, 10.0));
                    }
                    for ch in text.chars() {
                        r.queue.push_back(vec![Event::Text(ch.to_string())]);
                        r.queue.push_back(vec![]);
                    }
                    r.queue.push_back(vec![]);
                } else if r.waited > 6 {
                    let (open, first) = self.active_query().map(|q| (q.ac.open, q.ac.items.first().map(|i| (i.label.clone(), i.insert.clone())))).unwrap_or((false, None));
                    let sql = self.active_query().map(|q| q.sql.clone()).unwrap_or_default();
                    r.log.push(format!("      typed {sql:?} → popup open: {open}, first: {first:?}"));
                    let ok = open && first.as_ref().map(|f| f.0.to_lowercase().starts_with("order")).unwrap_or(false);
                    r.check(ok, &format!("typing {text:?} suggests tables ({})", if upper { "UPPER case" } else { "lower case" }));
                    next(r);
                }
            }
            // 18/19: keywords suggested while typing, inserted in the case typed
            18 | 19 => {
                let upper = r.step == 19;
                let text = if upper { "SEL" } else { "sel" };
                if r.waited == 1 {
                    if let Some(s) = r.sid {
                        self.new_query_tab(s, String::new());
                    }
                } else if r.waited == 4 {
                    if let Some(rect) = ctx_rect(self, "robot_editor_rect") {
                        r.click(rect.left_top() + Vec2::new(30.0, 10.0));
                    }
                    for ch in text.chars() {
                        r.queue.push_back(vec![Event::Text(ch.to_string())]);
                        r.queue.push_back(vec![]);
                    }
                    r.key(Key::Enter, Modifiers::NONE);
                } else if r.waited > 6 {
                    let sql = self.active_query().map(|q| q.sql.clone()).unwrap_or_default();
                    let want = if upper { "SELECT" } else { "select" };
                    r.check(sql.starts_with(want), &format!("typing {text:?} + Enter completes to {want:?} (got {sql:?})"));
                    next(r);
                }
            }
            _ => r.done = true,
        }
    }

    fn monkey(&mut self, r: &mut Robot, ctx: &egui::Context, frames: u64) {
        if r.frames == 1 {
            let cfg = self.connections[0].clone();
            self.connect(cfg, false);
        }
        if r.frames >= frames {
            r.log.push(format!("monkey: {} frames of random input without a crash", r.frames));
            r.done = true;
            return;
        }
        if !r.queue.is_empty() {
            return;
        }
        let screen = ctx.screen_rect();
        let x = screen.left() + (r.rand() % 1000) as f32 / 1000.0 * screen.width();
        let y = screen.top() + (r.rand() % 1000) as f32 / 1000.0 * screen.height();
        let pos = Pos2::new(x, y);
        match r.rand() % 10 {
            0..=4 => r.click(pos),
            5 => {
                let keys = [Key::Enter, Key::Escape, Key::Tab, Key::ArrowDown, Key::ArrowUp, Key::Backspace, Key::F5, Key::Space];
                let k = keys[(r.rand() % keys.len() as u64) as usize];
                let m = match r.rand() % 4 {
                    0 => Modifiers::COMMAND,
                    1 => Modifiers::SHIFT,
                    _ => Modifiers::NONE,
                };
                r.key(k, m);
            }
            6 => {
                // shortcuts, but not the lock (it would stop the run)
                let keys = [Key::T, Key::W, Key::K, Key::B, Key::J, Key::L, Key::F, Key::R, Key::Space, Key::Slash];
                let k = keys[(r.rand() % keys.len() as u64) as usize];
                r.key(k, Modifiers::COMMAND);
            }
            7 => {
                let words = ["select", " * from ", "orders", " where ", "status = '", "p.", "honey", "1", "'", ";"];
                let w = words[(r.rand() % words.len() as u64) as usize];
                r.queue.push_back(vec![Event::Text(w.to_string())]);
            }
            8 => {
                let dy = if r.rand() % 2 == 0 { 120.0 } else { -120.0 };
                r.queue.push_back(vec![
                    Event::PointerMoved(pos),
                    Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, dy), modifiers: Modifiers::NONE },
                ]);
            }
            _ => {
                // drag
                let to = Pos2::new(x + 60.0, y + 20.0);
                r.queue.push_back(vec![Event::PointerMoved(pos)]);
                r.queue.push_back(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
                r.queue.push_back(vec![Event::PointerMoved(to)]);
                r.queue.push_back(vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
            }
        }
        // the PIN lock would block the run; a monkey must never set it
        if !self.settings.pin_hash.is_empty() {
            self.settings.pin_hash.clear();
            self.locked = false;
        }
    }
}

fn ctx_rect(app: &App, key: &str) -> Option<egui::Rect> {
    app.robot_ctx.as_ref().and_then(|c| c.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(key))))
}
