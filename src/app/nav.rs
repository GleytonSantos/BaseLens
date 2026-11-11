//! One navigator for every open database, the breadcrumb that says where you
//! are, and the "Ask" bot panel.
//!
//! Every engine is shown with the same four levels so nothing looks
//! different between SQLite, PostgreSQL and MySQL:
//! connection → database → schema (only when there is more than one) → table
//! → columns.

use super::*;
use crate::widgets;
use eframe::egui::{text::LayoutJob, CornerRadius, FontId, Margin, RichText, Sense, Stroke, TextFormat, Vec2};

enum NavAct {
    Open(u64, TableInfo),
    Query(u64, String),
    Toggle(String),
    Switch(u64, String),
    NewInstance(u64, Option<String>),
    Disconnect(u64),
    Activate(u64),
    Er(u64),
    Ask(u64, TableInfo),
    Props(u64, TableInfo),
}

enum AskAct {
    Open(u64, TableInfo, Option<String>, Option<String>),
    SearchData(usize),
    Example(String),
}

/// Text with `code` parts shown in the accent colour.
fn rich_text(ui: &mut egui::Ui, text: &str, p: &Palette, color: Color32) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let body = egui::TextStyle::Body.resolve(ui.style());
    for (i, part) in text.split('`').enumerate() {
        let fmt = if i % 2 == 1 {
            TextFormat { font_id: FontId::monospace(body.size - 0.5), color: p.primary, background: theme::mix(p.panel_alt, p.primary, 0.12), ..Default::default() }
        } else {
            TextFormat { font_id: body.clone(), color, ..Default::default() }
        };
        job.append(part, 0.0, fmt);
    }
    ui.label(job);
}

fn level_row(ui: &mut egui::Ui, indent: f32, f: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(indent);
        f(ui);
    })
    .response
}

impl App {
    /// "Connection › database › schema › table" above the open tab.
    pub(super) fn breadcrumb(&self, ui: &mut egui::Ui, tid: u64) {
        let p = self.pal;
        let Some(t) = self.tabs.iter().find(|t| t.id == tid) else { return };
        let Some(o) = self.conn(t.conn) else { return };
        let mut parts: Vec<(String, Color32)> = vec![(o.label(), p.text)];
        let db = if o.current_db.is_empty() { "main".to_string() } else { o.current_db.clone() };
        parts.push((format!("🗄 {db}"), p.text));
        match &t.kind {
            TabKind::Table(tt) => {
                if let Some(s) = &tt.schema {
                    parts.push((format!("📁 {s}"), p.text_light));
                }
                parts.push((tt.table.clone(), p.text_dark));
            }
            _ => parts.push((t.title.clone(), p.text_dark)),
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, o.color());
            for (i, (s, c)) in parts.iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("›").color(p.text_lighter));
                }
                ui.label(RichText::new(s).small().color(*c));
            }
        });
    }

    pub(super) fn side_navigator(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Databases").size(17.0).strong().color(p.text_dark));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("⟳").frame(false)).on_hover_text("Refresh every open connection  (Ctrl+R)").clicked() {
                    for o in &self.open {
                        let _ = o.handle.send(Request::Refresh);
                    }
                }
                if ui.add(egui::Button::new("+").frame(false)).on_hover_text("Connect another database").clicked() {
                    self.side_tab = SideTab::Connections;
                }
            });
        });
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.table_filter).hint_text("🔍  Find a table or column everywhere").desired_width(f32::INFINITY).margin(Margin::symmetric(10, 6)));
        ui.add_space(6.0);
        if self.open.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("🗄").size(30.0).color(p.text_lighter));
                ui.label(RichText::new("Nothing open yet.").color(p.text_light));
                if ui.button("Choose a connection").clicked() {
                    self.side_tab = SideTab::Connections;
                }
            });
            return;
        }
        let filter = self.table_filter.to_lowercase();
        let active_table = self.active_tab.and_then(|id| self.tabs.iter().find(|x| x.id == id)).and_then(|x| match &x.kind {
            TabKind::Table(tt) => Some((x.conn, tt.table.clone())),
            _ => None,
        });
        let mut acts: Vec<NavAct> = Vec::new();
        let mut rects: Vec<(String, egui::Rect)> = Vec::new();
        egui::ScrollArea::vertical().id_salt("nav").auto_shrink([false, false]).show(ui, |ui| {
            for o in &self.open {
                let sid = o.sid;
                let ckey = format!("c:{sid}");
                // connections are open by default; the set stores the closed ones
                let open_conn = !self.expanded.contains(&ckey) || !filter.is_empty();
                let is_active = Some(sid) == self.active_conn;
                // ---- level 1: connection
                let fr = egui::Frame::new()
                    .fill(if is_active { theme::mix(p.panel_alt, p.primary, 0.08) } else { p.panel_alt })
                    .stroke(Stroke::new(1.0, if is_active { theme::mix(p.border, p.primary, 0.6) } else { p.border }))
                    .corner_radius(10)
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(if open_conn { "⏷" } else { "⏵" }).small().color(p.text_lighter));
                            let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 26.0), Sense::hover());
                            ui.painter().rect_filled(r, 2.0, o.color());
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.add(egui::Label::new(RichText::new(o.label()).strong().color(p.text_dark)).truncate().selectable(false));
                                let st = match &o.status {
                                    ConnStatus::Ready => format!("{} · {}", o.cfg.kind.label(), o.server),
                                    ConnStatus::Connecting => "Connecting…".into(),
                                    ConnStatus::Failed(e) => format!("⚠ {e}"),
                                };
                                ui.add(egui::Label::new(RichText::new(st).small().color(p.text_light)).truncate().selectable(false));
                            });
                        });
                    });
                let r = ui.interact(fr.response.rect, ui.id().with(("navc", sid)), Sense::click());
                if r.clicked() {
                    acts.push(NavAct::Toggle(ckey.clone()));
                    acts.push(NavAct::Activate(sid));
                }
                r.on_hover_text("Click to fold / unfold").context_menu(|ui| {
                    if ui.button("Open again as a new instance").clicked() {
                        acts.push(NavAct::NewInstance(sid, None));
                        ui.close_menu();
                    }
                    if ui.button("Relationship diagram").clicked() {
                        acts.push(NavAct::Er(sid));
                        ui.close_menu();
                    }
                    if ui.button(RichText::new("Disconnect").color(p.danger)).clicked() {
                        acts.push(NavAct::Disconnect(sid));
                        ui.close_menu();
                    }
                });
                if !open_conn {
                    ui.add_space(4.0);
                    continue;
                }
                // ---- level 2: databases (current one unfolds into its tables)
                let current = if o.current_db.is_empty() { "main".to_string() } else { o.current_db.clone() };
                let mut dbs: Vec<String> = if o.cfg.kind == DbKind::Sqlite { vec![current.clone()] } else { o.databases.clone() };
                if !dbs.contains(&current) {
                    dbs.insert(0, current.clone());
                }
                for db in dbs {
                    let is_cur = db == current;
                    if !is_cur && !filter.is_empty() {
                        continue;
                    }
                    let row = level_row(ui, 10.0, |ui| {
                        ui.label(RichText::new(if is_cur { "⏷" } else { "⏵" }).small().color(p.text_lighter));
                        ui.label(RichText::new("🗄").color(if is_cur { p.secondary } else { p.text_lighter }));
                        ui.add(egui::Label::new(RichText::new(&db).color(if is_cur { p.text_dark } else { p.text_light })).truncate().selectable(false));
                        if is_cur && o.cfg.kind != DbKind::Sqlite {
                            super::panels::chip(ui, "in use", p.secondary, theme::mix(p.sidebar, p.secondary, 0.15));
                        }
                    });
                    let r = ui.interact(row.rect, ui.id().with(("navd", sid, &db)), Sense::click());
                    if r.hovered() {
                        ui.painter().rect_stroke(row.rect.expand(2.0), 6.0, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                    }
                    if !is_cur {
                        if r.clicked() {
                            acts.push(NavAct::Switch(sid, db.clone()));
                        }
                        r.on_hover_text("Click to switch to this database · right-click to open it side by side").context_menu(|ui| {
                            if ui.button("Switch to this database").clicked() {
                                acts.push(NavAct::Switch(sid, db.clone()));
                                ui.close_menu();
                            }
                            if ui.button("Open side by side (new instance)").clicked() {
                                acts.push(NavAct::NewInstance(sid, Some(db.clone())));
                                ui.close_menu();
                            }
                        });
                        continue;
                    }
                    // ---- level 3: schemas, level 4: tables
                    let cols_of = |t: &TableInfo| o.columns.get(&t.key()).cloned().unwrap_or_default();
                    let tables: Vec<(TableInfo, Vec<ColumnInfo>)> = o
                        .tables
                        .iter()
                        .map(|t| (t.clone(), cols_of(t)))
                        .filter(|(t, c)| filter.is_empty() || t.key().to_lowercase().contains(&filter) || c.iter().any(|x| x.name.to_lowercase().contains(&filter)))
                        .collect();
                    let mut schemas: Vec<Option<String>> = Vec::new();
                    for (t, _) in &tables {
                        if !schemas.contains(&t.schema) {
                            schemas.push(t.schema.clone());
                        }
                    }
                    let show_schema = schemas.len() > 1 || schemas.first().map(|s| s.is_some()).unwrap_or(false);
                    if tables.is_empty() {
                        level_row(ui, 30.0, |ui| {
                            ui.label(RichText::new(if filter.is_empty() { "No tables" } else { "No match here" }).small().color(p.text_lighter));
                        });
                    }
                    for sch in schemas {
                        let skey = format!("s:{sid}:{}", sch.clone().unwrap_or_default());
                        let mut indent = 24.0;
                        if show_schema {
                            let n = tables.iter().filter(|(t, _)| t.schema == sch).count();
                            let folded = self.expanded.contains(&skey) && filter.is_empty();
                            let row = level_row(ui, 24.0, |ui| {
                                ui.label(RichText::new(if folded { "⏵" } else { "⏷" }).small().color(p.text_lighter));
                                ui.label(RichText::new("📁").color(p.warning));
                                ui.label(RichText::new(sch.clone().unwrap_or_else(|| "default".into())).color(p.text));
                                ui.label(RichText::new(n.to_string()).small().color(p.text_lighter));
                            });
                            if ui.interact(row.rect, ui.id().with(("navs", &skey)), Sense::click()).clicked() {
                                acts.push(NavAct::Toggle(skey.clone()));
                            }
                            if folded {
                                continue;
                            }
                            indent = 38.0;
                        }
                        for (t, cols) in tables.iter().filter(|(t, _)| t.schema == sch) {
                            let tkey = format!("{sid}:{}", t.key());
                            let expanded = self.expanded.contains(&tkey) || (!filter.is_empty() && cols.iter().any(|x| x.name.to_lowercase().contains(&filter)));
                            let bg = ui.painter().add(egui::Shape::Noop);
                            let mut chev_rect = egui::Rect::NOTHING;
                            let row = level_row(ui, indent, |ui| {
                                let chev = ui.add(egui::Label::new(RichText::new(if expanded { "⏷" } else { "⏵" }).small().color(p.text_lighter)).sense(Sense::click()).selectable(false));
                                chev_rect = chev.rect;
                                if chev.clicked() {
                                    acts.push(NavAct::Toggle(tkey.clone()));
                                }
                                widgets::table_icon(ui, if t.is_view { p.secondary } else { p.primary }, t.is_view);
                                ui.add(egui::Label::new(RichText::new(&t.name).color(p.text_dark)).truncate().selectable(false));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if !cols.is_empty() {
                                        ui.label(RichText::new(cols.len().to_string()).small().color(p.text_lighter));
                                    }
                                });
                            });
                            let r = ui.interact(row.rect, ui.id().with(("navt", &tkey)), Sense::click());
                            let hl = row.rect.expand2(Vec2::new(4.0, 2.0));
                            if active_table.as_ref().map(|(s, n)| *s == sid && *n == t.name).unwrap_or(false) {
                                ui.painter().set(bg, egui::Shape::rect_filled(hl, 8.0, p.selection));
                            } else if r.hovered() {
                                ui.painter().set(bg, egui::Shape::rect_filled(hl, 8.0, theme::mix(p.sidebar, p.text_dark, 0.06)));
                            }
                            rects.push((format!("navchev:{tkey}"), chev_rect));
                            rects.push((format!("navrow:{tkey}"), row.rect));
                            // the row area sits on top of the ▸: a click there unfolds the
                            // columns instead of opening the table
                            if r.clicked() {
                                let on_chev = r.interact_pointer_pos().map(|pos| chev_rect.expand(6.0).contains(pos)).unwrap_or(false);
                                if on_chev {
                                    acts.push(NavAct::Toggle(tkey.clone()));
                                } else {
                                    acts.push(NavAct::Open(sid, t.clone()));
                                }
                            }
                            if r.double_clicked() {
                                acts.push(NavAct::Toggle(tkey.clone()));
                            }
                            let kind = o.cfg.kind;
                            r.context_menu(|ui| {
                                if ui.button("Open data").clicked() {
                                    acts.push(NavAct::Open(sid, t.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("ⓘ Properties").clicked() {
                                    acts.push(NavAct::Props(sid, t.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("◎ Ask Lens about this table").clicked() {
                                    acts.push(NavAct::Ask(sid, t.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("New query: SELECT").clicked() {
                                    acts.push(NavAct::Query(sid, format!("SELECT * FROM {} LIMIT 100;", Self::qualified(kind, &t.schema, &t.name))));
                                    ui.close_menu();
                                }
                                if ui.button("Count rows").clicked() {
                                    acts.push(NavAct::Query(sid, format!("SELECT COUNT(*) AS total FROM {};", Self::qualified(kind, &t.schema, &t.name))));
                                    ui.close_menu();
                                }
                                if ui.button("Copy name").clicked() {
                                    ui.ctx().copy_text(t.name.clone());
                                    ui.close_menu();
                                }
                            });
                            if expanded {
                                for c in cols {
                                    let hit = !filter.is_empty() && c.name.to_lowercase().contains(&filter);
                                    level_row(ui, indent + 22.0, |ui| {
                                        ui.label(RichText::new(if c.pk { "🔑" } else { "•" }).small().color(if c.pk { p.warning } else { p.text_lighter }));
                                        ui.add(egui::Label::new(RichText::new(&c.name).color(if hit { p.primary } else { p.text })).truncate().selectable(false));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new(c.data_type.to_lowercase()).small().monospace().color(p.text_lighter));
                                        });
                                    });
                                }
                            }
                        }
                    }
                }
                ui.add_space(8.0);
            }
        });
        self.test_rects.extend(rects);
        for a in acts {
            match a {
                NavAct::Open(sid, t) => {
                    self.active_conn = Some(sid);
                    self.open_table(sid, &t);
                }
                NavAct::Query(sid, q) => {
                    let id = self.new_query_tab(sid, q);
                    self.run_query_tab(id, false);
                }
                NavAct::Toggle(k) => {
                    if !self.expanded.remove(&k) {
                        self.expanded.insert(k);
                    }
                }
                NavAct::Switch(sid, db) => self.switch_database(sid, db),
                NavAct::NewInstance(sid, db) => {
                    if let Some(o) = self.conn(sid) {
                        let mut cfg = o.cfg.clone();
                        if let Some(d) = db {
                            cfg.database = d;
                        }
                        let pw = o.password.clone();
                        self.spawn_conn(cfg, pw);
                    }
                }
                NavAct::Disconnect(sid) => self.close_conn(sid),
                NavAct::Activate(sid) => {
                    self.active_conn = Some(sid);
                    if let Some(t) = self.tabs.iter().find(|t| t.conn == sid) {
                        if self.active_tab.and_then(|id| self.tabs.iter().find(|x| x.id == id)).map(|x| x.conn) != Some(sid) {
                            self.active_tab = Some(t.id);
                        }
                    }
                }
                NavAct::Er(sid) => self.new_er_tab(sid),
                NavAct::Props(sid, t) => {
                    self.active_conn = Some(sid);
                    self.open_table(sid, &t);
                    let tid = self.active_tab;
                    let mut load = false;
                    if let Some(tt) = tid.and_then(|id| self.tabs.iter_mut().find(|x| x.id == id)).and_then(|x| match &mut x.kind {
                        TabKind::Table(tt) => Some(tt),
                        _ => None,
                    }) {
                        tt.props_open = true;
                        load = tt.props.is_none() && tt.props_tag == 0;
                    }
                    if let (true, Some(tid)) = (load, tid) {
                        self.do_action(tid, sid, Action::LoadProps);
                    }
                }
                NavAct::Ask(sid, t) => {
                    self.active_conn = Some(sid);
                    self.open_table(sid, &t);
                    self.ask.scope = AskScope::ThisTable;
                    self.side_tab = SideTab::Ask;
                    self.ask.focus_input = true;
                }
            }
        }
    }

    pub(super) fn side_ask(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            crate::brand::lens_glyph(ui, p.primary, 20.0);
            ui.label(RichText::new("Ask Lens").size(17.0).strong().color(p.text_dark));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.ask.msgs.is_empty() && ui.add(egui::Button::new(RichText::new("Clear").small()).frame(false)).clicked() {
                    self.ask.msgs.clear();
                    self.ask.jobs.clear();
                }
            });
        });
        ui.label(RichText::new("Is it in this table? I check table names, columns and data. Works offline.").small().color(p.text_light));
        ui.add_space(6.0);
        // scope
        let focus = self.ask_focus();
        egui::Frame::new().fill(theme::mix(p.sidebar, p.text_dark, 0.05)).corner_radius(9).inner_margin(2).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let this = focus.as_ref().map(|f| f.1.clone()).unwrap_or_else(|| "This table".into());
                for (sc, label) in [(AskScope::ThisTable, this.as_str()), (AskScope::Database, "This database"), (AskScope::Everything, "Everywhere")] {
                    let sel = self.ask.scope == sc;
                    let b = egui::Button::new(RichText::new(label).small().color(if sel { p.on_primary } else { p.text_light }))
                        .fill(if sel { p.primary } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(7);
                    if ui.add(b).clicked() {
                        self.ask.scope = sc;
                    }
                }
            });
        });
        if self.ask.scope == AskScope::ThisTable && focus.is_none() {
            ui.label(RichText::new("Open a table to ask about it — until then I look in the whole database.").small().color(p.text_lighter));
        }
        ui.add_space(6.0);
        let input_h = 46.0;
        let h = (ui.available_height() - input_h).max(80.0);
        let mut acts: Vec<AskAct> = Vec::new();
        egui::ScrollArea::vertical().id_salt("askmsgs").max_height(h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if self.ask.msgs.is_empty() {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    crate::brand::lens_glyph(ui, p.primary, 40.0);
                    ui.label(RichText::new("Try one of these:").color(p.text_light));
                });
                ui.add_space(4.0);
                for ex in ["tem email nessa tabela?", "onde fica o preço?", "is there a status column?", "tem 'Honey Jar' em algum lugar?", "cadê os clientes?"] {
                    let b = egui::Button::new(RichText::new(ex).color(p.text_dark)).fill(p.panel_alt).stroke(Stroke::new(1.0, p.border)).corner_radius(14);
                    if ui.add(b).clicked() {
                        acts.push(AskAct::Example(ex.to_string()));
                    }
                }
            }
            for (mi, m) in self.ask.msgs.iter().enumerate() {
                if m.user {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        egui::Frame::new().fill(p.primary).corner_radius(CornerRadius { nw: 12, ne: 12, sw: 12, se: 3 }).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                            ui.set_max_width(220.0);
                            ui.label(RichText::new(&m.text).color(p.on_primary));
                        });
                    });
                    ui.add_space(4.0);
                    continue;
                }
                let pt = m.lang == Some(finder::Lang::Pt);
                egui::Frame::new().fill(p.panel_alt).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius { nw: 3, ne: 12, sw: 12, se: 12 }).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    rich_text(ui, &m.text, &p, p.text_dark);
                    for h in &m.hits {
                        let fr = egui::Frame::new().fill(theme::mix(p.panel_alt, p.primary, 0.06)).corner_radius(8).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                widgets::table_icon(ui, if h.table.is_view { p.secondary } else { p.primary }, h.table.is_view);
                                let label = match &h.column {
                                    Some((c, _)) => format!("{} · {c}", h.table.name),
                                    None => h.table.name.clone(),
                                };
                                let w = (ui.available_width() - 90.0).max(40.0);
                                ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                                    ui.add(egui::Label::new(RichText::new(label).monospace().color(p.text_dark)).truncate().selectable(false));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new("open ›").small().color(p.primary));
                                    if let Some((_, ty)) = &h.column {
                                        ui.label(RichText::new(ty).small().color(p.text_lighter));
                                    }
                                });
                            });
                        });
                        let r = ui.interact(fr.response.rect, ui.id().with(("askhit", mi, h.table.key(), h.column.as_ref().map(|c| c.0.clone()))), Sense::click());
                        if r.hovered() {
                            ui.painter().rect_stroke(fr.response.rect, 8.0, Stroke::new(1.0, p.primary), egui::StrokeKind::Inside);
                        }
                        if r.on_hover_text(format!("{} · {}", h.conn, h.table.key())).clicked() {
                            acts.push(AskAct::Open(h.sid, h.table.clone(), h.column.as_ref().map(|c| c.0.clone()), None));
                        }
                    }
                    if let Some(v) = &m.value {
                        if !m.searched {
                            let label = if pt { format!("🔍 Procurar “{v}” nos dados") } else { format!("🔍 Look for “{v}” in the data") };
                            if ui.add(egui::Button::new(RichText::new(label).color(p.primary)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, p.primary)).corner_radius(8)).clicked() {
                                acts.push(AskAct::SearchData(mi));
                            }
                        } else if m.pending > 0 {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new(if pt { format!("Olhando {} tabela(s)…", m.pending) } else { format!("Checking {} table(s)…", m.pending) }).small().color(p.text_light));
                            });
                        } else if m.data.is_empty() {
                            rich_text(ui, &if pt { format!("❌ “{v}” não aparece nos dados.") } else { format!("❌ “{v}” is not in the data.") }, &p, p.text);
                        } else {
                            let total: i64 = m.data.iter().map(|d| d.2).sum();
                            rich_text(
                                ui,
                                &if pt { format!("📦 “{v}” aparece em {total} linha(s), em {} tabela(s):", m.data.len()) } else { format!("📦 “{v}” shows up in {total} row(s) across {} table(s):", m.data.len()) },
                                &p,
                                p.text_dark,
                            );
                            for (dsid, t, n) in &m.data {
                                let fr = egui::Frame::new().fill(theme::mix(p.panel_alt, p.success, 0.08)).corner_radius(8).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        widgets::table_icon(ui, p.success, false);
                                        ui.label(RichText::new(&t.name).monospace().color(p.text_dark));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new("show ›").small().color(p.success));
                                            ui.label(RichText::new(format!("{n} row(s)")).small().color(p.text_light));
                                        });
                                    });
                                });
                                if ui.interact(fr.response.rect, ui.id().with(("askdata", mi, t.key())), Sense::click()).clicked() {
                                    acts.push(AskAct::Open(*dsid, t.clone(), None, Some(v.clone())));
                                }
                            }
                        }
                    }
                });
                ui.add_space(6.0);
            }
        });
        // input
        let mut send = false;
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 34.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let b = egui::Button::new(RichText::new("▶").color(p.on_primary)).fill(p.primary).corner_radius(10).min_size(Vec2::new(34.0, 30.0));
            if ui.add(b).on_hover_text("Ask  (Enter)").clicked() {
                send = true;
            }
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.ask.input)
                    .hint_text("Is there an email here?")
                    .desired_width(ui.available_width())
                    .margin(Margin::symmetric(10, 7)),
            );
            if self.ask.focus_input {
                r.request_focus();
                self.ask.focus_input = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                send = true;
                self.ask.focus_input = true;
            }
        });
        for a in acts {
            match a {
                AskAct::Open(sid, t, col, search) => self.ask_open(sid, &t, col, search),
                AskAct::SearchData(mi) => self.ask_search_data(mi),
                AskAct::Example(q) => {
                    self.ask.input = q;
                    send = true;
                }
            }
        }
        if send {
            self.ask_send();
        }
    }
}
