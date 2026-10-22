//! Éditeur GRAPH (SFC) : chaîne d'étapes et de transitions à gauche,
//! propriétés de l'élément sélectionné à droite. En visualisation, l'étape
//! active est en vert avec son temps.

use eframe::egui::{self, FontId, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use takt_compiler::lang::sfc::{Action, Qualifier, STEP_TIME_VAR, STEP_VAR, Step, Transition};
use takt_compiler::project::Block;

use crate::theme::{self, icon};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SfcSel {
    Step(usize),
    Transition(usize),
}

/// Valeurs en ligne : étape active et temps passé dedans.
pub struct SfcMonitor {
    pub active: Option<u16>,
    pub time: Option<String>,
}

const STEP_W: f32 = 170.0;
const STEP_H: f32 = 46.0;

/// `errors` : (index d'étape ou de transition + 1, message).
pub fn editor(
    ui: &mut Ui,
    block: &mut Block,
    sel: &mut Option<SfcSel>,
    mon: Option<&SfcMonitor>,
    errors: &[(u32, String)],
) -> bool {
    let c = theme::colors(ui);
    let mut changed = false;
    ui.horizontal_top(|ui| {
        // Graphe
        let graph_w = (ui.available_width() - 460.0).max(420.0);
        theme::card(ui, |ui| {
            ui.set_width(graph_w);
            ui.label(
                RichText::new(format!(
                    "Variables disponibles : #{STEP_VAR} (étape active), #{STEP_TIME_VAR} (temps dans l'étape)"
                ))
                .size(12.0)
                .color(c.soft),
            );
            ui.add_space(10.0);
            let g = &block.sfc;
            let order: Vec<u16> = g.steps.iter().map(|s| s.number).collect();
            for (i, step) in g.steps.iter().enumerate() {
                let x0 = ui.cursor().left() + 30.0;
                let active = mon.and_then(|m| m.active) == Some(step.number);
                let selected = *sel == Some(SfcSel::Step(i));
                let has_err = errors.iter().any(|(l, m)| *l == i as u32 + 1 && m.starts_with("étape"));
                let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), STEP_H), Sense::click());
                let r = Rect::from_min_size(pos2(x0, row.top()), vec2(STEP_W, STEP_H));
                let fill = if active { c.green.gamma_multiply(0.22) } else { c.bg };
                let stroke = if has_err {
                    Stroke::new(1.8_f32, c.red)
                } else if selected {
                    Stroke::new(1.8_f32, c.accent)
                } else {
                    Stroke::new(1.2_f32, c.text)
                };
                ui.painter().rect(r, 6.0, fill, stroke, egui::StrokeKind::Inside);
                if i == 0 {
                    // Étape initiale : double cadre
                    ui.painter().rect_stroke(r.shrink(4.0), 4.0, stroke, egui::StrokeKind::Inside);
                }
                ui.painter().text(
                    pos2(r.left() + 12.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("S{}", step.number),
                    FontId::new(14.0, theme::semi()),
                    if active { c.green } else { c.text },
                );
                ui.painter().text(
                    pos2(r.left() + 50.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    &step.name,
                    FontId::new(13.0, theme::medium()),
                    c.text,
                );
                if active && let Some(t) = mon.and_then(|m| m.time.clone()) {
                    ui.painter().text(
                        r.right_top() + vec2(-6.0, 9.0),
                        egui::Align2::RIGHT_CENTER,
                        t,
                        FontId::monospace(10.0),
                        c.green,
                    );
                }
                // Actions à droite de l'étape, comme dans TIA
                let ax = r.right() + 16.0;
                if !step.actions.is_empty() {
                    ui.painter().line_segment(
                        [pos2(r.right(), r.center().y), pos2(ax, r.center().y)],
                        Stroke::new(1.0_f32, c.soft),
                    );
                }
                for (k, a) in step.actions.iter().enumerate() {
                    let ay = r.top() + 10.0 + k as f32 * 17.0;
                    ui.painter().text(
                        pos2(ax + 4.0, ay),
                        egui::Align2::LEFT_CENTER,
                        a.qualifier.label(),
                        FontId::new(11.0, theme::semi()),
                        c.accent,
                    );
                    ui.painter().text(
                        pos2(ax + 30.0, ay),
                        egui::Align2::LEFT_CENTER,
                        &a.operand,
                        FontId::monospace(11.5),
                        c.text,
                    );
                }
                let extra = (step.actions.len() as f32 * 17.0 - STEP_H + 12.0).max(0.0);
                if extra > 0.0 {
                    ui.add_space(extra);
                }
                if resp.clicked() {
                    *sel = Some(SfcSel::Step(i));
                }

                // Transitions sortantes
                let outgoing: Vec<(usize, &Transition)> =
                    g.transitions.iter().enumerate().filter(|(_, t)| t.from == step.number).collect();
                let next = order.get(i + 1).copied();
                let cx = x0 + STEP_W / 2.0;
                for (k, t) in &outgoing {
                    let (tr, tresp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    let selected = *sel == Some(SfcSel::Transition(*k));
                    let has_err = errors.iter().any(|(l, m)| *l == *k as u32 + 1 && m.starts_with("transition"));
                    let s = Stroke::new(
                        if selected { 2.0_f32 } else { 1.2 },
                        if has_err {
                            c.red
                        } else if selected {
                            c.accent
                        } else {
                            c.text
                        },
                    );
                    ui.painter()
                        .line_segment([pos2(cx, tr.top()), pos2(cx, tr.bottom())], Stroke::new(1.2_f32, c.text));
                    ui.painter().line_segment(
                        [pos2(cx - 14.0, tr.center().y), pos2(cx + 14.0, tr.center().y)],
                        Stroke::new(3.0_f32, s.color),
                    );
                    let cond = if t.condition.trim().is_empty() { "(jamais)" } else { t.condition.trim() };
                    ui.painter().text(
                        pos2(cx + 22.0, tr.center().y),
                        egui::Align2::LEFT_CENTER,
                        cond,
                        FontId::monospace(12.0),
                        c.text,
                    );
                    if Some(t.to) != next {
                        ui.painter().text(
                            pos2(cx - 22.0, tr.center().y),
                            egui::Align2::RIGHT_CENTER,
                            theme_jump(t.to),
                            FontId::new(12.0, theme::semi()),
                            c.accent,
                        );
                    }
                    if tresp.clicked() {
                        *sel = Some(SfcSel::Transition(*k));
                    }
                }
                if outgoing.is_empty() {
                    ui.add_space(14.0);
                } else {
                    let (gap, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    if outgoing.iter().any(|(_, t)| Some(t.to) == next) {
                        ui.painter()
                            .line_segment([pos2(cx, gap.top()), pos2(cx, gap.bottom())], Stroke::new(1.2_f32, c.text));
                    }
                }
            }
        });
        ui.add_space(12.0);
        // Propriétés
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width().min(420.0));
            ui.visuals_mut().extreme_bg_color = c.bg;
            changed |= properties(ui, block, sel);
        });
    });
    changed
}

fn theme_jump(to: u16) -> String {
    format!("{}  S{to}", '\u{2192}')
}

fn properties(ui: &mut Ui, block: &mut Block, sel: &mut Option<SfcSel>) -> bool {
    let c = theme::colors(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        if theme::secondary_button(ui, icon::PLUS, "Étape")
            .on_hover_text("Ajoute une étape après l'étape sélectionnée")
            .clicked()
        {
            add_step(block, sel);
            changed = true;
        }
        if theme::secondary_button(ui, icon::PLUS, "Transition")
            .on_hover_text("Ajoute une transition depuis l'étape sélectionnée")
            .clicked()
        {
            let from = match sel {
                Some(SfcSel::Step(i)) => block.sfc.steps.get(*i).map(|s| s.number),
                Some(SfcSel::Transition(k)) => block.sfc.transitions.get(*k).map(|t| t.from),
                None => block.sfc.steps.last().map(|s| s.number),
            };
            if let Some(from) = from {
                block.sfc.transitions.push(Transition { from, to: block.sfc.initial(), condition: String::new() });
                *sel = Some(SfcSel::Transition(block.sfc.transitions.len() - 1));
                changed = true;
            }
        }
    });
    ui.add_space(10.0);
    match *sel {
        Some(SfcSel::Step(i)) if i < block.sfc.steps.len() => {
            theme::section_title(ui, &format!("Étape S{}", block.sfc.steps[i].number));
            ui.add_space(6.0);
            let numbers: Vec<u16> = block.sfc.steps.iter().map(|s| s.number).collect();
            let step = &mut block.sfc.steps[i];
            egui::Grid::new("step_props").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label(RichText::new("Numéro").color(c.soft));
                let old = step.number;
                if ui.add(egui::DragValue::new(&mut step.number).range(1..=9999)).changed() {
                    if numbers.iter().filter(|n| **n == step.number).count() > 0 && step.number != old {
                        step.number = old;
                    } else {
                        for t in &mut block.sfc.transitions {
                            if t.from == old {
                                t.from = step.number;
                            }
                            if t.to == old {
                                t.to = step.number;
                            }
                        }
                        changed = true;
                    }
                }
                ui.end_row();
                ui.label(RichText::new("Nom").color(c.soft));
                changed |= ui.add_sized([220.0, 24.0], egui::TextEdit::singleline(&mut step.name)).changed();
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Actions").font(FontId::new(13.5, theme::semi())));
            let mut remove = None;
            for (k, a) in step.actions.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("qual", i, k))
                        .width(52.0)
                        .selected_text(a.qualifier.label())
                        .show_ui(ui, |ui| {
                            for q in Qualifier::ALL {
                                changed |= ui.selectable_value(&mut a.qualifier, q, q.label()).changed();
                            }
                        });
                    let hint = if a.qualifier == Qualifier::Code { "instruction ST" } else { "\"Sortie\"" };
                    changed |= ui
                        .add_sized(
                            [230.0, 24.0],
                            egui::TextEdit::singleline(&mut a.operand).hint_text(hint).font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    if theme::ghost_icon(ui, icon::TRASH, "Supprimer l'action").clicked() {
                        remove = Some(k);
                    }
                });
            }
            if let Some(k) = remove {
                step.actions.remove(k);
                changed = true;
            }
            let add = theme::rich(
                &format!("{}  Ajouter une action", icon::PLUS),
                13.0,
                egui::FontFamily::Proportional,
                c.accent,
            );
            if ui.add(egui::Button::new(add).frame(false)).clicked() {
                step.actions.push(Action { qualifier: Qualifier::N, operand: String::new() });
                changed = true;
            }
            ui.label(
                RichText::new(
                    "N : vrai tant que l'étape est active · S/R : mise à 1 / à 0 · ST : instruction à chaque cycle",
                )
                .size(11.5)
                .color(c.soft),
            );
            ui.add_space(8.0);
            if block.sfc.steps.len() > 1 && theme::secondary_button(ui, icon::TRASH, "Supprimer l'étape").clicked() {
                let n = block.sfc.steps[i].number;
                block.sfc.steps.remove(i);
                block.sfc.transitions.retain(|t| t.from != n && t.to != n);
                *sel = None;
                changed = true;
            }
        }
        Some(SfcSel::Transition(k)) if k < block.sfc.transitions.len() => {
            let numbers: Vec<(u16, String)> = block.sfc.steps.iter().map(|s| (s.number, s.name.clone())).collect();
            let t = &mut block.sfc.transitions[k];
            theme::section_title(ui, &format!("Transition S{} → S{}", t.from, t.to));
            ui.add_space(6.0);
            egui::Grid::new("trans_props").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                for (label, value) in [("De", &mut t.from), ("Vers", &mut t.to)] {
                    ui.label(RichText::new(label).color(c.soft));
                    egui::ComboBox::from_id_salt(("tr", label, k))
                        .width(200.0)
                        .selected_text(format!("S{value}"))
                        .show_ui(ui, |ui| {
                            for (n, name) in &numbers {
                                changed |= ui.selectable_value(value, *n, format!("S{n}  {name}")).changed();
                            }
                        });
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Condition (ST)").font(FontId::new(13.5, theme::semi())));
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut t.condition)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("\"Capteur\" AND T_Etape >= T#2s"),
                )
                .changed();
            ui.add_space(8.0);
            if theme::secondary_button(ui, icon::TRASH, "Supprimer la transition").clicked() {
                block.sfc.transitions.remove(k);
                *sel = None;
                changed = true;
            }
        }
        _ => {
            ui.label(RichText::new("Sélectionnez une étape ou une transition pour la modifier.").color(c.soft));
        }
    }
    changed
}

/// Ajoute une étape après l'étape sélectionnée, reliée par une transition.
pub fn add_step(block: &mut Block, sel: &mut Option<SfcSel>) {
    let g = &mut block.sfc;
    let n = g.next_number();
    let at = match sel {
        Some(SfcSel::Step(i)) => *i + 1,
        _ => g.steps.len(),
    };
    let prev = if at > 0 { g.steps.get(at - 1).map(|s| s.number) } else { None };
    g.steps.insert(at, Step { number: n, name: format!("Étape {n}"), actions: vec![] });
    if let Some(p) = prev {
        // La transition qui partait de l'étape précédente vers la suivante passe par la nouvelle
        let next = g.steps.get(at + 1).map(|s| s.number);
        if let Some(t) = g.transitions.iter_mut().find(|t| t.from == p && Some(t.to) == next) {
            t.to = n;
            if let Some(nx) = next {
                g.transitions.push(Transition { from: n, to: nx, condition: String::new() });
            }
        } else {
            g.transitions.push(Transition { from: p, to: n, condition: String::new() });
        }
    }
    *sel = Some(SfcSel::Step(at));
}
