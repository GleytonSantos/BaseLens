//! BaseLens visual identity.
//!
//! The mark: a rounded tile with the brand gradient (violet → teal), and a
//! lens looking into three stacked "rows" of data, with a short handle.
//! Base = the database rows, Lens = the ring that looks into them.
//! The same geometry is used for the in-app logo (vector, via egui shapes),
//! the window icon (rasterised here) and `assets/baselens.svg`.

use crate::theme::{hex, mix, Palette};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2};

pub const NAME: &str = "BaseLens";
pub const TAGLINE: &str = "See into every database.";
/// Brand gradient: same pair as the Midnight theme accents.
pub const VIOLET: &str = "#8b7bff";
pub const TEAL: &str = "#38d6c4";
pub const INK: &str = "#0d1017";

// Geometry, as fractions of the tile size.
const TILE_RADIUS: f32 = 0.28;
const LENS_C: (f32, f32) = (0.45, 0.43);
const LENS_R: f32 = 0.235;
const RING_W: f32 = 0.075;
const HANDLE_FROM: f32 = 1.0; // handle starts at the ring, along the 45° diagonal (in lens radii)
const HANDLE_TO: (f32, f32) = (0.80, 0.78);
const HANDLE_W: f32 = 0.095;
const BARS: [(f32, f32); 3] = [(0.355, 0.19), (0.43, 0.24), (0.505, 0.19)]; // (y, width)
const BAR_H: f32 = 0.048;

fn tile_colors(p: Option<&Palette>) -> (Color32, Color32) {
    match p {
        Some(p) => (p.primary, p.secondary),
        None => (hex(VIOLET), hex(TEAL)),
    }
}

/// Diagonal gradient rounded tile (approximated with a fan of vertex colours).
fn gradient_tile(painter: &egui::Painter, r: Rect, radius: f32, a: Color32, b: Color32) {
    // rounded-rect outline points
    let mut pts = Vec::new();
    let corners = [
        (r.right() - radius, r.top() + radius, -90.0f32),
        (r.right() - radius, r.bottom() - radius, 0.0),
        (r.left() + radius, r.bottom() - radius, 90.0),
        (r.left() + radius, r.top() + radius, 180.0),
    ];
    for (cx, cy, start) in corners {
        for k in 0..=6 {
            let ang = (start + k as f32 * 15.0).to_radians();
            pts.push(Pos2::new(cx + radius * ang.cos(), cy + radius * ang.sin()));
        }
    }
    let col_at = |p: Pos2| {
        let t = (((p.x - r.left()) + (p.y - r.top())) / (r.width() + r.height())).clamp(0.0, 1.0);
        mix(a, b, t)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.center(), col_at(r.center()));
    for p in &pts {
        mesh.colored_vertex(*p, col_at(*p));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
}

/// The BaseLens mark at `size` px.
pub fn mark(ui: &mut egui::Ui, p: &Palette, size: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_mark(ui.painter(), r, Some(p));
    resp
}

pub fn paint_mark(painter: &egui::Painter, r: Rect, p: Option<&Palette>) {
    let s = r.width();
    let (a, b) = tile_colors(p);
    gradient_tile(painter, r, s * TILE_RADIUS, a, b);
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * s, r.top() + y * s);
    let white = Color32::WHITE;
    let c = at(LENS_C.0, LENS_C.1);
    let lr = LENS_R * s;
    // handle (drawn first so the ring sits on top)
    let d = std::f32::consts::FRAC_1_SQRT_2 * lr * HANDLE_FROM;
    let h0 = Pos2::new(c.x + d, c.y + d);
    let h1 = at(HANDLE_TO.0, HANDLE_TO.1);
    painter.line_segment([h0, h1], Stroke::new(HANDLE_W * s, white));
    painter.circle_filled(h1, HANDLE_W * s / 2.0, white);
    // glass + rows of data seen through it
    painter.circle_filled(c, lr, Color32::from_black_alpha(46));
    for (y, w) in BARS {
        let bar = Rect::from_center_size(at(LENS_C.0, y + 0.0), Vec2::new(w * s, BAR_H * s));
        painter.rect_filled(bar, BAR_H * s / 2.0, white.gamma_multiply(0.92));
    }
    painter.circle_stroke(c, lr, Stroke::new(RING_W * s, white));
    // glint
    painter.circle_filled(Pos2::new(c.x - lr * 0.55, c.y - lr * 0.45), s * 0.028, white.gamma_multiply(0.85));
}

/// Small lens glyph (for the Lens assistant and search fields).
pub fn lens_glyph(ui: &mut egui::Ui, color: Color32, size: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_lens(ui.painter(), r, color);
    resp
}

pub fn paint_lens(painter: &egui::Painter, r: Rect, color: Color32) {
    let size = r.width();
    let c = Pos2::new(r.left() + size * 0.43, r.top() + size * 0.43);
    let lr = size * 0.30;
    let d = std::f32::consts::FRAC_1_SQRT_2 * lr;
    painter.line_segment([Pos2::new(c.x + d, c.y + d), Pos2::new(r.right() - size * 0.1, r.bottom() - size * 0.1)], Stroke::new(size * 0.13, color));
    painter.circle_stroke(c, lr, Stroke::new(size * 0.11, color));
    painter.line_segment([Pos2::new(c.x - lr * 0.45, c.y), Pos2::new(c.x + lr * 0.45, c.y)], Stroke::new(size * 0.07, color));
}

/// "Base" + "Lens" wordmark.
pub fn wordmark(ui: &mut egui::Ui, p: &Palette, size: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(egui::RichText::new("Base").size(size).strong().color(p.text_dark));
        ui.label(egui::RichText::new("Lens").size(size).strong().color(p.primary));
    });
}

// ------------------------------------------------------------ raster icon

fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, rad: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - rad);
    let qy = (py - cy).abs() - (hh - rad);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - rad
}

fn sd_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (pax, pay, bax, bay) = (px - ax, py - ay, bx - ax, by - ay);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt()
}

/// RGBA pixels of the mark (straight alpha), `n`×`n`, 4×4 supersampled.
pub fn icon_rgba(n: u32) -> Vec<u8> {
    let (a, b) = (hex(VIOLET), hex(TEAL));
    let mut out = vec![0u8; (n * n * 4) as usize];
    let ss = 4;
    let (lcx, lcy) = LENS_C;
    let d = std::f32::consts::FRAC_1_SQRT_2 * LENS_R * HANDLE_FROM;
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = (x as f32 + (sx as f32 + 0.5) / ss as f32) / n as f32;
                    let py = (y as f32 + (sy as f32 + 0.5) / ss as f32) / n as f32;
                    if sd_round_rect(px, py, 0.5, 0.5, 0.5, 0.5, TILE_RADIUS) > 0.0 {
                        continue;
                    }
                    let t = ((px + py) / 2.0).clamp(0.0, 1.0);
                    let g = mix(a, b, t);
                    let mut col = [g.r() as f32, g.g() as f32, g.b() as f32];
                    let dist_c = ((px - lcx).powi(2) + (py - lcy).powi(2)).sqrt();
                    let blend = |col: &mut [f32; 3], over: [f32; 3], alpha: f32| {
                        for k in 0..3 {
                            col[k] = col[k] * (1.0 - alpha) + over[k] * alpha;
                        }
                    };
                    let white = [255.0, 255.0, 255.0];
                    if sd_segment(px, py, lcx + d, lcy + d, HANDLE_TO.0, HANDLE_TO.1) < HANDLE_W / 2.0 {
                        blend(&mut col, white, 1.0);
                    }
                    if dist_c < LENS_R {
                        blend(&mut col, [0.0, 0.0, 0.0], 0.18);
                        for (by, bw) in BARS {
                            if sd_round_rect(px, py, lcx, by, bw / 2.0, BAR_H / 2.0, BAR_H / 2.0) < 0.0 {
                                blend(&mut col, white, 0.92);
                            }
                        }
                        let gx = lcx - LENS_R * 0.55;
                        let gy = lcy - LENS_R * 0.45;
                        if ((px - gx).powi(2) + (py - gy).powi(2)).sqrt() < 0.028 {
                            blend(&mut col, white, 0.85);
                        }
                    }
                    if (dist_c - LENS_R).abs() < RING_W / 2.0 {
                        blend(&mut col, white, 1.0);
                    }
                    acc[0] += col[0];
                    acc[1] += col[1];
                    acc[2] += col[2];
                    acc[3] += 255.0;
                }
            }
            let k = (ss * ss) as f32;
            let i = ((y * n + x) * 4) as usize;
            let alpha = acc[3] / k;
            let cov = (alpha / 255.0).max(1e-6);
            out[i] = (acc[0] / k / cov).round() as u8;
            out[i + 1] = (acc[1] / k / cov).round() as u8;
            out[i + 2] = (acc[2] / k / cov).round() as u8;
            out[i + 3] = alpha.round() as u8;
        }
    }
    out
}

pub fn window_icon() -> egui::IconData {
    let n = 128;
    egui::IconData { rgba: icon_rgba(n), width: n, height: n }
}

/// The mark as SVG (kept in sync with the geometry constants above).
pub fn svg(size: u32) -> String {
    let s = size as f32;
    let f = |v: f32| format!("{:.2}", v * s);
    let (lcx, lcy) = LENS_C;
    let d = std::f32::consts::FRAC_1_SQRT_2 * LENS_R * HANDLE_FROM;
    let mut bars = String::new();
    for (y, w) in BARS {
        bars.push_str(&format!(
            "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"#fff\" fill-opacity=\"0.92\"/>\n",
            f(lcx - w / 2.0),
            f(y - BAR_H / 2.0),
            f(w),
            f(BAR_H),
            f(BAR_H / 2.0)
        ));
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 {size} {size}\">\n\
  <title>{NAME}</title>\n\
  <defs><linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{VIOLET}\"/><stop offset=\"1\" stop-color=\"{TEAL}\"/></linearGradient></defs>\n\
  <rect width=\"{size}\" height=\"{size}\" rx=\"{}\" fill=\"url(#g)\"/>\n\
  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#fff\" stroke-width=\"{}\" stroke-linecap=\"round\"/>\n\
  <circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#000\" fill-opacity=\"0.18\"/>\n\
{bars}  <circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"#fff\" stroke-width=\"{}\"/>\n\
  <circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#fff\" fill-opacity=\"0.85\"/>\n\
</svg>\n",
        f(TILE_RADIUS),
        f(lcx + d),
        f(lcy + d),
        f(HANDLE_TO.0),
        f(HANDLE_TO.1),
        f(HANDLE_W),
        f(lcx),
        f(lcy),
        f(LENS_R),
        f(lcx),
        f(lcy),
        f(LENS_R),
        f(RING_W),
        f(lcx - LENS_R * 0.55),
        f(lcy - LENS_R * 0.45),
        f(0.028),
    )
}

/// Writes the logo files (`--export-brand <folder>`): SVG, PNG and a
/// Linux .desktop entry.
/// Reverse-DNS id used by Flatpak, AppStream and macOS.
pub const APP_ID: &str = "dev.baselens.BaseLens";
/// Icon sizes written for desktop packages (hicolor theme, .ico, .icns).
pub const ICON_SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];

/// A Windows .ico holding PNG images (valid since Windows Vista).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (n, png) in images {
        let d = if *n >= 256 { 0 } else { *n as u8 };
        out.extend_from_slice(&[d, d, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

fn png_bytes(n: u32) -> std::io::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(&mut buf, &icon_rgba(n), n, n, image::ExtendedColorType::Rgba8, image::ImageFormat::Png).map_err(std::io::Error::other)?;
    Ok(buf.into_inner())
}

pub fn export(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("baselens.svg"), svg(256))?;
    let mut ico_imgs = Vec::new();
    for n in ICON_SIZES {
        let png = png_bytes(n)?;
        std::fs::write(dir.join(format!("baselens-{n}.png")), &png)?;
        if matches!(n, 16 | 24 | 32 | 48 | 64 | 256) {
            ico_imgs.push((n, png));
        }
    }
    std::fs::write(dir.join("baselens.ico"), ico(&ico_imgs))?;
    std::fs::write(
        dir.join("baselens.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName={NAME}\nGenericName=Database Editor\nComment={TAGLINE} Free, private database editor.\nExec=baselens %F\nIcon=baselens\nCategories=Development;Database;\nKeywords=sql;database;postgres;mysql;sqlite;json;\nStartupWMClass=baselens\nTerminal=false\n"
        ),
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn ink() -> Color32 {
    hex(INK)
}
