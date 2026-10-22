//! Éditeur Structured Text : coloration, numéros de ligne, erreurs, et vue
//! « visualisation » qui affiche les valeurs en ligne à côté du code, comme
//! les lunettes de TIA Portal.

use eframe::egui::{
    self, Align, Color32, FontId, Id, Rect, RichText, TextFormat, Ui, pos2, text::CCursor, text::CCursorRange,
    text::LayoutJob, vec2,
};
use takt_compiler::Severity;

const KEYWORDS: &[&str] = &[
    "IF",
    "THEN",
    "ELSIF",
    "ELSE",
    "END_IF",
    "CASE",
    "OF",
    "END_CASE",
    "FOR",
    "TO",
    "BY",
    "DO",
    "END_FOR",
    "WHILE",
    "END_WHILE",
    "REPEAT",
    "UNTIL",
    "END_REPEAT",
    "RETURN",
    "AND",
    "OR",
    "XOR",
    "NOT",
    "MOD",
    "TRUE",
    "FALSE",
    "VAR",
    "VAR_INPUT",
    "VAR_OUTPUT",
    "VAR_IN_OUT",
    "VAR_TEMP",
    "VAR_STAT",
    "CONSTANT",
    "RETAIN",
    "END_VAR",
    "PROGRAM",
    "END_PROGRAM",
    "ORGANIZATION_BLOCK",
    "END_ORGANIZATION_BLOCK",
    "FUNCTION_BLOCK",
    "END_FUNCTION_BLOCK",
    "FUNCTION",
    "END_FUNCTION",
    "BEGIN",
    "ARRAY",
    "VOID",
];

const TYPES: &[&str] =
    &["BOOL", "INT", "DINT", "REAL", "TIME", "TON", "TOF", "TP", "R_TRIG", "F_TRIG", "CTU", "CTD", "SR", "RS"];

pub struct Palette {
    pub text: Color32,
    pub keyword: Color32,
    pub ty: Color32,
    pub comment: Color32,
    pub global: Color32,
    pub local: Color32,
    pub number: Color32,
    pub line_no: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            text: Color32::from_rgb(220, 220, 220),
            keyword: Color32::from_rgb(86, 156, 214),
            ty: Color32::from_rgb(78, 201, 176),
            comment: Color32::from_rgb(106, 153, 85),
            global: Color32::from_rgb(214, 157, 133),
            local: Color32::from_rgb(197, 134, 192),
            number: Color32::from_rgb(181, 206, 168),
            line_no: Color32::from_gray(120),
        }
    } else {
        Palette {
            text: Color32::from_rgb(30, 30, 30),
            keyword: Color32::from_rgb(0, 0, 192),
            ty: Color32::from_rgb(0, 112, 112),
            comment: Color32::from_rgb(0, 128, 0),
            global: Color32::from_rgb(140, 30, 30),
            local: Color32::from_rgb(90, 60, 150),
            number: Color32::from_rgb(9, 134, 88),
            line_no: Color32::from_gray(140),
        }
    }
}

/// Colore une source ST. Le découpage est volontairement simple et ne
/// dépend pas du compilateur : il doit marcher sur du code incomplet.
pub fn highlight(text: &str, font: &FontId, pal: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |c: Color32| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
    let b: Vec<char> = text.chars().collect();
    let mut i = 0;
    let push = |job: &mut LayoutJob, s: &[char], c: Color32| {
        job.append(&s.iter().collect::<String>(), 0.0, fmt(c));
    };
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        let start = i;
        let color;
        if c == '/' && next == Some('/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            color = pal.comment;
        } else if (c == '(' || c == '/') && next == Some('*') {
            let close = if c == '(' { ')' } else { '/' };
            i += 2;
            while i < b.len() && !(b[i - 1] == '*' && b[i] == close && i > start + 2) {
                i += 1;
            }
            i = (i + 1).min(b.len());
            color = pal.comment;
        } else if c == '{' {
            while i < b.len() && b[i] != '}' {
                i += 1;
            }
            i = (i + 1).min(b.len());
            color = pal.comment;
        } else if c == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' && b[i] != '\n' {
                i += 1;
            }
            i = (i + 1).min(b.len());
            color = pal.global;
        } else if c == '#' || c.is_ascii_alphabetic() || c == '_' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let word: String = b[start..i].iter().collect::<String>().to_ascii_uppercase();
            if (word == "T" || word == "TIME") && b.get(i) == Some(&'#') {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '.' || b[i] == '_') {
                    i += 1;
                }
                color = pal.number;
            } else if c == '#' {
                color = pal.local;
            } else if KEYWORDS.contains(&word.as_str()) {
                color = pal.keyword;
            } else if TYPES.contains(&word.as_str()) || word.contains("_TO_") {
                color = pal.ty;
            } else {
                color = pal.text;
            }
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '.' || b[i] == '#' || b[i] == '_') {
                if b[i] == '.' && b.get(i + 1) == Some(&'.') {
                    break;
                }
                i += 1;
            }
            color = pal.number;
        } else {
            i += 1;
            while i < b.len() && !"/(\"#{_".contains(b[i]) && !b[i].is_ascii_alphanumeric() {
                i += 1;
            }
            color = pal.text;
        }
        push(&mut job, &b[start..i], color);
    }
    job
}

pub struct LineMark {
    pub line: u32,
    pub severity: Severity,
    pub msg: String,
}

/// Éditeur de code. Renvoie vrai si le texte a changé.
pub fn code_editor(ui: &mut Ui, id: Id, source: &mut String, marks: &[LineMark], goto_line: &mut Option<u32>) -> bool {
    let font = FontId::monospace(13.5);
    let pal = palette(ui.visuals().dark_mode);
    let row_h = ui.fonts(|f| f.row_height(&font));
    let n_lines = source.lines().count().max(1) + usize::from(source.ends_with('\n'));
    let mut changed = false;

    egui::ScrollArea::both().id_salt(id.with("scroll")).auto_shrink([false; 2]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // Colonne des numéros de ligne, avec les erreurs en rouge
            let mut nums = LayoutJob::default();
            for l in 1..=n_lines as u32 {
                let mark = marks.iter().filter(|m| m.line == l).map(|m| m.severity).min();
                let (sym, color) = match mark {
                    Some(Severity::Error) => ("●", Color32::from_rgb(220, 40, 40)),
                    Some(Severity::Warning) => ("▲", Color32::from_rgb(220, 150, 0)),
                    _ => (" ", pal.line_no),
                };
                let line = format!("{sym}{l:>4}{}", if l < n_lines as u32 { "\n" } else { "" });
                nums.append(&line, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
            }
            let resp = ui.add(egui::Label::new(nums).selectable(false));
            for m in marks {
                let y = resp.rect.top() + (m.line.saturating_sub(1)) as f32 * row_h;
                let r = Rect::from_min_size(pos2(resp.rect.left(), y), vec2(resp.rect.width(), row_h));
                if ui.rect_contains_pointer(r) {
                    egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), id.with(("mark", m.line)), |ui| {
                        ui.label(&m.msg);
                    });
                }
            }

            let mut layouter = |ui: &Ui, text: &str, _wrap: f32| {
                let job = highlight(text, &font, &pal);
                ui.fonts(|f| f.layout_job(job))
            };
            let out = egui::TextEdit::multiline(source)
                .id(id)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(n_lines.max(30))
                .lock_focus(true)
                .frame(false)
                .layouter(&mut layouter)
                .show(ui);
            changed = out.response.changed();

            if let Some(line) = goto_line.take() {
                let idx: usize =
                    source.lines().take(line.saturating_sub(1) as usize).map(|l| l.chars().count() + 1).sum();
                let mut state = out.state;
                state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
                state.store(ui.ctx(), id);
                ui.memory_mut(|m| m.request_focus(id));
                let y = out.response.rect.top() + (line.saturating_sub(1)) as f32 * row_h;
                ui.scroll_to_rect(
                    Rect::from_min_size(pos2(out.response.rect.left(), y), vec2(1.0, row_h)),
                    Some(Align::Center),
                );
            }
        });
    });
    changed
}

/// Insère un extrait à la position du curseur de l'éditeur `id`.
pub fn insert_snippet(ctx: &egui::Context, id: Id, source: &mut String, snippet: &str) {
    let state = egui::TextEdit::load_state(ctx, id);
    let char_idx = state
        .as_ref()
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.primary.index)
        .unwrap_or_else(|| source.chars().count());
    let byte = source.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(source.len());
    source.insert_str(byte, snippet);
    if let Some(mut s) = state {
        s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(char_idx + snippet.chars().count()))));
        s.store(ctx, id);
    }
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Noms de variables présents dans une ligne : `#x`, `"Tag"`, `a.b.c`, `t[2]`.
pub fn names_in_line(line: &str) -> Vec<String> {
    let code = match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    };
    let b: Vec<char> = code.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '"' {
            let s = i + 1;
            i += 1;
            while i < b.len() && b[i] != '"' {
                i += 1;
            }
            let name: String = b[s..i.min(b.len())].iter().collect();
            i += 1;
            let mut path = name;
            while i + 1 < b.len() && b[i] == '.' && b[i + 1] != '.' {
                let s = i + 1;
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                    i += 1;
                }
                path.push('.');
                path.extend(&b[s..i]);
            }
            if !out.contains(&path) {
                out.push(path);
            }
        } else if c == '#' || c.is_ascii_alphabetic() || c == '_' {
            let s = if c == '#' { i + 1 } else { i };
            i += 1;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || b[i] == '_' || (b[i] == '.' && b.get(i + 1) != Some(&'.')))
            {
                i += 1;
            }
            let mut path: String = b[s..i].iter().collect();
            // Index constant : t[3]
            if b.get(i) == Some(&'[') {
                let close = b[i..].iter().position(|c| *c == ']').map(|p| p + i);
                if let Some(close) = close {
                    let idx: String = b[i + 1..close].iter().collect();
                    if idx.trim().parse::<i64>().is_ok() {
                        path = format!("{path}[{}]", idx.trim());
                    }
                }
            }
            if b.get(i) == Some(&'#') {
                // littéral T#…
                while i < b.len() && !b[i].is_whitespace() && b[i] != ';' && b[i] != ')' {
                    i += 1;
                }
                continue;
            }
            if !out.contains(&path) {
                out.push(path);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Vue de visualisation : code en lecture seule, valeurs à droite de chaque ligne.
/// Valeur affichée et état d'un BOOL (pour la pastille verte/grise).
pub type MonitorValue = (String, Option<bool>);

pub fn monitor_view(ui: &mut Ui, id: Id, source: &str, value_of: &dyn Fn(&str) -> Option<MonitorValue>) {
    let font = FontId::monospace(13.5);
    let pal = palette(ui.visuals().dark_mode);
    egui::ScrollArea::both().id_salt(id.with("monitor")).auto_shrink([false; 2]).show(ui, |ui| {
        egui::Grid::new(id.with("grid")).num_columns(3).spacing(vec2(10.0, 2.0)).striped(true).show(ui, |ui| {
            for (n, line) in source.lines().enumerate() {
                ui.label(RichText::new(format!("{:>4}", n + 1)).font(font.clone()).color(pal.line_no));
                ui.add(egui::Label::new(highlight(line, &font, &pal)).extend());
                ui.horizontal(|ui| {
                    for name in names_in_line(line) {
                        if let Some((text, state)) = value_of(&name) {
                            let color = match state {
                                Some(true) => Color32::from_rgb(0, 150, 60),
                                Some(false) => Color32::from_gray(130),
                                None => ui.visuals().text_color(),
                            };
                            let label = format!("{} = {text}", name.rsplit('.').next().unwrap_or(&name));
                            egui::Frame::new()
                                .fill(ui.visuals().extreme_bg_color)
                                .corner_radius(3.0)
                                .inner_margin(egui::Margin::symmetric(4, 0))
                                .show(ui, |ui| {
                                    if state.is_some() {
                                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                                        ui.painter().circle_filled(r.center(), 4.0, color);
                                    }
                                    ui.label(RichText::new(label).font(FontId::monospace(12.0)).color(color));
                                })
                                .response
                                .on_hover_text(&name);
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_dune_ligne() {
        let n = names_in_line("#moteur(Marche := \"BP_Marche\", PT := T#5s); x := #t[3] + a.b; // \"ignoré\"");
        assert_eq!(n, vec!["moteur", "Marche", "BP_Marche", "PT", "x", "t[3]", "a.b"]);
    }
}
