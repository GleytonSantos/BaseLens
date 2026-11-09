//! Window frame: icon rail, side panel, tab header, status bar, inspector and
//! the central card. Own design: a slim icon rail on the far left, a card-style
//! side panel, pill tabs with a command field on top.

use super::views::{ai_view, query_view, table_view};
use super::*;
use crate::widgets;
use eframe::egui::{CornerRadius, Margin, RichText, Sense, Stroke, Vec2};

/// Square icon button for the rail. Returns the response.
fn rail_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(40.0, 40.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 10.0, theme::mix(p.rail, p.primary, 0.22));
        let bar = egui::Rect::from_min_size(egui::pos2(rect.left() - 8.0, rect.top() + 10.0), Vec2::new(3.0, 20.0));
        painter.rect_filled(bar, 2.0, p.primary);
    } else if resp.hovered() {
        painter.rect_filled(rect, 10.0, theme::mix(p.rail, p.text_dark, 0.07));
    }
    let col = if selected { p.primary } else if resp.hovered() { p.text_dark } else { p.text_light };
    if icon == "lens" {
        crate::brand::paint_lens(painter, egui::Rect::from_center_size(rect.center(), Vec2::splat(20.0)), col);
    } else {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(18.0), col);
    }
    resp.on_hover_text(tip)
}

/// Small rounded "chip" label.
pub(super) fn chip(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new().fill(bg).corner_radius(10).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).small().strong().color(fg));
    });
}

fn section_title(ui: &mut egui::Ui, p: &Palette, t: &str) {
    ui.label(RichText::new(t).size(17.0).strong().color(p.text_dark));
}

fn avatar(ui: &mut egui::Ui, color: Color32, name: &str, size: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().rect_filled(r, size * 0.3, color);
    let letter = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(size * 0.5), Color32::from_rgb(15, 15, 25));
}

impl App {
    pub(super) fn rail(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::SidePanel::left("rail")
            .exact_width(56.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.rail).inner_margin(Margin::symmetric(8, 10)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    widgets::logo(ui, &p, 30.0);
                    ui.add_space(14.0);
                    let items = [
                        (SideTab::Connections, "🔌", "Connections  (Alt+Shift+1)"),
                        (SideTab::Tables, "🗄", "Databases & tables  (Alt+Shift+2)"),
                        (SideTab::Ask, "lens", "Ask Lens: is it in this table?  (Ctrl+Shift+K)"),
                        (SideTab::Saved, "⭐", "Saved queries  (Alt+Shift+3)"),
                        (SideTab::History, "🕓", "History  (Alt+Shift+4)"),
                    ];
                    for (tab, icon, tip) in items {
                        let sel = self.sidebar_open && self.side_tab == tab;
                        if rail_button(ui, &p, icon, tip, sel).clicked() {
                            if sel {
                                self.sidebar_open = false;
                            } else {
                                self.side_tab = tab;
                                self.sidebar_open = true;
                            }
                        }
                        ui.add_space(2.0);
                    }
                    ui.add_space(8.0);
                    let (r, _) = ui.allocate_exact_size(Vec2::new(28.0, 1.0), Sense::hover());
                    ui.painter().rect_filled(r, 0.0, p.border);
                    ui.add_space(8.0);
                    let sid = self.active_conn;
                    if rail_button(ui, &p, "✨", "AI assistant  (Ctrl+L)", self.active_kind_is(|k| matches!(k, TabKind::Ai(_)))).clicked() {
                        if let Some(s) = sid {
                            self.new_ai_tab(s);
                        } else {
                            self.toast("Connect to a database first.", false);
                        }
                    }
                    ui.add_space(2.0);
                    if rail_button(ui, &p, "⛓", "Relationship diagram  (Ctrl+Shift+E)", self.active_kind_is(|k| matches!(k, TabKind::Er(_)))).clicked() {
                        if let Some(s) = sid {
                            self.new_er_tab(s);
                        } else {
                            self.toast("Connect to a database first.", false);
                        }
                    }
                });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if rail_button(ui, &p, "⚙", "Settings  (Ctrl+,)", self.settings_ui.is_some()).clicked() {
                        self.open_settings(SettingsPage::General);
                    }
                    ui.add_space(2.0);
                    if rail_button(ui, &p, if p.dark { "☀" } else { "🌙" }, "Light / dark  (Ctrl+Shift+L)", false).clicked() {
                        self.run_command(Cmd::ToggleTheme);
                    }
                    ui.add_space(2.0);
                    if rail_button(ui, &p, "⌨", "Keyboard shortcuts  (F1)", self.show_shortcuts).clicked() {
                        self.show_shortcuts = !self.show_shortcuts;
                    }
                    if !self.settings.pin_hash.is_empty() {
                        ui.add_space(2.0);
                        if rail_button(ui, &p, "🔒", "Lock now  (Ctrl+Shift+Q)", false).clicked() {
                            self.locked = true;
                        }
                    }
                });
            });
    }

    pub(super) fn header(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::TopBottomPanel::top("header")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin { left: 6, right: 12, top: 8, bottom: 4 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let mut activate = None;
                    let mut close = None;
                    let mut new_kind: Option<u8> = None;
                    let right_w = 330.0;
                    let w = (ui.available_width() - right_w).max(120.0);
                    ui.allocate_ui(Vec2::new(w, 32.0), |ui| {
                        egui::ScrollArea::horizontal().id_salt("tabstrip").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for t in &self.tabs {
                                    let active = Some(t.id) == self.active_tab;
                                    let color = self.conn(t.conn).map(|o| o.color()).unwrap_or(p.text_light);
                                    let fr = egui::Frame::new()
                                        .fill(if active { p.panel_alt } else { Color32::TRANSPARENT })
                                        .stroke(Stroke::new(1.0, if active { p.border } else { Color32::TRANSPARENT }))
                                        .corner_radius(14)
                                        .inner_margin(Margin { left: 10, right: 6, top: 4, bottom: 4 })
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 6.0;
                                                let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                                ui.painter().circle_filled(r.center(), 4.0, color);
                                                let icon = match t.kind {
                                                    TabKind::Query(_) => "⌨",
                                                    TabKind::Table(_) => "🗄",
                                                    TabKind::Ai(_) => "✨",
                                                    TabKind::Er(_) => "⛓",
                                                };
                                                let title = RichText::new(format!("{icon}  {}", t.title)).color(if active { p.text_dark } else { p.text_light });
                                                if ui.add(egui::Label::new(title).sense(Sense::click()).selectable(false)).clicked() {
                                                    activate = Some(t.id);
                                                }
                                                // close button: a round hit area that turns red on hover
                                                let (xr, xresp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                                                let xhover = ui.rect_contains_pointer(xr);
                                                if xhover {
                                                    ui.painter().circle_filled(xr.center(), 9.0, theme::mix(p.panel_alt, p.danger, 0.25));
                                                }
                                                ui.painter().text(xr.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(14.0), if xhover { p.danger } else { p.text_lighter });
                                                if xresp.on_hover_text("Close  (Ctrl+W)").clicked() {
                                                    close = Some(t.id);
                                                }
                                                xr
                                            })
                                            .inner
                                        });
                                    let x_rect = fr.inner;
                                    self.test_rects.insert(format!("tabclose:{}", t.id), x_rect);
                                    self.test_rects.insert(format!("tab:{}", t.id), fr.response.rect);
                                    // the whole pill is clickable; a click on the × closes instead
                                    // (this area sits on top of the × and used to swallow its click)
                                    let r = ui.interact(fr.response.rect, ui.id().with(("tab", t.id)), Sense::click());
                                    if r.clicked() {
                                        let on_x = r.interact_pointer_pos().map(|pos| x_rect.expand(3.0).contains(pos)).unwrap_or(false);
                                        if on_x {
                                            close = Some(t.id);
                                        } else {
                                            activate = Some(t.id);
                                        }
                                    }
                                    if r.middle_clicked() {
                                        close = Some(t.id);
                                    }
                                    if r.hovered() && !active {
                                        ui.painter().rect_stroke(fr.response.rect, 14.0, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                                    }
                                }
                                if self.active_conn.is_some() {
                                    ui.menu_button(RichText::new(" + ").strong().color(p.text_dark), |ui| {
                                        if ui.button("⌨  New query            Ctrl+T").clicked() {
                                            new_kind = Some(0);
                                            ui.close_menu();
                                        }
                                        if ui.button("✨  AI assistant         Ctrl+L").clicked() {
                                            new_kind = Some(1);
                                            ui.close_menu();
                                        }
                                        if ui.button("⛓  Relationship diagram").clicked() {
                                            new_kind = Some(2);
                                            ui.close_menu();
                                        }
                                    });
                                }
                            });
                        });
                    });
                    if let Some(id) = activate {
                        self.active_tab = Some(id);
                    }
                    if let Some(id) = close {
                        self.close_tab(id);
                    }
                    if let (Some(k), Some(sid)) = (new_kind, self.active_conn) {
                        match k {
                            0 => {
                                self.new_query_tab(sid, String::new());
                            }
                            1 => self.new_ai_tab(sid),
                            _ => self.new_er_tab(sid),
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let js = self.settings.show_json;
                        let jb = egui::Button::new(RichText::new("{ }").monospace().color(if js { p.primary } else { p.text_light })).frame(js).corner_radius(8);
                        if ui.add(jb).on_hover_text("JSON inspector  (Ctrl+J)").clicked() {
                            self.settings.show_json = !js;
                        }
                        // command field
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(270.0, 30.0), Sense::click());
                        let fill = if resp.hovered() { theme::mix(p.panel_alt, p.primary, 0.08) } else { p.panel_alt };
                        ui.painter().rect(rect, 15.0, fill, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                        ui.painter().text(rect.left_center() + Vec2::new(12.0, 0.0), egui::Align2::LEFT_CENTER, "🔍  Search or run a command…", egui::FontId::proportional(13.0), p.text_light);
                        let kb = egui::Rect::from_min_size(egui::pos2(rect.right() - 56.0, rect.top() + 6.0), Vec2::new(46.0, 18.0));
                        ui.painter().rect_filled(kb, 5.0, theme::mix(p.panel_alt, p.text_dark, 0.08));
                        ui.painter().text(kb.center(), egui::Align2::CENTER_CENTER, "Ctrl K", egui::FontId::proportional(10.5), p.text_light);
                        if resp.on_hover_text("Command palette: tables, connections, saved queries and every action").clicked() {
                            self.quick = Some((String::new(), 0));
                        }
                    });
                });
            });
    }

    /// Debug panel (F12): last queries with their timeline, queue sizes, fps.
    pub(super) fn debug_panel(&mut self, ctx: &egui::Context) {
        if !self.dbg.show {
            return;
        }
        let p = self.pal;
        let mut open = true;
        egui::Window::new("🐞 Debug · timings")
            .open(&mut open)
            .default_size([760.0, 380.0])
            .default_pos([120.0, 120.0])
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{:.0} fps", self.dbg.fps())).strong());
                    ui.label(format!("main area drawn in {:.1} ms", self.dbg.last_draw_ms));
                    ui.label(if crate::debug::DEV_BUILD { RichText::new("development build (slow)").color(p.danger) } else { RichText::new("release build").color(p.success) });
                    for o in &self.open {
                        ui.label(format!("{}: {} waiting", o.label(), o.handle.queued()));
                    }
                });
                ui.label(RichText::new(format!("log: {}", crate::debug::log_path().display())).small().color(p.text_light));
                ui.separator();
                for c in &self.dbg.connects {
                    ui.label(RichText::new(c).small().monospace());
                }
                if !self.dbg.connects.is_empty() {
                    ui.separator();
                }
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("dbg_grid").striped(true).num_columns(10).show(ui, |ui| {
                        for h in ["queue", "behind", "server", "rows", "size", "trips", "build", "draw", "", "query"] {
                            ui.label(RichText::new(h).small().strong());
                        }
                        ui.end_row();
                        for t in &self.dbg.traces {
                            let ms = |v: f64| RichText::new(format!("{v:.1} ms")).small().monospace();
                            ui.label(ms(t.queue_ms).color(if t.queue_ms > 200.0 { p.danger } else { p.text }));
                            ui.label(RichText::new(t.queue_behind.to_string()).small());
                            ui.label(ms(t.run_ms));
                            ui.label(RichText::new(t.rows.to_string()).small());
                            ui.label(RichText::new(crate::debug::human_bytes(t.bytes)).small());
                            ui.label(RichText::new(t.batches.to_string()).small());
                            ui.label(ms(t.build_ms));
                            ui.label(t.draw_ms.map(ms).unwrap_or_else(|| RichText::new("…").small()));
                            ui.label(if t.error { RichText::new("error").small().color(p.danger) } else { RichText::new("") });
                            ui.label(RichText::new(crate::debug::one_line(&t.sql, 70)).small().monospace()).on_hover_text(format!("{} · {}", t.when, t.conn));
                            ui.end_row();
                        }
                    });
                });
            });
        if !open {
            self.dbg.show = false;
        }
    }

    pub(super) fn status_bar(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        let sid = self.active_conn;
        egui::TopBottomPanel::bottom("status")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.rail).inner_margin(Margin::symmetric(12, 4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    if let Some(o) = sid.and_then(|s| self.conn(s)) {
                        let (dot, txt) = match &o.status {
                            ConnStatus::Ready => (p.success, "Connected"),
                            ConnStatus::Connecting => (p.warning, "Connecting…"),
                            ConnStatus::Failed(_) => (p.danger, "Failed"),
                        };
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, dot);
                        ui.label(RichText::new(format!("{} · {}", o.label(), txt)).small().color(p.text));
                        ui.label(RichText::new(&o.server).small().color(p.text_light));
                        if !o.current_db.is_empty() {
                            ui.label(RichText::new(format!("db {}", o.current_db)).small().color(p.text_light));
                        }
                        if o.cfg.read_only {
                            ui.label(RichText::new("READ-ONLY").small().strong().color(p.warning));
                        }
                        if o.cfg.ssl && o.cfg.ssl_verify {
                            ui.label(RichText::new("🔒 TLS").small().color(p.success));
                        } else if o.cfg.ssl {
                            ui.label(RichText::new("🔓 TLS not verified").small().color(p.warning)).on_hover_text(
                                "Encrypted, but the server's certificate isn't checked: someone on the network could pretend to be the server. Tick “Verify certificate” in the connection when possible.",
                            );
                        }
                        if o.cfg.read_only && o.cfg.kind == crate::db::DbKind::Mysql {
                            ui.label(RichText::new("best effort").small().color(p.text_lighter)).on_hover_text(
                                "MySQL read-only: BaseLens only sends reads and asks the server for read-only transactions, but MySQL doesn't block every schema change that way. For a guarantee, use a MySQL user with only SELECT rights.",
                            );
                        }
                    } else {
                        ui.label(RichText::new("Not connected").small().color(p.text_light));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("F1 shortcuts · Ctrl+K commands").small().color(p.text_lighter));
                        if crate::debug::on() {
                            let b = ui.add(egui::Button::new(RichText::new(format!("🐞 debug · {:.0} fps", self.dbg.fps())).small().color(p.warning)).frame(false));
                            if b.on_hover_text("Debug mode is on (F12 shows the timings)").clicked() {
                                self.dbg.show = !self.dbg.show;
                            }
                        }
                        // the unoptimised build is several times slower (TLS especially)
                        if crate::debug::DEV_BUILD && !crate::store::robot_mode() {
                            ui.label(RichText::new("DEV BUILD · slow").small().strong().color(p.danger)).on_hover_text(
                                "This is the development build (target/debug): no optimisations, so TLS and big results are much slower. Use target/release/baselens to compare speed.",
                            );
                        }
                        let ai = &self.settings.ai;
                        let loc = if ai.provider.is_local() {
                            "local"
                        } else if ai.provider == Provider::ClaudeCode {
                            "CLI"
                        } else {
                            "cloud"
                        };
                        let pname = ai.provider.label().split(" (").next().unwrap_or("");
                        ui.label(RichText::new(format!("✨ {pname} · {} ({loc})", ai.model_label())).small().color(p.text_light));
                        if let Some(t) = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id)) {
                            let info = match &t.kind {
                                TabKind::Query(q) => q.results.get(q.active).map(|r| {
                                    if r.columns.is_empty() {
                                        format!("{} affected · {:.1} ms", r.affected.unwrap_or(0), r.elapsed_ms)
                                    } else {
                                        format!("{} rows{} · {:.1} ms", r.rows.len(), if r.truncated { "+" } else { "" }, r.elapsed_ms)
                                    }
                                }),
                                TabKind::Table(tt) => tt.data.as_ref().map(|d| {
                                    format!("{} rows shown{} · {:.1} ms", d.rows.len(), tt.total.map(|n| format!(" of {n}")).unwrap_or_default(), d.elapsed_ms)
                                }),
                                _ => None,
                            };
                            if let Some(i) = info {
                                ui.label(RichText::new(i).small().color(p.text));
                            }
                        }
                    });
                });
            });
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        if !self.sidebar_open {
            return;
        }
        let p = self.pal;
        egui::SidePanel::left("sidebar")
            .default_width(SIDEBAR_W + 20.0)
            .min_width(SIDEBAR_MIN_W + 30.0)
            .max_width((ctx.screen_rect().width() * PANEL_MAX_SHARE).clamp(200.0, 520.0))
            .resizable(true)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.sidebar).inner_margin(Margin { left: 12, right: 12, top: 12, bottom: 8 }).stroke(Stroke::new(1.0, p.border)))
            .show(ctx, |ui| match self.side_tab {
                SideTab::Connections => self.side_connections(ui),
                SideTab::Tables => self.side_navigator(ui),
                SideTab::Ask => self.side_ask(ui),
                SideTab::Saved | SideTab::History => {
                    let Some(sid) = self.active_conn else {
                        section_title(ui, &p, match self.side_tab {
                            SideTab::Tables => "Tables",
                            SideTab::Saved => "Saved queries",
                            _ => "History",
                        });
                        ui.add_space(20.0);
                        ui.label(RichText::new("Connect to a database to see this.").color(p.text_light));
                        if ui.button("Choose a connection").clicked() {
                            self.side_tab = SideTab::Connections;
                        }
                        return;
                    };
                    match self.side_tab {
                        SideTab::Saved => self.side_saved(ui, sid),
                        _ => self.side_history(ui, sid),
                    }
                }
            });
    }

    fn side_connections(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            section_title(ui, &p, "Connections");
        });
        ui.add_space(6.0);
        let new = egui::Button::new(RichText::new("+  New connection").strong().color(p.on_primary)).fill(p.primary).min_size(Vec2::new(ui.available_width(), 32.0)).corner_radius(10);
        if ui.add(new).on_hover_text("Ctrl+Shift+N").clicked() {
            self.run_command(Cmd::NewConnection);
        }
        ui.add_space(8.0);
        let mut do_connect = None;
        let mut do_edit = None;
        let mut do_delete = None;
        let mut do_dup = None;
        let mut act = None;
        let mut close = None;
        egui::ScrollArea::vertical().id_salt("connlist").auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("SAVED").small().strong().color(p.text_lighter));
            for c in &self.connections {
                let open_n = self.open.iter().filter(|o| o.cfg.id == c.id).count();
                let col = Color32::from_rgb(c.color[0], c.color[1], c.color[2]);
                let fr = egui::Frame::new()
                    .fill(p.panel_alt)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(10)
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            avatar(ui, col, &c.name, 30.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.horizontal(|ui| {
                                    ui.add(egui::Label::new(RichText::new(&c.name).strong().color(p.text_dark)).truncate().selectable(false));
                                    if open_n > 0 {
                                        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                        ui.painter().circle_filled(r.center(), 3.5, p.success);
                                    }
                                });
                                ui.add(egui::Label::new(RichText::new(format!("{} · {}", c.kind.label(), short_path(&c.subtitle()))).small().color(p.text_light)).truncate().selectable(false));
                            });
                        });
                    });
                let resp = ui.interact(fr.response.rect, ui.id().with(("conn", c.id)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_stroke(fr.response.rect, 10.0, Stroke::new(1.5, p.primary), egui::StrokeKind::Inside);
                }
                if resp.clicked() {
                    do_connect = Some((c.clone(), false));
                }
                resp.on_hover_text("Click to connect · right-click for more").context_menu(|ui| {
                    if ui.button("Connect").clicked() {
                        do_connect = Some((c.clone(), false));
                        ui.close_menu();
                    }
                    if ui.button("Connect another instance").clicked() {
                        do_connect = Some((c.clone(), true));
                        ui.close_menu();
                    }
                    if ui.button("Edit").clicked() {
                        do_edit = Some(c.clone());
                        ui.close_menu();
                    }
                    if ui.button("Duplicate").clicked() {
                        do_dup = Some(c.clone());
                        ui.close_menu();
                    }
                    if ui.button(RichText::new("Delete").color(p.danger)).clicked() {
                        do_delete = Some(c.id);
                        ui.close_menu();
                    }
                });
                ui.add_space(2.0);
            }
            if !self.open.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new("OPEN NOW").small().strong().color(p.text_lighter));
                for o in &self.open {
                    let sel = Some(o.sid) == self.active_conn;
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 18.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, o.color());
                        let mut label = o.label();
                        if !o.current_db.is_empty() && o.cfg.kind != DbKind::Sqlite {
                            label.push_str(&format!(" / {}", o.current_db));
                        }
                        if ui.selectable_label(sel, label).clicked() {
                            act = Some(o.sid);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("⏏").small().frame(false)).on_hover_text("Disconnect").clicked() {
                                close = Some(o.sid);
                            }
                        });
                    });
                }
            }
        });
        if let Some((c, n)) = do_connect {
            self.connect(c, n);
        }
        if let Some(c) = do_edit {
            let pw = store::decrypt(&c.password_enc);
            self.editor = Some(ConnEditor { cfg: c, password: pw, is_new: false, test: None, test_msg: None });
        }
        if let Some(mut c) = do_dup {
            c.id = self.id();
            c.name = format!("{} (copy)", c.name);
            self.connections.push(c);
            self.persist();
        }
        if let Some(id) = do_delete {
            self.connections.retain(|c| c.id != id);
            self.persist();
        }
        if let Some(s) = act {
            self.active_conn = Some(s);
            self.side_tab = SideTab::Tables;
            if let Some(t) = self.tabs.iter().find(|t| t.conn == s) {
                self.active_tab = Some(t.id);
            }
        }
        if let Some(s) = close {
            self.close_conn(s);
        }
    }

    fn side_saved(&mut self, ui: &mut egui::Ui, sid: u64) {
        let p = self.pal;
        section_title(ui, &p, "Saved queries");
        ui.add_space(6.0);
        let mut open = None;
        let mut del = None;
        if self.saved.is_empty() {
            ui.label(RichText::new("Nothing saved yet. Use “Save” in a query tab.").color(p.text_light));
        }
        egui::ScrollArea::vertical().id_salt("saved").auto_shrink([false, false]).show(ui, |ui| {
            for (i, s) in self.saved.iter().enumerate() {
                let fr = egui::Frame::new().fill(p.panel_alt).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("⭐ {}", s.name)).strong().color(p.text_dark));
                    let first: String = s.sql.lines().next().unwrap_or("").chars().take(60).collect();
                    ui.add(egui::Label::new(RichText::new(first).small().monospace().color(p.text_light)).truncate().selectable(false));
                });
                let r = ui.interact(fr.response.rect, ui.id().with(("sv", i)), Sense::click());
                if r.clicked() {
                    open = Some(s.sql.clone());
                }
                r.on_hover_text(&s.sql).context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        del = Some(i);
                        ui.close_menu();
                    }
                });
                ui.add_space(2.0);
            }
        });
        if let Some(sql) = open {
            self.new_query_tab(sid, sql);
        }
        if let Some(i) = del {
            self.saved.remove(i);
            store::save("saved_queries.json", &self.saved);
        }
    }

    fn side_history(&mut self, ui: &mut egui::Ui, sid: u64) {
        let p = self.pal;
        section_title(ui, &p, "History");
        ui.add_space(6.0);
        let mut open = None;
        if self.history.is_empty() {
            ui.label(RichText::new("Queries you run show up here.").color(p.text_light));
        }
        egui::ScrollArea::vertical().id_salt("history").auto_shrink([false, false]).show(ui, |ui| {
            for (i, h) in self.history.iter().take(200).enumerate() {
                let first = h.sql.lines().find(|l| !l.trim().starts_with("--")).unwrap_or("").chars().take(60).collect::<String>();
                let bg = ui.painter().add(egui::Shape::Noop);
                let r = ui
                    .vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(first).monospace().color(p.text_dark)).truncate().selectable(false));
                        ui.label(RichText::new(format!("{} · {} rows · {:.0} ms · {}", h.when, h.rows, h.ms, h.conn)).small().color(p.text_lighter));
                    })
                    .response;
                let r = ui.interact(r.rect, ui.id().with(("h", i)), Sense::click());
                if r.hovered() {
                    ui.painter().set(bg, egui::Shape::rect_filled(r.rect.expand(4.0), 8.0, theme::mix(p.sidebar, p.text_dark, 0.06)));
                }
                if r.clicked() {
                    open = Some(h.sql.clone());
                }
                ui.add_space(6.0);
            }
        });
        if let Some(sql) = open {
            self.new_query_tab(sid, sql);
        }
    }

    pub(super) fn json_sidebar(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::SidePanel::right("json")
            .default_width(SIDEBAR_W + 30.0)
            .min_width(SIDEBAR_MIN_W + 30.0)
            .max_width((ctx.screen_rect().width() * PANEL_MAX_SHARE).clamp(200.0, 560.0))
            .resizable(true)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin { left: 0, right: 12, top: 0, bottom: 8 }))
            .show(ctx, |ui| {
                egui::Frame::new().fill(p.panel_alt).stroke(Stroke::new(1.0, p.border)).corner_radius(12).inner_margin(12).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("{ }").monospace().strong().color(p.primary));
                        ui.label(RichText::new("JSON inspector").strong().color(p.text_dark));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("×").frame(false).small()).on_hover_text("Hide  (Ctrl+J)").clicked() {
                                self.settings.show_json = false;
                            }
                        });
                    });
                    let sel = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id)).and_then(|t| match &t.kind {
                        TabKind::Query(q) => q.grid.selected.and_then(|(r, c)| q.results.get(q.active).map(|res| (res.clone(), r, c, None))),
                        TabKind::Table(tt) => tt.grid.selected.and_then(|(r, c)| tt.data.as_ref().map(|d| (d.clone(), r, c, Some(tt.edits.clone())))),
                        _ => None,
                    });
                    let Some((mut res, r, c, edits)) = sel else {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("{ }").monospace().size(28.0).color(p.text_lighter));
                            ui.label(RichText::new("Click any row to inspect it here. JSON stored in text columns opens as a tree.").color(p.text_light));
                        });
                        return;
                    };
                    if r >= res.rows.len() {
                        return;
                    }
                    if let Some(e) = edits {
                        for ((er, ec), v) in e {
                            if er == r {
                                res.rows[r][ec] = v;
                            }
                        }
                    }
                    let v = widgets::row_json(&res, r);
                    ui.add_space(4.0);
                    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 28.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Copy").on_hover_text("Copy row as JSON").clicked() {
                            ui.ctx().copy_text(serde_json::to_string_pretty(&v).unwrap_or_default());
                            self.toast = Some(("Row copied as JSON".into(), Instant::now(), true));
                        }
                        let w = (ui.available_width() - 10.0).max(40.0);
                        ui.add(egui::TextEdit::singleline(&mut self.json_filter).hint_text("🔍  Filter keys / values").desired_width(w));
                    });
                    ui.horizontal(|ui| {
                        chip(ui, &format!("row {}", r + 1), p.primary, p.selection);
                        if let Some(n) = res.columns.get(c) {
                            chip(ui, n, p.secondary, theme::mix(p.panel_alt, p.secondary, 0.15));
                        }
                    });
                    ui.add_space(4.0);
                    egui::Frame::new().fill(p.editor_bg).corner_radius(8).inner_margin(8).show(ui, |ui| {
                        egui::ScrollArea::both().id_salt("jsontree").auto_shrink([false, false]).show(ui, |ui| {
                            if let Some(cell) = res.rows[r].get(c) {
                                if let Some(j) = cell.json() {
                                    widgets::json_tree(ui, &res.columns[c], &j, "$cell", 0, &p, &self.json_filter);
                                    ui.separator();
                                }
                            }
                            widgets::json_tree(ui, "row", &v, "$", 0, &p, &self.json_filter);
                        });
                    });
                });
            });
    }

    pub(super) fn central(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.active_tab else {
            self.welcome(ui);
            return;
        };
        let Some(idx) = self.tabs.iter().position(|t| t.id == id) else {
            self.active_tab = None;
            return;
        };
        let sid = self.tabs[idx].conn;
        let p = self.pal;
        let mut actions: Vec<Action> = Vec::new();
        let read_only = self.conn(sid).map(|o| o.cfg.read_only).unwrap_or(false);
        let status_msg = self.conn(sid).and_then(|o| match &o.status {
            ConnStatus::Connecting => Some("Connecting…".to_string()),
            ConnStatus::Failed(e) => Some(format!("Connection failed: {e}")),
            _ => None,
        });
        let page_size = self.settings.page_size.max(1);
        let ai_settings = self.settings.ai.clone();
        let schema = if matches!(self.tabs[idx].kind, TabKind::Query(_)) { self.schema_info(sid) } else { SchemaInfo::default() };
        self.breadcrumb(ui, id);
        ui.add_space(4.0);
        // sampled values of the open table's columns, for the filter rows
        let table_vals: HashMap<String, Vec<(String, i64)>> = match &self.tabs[idx].kind {
            TabKind::Table(tt) => {
                let prefix = format!("{}.", tt.table.to_lowercase());
                self.values_cache
                    .iter()
                    .filter(|((s, k), _)| *s == sid && k.starts_with(&prefix))
                    .map(|((_, k), v)| (k[prefix.len()..].to_string(), v.clone()))
                    .collect()
            }
            _ => HashMap::new(),
        };
        let tab = &mut self.tabs[idx];
        egui::Frame::new()
            .fill(p.panel_alt)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                if let Some(msg) = status_msg {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(msg).color(p.text_light).size(15.0)));
                }
                match &mut tab.kind {
                    TabKind::Query(q) => query_view(ui, q, id, &p, &mut actions, &schema),
                    TabKind::Table(tt) => table_view(ui, tt, id, &p, &mut actions, page_size, read_only, &table_vals),
                    TabKind::Ai(a) => ai_view(ui, a, id, &p, &mut actions, &ai_settings),
                    TabKind::Er(er) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("⛓  Relationships").strong().size(16.0).color(p.text_dark));
                            ui.label(RichText::new("Drag tables by their title; drag the background to move around.").color(p.text_light));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("⟳ Reload").clicked() {
                                    er.loaded = false;
                                    actions.push(Action::ReloadEr);
                                }
                            });
                        });
                        ui.add_space(6.0);
                        widgets::er_diagram(ui, er, &p);
                    }
                }
            });
        for a in actions {
            self.do_action(id, sid, a);
        }
    }

    pub(super) fn do_action(&mut self, tid: u64, sid: u64, a: Action) {
        match a {
            Action::Run(cur) => self.run_query_tab(tid, cur),
            Action::Reload => self.load_table(tid),
            Action::Save => self.save_table(tid),
            Action::SaveQuery(sql) => self.save_query = Some((tid, sql)),
            Action::AiSend => self.ai_send(tid),
            Action::AiRun(idx, sql) => {
                if let Err(e) = self.send(sid, sql, Purpose::AiRun(tid, idx), 200) {
                    if let Some(t) = self.tab_mut(tid) {
                        if let TabKind::Ai(a) = &mut t.kind {
                            a.runs.insert(idx, Err(e));
                        }
                    }
                }
            }
            Action::OpenInEditor(sql) => {
                self.new_query_tab(sid, sql);
            }
            Action::ShowJson => self.settings.show_json = true,
            Action::Argon2 { check, cell } => {
                use crate::argon2tool::{Mode, Tool};
                let mut t = Tool::new(if check.is_some() { Mode::Verify } else { Mode::Hash });
                t.check_hash = check.unwrap_or_default();
                t.target = cell.map(|(r, c)| (tid, r, c));
                self.argon2 = Some(t);
            }
            Action::Export(json) => {
                let res = self.tabs.iter().find(|t| t.id == tid).and_then(|t| match &t.kind {
                    TabKind::Query(q) => q.results.get(q.active).cloned(),
                    TabKind::Table(tt) => tt.data.clone(),
                    _ => None,
                });
                if let Some(r) = res {
                    self.export(&r, json);
                }
            }
            Action::AddRow => {
                let cols = self.tabs.iter().find(|t| t.id == tid).and_then(|t| match &t.kind {
                    TabKind::Table(tt) => tt.cols.clone(),
                    _ => None,
                });
                if let Some(cols) = cols {
                    let n = cols.len();
                    self.new_row = Some(NewRow { tab: tid, cols, values: vec![String::new(); n] });
                }
            }
            Action::ReloadEr => {
                let tag = self.id();
                if let Some(o) = self.conn(sid) {
                    let _ = o.handle.send(Request::Graph { tag });
                }
            }
            Action::Settings => self.open_settings(SettingsPage::Ai),
            Action::FetchValues(t, c) => self.fetch_values(sid, &t, &c),
        }
    }

    fn export(&mut self, r: &QueryResult, json: bool) {
        let name = if json { "result.json" } else { "result.csv" };
        let picked = if store::robot_mode() { Some(store::data_dir().join(name)) } else { rfd::FileDialog::new().set_file_name(name).save_file() };
        let Some(path) = picked else { return };
        let body = if json {
            serde_json::to_string_pretty(&widgets::result_json(r, usize::MAX)).unwrap_or_default()
        } else {
            let esc = |s: String| if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s };
            let mut out = r.columns.iter().cloned().map(esc).collect::<Vec<_>>().join(",") + "\n";
            for row in &r.rows {
                out.push_str(&row.iter().map(|c| if c.is_null() { String::new() } else { esc(c.display()) }).collect::<Vec<_>>().join(","));
                out.push('\n');
            }
            out
        };
        match std::fs::write(&path, body) {
            Ok(_) => self.toast(format!("Exported to {}", path.display()), true),
            Err(e) => self.toast(format!("Export failed: {e}"), false),
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(36.0);
            let w = ui.available_width().min(760.0);
            let pad = ((ui.available_width() - w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(w);
                    // hero banner with a soft accent gradient
                    let bg = ui.painter().add(egui::Shape::Noop);
                    let hero = egui::Frame::new().inner_margin(Margin::symmetric(22, 20)).show(ui, |ui| {
                        ui.set_width(w - 44.0);
                        ui.horizontal(|ui| {
                            widgets::logo(ui, &p, 52.0);
                            ui.add_space(6.0);
                            ui.vertical(|ui| {
                                crate::brand::wordmark(ui, &p, 32.0);
                                ui.label(RichText::new(crate::brand::TAGLINE).size(15.0).color(p.text_dark));
                                ui.label(RichText::new("All your databases in one place. Ask Lens where things are, filter without SQL.").color(p.text));
                            });
                        });
                    });
                    let hr = hero.response.rect;
                    let left = theme::mix(p.panel_alt, p.primary, if p.dark { 0.30 } else { 0.18 });
                    let right = theme::mix(p.panel_alt, p.secondary, if p.dark { 0.22 } else { 0.14 });
                    let mut mesh = egui::Mesh::default();
                    mesh.colored_vertex(hr.left_top(), left);
                    mesh.colored_vertex(hr.right_top(), right);
                    mesh.colored_vertex(hr.right_bottom(), right);
                    mesh.colored_vertex(hr.left_bottom(), left);
                    mesh.add_triangle(0, 1, 2);
                    mesh.add_triangle(0, 2, 3);
                    ui.painter().set(bg, egui::Shape::mesh(mesh));
                    ui.painter().rect_stroke(hr, 14.0, Stroke::new(6.0, p.panel_alt), egui::StrokeKind::Outside);
                    ui.painter().rect_stroke(hr, 14.0, Stroke::new(1.0, theme::mix(p.border, p.primary, 0.4)), egui::StrokeKind::Inside);
                    ui.add_space(22.0);
                    // quick actions
                    let actions: [(&str, &str, &str, u8); 4] = [
                        ("+", "New connection", "Ctrl+Shift+N", 0),
                        ("▶", "Open the demo", "SQLite sample data", 1),
                        ("🔍", "Command palette", "Ctrl+K", 2),
                        ("⌨", "Shortcuts", "F1", 3),
                    ];
                    let mut clicked = None;
                    ui.horizontal(|ui| {
                        let cw = (w - 3.0 * 10.0) / 4.0;
                        for (i, (icon, title, sub, code)) in actions.iter().enumerate() {
                            let primary = i == 0;
                            let fr = egui::Frame::new()
                                .fill(if primary { p.primary } else { p.panel_alt })
                                .stroke(Stroke::new(1.0, if primary { p.primary } else { p.border }))
                                .corner_radius(12)
                                .inner_margin(14)
                                .show(ui, |ui| {
                                    ui.set_width(cw - 28.0);
                                    let fg = if primary { p.on_primary } else { p.text_dark };
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(*icon).size(20.0).color(if primary { p.on_primary } else { p.primary }));
                                        ui.label(RichText::new(*title).strong().color(fg));
                                        ui.label(RichText::new(*sub).small().color(if primary { theme::mix(p.on_primary, p.primary, 0.3) } else { p.text_light }));
                                    });
                                });
                            let r = ui.interact(fr.response.rect, ui.id().with(("qa", i)), Sense::click());
                            if r.hovered() {
                                ui.painter().rect_stroke(fr.response.rect, 12.0, Stroke::new(2.0, p.secondary), egui::StrokeKind::Inside);
                            }
                            if r.clicked() {
                                clicked = Some(*code);
                            }
                        }
                    });
                    match clicked {
                        Some(0) => self.run_command(Cmd::NewConnection),
                        Some(1) => {
                            if let Some(c) = self.connections.first().cloned() {
                                self.connect(c, false);
                            }
                        }
                        Some(2) => self.quick = Some((String::new(), 0)),
                        Some(3) => self.show_shortcuts = true,
                        _ => {}
                    }
                    ui.add_space(26.0);
                    ui.label(RichText::new("RECENT").small().strong().color(p.text_lighter));
                    ui.add_space(4.0);
                    let mut recent: Vec<ConnConfig> = Vec::new();
                    for c in self.settings.recent.iter().filter_map(|id| self.connections.iter().find(|c| c.id == *id)).chain(self.connections.iter()) {
                        if !recent.iter().any(|x| x.id == c.id) {
                            recent.push(c.clone());
                        }
                    }
                    recent.truncate(RECENT_LIMIT);
                    let mut go = None;
                    egui::Grid::new("recent").num_columns(2).spacing([10.0, 10.0]).show(ui, |ui| {
                        for (i, c) in recent.iter().enumerate() {
                            let col = Color32::from_rgb(c.color[0], c.color[1], c.color[2]);
                            let fr = egui::Frame::new().fill(p.panel_alt).stroke(Stroke::new(1.0, p.border)).corner_radius(12).inner_margin(12).show(ui, |ui| {
                                ui.set_width(w / 2.0 - 30.0);
                                ui.horizontal(|ui| {
                                    avatar(ui, col, &c.name, 34.0);
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&c.name).strong().color(p.text_dark));
                                        ui.add(egui::Label::new(RichText::new(format!("{} · {}", c.kind.label(), short_path(&c.subtitle()))).small().color(p.text_light)).truncate());
                                    });
                                });
                            });
                            let r = ui.interact(fr.response.rect, ui.id().with(("w", c.id)), Sense::click());
                            if r.hovered() {
                                ui.painter().rect_stroke(fr.response.rect, 12.0, Stroke::new(1.5, p.primary), egui::StrokeKind::Inside);
                            }
                            if r.clicked() {
                                go = Some(c.clone());
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                    if let Some(c) = go {
                        self.connect(c, false);
                    }
                });
            });
        });
    }

    pub(super) fn lock_screen(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ctx, |ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.vertical_centered(|ui| {
                widgets::logo(ui, &p, 52.0);
                ui.add_space(6.0);
                ui.label(RichText::new(format!("{} is locked", crate::brand::NAME)).size(20.0).color(p.text_dark));
                ui.label(RichText::new("Enter your PIN to continue.").color(p.text_light));
                ui.add_space(8.0);
                let r = ui.add(egui::TextEdit::singleline(&mut self.pin_input).password(true).desired_width(200.0).hint_text("PIN"));
                r.request_focus();
                let go = ui.add(egui::Button::new(RichText::new("Unlock").color(p.on_primary)).fill(p.primary)).clicked() || ui.input(|i| i.key_pressed(Key::Enter));
                if go {
                    let (ok, upgrade) = store::verify_pin(&self.pin_input, &self.settings.pin_salt, &self.settings.pin_hash);
                    if ok {
                        if upgrade {
                            // an old SHA-256 hash: replace it with Argon2id
                            self.settings.pin_hash = store::hash_pin_argon2(&self.pin_input);
                            self.settings.pin_salt = String::new();
                            store::save("settings.json", &self.settings);
                        }
                        self.locked = false;
                        self.pin_error = false;
                        self.last_activity = Instant::now();
                    } else {
                        self.pin_error = true;
                    }
                    self.pin_input.clear();
                }
                if self.pin_error {
                    ui.label(RichText::new("Wrong PIN").color(p.danger));
                }
            });
        });
    }
}
