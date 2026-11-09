//! Floating windows: connection editor, settings, quick search, add row,
//! save query and the toast message.

use super::*;
use eframe::egui::{RichText, Vec2};

impl App {
    pub(super) fn windows(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        if let Some((msg, t, ok)) = &self.toast {
            if t.elapsed().as_secs_f32() < 4.0 {
                let (msg, ok) = (msg.clone(), *ok);
                egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -36.0]).show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(if ok { p.success } else { p.danger })
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_max_width(420.0);
                            ui.label(RichText::new(msg).color(p.on_primary));
                        });
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            } else {
                self.toast = None;
            }
        }
        self.conn_editor_window(ctx);
        self.settings_window(ctx);
        self.new_row_window(ctx);
        self.quick_search(ctx);
        self.shortcuts_window(ctx);
        self.save_query_window(ctx);
    }

    fn save_query_window(&mut self, ctx: &egui::Context) {
        let Some((_, sql)) = self.save_query.clone() else { return };
        let id = egui::Id::new("sqname");
        let mut name = ctx.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
        let mut close = false;
        egui::Window::new("Save query").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut name);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() && !name.trim().is_empty() {
                    self.saved.insert(0, SavedQuery { name: name.trim().to_string(), sql: sql.clone() });
                    store::save("saved_queries.json", &self.saved);
                    self.side_tab = SideTab::Saved;
                    close = true;
                    name.clear();
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        ctx.data_mut(|d| d.insert_temp(id, name));
        if close {
            self.save_query = None;
        }
    }

    fn quick_search(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        let Some((q, sel)) = self.quick.clone() else { return };
        let mut q2 = q;
        let mut sel = sel;
        let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let mut pick: Option<QuickItem> = None;
        // build and rank the items
        let f = q2.trim().to_lowercase();
        let matches = |text: &str| -> Option<u32> {
            if f.is_empty() {
                return Some(5);
            }
            let t = text.to_lowercase();
            if t.starts_with(&f) {
                Some(0)
            } else if t.split([' ', '_', '.']).any(|w| w.starts_with(&f)) {
                Some(1)
            } else if t.contains(&f) {
                Some(2)
            } else {
                let mut it = t.chars();
                if f.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|x| x == c)) {
                    Some(4)
                } else {
                    None
                }
            }
        };
        let mut items: Vec<(u32, u32, QuickItem)> = Vec::new();
        for c in Cmd::ALL {
            if let Some(s) = matches(c.label()) {
                items.push((if f.is_empty() { 0 } else { 1 }, s, QuickItem::Command(c)));
            }
        }
        for o in &self.open {
            for t in &o.tables {
                if let Some(s) = matches(&t.key()) {
                    items.push((if f.is_empty() { 1 } else { 0 }, s, QuickItem::Table(o.sid, t.clone(), o.label())));
                }
            }
        }
        for c in &self.connections {
            if let Some(s) = matches(&c.name) {
                items.push((2, s, QuickItem::Conn(c.clone())));
            }
        }
        for s in &self.saved {
            if let Some(sc) = matches(&s.name) {
                items.push((2, sc, QuickItem::Saved(s.sql.clone(), s.name.clone())));
            }
        }
        items.sort_by_key(|x| (x.1, x.0));
        let items: Vec<QuickItem> = items.into_iter().map(|x| x.2).take(40).collect();
        let n = items.len();
        ctx.input_mut(|i| {
            if n > 0 && i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                sel = (sel + 1) % n;
            }
            if n > 0 && i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                sel = (sel + n - 1) % n;
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                pick = items.get(sel.min(n.saturating_sub(1))).cloned();
            }
        });
        // dim the window behind
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("palette_dim")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(if p.dark { 120 } else { 60 }));
            if resp.clicked() {
                close = true;
            }
        });
        egui::Area::new(egui::Id::new("palette")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, 90.0]).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(p.panel_alt).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(14).inner_margin(10).show(ui, |ui| {
                ui.set_width(560.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut q2)
                        .hint_text("Type a command, a table, a connection…")
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::proportional(16.0))
                        .margin(egui::Margin::symmetric(10, 8)),
                );
                r.request_focus();
                if r.changed() {
                    sel = 0;
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    if items.is_empty() {
                        ui.label(RichText::new("Nothing found").color(p.text_light));
                    }
                    for (i, it) in items.iter().enumerate() {
                        let (icon, title, hint) = match it {
                            QuickItem::Command(c) => ("›", c.label().to_string(), c.keys_label()),
                            QuickItem::Table(_, t, c) => (if t.is_view { "◫" } else { "🗄" }, t.key(), format!("table · {c}")),
                            QuickItem::Conn(c) => ("🔌", c.name.clone(), "connect".to_string()),
                            QuickItem::Saved(_, nm) => ("⭐", nm.clone(), "saved query".to_string()),
                        };
                        let is_sel = i == sel;
                        let fr = egui::Frame::new().fill(if is_sel { p.selection } else { Color32::TRANSPARENT }).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).color(if is_sel { p.primary } else { p.text_light }));
                                ui.label(RichText::new(title).color(p.text_dark));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if matches!(it, QuickItem::Command(_)) {
                                        egui::Frame::new().fill(theme::mix(p.panel_alt, p.text_dark, 0.07)).corner_radius(5).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                                            ui.label(RichText::new(&hint).small().monospace().color(p.text_light));
                                        });
                                    } else {
                                        ui.label(RichText::new(&hint).small().color(p.text_lighter));
                                    }
                                });
                            });
                        });
                        let resp = ui.interact(fr.response.rect, ui.id().with(("pal", i)), egui::Sense::click());
                        if resp.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                            sel = i;
                        }
                        if resp.clicked() {
                            pick = Some(it.clone());
                        }
                        if is_sel {
                            resp.scroll_to_me(None);
                        }
                    }
                });
                ui.separator();
                ui.label(RichText::new("⬆⬇ move · Enter open · Esc close").small().color(p.text_lighter));
            });
        });
        if let Some(it) = pick {
            close = true;
            self.quick = None;
            match it {
                QuickItem::Command(c) => self.run_command(c),
                QuickItem::Table(sid, t, _) => self.open_table(sid, &t),
                QuickItem::Conn(c) => self.connect(c, false),
                QuickItem::Saved(sql, _) => {
                    if let Some(sid) = self.active_conn {
                        self.new_query_tab(sid, sql);
                    }
                }
            }
        }
        if close {
            self.quick = None;
        } else {
            self.quick = Some((q2, sel));
        }
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let p = self.pal;
        let mut open = true;
        egui::Window::new("⌨  Keyboard shortcuts").open(&mut open).collapsible(false).resizable(false).default_width(640.0).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(RichText::new("Everything is also in the command palette (Ctrl+K). In the editor: Ctrl+Space suggestions, Tab/Enter to accept.").color(p.text_light));
            ui.add_space(6.0);
            egui::Grid::new("kb").num_columns(4).spacing([18.0, 6.0]).striped(true).show(ui, |ui| {
                let all: Vec<Cmd> = Cmd::ALL.to_vec();
                let extra = [("Command palette", "Ctrl+K · Ctrl+P"), ("Switch to tab 1–9", "Alt+1…9"), ("Next / previous page", "Ctrl+→ / Ctrl+←"), ("Edit cell", "Double-click")];
                let mut rows: Vec<(String, String)> = all.iter().map(|c| (c.label().to_string(), c.keys_label())).collect();
                rows.extend(extra.iter().map(|(a, b)| (a.to_string(), b.to_string())));
                for pair in rows.chunks(2) {
                    for (label, keys) in pair {
                        ui.label(RichText::new(label).color(p.text_dark));
                        egui::Frame::new().fill(theme::mix(p.panel_alt, p.text_dark, 0.07)).corner_radius(5).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                            ui.label(RichText::new(keys).small().monospace().color(p.primary));
                        });
                    }
                    ui.end_row();
                }
            });
        });
        if !open {
            self.show_shortcuts = false;
        }
    }

    fn new_row_window(&mut self, ctx: &egui::Context) {
        let Some(nr) = &mut self.new_row else { return };
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("Add row").collapsible(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("Leave a field empty to use the column's default value.");
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                egui::Grid::new("newrow").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    for (i, c) in nr.cols.iter().enumerate() {
                        ui.label(format!("{}{}  ({})", if c.pk { "🔑 " } else { "" }, c.name, c.data_type.to_lowercase()));
                        ui.add(egui::TextEdit::singleline(&mut nr.values[i]).desired_width(260.0));
                        ui.end_row();
                    }
                });
            });
            ui.horizontal(|ui| {
                if ui.button("Insert").clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if submit {
            let Some(nr) = self.new_row.take() else { return };
            let Some(t) = self.tabs.iter().find(|t| t.id == nr.tab) else { return };
            let sid = t.conn;
            let TabKind::Table(tt) = &t.kind else { return };
            let kind = self.conn(sid).map(|o| o.cfg.kind).unwrap_or_default();
            let q = Self::qualified(kind, &tt.schema, &tt.table);
            let pairs: Vec<(String, String)> = nr
                .cols
                .iter()
                .zip(nr.values.iter())
                .filter(|(_, v)| !v.is_empty())
                .map(|(c, v)| (kind.quote(&c.name), if v == "NULL" { "NULL".to_string() } else { db::sql_string(v) }))
                .collect();
            let sql = if pairs.is_empty() {
                if kind == DbKind::Mysql {
                    format!("INSERT INTO {q} () VALUES ()")
                } else {
                    format!("INSERT INTO {q} DEFAULT VALUES")
                }
            } else {
                format!(
                    "INSERT INTO {q} ({}) VALUES ({})",
                    pairs.iter().map(|x| x.0.clone()).collect::<Vec<_>>().join(", "),
                    pairs.iter().map(|x| x.1.clone()).collect::<Vec<_>>().join(", ")
                )
            };
            if let Err(e) = self.send(sid, sql, Purpose::Insert(nr.tab), 1) {
                self.toast(e, false);
            }
        } else if cancel {
            self.new_row = None;
        }
    }

    fn conn_editor_window(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        if self.editor.is_none() {
            self.diag_lines.clear();
            self.diag_rx = None;
            self.show_pw = false;
            return;
        }
        if let Some(rx) = &self.diag_rx {
            if let Ok(lines) = rx.try_recv() {
                self.diag_lines = lines;
                self.diag_rx = None;
            }
        }
        let diag_running = self.diag_rx.is_some();
        if diag_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let diag_lines = self.diag_lines.clone();
        let mut show_pw = self.show_pw;
        let mut diagnose = false;
        let Some(ed) = &mut self.editor else { return };
        let mut save = false;
        let mut connect = false;
        let mut cancel = false;
        let mut test = false;
        egui::Window::new(if ed.is_new { "New connection" } else { "Edit connection" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Grid::new("connform").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut ed.cfg.name).hint_text("My database").desired_width(300.0));
                    ui.end_row();
                    // paste a whole connection URL instead of filling every field
                    ui.label("URL");
                    ui.horizontal(|ui| {
                        let id = egui::Id::new("conn_url_paste");
                        let mut url = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
                        ui.add(egui::TextEdit::singleline(&mut url).hint_text("postgresql://user:password@host:5433/database").desired_width(230.0));
                        if ui.add_enabled(!url.trim().is_empty(), egui::Button::new("Fill")).on_hover_text("Fill type, host, port, user, password and database from the URL").clicked() {
                            match db::parse_connection_url(&url) {
                                Ok((c, pw)) => {
                                    let name = ed.cfg.name.clone();
                                    let (id_keep, color) = (ed.cfg.id, ed.cfg.color);
                                    ed.cfg = ConnConfig { id: id_keep, name, color, save_password: true, ..c };
                                    if !pw.is_empty() {
                                        ed.password = pw;
                                    }
                                    ed.test_msg = Some((true, "Filled from the URL.".into()));
                                    url.clear();
                                }
                                Err(e) => ed.test_msg = Some((false, e)),
                            }
                        }
                        ui.data_mut(|d| d.insert_temp(id, url));
                    });
                    ui.end_row();
                    ui.label("Type");
                    let before = ed.cfg.kind;
                    egui::ComboBox::from_id_salt("kind").selected_text(ed.cfg.kind.label()).width(300.0).show_ui(ui, |ui| {
                        for k in DbKind::ALL {
                            ui.selectable_value(&mut ed.cfg.kind, k, k.label());
                        }
                    });
                    // only replace the port if it was the previous type's default
                    if before != ed.cfg.kind && (ed.cfg.port == before.default_port() || ed.cfg.port == 0) {
                        ed.cfg.port = ed.cfg.kind.default_port();
                    }
                    ui.end_row();
                    if ed.cfg.kind == DbKind::Sqlite {
                        ui.label("File");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.cfg.path).desired_width(200.0).hint_text("/path/to/file.db"));
                            if ui.button("Browse…").clicked() {
                                if let Some(f) = if store::robot_mode() { None } else { rfd::FileDialog::new().pick_file() } {
                                    ed.cfg.path = f.to_string_lossy().to_string();
                                }
                            }
                            if ui.button("New…").on_hover_text("Create a new empty database file").clicked() {
                                if let Some(f) = if store::robot_mode() { None } else { rfd::FileDialog::new().set_file_name("new.db").save_file() } {
                                    ed.cfg.path = f.to_string_lossy().to_string();
                                }
                            }
                        });
                        ui.end_row();
                    } else {
                        ui.label("Host");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.cfg.host).desired_width(190.0));
                            ui.label("Port");
                            // a plain text box: easier to type 5433 than to drag a number
                            let id = egui::Id::new(("portbuf", ed.cfg.id));
                            let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| ed.cfg.port.to_string());
                            let r = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(64.0).char_limit(5));
                            buf.retain(|c| c.is_ascii_digit());
                            if r.changed() {
                                if let Ok(v) = buf.parse::<u16>() {
                                    if v > 0 {
                                        ed.cfg.port = v;
                                    }
                                }
                            }
                            if !r.has_focus() {
                                buf = ed.cfg.port.to_string();
                            }
                            ui.data_mut(|d| d.insert_temp(id, buf));
                        });
                        ui.end_row();
                        ui.label("User");
                        ui.add(egui::TextEdit::singleline(&mut ed.cfg.user).desired_width(300.0));
                        ui.end_row();
                        ui.label("Password");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.password).password(!show_pw).desired_width(170.0));
                            let eye = egui::Button::new(if show_pw { "🙈" } else { "👁" }).frame(false);
                            if ui.add(eye).on_hover_text("Show / hide the password").clicked() {
                                show_pw = !show_pw;
                            }
                            ui.checkbox(&mut ed.cfg.save_password, "Save (encrypted)");
                        });
                        ui.end_row();
                        // what is really in the box (the field is masked)
                        ui.label("");
                        {
                            let n = ed.password.chars().count();
                            let mut notes = vec![if n == 0 { "empty".to_string() } else { format!("{n} characters") }];
                            if ed.password != ed.password.trim() {
                                notes.push("⚠ space at the start or end".into());
                            }
                            if let Some(t) = db::password_host_tail(&ed.password) {
                                notes.push(format!("⚠ ends with “{t}” — looks like part of an address was pasted"));
                            }
                            if ed.password.chars().any(|c| "'\"`´~^¨".contains(c)) {
                                notes.push("has accent/quote keys — check with 👁 (dead keys)".into());
                            }
                            let warn = notes.len() > 1;
                            ui.label(RichText::new(notes.join(" · ")).small().color(if warn { p.warning } else { p.text_lighter }));
                        }
                        ui.end_row();
                        ui.label("Database");
                        ui.add(egui::TextEdit::singleline(&mut ed.cfg.database).desired_width(300.0).hint_text("optional"));
                        ui.end_row();
                        ui.label("Security");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut ed.cfg.ssl, "Use TLS/SSL");
                            ui.add_enabled(ed.cfg.ssl, egui::Checkbox::new(&mut ed.cfg.ssl_verify, "Verify certificate"));
                        });
                        ui.end_row();
                        if ed.cfg.ssl && !ed.cfg.ssl_verify {
                            ui.label("");
                            ui.label(RichText::new("⚠ Encrypted but not verified (like sslmode=require): someone on the network could pretend to be the server and read the password. Verify the certificate when you can.").small().color(p.warning));
                            ui.end_row();
                        }
                    }
                    ui.label("Mode");
                    ui.checkbox(&mut ed.cfg.read_only, "Read-only (block changes)");
                    ui.end_row();
                    if ed.cfg.read_only && ed.cfg.kind == DbKind::Mysql {
                        ui.label("");
                        ui.label(RichText::new("MySQL: best effort. For a guarantee, connect with a user that only has SELECT rights.").small().color(p.text_light));
                        ui.end_row();
                    }
                    ui.label("Color");
                    ui.horizontal(|ui| {
                        for s in SWATCHES {
                            let col = Color32::from_rgb(s[0], s[1], s[2]);
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::click());
                            ui.painter().circle_filled(r.center(), 8.0, col);
                            if ed.cfg.color == s {
                                ui.painter().circle_stroke(r.center(), 9.5, egui::Stroke::new(2.0_f32, p.text_dark));
                            }
                            if resp.clicked() {
                                ed.cfg.color = s;
                            }
                        }
                    });
                    ui.end_row();
                });
                if let Some((ok, m)) = &ed.test_msg {
                    ui.label(RichText::new(m).color(if *ok { p.success } else { p.danger }));
                }
                if ed.test.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Testing…");
                    });
                }
                if diag_running || !diag_lines.is_empty() {
                    egui::Frame::new().fill(theme::mix(p.panel_alt, p.text_dark, 0.04)).corner_radius(8).inner_margin(8).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("Step-by-step check").strong().color(p.text_dark));
                        for (ok, line) in &diag_lines {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(if *ok { "✔" } else { "✖" }).color(if *ok { p.success } else { p.danger }));
                                ui.label(RichText::new(line).small().color(p.text));
                            });
                        }
                        if diag_running {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Checking…").small().color(p.text_light));
                            });
                        }
                    });
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Test").clicked() {
                        test = true;
                    }
                    if ed.cfg.kind == DbKind::Postgres && ui.button("🔎 Diagnose").on_hover_text("Check step by step: address, server, login type, password, login").clicked() {
                        diagnose = true;
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("Save & connect").color(p.on_primary)).fill(p.primary)).clicked() {
                            save = true;
                            connect = true;
                        }
                        if ui.button("Save").clicked() {
                            save = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            });
        if test {
            ed.test_msg = None;
            ed.test = Some(db::spawn(ed.cfg.clone(), ed.password.clone()));
        }
        if diagnose {
            let (tx, rx) = std::sync::mpsc::channel();
            let (cfg, pw) = (ed.cfg.clone(), ed.password.clone());
            std::thread::spawn(move || {
                let _ = tx.send(db::pg_diagnose(&cfg, &pw));
            });
            self.diag_rx = Some(rx);
            self.diag_lines.clear();
        }
        self.show_pw = show_pw;
        if save {
            let Some(mut ed) = self.editor.take() else { return };
            if ed.cfg.name.trim().is_empty() {
                ed.cfg.name = match ed.cfg.kind {
                    DbKind::Sqlite => short_path(&ed.cfg.path),
                    _ => format!("{}@{}", ed.cfg.user, ed.cfg.host),
                };
            }
            ed.cfg.password_enc = if ed.cfg.save_password { store::encrypt(&ed.password) } else { String::new() };
            if ed.cfg.id == 0 {
                ed.cfg.id = self.id();
            }
            if let Some(c) = self.connections.iter_mut().find(|c| c.id == ed.cfg.id) {
                *c = ed.cfg.clone();
            } else {
                self.connections.push(ed.cfg.clone());
            }
            self.persist();
            if connect {
                self.spawn_conn(ed.cfg, ed.password);
            }
        } else if cancel {
            self.editor = None;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        let Some(su) = &mut self.settings_ui else { return };
        let mut open = true;
        let mut changed = false;
        let sys_dark = self.system_dark;
        let mut forget_learned = false;
        let s = &mut self.settings;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut su.page, SettingsPage::General, "General");
                    ui.selectable_value(&mut su.page, SettingsPage::Ai, "AI");
                    ui.selectable_value(&mut su.page, SettingsPage::Security, "Security");
                });
                ui.separator();
                match su.page {
                    SettingsPage::General => {
                        ui.label(RichText::new("Theme").strong());
                        ui.horizontal(|ui| {
                            for t in ThemeKind::ALL {
                                let tp = theme::palette(t.resolve(sys_dark));
                                let fr = egui::Frame::new()
                                    .fill(tp.bg)
                                    .stroke(egui::Stroke::new(if s.theme == t { 2.0_f32 } else { 1.0_f32 }, if s.theme == t { p.primary } else { p.border }))
                                    .corner_radius(6)
                                    .inner_margin(8)
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                for c in [tp.primary, tp.secondary, tp.success, tp.pink] {
                                                    let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                                                    ui.painter().circle_filled(r.center(), 5.0, c);
                                                }
                                            });
                                            ui.label(RichText::new(t.label()).color(tp.text_dark));
                                        });
                                    });
                                if ui.interact(fr.response.rect, ui.id().with(("th", t.label())), egui::Sense::click()).clicked() {
                                    s.theme = t;
                                    changed = true;
                                }
                            }
                        });
                        ui.add_space(8.0);
                        changed |= ui.add(egui::Slider::new(&mut s.font_scale, 0.85..=1.4).text("Text size")).changed();
                        changed |= ui.add(egui::Slider::new(&mut s.page_size, 25..=1000).text("Rows per table page")).changed();
                        changed |= ui.add(egui::Slider::new(&mut s.max_results, 100..=200000).logarithmic(true).text("Max rows per query")).changed();
                    }
                    SettingsPage::Ai => {
                        let a = &mut s.ai;
                        ui.label(RichText::new("Provider").strong());
                        egui::ComboBox::from_id_salt("prov").selected_text(a.provider.label()).width(320.0).show_ui(ui, |ui| {
                            for pr in Provider::ALL {
                                changed |= ui.selectable_value(&mut a.provider, pr, pr.label()).changed();
                            }
                        });
                        let note = if a.provider.is_local() {
                            "Runs on your machine: your data and schema never leave this computer."
                        } else if a.provider == Provider::ClaudeCode {
                            "Uses the Claude Code program installed on this computer (with your Claude login)."
                        } else {
                            "Cloud service: table names, columns and your questions are sent to the provider."
                        };
                        ui.label(RichText::new(note).small().color(if a.provider.is_local() { p.success } else { p.warning }));
                        ui.add_space(6.0);
                        egui::Grid::new("aigrid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                            match a.provider {
                                Provider::Ollama => {
                                    ui.label("Server");
                                    changed |= ui.text_edit_singleline(&mut a.ollama_url).changed();
                                    ui.end_row();
                                    ui.label("Model");
                                    ui.horizontal(|ui| {
                                        let shown = if a.ollama_model.is_empty() { "(first available)".to_string() } else { a.ollama_model.clone() };
                                        egui::ComboBox::from_id_salt("om").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                                            for m in &su.ollama_models {
                                                changed |= ui.selectable_value(&mut a.ollama_model, m.clone(), m).changed();
                                            }
                                        });
                                        if ui.button("⟳ Load models").clicked() {
                                            match ai::ollama_models(&a.ollama_url) {
                                                Ok(m) => {
                                                    su.msg = format!("{} model(s) found", m.len());
                                                    su.ollama_models = m;
                                                }
                                                Err(e) => su.msg = e,
                                            }
                                        }
                                    });
                                    ui.end_row();
                                }
                                Provider::OpenAiCompat => {
                                    ui.label("Base URL");
                                    changed |= ui.text_edit_singleline(&mut a.compat_url).changed();
                                    ui.end_row();
                                    ui.label("Model");
                                    changed |= ui.text_edit_singleline(&mut a.compat_model).changed();
                                    ui.end_row();
                                    ui.label("API key");
                                    let k = su.keys.entry("compat").or_default();
                                    if ui.add(egui::TextEdit::singleline(k).password(true).hint_text("optional")).changed() {
                                        a.compat_key_enc = store::encrypt(k);
                                        changed = true;
                                    }
                                    ui.end_row();
                                }
                                Provider::ClaudeCode => {
                                    ui.label("Program path");
                                    ui.horizontal(|ui| {
                                        changed |= ui.add(egui::TextEdit::singleline(&mut a.claude_path).desired_width(250.0)).changed();
                                        if ui.button("Detect").clicked() {
                                            match ai::detect_claude() {
                                                Some(pth) => {
                                                    a.claude_path = pth;
                                                    changed = true;
                                                }
                                                None => su.msg = "Claude Code was not found in the usual places.".into(),
                                            }
                                        }
                                        if ui.button("Check").clicked() {
                                            su.claude_ver = ai::claude_version(&a.claude_path).unwrap_or_else(|e| format!("Error: {e}"));
                                        }
                                    });
                                    ui.end_row();
                                    ui.label("Model (optional)");
                                    changed |= ui.add(egui::TextEdit::singleline(&mut a.claude_model).hint_text("e.g. sonnet, opus")).changed();
                                    ui.end_row();
                                    if !su.claude_ver.is_empty() {
                                        ui.label("Version");
                                        ui.label(su.claude_ver.clone());
                                        ui.end_row();
                                    }
                                }
                                Provider::Anthropic | Provider::OpenAi | Provider::Google => {
                                    let prov = a.provider;
                                    let (keyname, models): (&'static str, &[&str]) = match prov {
                                        Provider::Anthropic => ("anthropic", &ai::ANTHROPIC_MODELS),
                                        Provider::OpenAi => ("openai", &ai::OPENAI_MODELS),
                                        _ => ("google", &ai::GOOGLE_MODELS),
                                    };
                                    let model = match prov {
                                        Provider::Anthropic => &mut a.anthropic_model,
                                        Provider::OpenAi => &mut a.openai_model,
                                        _ => &mut a.google_model,
                                    };
                                    ui.label("Model");
                                    ui.horizontal(|ui| {
                                        changed |= ui.add(egui::TextEdit::singleline(model).desired_width(200.0)).changed();
                                        egui::ComboBox::from_id_salt("cm").selected_text("▾").width(30.0).show_ui(ui, |ui| {
                                            for m in models {
                                                if ui.selectable_label(model.as_str() == *m, *m).clicked() {
                                                    *model = m.to_string();
                                                    changed = true;
                                                }
                                            }
                                        });
                                    });
                                    ui.end_row();
                                    ui.label("API key");
                                    let k = su.keys.entry(keyname).or_default();
                                    if ui.add(egui::TextEdit::singleline(k).password(true).desired_width(300.0)).changed() {
                                        let enc = store::encrypt(k);
                                        match prov {
                                            Provider::Anthropic => a.anthropic_key_enc = enc,
                                            Provider::OpenAi => a.openai_key_enc = enc,
                                            _ => a.google_key_enc = enc,
                                        }
                                        changed = true;
                                    }
                                    ui.end_row();
                                }
                            }
                            ui.label("Temperature");
                            changed |= ui.add(egui::Slider::new(&mut a.temperature, 0.0..=1.0)).changed();
                            ui.end_row();
                        });
                        changed |= ui.checkbox(&mut a.allow_read_only_auto, "Run read-only queries from the AI without asking").changed();
                        ui.label(RichText::new("Only on connections marked read-only, and never queries that read server files, sleep or open network links. Queries that change data always ask first.").small().color(p.text_light));
                        ui.label("Extra instructions for the AI");
                        changed |= ui.add(egui::TextEdit::multiline(&mut a.custom_instructions).desired_rows(3).desired_width(f32::INFINITY)).changed();
                        if !su.msg.is_empty() {
                            ui.label(RichText::new(&su.msg).small().color(p.text_light));
                        }
                    }
                    SettingsPage::Security => {
                        ui.label(RichText::new("How your data is protected").strong());
                        for line in [
                            "• Passwords and API keys are encrypted (ChaCha20-Poly1305) with a master key kept in your system keyring.",
                            "• Settings files are private to your user (permissions 600).",
                            "• No tracking, no telemetry, no account needed.",
                            "• Read-only connections block any query that changes data (MySQL: best effort).",
                            "• TLS/SSL with certificate checking for PostgreSQL and MySQL; an automatic switch to TLS always checks it.",
                            "• The app-lock PIN is stored as an Argon2id hash.",
                            "• Loading native SQLite extensions is turned off.",
                        ] {
                            ui.label(RichText::new(line).color(p.text));
                        }
                        let backend = store::key_backend();
                        let col = if backend == store::KeyBackend::Basic { p.warning } else { p.success };
                        ui.label(RichText::new(format!("Master key stored in: {}", backend.label())).color(col));
                        if backend == store::KeyBackend::Basic {
                            ui.label(RichText::new("⚠ No system keyring was found, so the master key is a private file next to your encrypted settings. Anyone who can read your home folder (a copied disk, a backup) can decrypt your passwords. Install and unlock a keyring (GNOME Keyring or KWallet) to protect them.").small().color(p.warning));
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new("App lock").strong());
                        if s.pin_hash.is_empty() {
                            ui.label(format!("Set a PIN (at least {MIN_PIN_LEN} characters) to lock the app."));
                        } else {
                            ui.label(RichText::new("PIN is set.").color(p.success));
                        }
                        egui::Grid::new("pin").num_columns(2).show(ui, |ui| {
                            ui.label("New PIN");
                            ui.add(egui::TextEdit::singleline(&mut su.pin1).password(true));
                            ui.end_row();
                            ui.label("Repeat PIN");
                            ui.add(egui::TextEdit::singleline(&mut su.pin2).password(true));
                            ui.end_row();
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Set PIN").clicked() {
                                if su.pin1.chars().count() < MIN_PIN_LEN {
                                    su.msg = format!("PIN must have at least {MIN_PIN_LEN} characters.");
                                } else if su.pin1 != su.pin2 {
                                    su.msg = "The two PINs are different.".into();
                                } else {
                                    s.pin_salt = String::new();
                                    s.pin_hash = store::hash_pin_argon2(&su.pin1);
                                    su.pin1.clear();
                                    su.pin2.clear();
                                    su.msg = "PIN saved.".into();
                                    changed = true;
                                }
                            }
                            if !s.pin_hash.is_empty() && ui.button("Remove PIN").clicked() {
                                s.pin_hash.clear();
                                s.pin_salt.clear();
                                su.msg = "PIN removed.".into();
                                changed = true;
                            }
                        });
                        ui.add_space(8.0);
                        ui.label(RichText::new("Smarter suggestions").strong());
                        changed |= ui.checkbox(&mut s.remember_suggestions, "Remember names and values from my queries (stored encrypted, only on this computer)").changed();
                        if ui.button("Forget what was learned").clicked() {
                            forget_learned = true;
                        }
                        ui.add_space(8.0);
                        changed |= ui.checkbox(&mut s.lock_on_idle, "Lock when idle").changed();
                        let mut mins = (s.idle_secs / 60).max(1);
                        if ui.add(egui::Slider::new(&mut mins, 1..=120).text("minutes idle")).changed() {
                            s.idle_secs = mins * 60;
                            changed = true;
                        }
                        if !su.msg.is_empty() {
                            ui.label(RichText::new(&su.msg).color(p.warning));
                        }
                    }
                }
            });
        if forget_learned {
            self.learned.clear();
            store::save("learned.json", &String::new());
            self.toast("Learned suggestions removed.", true);
        }
        if changed {
            self.persist();
        }
        if !open {
            self.settings_ui = None;
        }
    }
}
