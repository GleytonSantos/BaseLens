//! Colour themes. Own design (not the original's palette): "Midnight" and
//! "Daylight" use a violet accent with a teal second accent; "Ocean" and
//! "Latte" are alternatives. "System" follows the desktop (rule from the
//! original's runtime, see MECHANICS.md).

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum ThemeKind {
    #[default]
    System,
    #[serde(alias = "Light")]
    Daylight,
    #[serde(alias = "Dark")]
    Midnight,
    #[serde(alias = "SolarizedDark")]
    Ocean,
    #[serde(alias = "Solarized")]
    Latte,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 5] = [ThemeKind::System, ThemeKind::Daylight, ThemeKind::Midnight, ThemeKind::Ocean, ThemeKind::Latte];
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::System => "System",
            ThemeKind::Daylight => "Daylight",
            ThemeKind::Midnight => "Midnight",
            ThemeKind::Ocean => "Ocean",
            ThemeKind::Latte => "Latte",
        }
    }
    /// "System" follows the desktop: dark when the desktop prefers dark.
    pub fn resolve(self, system_dark: bool) -> ThemeKind {
        match self {
            ThemeKind::System => {
                if system_dark {
                    ThemeKind::Midnight
                } else {
                    ThemeKind::Daylight
                }
            }
            t => t,
        }
    }
    /// The light/dark partner used by the quick toggle.
    pub fn flipped(self) -> ThemeKind {
        match self {
            ThemeKind::Daylight => ThemeKind::Midnight,
            ThemeKind::Midnight => ThemeKind::Daylight,
            ThemeKind::Ocean => ThemeKind::Latte,
            ThemeKind::Latte => ThemeKind::Ocean,
            ThemeKind::System => ThemeKind::Daylight,
        }
    }
}

/// Reads the desktop's colour preference the way the original's runtime does
/// (functions 0xb2eb7c0 / 0xb2eb4c0, dark_mode_manager_linux): the
/// org.freedesktop.portal.Settings value `org.freedesktop.appearance`
/// `color-scheme`; 1 means "prefer dark". Falls back to GNOME's gsettings,
/// then to the GTK_THEME name.
pub fn system_prefers_dark() -> bool {
    let out = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout",
            "2",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.Read",
            "org.freedesktop.appearance",
            "color-scheme",
        ])
        .output();
    if let Ok(o) = out {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout);
            if let Some(v) = s.split("uint32").nth(1) {
                let n: String = v.chars().skip_while(|c| c.is_whitespace()).take_while(|c| c.is_ascii_digit()).collect();
                return n == "1";
            }
        }
    }
    if let Ok(o) = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]).output() {
        if o.status.success() {
            return String::from_utf8_lossy(&o.stdout).contains("dark");
        }
    }
    std::env::var("GTK_THEME").map(|t| t.to_lowercase().contains("dark")).unwrap_or(false)
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// window background (behind the cards)
    pub bg: Color32,
    /// icon rail
    pub rail: Color32,
    /// side panels
    pub sidebar: Color32,
    /// cards, popups, raised surfaces
    pub panel_alt: Color32,
    pub editor_bg: Color32,
    pub text: Color32,
    pub text_dark: Color32,
    pub text_light: Color32,
    pub text_lighter: Color32,
    pub border: Color32,
    pub primary: Color32,
    pub on_primary: Color32,
    pub secondary: Color32,
    pub selection: Color32,
    pub row_alt: Color32,
    pub row_add: Color32,
    pub row_delete: Color32,
    pub cell_edited: Color32,
    pub info: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub purple: Color32,
    pub pink: Color32,
}

pub fn hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0);
    Color32::from_rgb(p(0), p(2), p(4))
}

/// Mix two colours (t = 0 → a, t = 1 → b).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

pub fn palette(kind: ThemeKind) -> Palette {
    match kind {
        ThemeKind::System | ThemeKind::Midnight => {
            let bg = hex("#0d1017");
            let primary = hex("#8b7bff");
            Palette {
                dark: true,
                bg,
                rail: hex("#090c12"),
                sidebar: hex("#11151d"),
                panel_alt: hex("#161b26"),
                editor_bg: hex("#0a0d14"),
                text: hex("#c3cadb"),
                text_dark: hex("#eef1f8"),
                text_light: hex("#8a93a8"),
                text_lighter: hex("#5b647a"),
                border: hex("#232a38"),
                primary,
                on_primary: hex("#ffffff"),
                secondary: hex("#38d6c4"),
                selection: mix(bg, primary, 0.24),
                row_alt: hex("#121722"),
                row_add: hex("#123a30"),
                row_delete: hex("#4a1d2a"),
                cell_edited: hex("#352f5e"),
                info: hex("#5aa9ff"),
                success: hex("#34d399"),
                warning: hex("#f5a524"),
                danger: hex("#f43f5e"),
                purple: hex("#a78bfa"),
                pink: hex("#f472b6"),
            }
        }
        ThemeKind::Daylight => {
            let bg = hex("#f3f4f9");
            let primary = hex("#6d4aff");
            Palette {
                dark: false,
                bg,
                rail: hex("#e8eaf3"),
                sidebar: hex("#f8f9fc"),
                panel_alt: hex("#ffffff"),
                editor_bg: hex("#ffffff"),
                text: hex("#3a4155"),
                text_dark: hex("#141927"),
                text_light: hex("#687088"),
                text_lighter: hex("#9aa1b5"),
                border: hex("#dde1ec"),
                primary,
                on_primary: hex("#ffffff"),
                secondary: hex("#0e9f9a"),
                selection: hex("#e7e1ff"),
                row_alt: hex("#f8f9fd"),
                row_add: hex("#d9f7e6"),
                row_delete: hex("#ffe0e6"),
                cell_edited: hex("#ece6ff"),
                info: hex("#2563eb"),
                success: hex("#059669"),
                warning: hex("#d97706"),
                danger: hex("#e11d48"),
                purple: hex("#7c3aed"),
                pink: hex("#db2777"),
            }
        }
        ThemeKind::Ocean => {
            let bg = hex("#0a1417");
            let primary = hex("#2dd4bf");
            Palette {
                dark: true,
                bg,
                rail: hex("#071013"),
                sidebar: hex("#0d1a1e"),
                panel_alt: hex("#112127"),
                editor_bg: hex("#071012"),
                text: hex("#bfd3d6"),
                text_dark: hex("#eaf6f6"),
                text_light: hex("#82a0a6"),
                text_lighter: hex("#557178"),
                border: hex("#1b3036"),
                primary,
                on_primary: hex("#04211d"),
                secondary: hex("#7dd3fc"),
                selection: mix(bg, primary, 0.22),
                row_alt: hex("#0e1b1f"),
                row_add: hex("#0f3b33"),
                row_delete: hex("#4a1f27"),
                cell_edited: hex("#16404a"),
                info: hex("#60a5fa"),
                success: hex("#4ade80"),
                warning: hex("#fbbf24"),
                danger: hex("#fb7185"),
                purple: hex("#c084fc"),
                pink: hex("#f9a8d4"),
            }
        }
        ThemeKind::Latte => {
            let bg = hex("#f7f2ea");
            let primary = hex("#d9480f");
            Palette {
                dark: false,
                bg,
                rail: hex("#efe7db"),
                sidebar: hex("#fbf7f1"),
                panel_alt: hex("#fffdf9"),
                editor_bg: hex("#fffdf9"),
                text: hex("#4a3f35"),
                text_dark: hex("#231c15"),
                text_light: hex("#7d6f62"),
                text_lighter: hex("#ab9d8f"),
                border: hex("#e6dccd"),
                primary,
                on_primary: hex("#ffffff"),
                secondary: hex("#0f766e"),
                selection: hex("#fde3d3"),
                row_alt: hex("#fcf8f2"),
                row_add: hex("#dcf3e1"),
                row_delete: hex("#fbdcdc"),
                cell_edited: hex("#fdebc8"),
                info: hex("#1d4ed8"),
                success: hex("#15803d"),
                warning: hex("#b45309"),
                danger: hex("#be123c"),
                purple: hex("#7e22ce"),
                pink: hex("#be185d"),
            }
        }
    }
}

pub fn chart_colors(p: &Palette) -> [Color32; 7] {
    [p.primary, p.secondary, p.info, p.pink, p.warning, p.success, p.purple]
}

pub fn apply(ctx: &egui::Context, p: &Palette, font_scale: f32) {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = Some(p.text);
    v.panel_fill = p.bg;
    v.window_fill = p.panel_alt;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_shadow = egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(if p.dark { 140 } else { 40 }) };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(if p.dark { 120 } else { 30 }) };
    v.extreme_bg_color = p.editor_bg;
    v.faint_bg_color = p.row_alt;
    v.code_bg_color = p.editor_bg;
    v.hyperlink_color = p.secondary;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.primary);
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.striped = true;
    v.text_cursor.stroke = Stroke::new(2.0, p.primary);

    let r = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel_alt;
    w.noninteractive.weak_bg_fill = p.panel_alt;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    let btn = if p.dark { mix(p.panel_alt, p.text_dark, 0.04) } else { p.panel_alt };
    w.inactive.bg_fill = btn;
    w.inactive.weak_bg_fill = btn;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text_dark);
    w.inactive.corner_radius = r;

    let hov = mix(btn, p.primary, 0.14);
    w.hovered.bg_fill = hov;
    w.hovered.weak_bg_fill = hov;
    w.hovered.bg_stroke = Stroke::new(1.0, mix(p.border, p.primary, 0.6));
    w.hovered.fg_stroke = Stroke::new(1.5, p.text_dark);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.primary;
    w.active.weak_bg_fill = p.primary;
    w.active.bg_stroke = Stroke::new(1.0, p.primary);
    w.active.fg_stroke = Stroke::new(1.5, p.on_primary);
    w.active.corner_radius = r;

    w.open = w.hovered;
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.combo_width = 140.0;
    use egui::{FontId, TextStyle};
    let base = 13.5 * font_scale;
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(base - 2.5)),
        (TextStyle::Body, FontId::proportional(base)),
        (TextStyle::Button, FontId::proportional(base)),
        (TextStyle::Heading, FontId::proportional(base + 8.0)),
        (TextStyle::Monospace, FontId::monospace(base - 0.5)),
    ]
    .into();
    ctx.set_style(style);
}

/// Use a modern system UI font when one is installed (Adwaita Sans is based
/// on Inter), keeping egui's own fonts as fallback for symbols and emoji.
/// Fonts are read from the system at start-up, never bundled.
pub fn install_fonts(ctx: &egui::Context) {
    let sans = [
        "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
        "/usr/share/fonts/inter/Inter-Regular.ttf",
        "/usr/share/fonts/TTF/Inter-Regular.ttf",
        "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ];
    let mono = [
        "/usr/share/fonts/Adwaita/AdwaitaMono-Regular.ttf",
        "/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        "C:\\Windows\\Fonts\\consola.ttf",
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut changed = false;
    for (name, list, fam) in [("ui-sans", &sans[..], egui::FontFamily::Proportional), ("ui-mono", &mono[..], egui::FontFamily::Monospace)] {
        if let Some(bytes) = list.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            if let Some(f) = fonts.families.get_mut(&fam) {
                f.insert(0, name.to_owned());
            }
            changed = true;
        }
    }
    if changed {
        ctx.set_fonts(fonts);
    }
}

/// Horizontal colour blend over a rectangle (used for the welcome banner).
pub fn gradient_rect(painter: &egui::Painter, rect: egui::Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    let i = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
    painter.add(egui::Shape::mesh(mesh));
}
