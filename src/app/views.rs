//! Tab contents: query editor (with autocomplete), table data (with
//! point-and-click filters), AI assistant.

use super::*;
use crate::assist::SugKind;
use crate::filter::{Op, Rule};
use crate::widgets::{self, GridEdits, GridOut};
use eframe::egui::{Margin, RichText, Sense, Stroke, Vec2};

/// Segmented control for Grid / Chart / JSON.
fn view_switch(ui: &mut egui::Ui, v: &mut View, p: &Palette) {
    egui::Frame::new().fill(theme::mix(p.panel_alt, p.text_dark, 0.05)).corner_radius(9).inner_margin(2).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // keep left-to-right order even inside right-aligned toolbars
        let mut items = [(View::Grid, "▦ Grid"), (View::Chart, "📊 Chart"), (View::Json, "{ } JSON")];
        if ui.layout().main_dir() == egui::Direction::RightToLeft {
            items.reverse();
        }
        for (val, label) in items {
            let sel = *v == val;
            let b = egui::Button::new(RichText::new(label).color(if sel { p.on_primary } else { p.text_light }))
                .fill(if sel { p.primary } else { Color32::TRANSPARENT })
                .stroke(Stroke::NONE)
                .corner_radius(7);
            if ui.add(b).clicked() {
                *v = val;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn result_body(
    ui: &mut egui::Ui,
    id: u64,
    res: &QueryResult,
    view: View,
    grid: &mut GridState,
    chart: &mut ChartCfg,
    edits: Option<GridEdits>,
    p: &Palette,
    offset: usize,
    sort: Option<(usize, bool)>,
    pks: &[usize],
    actions: &mut Vec<Action>,
) -> GridOut {
    match view {
        View::Grid => {
            let out = widgets::grid(ui, ("grid", id), res, grid, edits, p, offset, sort, pks);
            if out.open_json {
                actions.push(Action::ShowJson);
            }
            if let Some((check, cell)) = out.argon2.clone() {
                actions.push(Action::Argon2 { check, cell });
            }
            out
        }
        View::Chart => {
            widgets::chart(ui, &format!("chart{id}"), res, chart, p);
            GridOut::default()
        }
        View::Json => {
            let v = widgets::result_json(res, 1000);
            egui::Frame::new().fill(p.editor_bg).corner_radius(8).inner_margin(8).show(ui, |ui| {
                egui::ScrollArea::both().id_salt(("rj", id)).auto_shrink([false, false]).show(ui, |ui| {
                    widgets::json_tree(ui, "rows", &v, &format!("$r{id}"), 0, p, "");
                });
            });
            GridOut::default()
        }
    }
}

fn primary_button(p: &Palette, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).strong().color(p.on_primary)).fill(p.primary).corner_radius(8)
}

fn ghost_button(p: &Palette, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(p.text_dark)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, p.border)).corner_radius(8)
}

fn set_cursor(ctx: &egui::Context, ed_id: egui::Id, pos: usize) {
    if let Some(mut st) = egui::TextEdit::load_state(ctx, ed_id) {
        let c = egui::text::CCursor::new(pos);
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(c)));
        st.store(ctx, ed_id);
    }
}

fn apply_completion(ctx: &egui::Context, ed_id: egui::Id, q: &mut QueryTab, idx: usize) {
    let Some(item) = q.ac.items.get(idx).cloned() else { return };
    let chars: Vec<char> = q.sql.chars().collect();
    let cur = q.cursor.0.min(chars.len());
    let start = q.ac.start.min(cur);
    let mut new: String = chars[..start].iter().collect();
    new.push_str(&item.insert);
    new.extend(chars[cur..].iter());
    q.sql = new;
    let pos = start + item.insert.chars().count();
    set_cursor(ctx, ed_id, pos);
    q.cursor = (pos, pos);
    q.ac.open = false;
    q.ac.closed_by = "accepted";
    q.ac.forced = false;
    q.ac.items.clear();
    q.ac.last_text_len = q.sql.len();
    ctx.memory_mut(|m| m.request_focus(ed_id));
}

fn kind_color(p: &Palette, k: SugKind) -> Color32 {
    match k {
        SugKind::Column => p.secondary,
        SugKind::Table => p.primary,
        SugKind::View => p.info,
        SugKind::Keyword => p.pink,
        SugKind::Function => p.warning,
        SugKind::Snippet => p.success,
        SugKind::Value => p.success,
        SugKind::Learned => p.purple,
    }
}

pub(super) fn query_view(ui: &mut egui::Ui, q: &mut QueryTab, id: u64, p: &Palette, actions: &mut Vec<Action>, schema: &SchemaInfo) {
    let ed_id = egui::Id::new(("sqled", id));
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        if ui.add(primary_button(p, "▶  Run")).on_hover_text("Run all, or the selection  (Ctrl+Enter / F5)").clicked() {
            actions.push(Action::Run(false));
        }
        if ui.add(ghost_button(p, "Run current")).on_hover_text("Run the statement under the cursor  (Ctrl+Shift+Enter)").clicked() {
            actions.push(Action::Run(true));
        }
        ui.add_space(6.0);
        if ui.add(ghost_button(p, "✨ Format")).on_hover_text("Tidy up the SQL  (Ctrl+Shift+F)").clicked() {
            q.editor_op = Some(EditorOp::Format);
        }
        if ui.add(ghost_button(p, "-- Comment")).on_hover_text("Comment / uncomment the line  (Ctrl+/)").clicked() {
            q.editor_op = Some(EditorOp::Comment);
        }
        if ui.add(ghost_button(p, "⭐ Save")).on_hover_text("Keep this query in Saved").clicked() {
            actions.push(Action::SaveQuery(q.sql.clone()));
        }
        if q.running {
            ui.spinner();
            let secs = q.started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
            let txt = if q.stop_asked.is_some() { "Stopping…".to_string() } else if secs >= 1 { format!("Running… {secs} s") } else { "Running…".to_string() };
            ui.label(RichText::new(txt).color(p.text_light));
            if q.stop_asked.is_none() && ui.add(ghost_button(p, "■ Stop")).on_hover_text("Stop this query on the server").clicked() {
                actions.push(Action::StopQuery);
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("⬇ Export", |ui| {
                if ui.button("CSV   Ctrl+E").clicked() {
                    actions.push(Action::Export(false));
                    ui.close_menu();
                }
                if ui.button("JSON  Ctrl+Shift+J").clicked() {
                    actions.push(Action::Export(true));
                    ui.close_menu();
                }
            });
        });
    });
    ui.add_space(6.0);

    // ---- editor operations requested by buttons / shortcuts
    if let Some(op) = q.editor_op.take() {
        let (a, b) = (q.cursor.0.min(q.cursor.1), q.cursor.0.max(q.cursor.1));
        match op {
            EditorOp::Format => {
                let chars: Vec<char> = q.sql.chars().collect();
                if a != b && b <= chars.len() {
                    let sel: String = chars[a..b].iter().collect();
                    let f = assist::format_sql(&sel);
                    let mut new: String = chars[..a].iter().collect();
                    new.push_str(f.trim_end());
                    new.extend(chars[b..].iter());
                    q.sql = new;
                } else {
                    q.sql = assist::format_sql(&q.sql);
                }
                set_cursor(&ctx, ed_id, q.sql.chars().count());
            }
            EditorOp::Comment => {
                let (t, cur) = assist::toggle_comment(&q.sql, a, b);
                q.sql = t;
                set_cursor(&ctx, ed_id, cur);
            }
            EditorOp::Complete => {
                set_cursor(&ctx, ed_id, q.cursor.0);
                q.ac.open = true;
                q.ac.forced = true;
                let (start, items) = assist::suggest(&q.sql, q.cursor.0, schema, true);
                q.ac.start = start;
                q.ac.items = items;
                q.ac.sel = 0;
            }
        }
        ctx.memory_mut(|m| m.request_focus(ed_id));
    }

    // ---- autocomplete keys (before the editor sees them)
    let mut accept = None;
    if q.ac.open && !q.ac.items.is_empty() {
        let n = q.ac.items.len();
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                q.ac.sel = (q.ac.sel + 1) % n;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                q.ac.sel = (q.ac.sel + n - 1) % n;
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) {
                accept = Some(q.ac.sel);
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                q.ac.open = false;
                q.ac.closed_by = "escape";
                q.ac.forced = false;
            }
        });
    }
    if let Some(idx) = accept {
        apply_completion(&ctx, ed_id, q, idx);
    }

    let total_h = ui.available_height();
    let ed_h = (total_h * q.split).clamp(70.0, (total_h - 90.0).max(70.0));
    // MECHANICS: editorFontSize default 14 (scaled with the text-size setting)
    let scale = egui::TextStyle::Body.resolve(ui.style()).size / 13.5;
    let font = egui::FontId::monospace(EDITOR_FONT_SIZE * scale);
    let pal = *p;
    let mut layouter = move |ui: &egui::Ui, text: &str, wrap: f32| {
        let job = widgets::sql_job(text, &pal, font.clone(), wrap);
        ui.fonts(|f| f.layout_job(job))
    };
    let mut caret_pos: Option<egui::Pos2> = None;
    let mut has_focus = false;
    egui::Frame::new().fill(p.editor_bg).corner_radius(10).stroke(Stroke::new(1.0, p.border)).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(("ed", id)).max_height(ed_h).min_scrolled_height(ed_h).auto_shrink([false, false]).show(ui, |ui| {
            let out = egui::TextEdit::multiline(&mut q.sql)
                .id(ed_id)
                .code_editor()
                .frame(false)
                .desired_width(f32::INFINITY)
                .desired_rows(((ed_h / 18.0) as usize).max(3))
                .hint_text("-- Write SQL here.  Ctrl+Space suggests tables and columns, Ctrl+Enter runs.")
                .layouter(&mut layouter)
                .show(ui);
            has_focus = out.response.has_focus();
            // where the editor is, for the test robot
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("robot_editor_rect"), out.response.rect));
            if let Some(cr) = out.cursor_range {
                q.cursor = (cr.primary.ccursor.index, cr.secondary.ccursor.index);
                let r = out.galley.pos_from_cursor(&cr.primary);
                caret_pos = Some(out.galley_pos + r.left_bottom().to_vec2() + Vec2::new(0.0, 4.0));
            }
            if out.response.changed() {
                // open while typing a word or right after a dot
                let chars: Vec<char> = q.sql.chars().collect();
                let c = q.cursor.0.min(chars.len());
                let before = if c > 0 { Some(chars[c - 1]) } else { None };
                let grew = q.sql.len() > q.ac.last_text_len;
                let in_value = assist::value_target(&q.sql, q.cursor.0, schema).is_some();
                match before {
                    // typing the value side of `col = …`, `IN (…`, `LIKE …`
                    Some(_) if in_value && grew => q.ac.open = true,
                    Some(ch) if (ch.is_alphanumeric() || ch == '_' || ch == '.') && grew => q.ac.open = true,
                    Some(ch) if ch.is_alphanumeric() || ch == '_' => {}
                    _ => {
                        q.ac.open = false;
                        q.ac.closed_by = "typed a separator";
                        q.ac.forced = false;
                    }
                }
                q.ac.sel = 0;
            }
            q.ac.last_text_len = q.sql.len();
        });
    });
    let mut value_loading: Option<String> = None;
    if q.ac.open {
        let (start, items) = assist::suggest(&q.sql, q.cursor.0, schema, q.ac.forced);
        q.ac.start = start;
        q.ac.items = items;
        if let Some(vt) = assist::value_target(&q.sql, q.cursor.0, schema) {
            // ask the database for this column's values the first time
            actions.push(Action::FetchValues(vt.table.clone(), vt.column.clone()));
            if !schema.values.contains_key(&vt.key) && q.ac.items.is_empty() && has_focus {
                value_loading = Some(format!("{}.{}", vt.table, vt.column));
            }
        }
        // close only when focus really went somewhere else: the editor's own
        // response can miss its focus for a single frame (seen by the test
        // robot), which used to close the popup in the middle of typing
        let focus_away = !has_focus && ctx.memory(|m| m.focused() != Some(ed_id));
        if (q.ac.items.is_empty() && value_loading.is_none()) || focus_away {
            q.ac.open = false;
            q.ac.closed_by = if focus_away { "editor lost focus" } else { "no suggestions" };
        }
        q.ac.sel = q.ac.sel.min(q.ac.items.len().saturating_sub(1));
    }
    if let (Some(what), Some(pos)) = (&value_loading, caret_pos) {
        egui::Area::new(egui::Id::new(("acload", id))).order(egui::Order::Foreground).fixed_pos(pos).show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(p.panel_alt).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(8).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Loading values of {what}…")).small().color(p.text_light));
                });
            });
        });
    }
    // ---- suggestion popup
    if q.ac.open && !q.ac.items.is_empty() {
        if let Some(pos) = caret_pos {
            let mut clicked = None;
            egui::Area::new(egui::Id::new(("ac", id))).order(egui::Order::Foreground).fixed_pos(pos).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(p.panel_alt).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(6).show(ui, |ui| {
                    ui.set_width(360.0);
                    for (i, s) in q.ac.items.iter().enumerate() {
                        let sel = i == q.ac.sel;
                        let fr = egui::Frame::new().fill(if sel { p.selection } else { Color32::TRANSPARENT }).corner_radius(6).inner_margin(Margin::symmetric(6, 3)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let kc = kind_color(p, s.kind);
                                egui::Frame::new().fill(theme::mix(p.panel_alt, kc, 0.2)).corner_radius(4).inner_margin(Margin::symmetric(4, 0)).show(ui, |ui| {
                                    ui.set_min_width(28.0);
                                    ui.label(RichText::new(s.kind.badge()).small().monospace().color(kc));
                                });
                                ui.label(RichText::new(&s.label).monospace().color(p.text_dark));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.add(egui::Label::new(RichText::new(&s.detail).small().color(p.text_lighter)).truncate());
                                });
                            });
                        });
                        let r = ui.interact(fr.response.rect, ui.id().with(("acrow", i)), Sense::click());
                        if r.clicked() {
                            clicked = Some(i);
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new("⬆⬇ choose · Enter / Tab insert · Esc close").small().color(p.text_lighter));
                });
            });
            if let Some(i) = clicked {
                apply_completion(&ctx, ed_id, q, i);
            }
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Ctrl+Space suggest · Ctrl+Shift+F format · Ctrl+/ comment · Ctrl+Enter run").small().color(p.text_lighter));
    });
    // drag handle between editor and results
    let (hr, hresp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::drag());
    let mid = hr.center();
    ui.painter().line_segment([mid - Vec2::new(22.0, 0.0), mid + Vec2::new(22.0, 0.0)], Stroke::new(3.0, if hresp.hovered() { p.primary } else { p.border }));
    if hresp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if hresp.dragged() && total_h > 0.0 {
        q.split = ((ed_h + hresp.drag_delta().y) / total_h).clamp(0.12, 0.85);
    }
    if let Some(e) = &q.error {
        egui::Frame::new().fill(theme::mix(p.panel_alt, p.danger, 0.14)).corner_radius(8).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("⚠  {e}")).color(p.danger));
        });
        return;
    }
    if q.results.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("▶").size(26.0).color(p.text_lighter));
            ui.label(RichText::new("Run a query to see results here.").color(p.text_light));
        });
        return;
    }
    ui.horizontal(|ui| {
        if q.results.len() > 1 {
            for (i, r) in q.results.iter().enumerate() {
                let label = if r.columns.is_empty() {
                    format!("Result {} · {} affected", i + 1, r.affected.unwrap_or(0))
                } else {
                    format!("Result {} · {} rows", i + 1, r.rows.len())
                };
                if ui.selectable_label(q.active == i, label).clicked() {
                    q.active = i;
                    q.grid = GridState::default();
                    q.chart.init = false;
                }
            }
            ui.separator();
        }
        view_switch(ui, &mut q.view, p);
        if let Some(r) = q.results.get(q.active) {
            if r.truncated {
                ui.label(RichText::new("Limited to max rows").small().color(p.warning));
            }
        }
    });
    ui.add_space(4.0);
    let active = q.active.min(q.results.len() - 1);
    let res = &q.results[active];
    result_body(ui, id, res, q.view, &mut q.grid, &mut q.chart, None, p, 0, None, &[], actions);
}

/// The filter area: search everywhere + optional rule rows. Returns true when
/// the data should reload now.
/// `vals`: sampled values of this table's columns (lower-case column name).
fn filter_bar(ui: &mut egui::Ui, tt: &mut TableTab, p: &Palette, id: u64, vals: &HashMap<String, Vec<(String, i64)>>, actions: &mut Vec<Action>) -> bool {
    let mut reload = false;
    let cols: Vec<String> = tt.cols.as_ref().map(|c| c.iter().map(|x| x.name.clone()).collect()).unwrap_or_default();
    ui.horizontal(|ui| {
        let sid = egui::Id::new(("search", id));
        let n_rules = tt.filter.rules.len();
        let w = (ui.available_width() - 250.0).max(100.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut tt.filter.search)
                .id(sid)
                .hint_text("🔍  Type to search every column…   (Ctrl+F)")
                .desired_width(w)
                .margin(Margin::symmetric(10, 6)),
        );
        if tt.focus_search {
            r.request_focus();
            tt.focus_search = false;
        }
        if r.changed() {
            tt.search_changed = Some(Instant::now());
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            reload = true;
        }
        let label = if n_rules > 0 { format!("⚙ Filters · {n_rules}") } else { "+ Filter".to_string() };
        let fb = if n_rules > 0 { primary_button(p, &label) } else { ghost_button(p, &label) };
        if ui.add(fb).on_hover_text("Filter by column without writing SQL  (Ctrl+Shift+A)").clicked() {
            if n_rules == 0 {
                tt.filter.rules.push(Rule { column: cols.first().cloned().unwrap_or_default(), op: Op::Contains, value: String::new(), enabled: true });
                tt.show_rules = true;
            } else {
                tt.show_rules = !tt.show_rules;
            }
        }
        let sql_b = egui::Button::new(RichText::new("SQL").monospace().color(if tt.filter.raw_mode { p.on_primary } else { p.text_light }))
            .fill(if tt.filter.raw_mode { p.secondary } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(8);
        if ui.add(sql_b).on_hover_text("Advanced: type your own WHERE condition").clicked() {
            tt.filter.raw_mode = !tt.filter.raw_mode;
            if !tt.filter.raw_mode && !tt.filter.raw.is_empty() {
                reload = true;
            }
        }
        if tt.filter.active_count() > 0 && ui.add(ghost_button(p, "× Clear")).on_hover_text("Remove all filters  (Ctrl+Shift+X)").clicked() {
            tt.filter = FilterSet::default();
            reload = true;
        }
    });
    if tt.filter.raw_mode {
        ui.horizontal(|ui| {
            ui.label(RichText::new("WHERE").monospace().strong().color(p.pink));
            let r = ui.add(egui::TextEdit::singleline(&mut tt.filter.raw).code_editor().hint_text("price > 20 AND category = 'Food'").desired_width(ui.available_width() - 80.0));
            if (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) || ui.add(ghost_button(p, "Apply")).clicked() {
                reload = true;
            }
        });
    }
    if tt.show_rules && !tt.filter.rules.is_empty() {
        let mut remove = None;
        egui::Frame::new().fill(theme::mix(p.panel_alt, p.primary, 0.05)).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, rule) in tt.filter.rules.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if i == 0 {
                        ui.label(RichText::new("Where").color(p.text_light));
                    } else {
                        let txt = if tt.filter.match_any { "or" } else { "and" };
                        if ui.add(egui::Button::new(RichText::new(txt).color(p.secondary)).frame(false)).on_hover_text("Click to switch AND / OR").clicked() {
                            tt.filter.match_any = !tt.filter.match_any;
                            reload = true;
                        }
                    }
                    if ui.checkbox(&mut rule.enabled, "").on_hover_text("Turn this filter on / off").changed() {
                        reload = true;
                    }
                    // sizes follow the card width so rows never spill out of it
                    let avail = ui.available_width();
                    let col_w = (avail * 0.24).clamp(80.0, 150.0);
                    let op_w = (avail * 0.2).clamp(72.0, 130.0);
                    egui::ComboBox::from_id_salt(("fcol", id, i)).selected_text(RichText::new(&rule.column).strong()).width(col_w).show_ui(ui, |ui| {
                        for c in &cols {
                            if ui.selectable_value(&mut rule.column, c.clone(), c).changed() {
                                reload = true;
                            }
                        }
                    });
                    egui::ComboBox::from_id_salt(("fop", id, i)).selected_text(rule.op.label()).width(op_w).show_ui(ui, |ui| {
                        for o in Op::ALL {
                            if ui.selectable_value(&mut rule.op, o, o.label()).changed() {
                                reload = true;
                            }
                        }
                    });
                    if rule.op.needs_value() {
                        let r = ui.add(egui::TextEdit::singleline(&mut rule.value).hint_text(rule.op.hint()).desired_width((ui.available_width() - 80.0).clamp(50.0, 260.0)));
                        if r.changed() {
                            tt.search_changed = Some(Instant::now());
                        }
                        if r.lost_focus() && ui.input(|inp| inp.key_pressed(Key::Enter)) {
                            reload = true;
                        }
                        // quick pick from the column's most common values
                        let known = vals.get(&rule.column.to_lowercase());
                        let mb = ui.menu_button(RichText::new("⏷").color(p.primary), |ui| {
                            ui.set_min_width(220.0);
                            match known {
                                None => {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(RichText::new("Loading values…").small().color(p.text_light));
                                    });
                                }
                                Some(list) if list.is_empty() => {
                                    ui.label(RichText::new("No values").small().color(p.text_light));
                                }
                                Some(list) => {
                                    ui.label(RichText::new(format!("Most common in {} (sample)", rule.column)).small().color(p.text_lighter));
                                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                                        for (v, n) in list {
                                            let label = format!("{v}   ·  {n}");
                                            if ui.selectable_label(rule.value == *v, label).clicked() {
                                                if matches!(rule.op, Op::InList) && !rule.value.trim().is_empty() {
                                                    rule.value = format!("{}, {v}", rule.value.trim_end_matches([',', ' ']));
                                                } else {
                                                    rule.value = v.clone();
                                                }
                                                reload = true;
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            }
                        });
                        if known.is_none() && (mb.response.clicked() || mb.inner.is_some()) {
                            actions.push(Action::FetchValues(tt.table.clone(), rule.column.clone()));
                        }
                        mb.response.on_hover_text("Pick one of the values in this column");
                    }
                    if ui.add(egui::Button::new("×").frame(false)).on_hover_text("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("+ Add condition").color(p.primary)).frame(false)).clicked() {
                    tt.filter.rules.push(Rule { column: cols.first().cloned().unwrap_or_default(), op: Op::Contains, value: String::new(), enabled: true });
                }
                ui.label(RichText::new("Tip: right-click any cell to filter by its value.").small().color(p.text_lighter));
            });
        });
        if let Some(i) = remove {
            tt.filter.rules.remove(i);
            reload = true;
        }
    }
    reload
}

#[allow(clippy::too_many_arguments)]
pub(super) fn table_view(ui: &mut egui::Ui, tt: &mut TableTab, id: u64, p: &Palette, actions: &mut Vec<Action>, page_size: usize, read_only: bool, vals: &HashMap<String, Vec<(String, i64)>>) {
    let pk_names: Vec<String> = tt.cols.as_ref().map(|c| c.iter().filter(|c| c.pk).map(|c| c.name.clone()).collect()).unwrap_or_default();
    let editable = !read_only && !tt.is_view && !pk_names.is_empty();
    ui.horizontal(|ui| {
        widgets::table_icon(ui, if tt.is_view { p.secondary } else { p.primary }, tt.is_view);
        ui.label(RichText::new(&tt.table).strong().size(17.0).color(p.text_dark));
        if let Some(n) = tt.total {
            super::panels::chip(ui, &format!("{n} rows"), p.text_light, theme::mix(p.panel_alt, p.text_dark, 0.06));
        }
        if tt.is_view {
            super::panels::chip(ui, "view", p.secondary, theme::mix(p.panel_alt, p.secondary, 0.15));
        }
        if read_only {
            super::panels::chip(ui, "read-only", p.warning, theme::mix(p.panel_alt, p.warning, 0.15));
        }
        if tt.loading {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(ghost_button(p, "⟳")).on_hover_text("Refresh  (F5)").clicked() {
                actions.push(Action::Reload);
            }
            view_switch(ui, &mut tt.view, p);
        });
    });
    ui.add_space(6.0);
    if filter_bar(ui, tt, p, id, vals, actions) {
        tt.page = 0;
        actions.push(Action::Reload);
    }
    ui.add_space(6.0);
    if let Some(e) = &tt.error {
        egui::Frame::new().fill(theme::mix(p.panel_alt, p.danger, 0.14)).corner_radius(8).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("⚠  {e}")).color(p.danger));
        });
    }
    let footer_h = 40.0;
    let Some(data) = &tt.data else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.spinner();
        });
        return;
    };
    let h = ui.available_height() - footer_h;
    let mut out = GridOut::default();
    let pk_res: Vec<usize> = pk_names.iter().filter_map(|n| data.columns.iter().position(|c| c == n)).collect();
    if data.rows.is_empty() && tt.filter.active_count() > 0 {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("🔍").size(26.0).color(p.text_lighter));
            ui.label(RichText::new("No rows match these filters.").color(p.text_light));
        });
    }
    ui.allocate_ui(Vec2::new(ui.available_width(), h.max(60.0)), |ui| {
        let edits = GridEdits { edits: &mut tt.edits, deletes: &mut tt.deletes, editable };
        out = result_body(ui, id, data, tt.view, &mut tt.grid, &mut tt.chart, Some(edits), p, tt.page * page_size, tt.sort, &pk_res, actions);
    });
    if let Some(c) = out.sort_clicked {
        tt.sort = match tt.sort {
            Some((sc, true)) if sc == c => Some((c, false)),
            Some((sc, false)) if sc == c => None,
            _ => Some((c, true)),
        };
        actions.push(Action::Reload);
    }
    if let Some(s) = out.sort_set {
        tt.sort = Some(s);
        actions.push(Action::Reload);
    }
    if let Some((col, op, value)) = out.filter {
        let needs_typing = op.needs_value() && value.is_empty();
        tt.filter.rules.push(Rule { column: col, op, value, enabled: true });
        tt.show_rules = true;
        if !needs_typing {
            tt.page = 0;
            actions.push(Action::Reload);
        }
    }
    // footer
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let last = tt.total.map(|n| (n.saturating_sub(1) / page_size as u64) as usize);
        if ui.add_enabled(tt.page > 0, egui::Button::new("⏮").corner_radius(8)).on_hover_text("First page").clicked() {
            tt.page = 0;
            actions.push(Action::Reload);
        }
        if ui.add_enabled(tt.page > 0, egui::Button::new("◀").corner_radius(8)).on_hover_text("Previous page  (Ctrl+←)").clicked() {
            tt.page -= 1;
            actions.push(Action::Reload);
        }
        ui.label(RichText::new(format!("Page {}{}", tt.page + 1, last.map(|l| format!(" of {}", l + 1)).unwrap_or_default())).color(p.text));
        let has_next = last.map(|l| tt.page < l).unwrap_or(data.rows.len() >= page_size);
        if ui.add_enabled(has_next, egui::Button::new("▶").corner_radius(8)).on_hover_text("Next page  (Ctrl+→)").clicked() {
            tt.page += 1;
            actions.push(Action::Reload);
        }
        if ui.add_enabled(last.map(|l| tt.page < l).unwrap_or(false), egui::Button::new("⏭").corner_radius(8)).on_hover_text("Last page").clicked() {
            tt.page = last.unwrap_or(0);
            actions.push(Action::Reload);
        }
        if let Some(n) = &tt.notice {
            ui.label(RichText::new(n).small().color(p.text_lighter));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n = tt.edits.len() + tt.deletes.len();
            if n > 0 {
                if ui.add(primary_button(p, &format!("Save {n} change(s)  Ctrl+S"))).clicked() {
                    actions.push(Action::Save);
                }
                if ui.add(ghost_button(p, "Discard")).clicked() {
                    tt.edits.clear();
                    tt.deletes.clear();
                }
            }
            ui.menu_button("⬇ Export", |ui| {
                if ui.button("CSV   Ctrl+E").clicked() {
                    actions.push(Action::Export(false));
                    ui.close_menu();
                }
                if ui.button("JSON  Ctrl+Shift+J").clicked() {
                    actions.push(Action::Export(true));
                    ui.close_menu();
                }
            });
            if editable {
                if ui.add(ghost_button(p, "+ Row")).clicked() {
                    actions.push(Action::AddRow);
                }
                ui.label(RichText::new("Double-click a cell to edit · right-click to filter").small().color(p.text_lighter));
            } else if pk_names.is_empty() && !tt.is_view && !read_only {
                ui.label(RichText::new("No primary key: editing is off").small().color(p.text_lighter));
            }
        });
    });
}

pub(super) fn ai_view(ui: &mut egui::Ui, a: &mut AiTab, id: u64, p: &Palette, actions: &mut Vec<Action>, s: &AiSettings) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("✨ AI Shell").strong().size(15.0).color(p.text_dark));
        let (txt, col) = if s.provider.is_local() {
            ("LOCAL", p.success)
        } else if s.provider == Provider::ClaudeCode {
            ("CLAUDE CODE", p.purple)
        } else {
            ("CLOUD", p.warning)
        };
        egui::Frame::new().fill(col.gamma_multiply(0.25)).corner_radius(4).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
            ui.label(RichText::new(txt).small().strong().color(col));
        });
        ui.label(RichText::new(format!("{} · {}", s.provider.label(), s.model_label())).color(p.text_light));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("⚙ AI settings").clicked() {
                actions.push(Action::Settings);
            }
            if ui.button("Clear").clicked() {
                a.msgs.truncate(1);
                a.runs.clear();
            }
        });
    });
    ui.separator();
    let input_h = 80.0 + if a.confirm.is_some() { 90.0 } else { 0.0 };
    let h = (ui.available_height() - input_h).max(80.0);
    let mut send_results: Option<String> = None;
    egui::ScrollArea::vertical().id_salt(("ai", id)).max_height(h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for (i, m) in a.msgs.iter().enumerate() {
            let (fill, who, col) = match m.role {
                Role::User => (p.selection, "You", p.primary),
                Role::Assistant => (p.panel_alt, "AI", p.secondary),
                Role::Note => (Color32::TRANSPARENT, "", p.text_light),
            };
            egui::Frame::new().fill(fill).corner_radius(8).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !who.is_empty() {
                    ui.label(RichText::new(who).small().strong().color(col));
                }
                render_markdownish(ui, &m.text, p);
                if m.role == Role::Assistant {
                    for (k, sql) in ai::code_blocks(&m.text).into_iter().enumerate() {
                        ui.horizontal(|ui| {
                            let read = db::is_read_query(&sql);
                            if ui.button(if read { "▶ Run" } else { "⚠ Run (changes data)" }).clicked() {
                                if read {
                                    actions.push(Action::AiRun(i, sql.clone()));
                                } else {
                                    a.confirm = Some((i, sql.clone()));
                                }
                            }
                            if ui.button("Open in editor").clicked() {
                                actions.push(Action::OpenInEditor(sql.clone()));
                            }
                            if ui.button("Copy").clicked() {
                                ui.ctx().copy_text(sql.clone());
                            }
                            ui.label(RichText::new(format!("query {}", k + 1)).small().color(p.text_lighter));
                        });
                    }
                    if let Some(r) = a.runs.get(&i) {
                        match r {
                            Ok(res) => {
                                ui.label(RichText::new(format!("{} row(s)", res.rows.len())).small().color(p.success));
                                if !res.columns.is_empty() {
                                    let mut g = GridState::default();
                                    let hh = (res.rows.len().min(8) as f32 + 1.5) * widgets::ROW_H + 10.0;
                                    ui.allocate_ui(Vec2::new(ui.available_width(), hh), |ui| {
                                        ui.set_height(hh);
                                        widgets::grid(ui, ("aigrid", id, i), res, &mut g, None, p, 0, None, &[]);
                                    });
                                }
                                if ui.button("↩ Send results to AI").clicked() {
                                    let j = widgets::result_json(res, 50);
                                    send_results = Some(format!(
                                        "Here are the results (first {} rows as JSON):\n```json\n{}\n```",
                                        res.rows.len().min(50),
                                        serde_json::to_string(&j).unwrap_or_default()
                                    ));
                                }
                            }
                            Err(e) => {
                                ui.label(RichText::new(format!("⚠ {e}")).color(p.danger));
                            }
                        }
                    }
                }
            });
            ui.add_space(6.0);
        }
        if a.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Thinking…").color(p.text_light));
            });
        }
    });
    if let Some(t) = send_results {
        a.input = t;
        actions.push(Action::AiSend);
    }
    if let Some((i, sql)) = a.confirm.clone() {
        egui::Frame::new().fill(p.warning.gamma_multiply(0.2)).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.label(RichText::new("This query changes data. Run it?").strong().color(p.warning));
            ui.label(RichText::new(&sql).monospace());
            ui.horizontal(|ui| {
                if ui.button("Yes, run it").clicked() {
                    actions.push(Action::AiRun(i, sql.clone()));
                    a.confirm = None;
                }
                if ui.button("Cancel").clicked() {
                    a.confirm = None;
                }
            });
        });
    }
    ui.horizontal(|ui| {
        let enter_pressed = ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
        let r = ui.add_sized(
            [ui.available_width() - 80.0, 60.0],
            egui::TextEdit::multiline(&mut a.input).hint_text("Ask anything, e.g. “Which products sold the most?”  (Enter sends, Shift+Enter new line)"),
        );
        let enter = r.has_focus() && enter_pressed;
        if enter {
            while a.input.ends_with('\n') {
                a.input.pop();
            }
        }
        let send = ui.add_enabled(!a.busy, egui::Button::new("Send").min_size(Vec2::new(70.0, 60.0))).clicked();
        if send || (enter && !a.busy) {
            actions.push(Action::AiSend);
        }
    });
}

/// Tiny markdown: fenced code in tinted boxes, headings bold, the rest text.
fn render_markdownish(ui: &mut egui::Ui, text: &str, p: &Palette) {
    let mut in_code = false;
    let mut buf = String::new();
    let flush_text = |ui: &mut egui::Ui, b: &str| {
        for line in b.lines() {
            let t = line.trim_start();
            if let Some(h) = t.strip_prefix('#') {
                ui.label(RichText::new(h.trim_start_matches('#').trim()).strong().color(p.text_dark));
            } else if !t.is_empty() {
                ui.label(RichText::new(line.replace("**", "")).color(p.text_dark));
            }
        }
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                let code = std::mem::take(&mut buf);
                egui::Frame::new().fill(p.editor_bg).corner_radius(4).inner_margin(6).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let job = widgets::sql_job(code.trim_end(), p, egui::TextStyle::Monospace.resolve(ui.style()), ui.available_width());
                    ui.label(job);
                });
            } else {
                flush_text(ui, &std::mem::take(&mut buf));
            }
            in_code = !in_code;
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if !buf.is_empty() {
        flush_text(ui, &buf);
    }
}
