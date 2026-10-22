//! Thème épuré façon Apple, le même que Sura Novi : beaucoup de blanc, gris
//! neutres, bleu d'accent, police Inter, icônes Lucide, coins arrondis, pas de
//! bordures.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response, RichText, Sense,
    Shadow, Stroke, TextStyle, Ui, vec2,
};

pub const RADIUS: u8 = 12;

#[derive(Clone, Copy)]
pub struct Colors {
    pub bg: Color32,
    /// Barre latérale.
    pub bar: Color32,
    /// Fond des cartes et des listes groupées.
    pub group: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub soft: Color32,
    pub very_soft: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub red: Color32,
    pub orange: Color32,
    pub green: Color32,
}

pub const LIGHT: Colors = Colors {
    bg: Color32::from_rgb(255, 255, 255),
    bar: Color32::from_rgb(245, 245, 247),
    group: Color32::from_rgb(245, 245, 247),
    hover: Color32::from_rgba_premultiplied(0, 0, 0, 11),
    text: Color32::from_rgb(29, 29, 31),
    soft: Color32::from_rgb(110, 110, 115),
    very_soft: Color32::from_rgb(199, 199, 204),
    accent: Color32::from_rgb(0, 122, 255),
    accent_soft: Color32::from_rgba_premultiplied(0, 15, 31, 31),
    red: Color32::from_rgb(255, 59, 48),
    orange: Color32::from_rgb(255, 149, 0),
    green: Color32::from_rgb(52, 199, 89),
};

pub const DARK: Colors = Colors {
    bg: Color32::from_rgb(28, 28, 30),
    bar: Color32::from_rgb(22, 22, 24),
    group: Color32::from_rgb(44, 44, 46),
    hover: Color32::from_rgba_premultiplied(18, 18, 18, 18),
    text: Color32::from_rgb(245, 245, 247),
    soft: Color32::from_rgb(152, 152, 157),
    very_soft: Color32::from_rgb(72, 72, 74),
    accent: Color32::from_rgb(10, 132, 255),
    accent_soft: Color32::from_rgba_premultiplied(2, 29, 56, 56),
    red: Color32::from_rgb(255, 69, 58),
    orange: Color32::from_rgb(255, 159, 10),
    green: Color32::from_rgb(48, 209, 88),
};

pub fn colors(ui: &Ui) -> Colors {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

/// Icônes Lucide (licence ISC), comme dans Sura Novi.
pub mod icon {
    pub const CPU: char = '\u{e0a9}';
    pub const TAG: char = '\u{e17f}';
    pub const CODE: char = '\u{e093}';
    pub const BOX: char = '\u{e061}';
    pub const FUNCTION: char = '\u{e22d}';
    pub const EYE: char = '\u{e0ba}';
    pub const PLUS: char = '\u{e13d}';
    pub const HAMMER: char = '\u{e0ec}';
    pub const DOWNLOAD: char = '\u{e0b2}';
    pub const PLUG: char = '\u{e37f}';
    pub const UNPLUG: char = '\u{e45d}';
    pub const MORE: char = '\u{e0b6}';
    pub const SAVE: char = '\u{e14d}';
    pub const FOLDER: char = '\u{e247}';
    pub const FILE_PLUS: char = '\u{e0c9}';
    pub const BOOK: char = '\u{e05f}';
    pub const MOON: char = '\u{e11e}';
    pub const SUN: char = '\u{e178}';
    pub const INFO: char = '\u{e0f9}';
    pub const ALERT: char = '\u{e077}';
    pub const WARNING: char = '\u{e193}';
    pub const CHECK: char = '\u{e07c}';
    pub const TRASH: char = '\u{e18e}';
    pub const TIMER: char = '\u{e1e0}';
    pub const SCROLL: char = '\u{e45f}';
    pub const BLOCKS: char = '\u{e4fa}';
    pub const LOCK_OPEN: char = '\u{e10c}';
    pub const ZAP: char = '\u{e1b4}';
    pub const REFRESH: char = '\u{e145}';
    pub const X: char = '\u{e1b2}';
    pub const CHEVRON_DOWN: char = '\u{e06d}';
    pub const CHEVRON_RIGHT: char = '\u{e06f}';
    pub const CHEVRON_UP: char = '\u{e070}';
    pub const FOLDER_CLOSED: char = '\u{e0d7}';
    pub const SETTINGS: char = '\u{e154}';
    pub const ACTIVITY: char = '\u{e038}';
}

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const INTER_SEMI: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const LUCIDE: &[u8] = include_bytes!("../assets/fonts/lucide.ttf");
pub const LOGO_PNG: &[u8] = include_bytes!("../assets/logo-256.png");

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn semi() -> FontFamily {
    FontFamily::Name("semi".into())
}

/// Famille réservée aux icônes : Lucide contient aussi des lettres et Inter
/// des glyphes dans la zone privée, on ne peut donc pas les chaîner.
pub fn icons() -> FontFamily {
    FontFamily::Name("icons".into())
}

fn is_icon(ch: char) -> bool {
    ('\u{e000}'..='\u{f8ff}').contains(&ch)
}

/// Texte où les icônes Lucide (zone privée d'Unicode) sont rendues avec leur
/// propre police : `format!("{}  Compiler", icon::HAMMER)`.
pub fn rich(text: &str, size: f32, family: FontFamily, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut run = String::new();
    let mut run_icon = false;
    let flush = |job: &mut egui::text::LayoutJob, run: &mut String, icon: bool| {
        if run.is_empty() {
            return;
        }
        let font = if icon { FontId::new(size + 1.0, icons()) } else { FontId::new(size, family.clone()) };
        job.append(
            run,
            0.0,
            egui::TextFormat { font_id: font, color, valign: egui::Align::Center, ..Default::default() },
        );
        run.clear();
    };
    for ch in text.chars() {
        let icon = is_icon(ch);
        if icon != run_icon {
            flush(&mut job, &mut run, run_icon);
            run_icon = icon;
        }
        run.push(ch);
    }
    flush(&mut job, &mut run, run_icon);
    job
}

/// Polices : Inter partout, Lucide en secours pour les icônes, et la police
/// à chasse fixe du système (Cascadia Mono ou Consolas) pour le code.
fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    for (name, bytes) in
        [("inter", INTER), ("inter-medium", INTER_MEDIUM), ("inter-semi", INTER_SEMI), ("lucide", LUCIDE)]
    {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let system_mono = ["C:\\Windows\\Fonts\\CascadiaMono.ttf", "C:\\Windows\\Fonts\\consola.ttf"]
        .iter()
        .find_map(|p| std::fs::read(p).ok());
    if let Some(bytes) = system_mono {
        f.font_data.insert("mono".into(), Arc::new(FontData::from_owned(bytes)));
        f.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "mono".into());
    }
    let prop = f.families.get_mut(&FontFamily::Proportional).unwrap();
    prop.insert(0, "inter".into());
    let fallback: Vec<String> = prop.iter().skip(1).cloned().collect();
    for (family, main) in [(medium(), "inter-medium"), (semi(), "inter-semi")] {
        let mut list = vec![main.to_string()];
        list.extend(fallback.iter().cloned());
        f.families.insert(family, list);
    }
    f.families.insert(icons(), vec!["lucide".into()]);
    f
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    apply(ctx, false);
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let c = if dark { DARK } else { LIGHT };
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = c.bg;
    v.window_fill = c.bg;
    v.extreme_bg_color = c.group;
    v.faint_bg_color = if dark { Color32::from_rgb(34, 34, 36) } else { Color32::from_rgb(250, 250, 252) };
    v.code_bg_color = c.group;
    v.hyperlink_color = c.accent;
    v.warn_fg_color = c.orange;
    v.error_fg_color = c.red;
    v.selection.bg_fill = c.accent_soft;
    v.selection.stroke = Stroke::new(1.0_f32, c.accent);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_stroke = Stroke::NONE;
    v.window_shadow = Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(60) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(40) };
    v.striped = false;
    let w = &mut v.widgets;
    let rad = CornerRadius::same(8);
    w.noninteractive.bg_fill = c.bg;
    w.noninteractive.weak_bg_fill = c.bg;
    w.noninteractive.bg_stroke =
        Stroke::new(1.0_f32, if dark { Color32::from_gray(52) } else { Color32::from_gray(232) });
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, c.text);
    w.noninteractive.corner_radius = rad;
    let hover = if dark { Color32::from_gray(58) } else { Color32::from_rgb(232, 232, 237) };
    let press = if dark { Color32::from_gray(68) } else { Color32::from_rgb(220, 220, 226) };
    for (st, fill) in
        [(&mut w.inactive, c.group), (&mut w.hovered, hover), (&mut w.active, press), (&mut w.open, hover)]
    {
        st.bg_fill = fill;
        st.weak_bg_fill = fill;
        st.bg_stroke = Stroke::NONE;
        st.fg_stroke = Stroke::new(1.0_f32, c.text);
        st.corner_radius = rad;
        st.expansion = 0.0;
    }
    w.inactive.fg_stroke = Stroke::new(1.0_f32, c.text);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::new(22.0, semi())),
            (TextStyle::Monospace, FontId::monospace(13.5)),
        ]
        .into();
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.window_margin = Margin::same(22);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
    });
}

// ---------------------------------------------------------------------------
// Composants

pub fn icon_text(ch: char, size: f32) -> RichText {
    RichText::new(ch.to_string()).font(FontId::new(size, icons()))
}

/// Titre de page, comme « Tâches » dans Sura Novi.
pub fn page_title(ui: &mut Ui, title: &str, subtitle: &str) {
    let c = colors(ui);
    ui.label(RichText::new(title).font(FontId::new(28.0, semi())).color(c.text));
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).size(14.0).color(c.soft));
    }
}

pub fn section_title(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).font(FontId::new(16.0, semi())).color(colors(ui).text));
}

/// Ligne de l'arborescence du projet, comme dans TIA : indentation, flèche
/// de dépliage, icône, libellé. Renvoie (réponse de la ligne, clic sur la flèche).
#[allow(clippy::too_many_arguments)]
pub fn tree_item(
    ui: &mut Ui,
    depth: usize,
    expanded: Option<bool>,
    ic: char,
    label: &str,
    selected: bool,
    trailing: Option<(String, Color32)>,
    label_color: Option<Color32>,
) -> (Response, bool) {
    let c = colors(ui);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 6.0, c.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, c.hover);
    }
    let x = rect.left() + 4.0 + depth as f32 * 14.0;
    let chevron = egui::Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), vec2(16.0, 22.0));
    let mut toggled = false;
    if let Some(open) = expanded {
        let ch = if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT };
        ui.painter().text(chevron.center(), egui::Align2::CENTER_CENTER, ch, FontId::new(12.0, icons()), c.soft);
        toggled = resp.clicked() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| chevron.contains(pos));
    }
    let fg = if selected { c.accent } else { label_color.unwrap_or(c.text) };
    let icon_fg = if selected { c.accent } else { label_color.unwrap_or(c.soft) };
    ui.painter().text(
        egui::pos2(x + 18.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        ic,
        FontId::new(14.0, icons()),
        icon_fg,
    );
    let font = FontId::new(13.5, if selected { medium() } else { FontFamily::Proportional });
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, fg);
    let text_x = x + 38.0;
    let clip = egui::Rect::from_min_max(egui::pos2(text_x, rect.top()), egui::pos2(rect.right() - 46.0, rect.bottom()));
    ui.painter().with_clip_rect(clip).galley(egui::pos2(text_x, rect.center().y - galley.size().y / 2.0), galley, fg);
    if let Some((t, color)) = trailing {
        ui.painter().text(
            rect.right_center() - vec2(8.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            t,
            FontId::proportional(11.5),
            color,
        );
    }
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), toggled)
}

/// Bouton icône discret (sans fond tant qu'on ne le survole pas).
pub fn ghost_icon(ui: &mut Ui, ic: char, tip: &str) -> Response {
    let c = colors(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 7.0, c.hover);
    }
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, ic, FontId::new(15.0, icons()), c.soft);
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Bouton bleu plein (« + Nouvelle tâche » dans Sura Novi).
pub fn primary_button(ui: &mut Ui, ic: char, label: &str) -> Response {
    let c = colors(ui);
    let text = rich(&format!("{ic}  {label}"), 14.0, medium(), Color32::WHITE);
    ui.add(egui::Button::new(text).fill(c.accent).corner_radius(9.0).min_size(vec2(0.0, 34.0)))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Bouton gris clair.
pub fn secondary_button(ui: &mut Ui, ic: char, label: &str) -> Response {
    let c = colors(ui);
    let text = rich(&format!("{ic}  {label}"), 14.0, medium(), c.text);
    ui.add(egui::Button::new(text).fill(c.group).corner_radius(9.0).min_size(vec2(0.0, 34.0)))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Bouton bascule : bleu clair quand il est actif.
pub fn toggle_button(ui: &mut Ui, ic: char, label: &str, on: bool) -> Response {
    let c = colors(ui);
    let fg = if on { c.accent } else { c.text };
    let text = rich(&format!("{ic}  {label}"), 14.0, medium(), fg);
    ui.add(
        egui::Button::new(text)
            .fill(if on { c.accent_soft } else { c.group })
            .corner_radius(9.0)
            .min_size(vec2(0.0, 34.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Pastille colorée (« Prioritaire », « Ideaton » dans Sura Novi).
pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .corner_radius(6.0)
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(rich(text, 12.5, medium(), color));
        })
        .response
}

/// Contrôle segmenté (« Boulot | Perso » dans Sura Novi).
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let c = colors(ui);
    let mut changed = false;
    egui::Frame::new().fill(c.group).corner_radius(9.0).inner_margin(Margin::same(3)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // Dans une rangée de droite à gauche, les options arrivent à l'envers
        let rtl = ui.layout().prefer_right_to_left();
        ui.horizontal(|ui| {
            let ordered: Vec<&(T, &str)> = if rtl { options.iter().rev().collect() } else { options.iter().collect() };
            for (v, label) in ordered {
                let selected = *value == *v;
                let text = RichText::new(*label).font(FontId::new(13.5, medium())).color(if selected {
                    c.text
                } else {
                    c.soft
                });
                let fill = if selected { c.bg } else { Color32::TRANSPARENT };
                let b = egui::Button::new(text).fill(fill).corner_radius(7.0).min_size(vec2(0.0, 26.0));
                if ui.add(b).clicked() && !selected {
                    *value = *v;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// Carte grise arrondie.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let c = colors(ui);
    egui::Frame::new()
        .fill(c.group)
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| ui.with_layout(egui::Layout::top_down(egui::Align::Min), add).inner)
        .inner
}

/// Logo de l'application, chargé une fois en texture.
pub fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let img = decode_png(LOGO_PNG);
    ctx.load_texture("takt-logo", img, egui::TextureOptions::LINEAR)
}

pub fn decode_png(bytes: &[u8]) -> egui::ColorImage {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("logo PNG");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("logo PNG");
    egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &buf[..info.buffer_size()])
}

/// Icône de la fenêtre (barre des tâches).
pub fn window_icon() -> egui::IconData {
    let img = decode_png(LOGO_PNG);
    let [w, h] = img.size;
    egui::IconData {
        rgba: img.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect(),
        width: w as u32,
        height: h as u32,
    }
}

/// Interrupteur façon iOS. Renvoie vrai s'il a basculé.
pub fn switch(ui: &mut Ui, on: &mut bool) -> bool {
    let c = colors(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(38.0, 22.0), Sense::click());
    let clicked = resp.clicked();
    if clicked {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let off = if ui.visuals().dark_mode { Color32::from_gray(70) } else { Color32::from_rgb(229, 229, 234) };
    let mut fill = lerp_color(off, c.green, t);
    if !ui.is_enabled() {
        fill = fill.gamma_multiply(0.5);
    }
    ui.painter().rect_filled(rect, 11.0, fill);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y + 0.5), 9.5, Color32::from_black_alpha(25));
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 9.0, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    clicked
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
