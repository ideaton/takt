//! Table d'interface d'un bloc (Input, Output, Static, Temp, Constant),
//! comme le volet « Interface » au-dessus de l'éditeur dans TIA Portal.

use eframe::egui::{self, RichText, Ui, vec2};
use takt_compiler::lang::{BlockType, IfaceVar, Section};
use takt_compiler::project::Block;

use crate::theme::{self, icon};

const TYPES: &[&str] = &["BOOL", "INT", "DINT", "REAL", "TIME", "TON", "TOF", "TP", "CTU", "CTD", "R_TRIG", "F_TRIG"];

/// Renvoie vrai si l'interface a changé.
pub fn interface(ui: &mut Ui, block: &mut Block, open: &mut bool) -> bool {
    let c = theme::colors(ui);
    let mut changed = false;
    let count = block.interface.iter().filter(|v| !v.name.trim().is_empty()).count();
    let header = theme::rich(
        &format!("{}  Interface  ·  {count} variable(s)", if *open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT }),
        13.5,
        theme::medium(),
        c.text,
    );
    if ui.add(egui::Button::new(header).frame(false)).clicked() {
        *open = !*open;
    }
    if !*open {
        return false;
    }
    ui.add_space(4.0);
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.visuals_mut().extreme_bg_color = c.bg;
        if block.kind == BlockType::Fc {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Return").color(c.soft));
                changed |= ui
                    .add_sized([120.0, 24.0], egui::TextEdit::singleline(&mut block.return_type).hint_text("VOID"))
                    .on_hover_text("Type renvoyé par la fonction (VOID si aucun). Affectez-le avec le nom de la FC.")
                    .changed();
            });
            ui.add_space(4.0);
        }
        let mut remove = None;
        egui::Grid::new("iface").num_columns(5).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            for h in ["", "Nom", "Type", "Valeur initiale", "Commentaire"] {
                ui.label(RichText::new(h).size(12.5).color(c.soft));
            }
            ui.end_row();
            for &sec in block.kind.sections() {
                ui.label(RichText::new(sec.label()).font(egui::FontId::new(13.0, theme::semi())).color(c.text));
                ui.end_row();
                for (i, v) in block.interface.iter_mut().enumerate().filter(|(_, v)| v.section == sec) {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        if theme::ghost_icon(ui, icon::TRASH, "Supprimer").clicked() {
                            remove = Some(i);
                        }
                    });
                    changed |= ui.add_sized([160.0, 24.0], egui::TextEdit::singleline(&mut v.name)).changed();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        changed |= ui.add_sized([110.0, 24.0], egui::TextEdit::singleline(&mut v.ty)).changed();
                        ui.menu_button(theme::icon_text(icon::CHEVRON_DOWN, 13.0).color(c.soft), |ui| {
                            for t in TYPES {
                                if ui.button(*t).clicked() {
                                    v.ty = t.to_string();
                                    changed = true;
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                    changed |= ui.add_sized([110.0, 24.0], egui::TextEdit::singleline(&mut v.init)).changed();
                    changed |= ui.add_sized([260.0, 24.0], egui::TextEdit::singleline(&mut v.comment)).changed();
                    ui.end_row();
                }
                ui.horizontal(|ui| ui.add_space(14.0));
                let add =
                    theme::rich(&format!("{}  Ajouter", icon::PLUS), 13.0, egui::FontFamily::Proportional, c.accent);
                if ui.add(egui::Button::new(add).frame(false)).clicked() {
                    let n = block.interface.iter().filter(|v| v.section == sec).count() + 1;
                    let ty = if sec == Section::Constant { "INT" } else { "BOOL" };
                    let mut v = IfaceVar::new(sec, &format!("{}_{n}", sec.label()), ty);
                    if sec == Section::Constant {
                        v.init = "0".into();
                    }
                    block.interface.push(v);
                    changed = true;
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            block.interface.remove(i);
            changed = true;
        }
    });
    changed
}
