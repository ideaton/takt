//! Vues du projet : appareil, variables API, tables de visualisation et
//! tiroir du bas (messages, diagnostic, simulation, journal).

use eframe::egui::{self, Color32, FontId, RichText, Sense, Ui, pos2, vec2};
use takt_compiler::board::{self, BOARDS, Board};
use takt_compiler::project::{Tag, WatchRow};
use takt_compiler::types::{Ty, format_value, parse_value};
use takt_compiler::{BLOCK_DEVICE, BLOCK_TAGS, Severity};
use takt_vm::image::IoKind;
use takt_vm::proto;
use takt_vm::runtime::Mode;

use crate::app::{App, BottomTab, View};
use crate::target;
use crate::theme::{self, Colors, icon};

pub fn board_name(id: &str) -> &'static str {
    board::find(id).map(|b| b.name).unwrap_or("carte inconnue")
}

fn kind_color(c: &Colors, k: IoKind) -> Color32 {
    match k {
        IoKind::Di => c.green,
        IoKind::Dq => c.orange,
        IoKind::Ai => c.accent,
        IoKind::Aq => Color32::from_rgb(175, 82, 222),
    }
}

fn kind_name(k: IoKind) -> &'static str {
    match k {
        IoKind::Di => "Entrée TOR",
        IoKind::Dq => "Sortie TOR",
        IoKind::Ai => "Entrée analogique",
        IoKind::Aq => "Sortie analogique",
    }
}

/// Variable API reliée à une voie, d'après la table du projet.
fn tag_at(tags: &[Tag], kind: IoKind, channel: u16) -> Option<&Tag> {
    tags.iter().find(|t| {
        matches!(board::parse_address(&t.address), Ok((board::Address::Io { kind: k, channel: c }, _)) if k == kind && c == channel)
    })
}

/// Petit libellé gris au-dessus d'une liste.
fn caption(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(theme::colors(ui).soft));
}

/// Champs blancs sur les cartes grises, comme les formulaires iOS.
fn white_fields(ui: &mut Ui) {
    let c = theme::colors(ui);
    ui.visuals_mut().extreme_bg_color = c.bg;
}

fn soft(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text).color(theme::colors(ui).soft)
}

/// Barre de progression fine et arrondie.
fn slim_bar(ui: &mut Ui, ratio: f32, color: Color32, width: f32) {
    let c = theme::colors(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 8.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, c.very_soft.gamma_multiply(0.5));
    let mut fill = rect;
    fill.set_width(rect.width() * ratio.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, 4.0, color);
}

// ---------------------------------------------------------------------------
// Appareil

pub fn device_view(app: &mut App, ui: &mut Ui) {
    let c = theme::colors(ui);
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let board = board::find(&app.project.device.board).unwrap_or(&BOARDS[0]);
            theme::card(ui, |ui| draw_board(ui, board, &app.project.tags));
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_max_width(460.0);
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    white_fields(ui);
                    device_settings(app, ui);
                });
                ui.add_space(12.0);
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    cycle_card(app, ui);
                });
            });
        });
        ui.add_space(18.0);
        theme::section_title(ui, "Voies d'entrées / sorties");
        ui.add_space(6.0);
        let board = board::find(&app.project.device.board).unwrap_or(&BOARDS[0]);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("io_overview").num_columns(5).spacing(vec2(28.0, 10.0)).striped(false).show(ui, |ui| {
                for h in ["Adresse", "Type", "Broche", "Variable", "Commentaire"] {
                    caption(ui, h);
                }
                ui.end_row();
                for kind in [IoKind::Di, IoKind::Dq, IoKind::Ai, IoKind::Aq] {
                    for (ch, pin) in board.pins(kind).iter().enumerate() {
                        let tag = tag_at(&app.project.tags, kind, ch as u16);
                        ui.label(RichText::new(Board::address_of(kind, ch as u16)).monospace());
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, kind_color(&c, kind));
                            ui.label(kind_name(kind));
                        });
                        ui.label(soft(ui, format!("GP{pin}")));
                        match tag {
                            Some(t) => ui.label(RichText::new(&t.name).font(FontId::new(14.0, theme::medium()))),
                            None => ui.label(soft(ui, "libre")),
                        };
                        ui.label(soft(ui, tag.map(|t| t.comment.as_str()).unwrap_or("")));
                        ui.end_row();
                    }
                }
            });
        });
        ui.add_space(24.0);
    });
}

fn device_settings(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    theme::section_title(ui, "Réglages");
    ui.add_space(8.0);
    egui::Grid::new("device_props").num_columns(2).spacing(vec2(18.0, 10.0)).show(ui, |ui| {
        ui.label(soft(ui, "Nom"));
        changed |= ui.add_sized([240.0, 26.0], egui::TextEdit::singleline(&mut app.project.device.name)).changed();
        ui.end_row();
        ui.label(soft(ui, "Carte"));
        let current = board_name(&app.project.device.board);
        egui::ComboBox::from_id_salt("board").selected_text(current).width(240.0).show_ui(ui, |ui| {
            for b in BOARDS {
                changed |= ui.selectable_value(&mut app.project.device.board, b.id.to_string(), b.name).changed();
            }
        });
        ui.end_row();
        ui.label("");
        ui.label(soft(ui, board::find(&app.project.device.board).map(|b| b.description).unwrap_or("")).size(12.5));
        ui.end_row();
        ui.label(soft(ui, "Temps de cycle"));
        changed |= ui
            .add(egui::DragValue::new(&mut app.project.device.cycle_us).range(100..=1_000_000).speed(10).suffix(" µs"))
            .on_hover_text("Période fixe du cycle automate. Le compilateur prouve que le programme tient dedans.")
            .changed();
        ui.end_row();
    });
    if changed {
        app.touch();
    }
}

fn cycle_card(app: &mut App, ui: &mut Ui) {
    let c = theme::colors(ui);
    theme::section_title(ui, "Temps de cycle garanti");
    ui.add_space(8.0);
    match (&app.compiled, app.up_to_date()) {
        (Some(cp), true) => {
            let w = &cp.wcet;
            let ratio = w.ratio() as f32;
            let color = if ratio > 1.0 {
                c.red
            } else if ratio > takt_compiler::WCET_WARN_RATIO as f32 {
                c.orange
            } else {
                c.green
            };
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} µs", w.total_us)).font(FontId::new(26.0, theme::semi())).color(color),
                );
                ui.label(soft(ui, format!("au pire, pour un cycle de {} µs", w.cycle_us)));
            });
            slim_bar(ui, ratio, color, ui.available_width());
            ui.label(soft(ui, format!("{:.0} % du cycle utilisé au pire cas", ratio * 100.0)).size(12.5));
            ui.add_space(8.0);
            egui::Grid::new("wcet").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                for b in &w.blocks {
                    ui.label(&b.name);
                    ui.label(soft(ui, format!("{} µs", takt_vm::cost::cycles_to_us(b.cycles))));
                    ui.end_row();
                }
                ui.label("Partie fixe (E/S, forçage, communication)");
                ui.label(soft(ui, format!("{} µs", takt_vm::cost::cycles_to_us(w.overhead_cycles))));
                ui.end_row();
            });
        }
        _ => {
            ui.label(soft(ui, "Compilez pour calculer le pire temps de cycle."));
            ui.add_space(4.0);
            if theme::secondary_button(ui, icon::HAMMER, "Compiler").clicked() {
                app.compile();
            }
        }
    }
}

/// Schéma épuré de la carte avec les voies utilisées.
fn draw_board(ui: &mut Ui, board: &Board, tags: &[Tag]) {
    let c = theme::colors(ui);
    let pin_h = 21.0;
    let mut used = Vec::new();
    for kind in [IoKind::Di, IoKind::Dq, IoKind::Ai, IoKind::Aq] {
        for (ch, pin) in board.pins(kind).iter().enumerate() {
            used.push((*pin, kind, ch as u16, tag_at(tags, kind, ch as u16).map(|t| t.name.clone())));
        }
    }
    used.sort_by_key(|u| u.0);
    let half = used.len().div_ceil(2);
    let size = vec2(420.0, half as f32 * pin_h + 70.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    let pcb = egui::Rect::from_center_size(rect.center(), vec2(96.0, half as f32 * pin_h + 50.0));
    p.rect_filled(pcb, 14.0, if ui.visuals().dark_mode { Color32::from_gray(60) } else { Color32::from_gray(225) });
    let chip = egui::Rect::from_center_size(pcb.center(), vec2(44.0, 44.0));
    p.rect_filled(chip, 6.0, if ui.visuals().dark_mode { Color32::from_gray(30) } else { Color32::from_gray(45) });
    p.text(chip.center(), egui::Align2::CENTER_CENTER, "RP2350", FontId::new(9.5, theme::medium()), Color32::WHITE);
    let usb = egui::Rect::from_center_size(pos2(pcb.center().x, pcb.top() + 2.0), vec2(28.0, 10.0));
    p.rect_filled(usb, 3.0, c.very_soft);
    for (i, (pin, kind, ch, tag)) in used.iter().enumerate() {
        let left = i < half;
        let row = if left { i } else { i - half } as f32;
        let y = pcb.top() + 34.0 + row * pin_h;
        let x = if left { pcb.left() } else { pcb.right() };
        let col = kind_color(&c, *kind);
        p.circle_filled(pos2(x, y), 4.5, if tag.is_some() { col } else { c.very_soft });
        let (anchor, tx) =
            if left { (egui::Align2::RIGHT_CENTER, x - 10.0) } else { (egui::Align2::LEFT_CENTER, x + 10.0) };
        let label = match tag {
            Some(t) => t.to_string(),
            None => format!("GP{pin}"),
        };
        let addr = Board::address_of(*kind, *ch);
        let text = if left { format!("{label}  {addr}") } else { format!("{addr}  {label}") };
        let color = if tag.is_some() { c.text } else { c.soft };
        p.text(pos2(tx, y), anchor, text, FontId::proportional(11.5), color);
    }
}

// ---------------------------------------------------------------------------
// Variables API

pub fn tags_view(app: &mut App, ui: &mut Ui) {
    let c = theme::colors(ui);
    ui.horizontal(|ui| {
        if theme::secondary_button(ui, icon::PLUS, "Ajouter une variable").clicked() {
            app.project.tags.push(Tag {
                name: format!("Variable_{}", app.project.tags.len() + 1),
                ty: "BOOL".into(),
                ..Default::default()
            });
            app.touch();
        }
    });
    ui.add_space(10.0);
    let online = app.online() && app.compiled.is_some();
    let errors: Vec<(u32, String)> = app
        .diags
        .iter()
        .filter(|d| d.block == BLOCK_TAGS && d.severity == Severity::Error)
        .map(|d| (d.pos.line, d.msg.clone()))
        .collect();
    let mut changed = false;
    let mut remove = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            white_fields(ui);
            let cols = if online { 7 } else { 6 };
            egui::Grid::new("tags").num_columns(cols).spacing(vec2(10.0, 8.0)).striped(false).show(ui, |ui| {
                ui.label("");
                for h in ["Nom", "Type", "Adresse", "Commentaire"] {
                    caption(ui, h);
                }
                if online {
                    caption(ui, "Valeur");
                }
                ui.label("");
                ui.end_row();
                for (i, t) in app.project.tags.iter_mut().enumerate() {
                    match errors.iter().find(|(l, _)| *l == i as u32 + 1) {
                        Some((_, m)) => {
                            ui.label(theme::icon_text(icon::ALERT, 15.0).color(c.red)).on_hover_text(m);
                        }
                        None => {
                            let dot = match board::parse_address(&t.address) {
                                Ok((board::Address::Io { kind, .. }, _)) => kind_color(&c, kind),
                                _ => c.very_soft,
                            };
                            let (r, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, dot);
                        }
                    }
                    changed |= ui.add_sized([180.0, 26.0], egui::TextEdit::singleline(&mut t.name)).changed();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        changed |= ui.add_sized([130.0, 26.0], egui::TextEdit::singleline(&mut t.ty)).changed();
                        ui.menu_button(theme::icon_text(icon::CHEVRON_DOWN, 13.0).color(c.soft), |ui| {
                            for ty in ["BOOL", "INT", "DINT", "REAL", "TIME", "ARRAY[0..9] OF INT"] {
                                if ui.button(ty).clicked() {
                                    t.ty = ty.into();
                                    changed = true;
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                    changed |= ui
                        .add_sized(
                            [80.0, 26.0],
                            egui::TextEdit::singleline(&mut t.address).font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    changed |= ui.add_sized([280.0, 26.0], egui::TextEdit::singleline(&mut t.comment)).changed();
                    if online {
                        let cp = app.compiled.as_ref().unwrap();
                        match cp.resolve(&t.name) {
                            Some((slot, ty)) => value_label(ui, app.values.get(&slot).copied(), &ty),
                            None => {
                                ui.label("");
                            }
                        }
                    }
                    if theme::ghost_icon(ui, icon::TRASH, "Supprimer la variable").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        });
        ui.add_space(24.0);
    });
    if let Some(i) = remove {
        app.project.tags.remove(i);
        changed = true;
    }
    if changed {
        app.touch();
    }
}

fn value_label(ui: &mut Ui, v: Option<u32>, ty: &Ty) {
    let c = theme::colors(ui);
    match v {
        Some(v) if *ty == Ty::Bool => {
            let on = v != 0;
            theme::chip(ui, if on { "TRUE" } else { "FALSE" }, if on { c.green } else { c.soft });
        }
        Some(v) => {
            ui.label(RichText::new(format_value(v, ty)).monospace().color(c.text));
        }
        None => {
            ui.label(soft(ui, "…"));
        }
    }
}

// ---------------------------------------------------------------------------
// Tables de visualisation et de forçage

pub fn watch_view(app: &mut App, ui: &mut Ui, i: usize) {
    let c = theme::colors(ui);
    let online = app.online();
    ui.horizontal(|ui| {
        ui.add_enabled_ui(online, |ui| {
            if theme::secondary_button(ui, icon::ZAP, "Modifier une fois")
                .on_hover_text("Écrit les valeurs de modification des lignes non forcées")
                .clicked()
            {
                modify_once(app, i);
            }
            if theme::secondary_button(ui, icon::LOCK_OPEN, "Tout déforcer").clicked()
                && app.with_target(|t| target::call(t, proto::UNFORCE_ALL, &[])).is_some()
            {
                for r in &mut app.project.watch_tables[i].rows {
                    r.force = false;
                }
                app.log("Tous les forçages sont supprimés.");
            }
        });
        ui.add_space(8.0);
        if !online {
            ui.label(soft(ui, "Connectez-vous pour visualiser et forcer."));
        } else if !app.up_to_date() {
            theme::chip(ui, "projet modifié depuis la compilation", c.orange);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut name = app.project.watch_tables[i].name.clone();
            white_fields(ui);
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0).hint_text("Nom de la table")).changed()
            {
                app.project.watch_tables[i].name = name;
                app.touch();
            }
            ui.label(soft(ui, "Nom"));
        });
    });
    ui.add_space(10.0);

    let mut actions: Vec<(usize, bool)> = Vec::new();
    let mut remove = None;
    let mut changed = false;
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        white_fields(ui);
        egui::Grid::new(("watch", i)).num_columns(6).spacing(vec2(14.0, 8.0)).striped(false).show(ui, |ui| {
            for h in ["Variable", "Type", "Valeur", "Valeur de modification", "Forcer", ""] {
                caption(ui, h);
            }
            ui.end_row();
            let compiled = app.compiled.as_ref();
            for (k, r) in app.project.watch_tables[i].rows.iter_mut().enumerate() {
                let resolved = compiled.and_then(|cp| cp.resolve(&r.path));
                changed |= ui
                    .add_sized([250.0, 26.0], egui::TextEdit::singleline(&mut r.path).font(egui::TextStyle::Monospace))
                    .changed();
                match &resolved {
                    Some((_, ty)) => ui.label(soft(ui, ty.name())),
                    None if compiled.is_some() => ui.label(RichText::new("introuvable").color(c.red)),
                    None => ui.label(""),
                };
                match &resolved {
                    Some((slot, ty)) if online => {
                        ui.horizontal(|ui| {
                            if r.force {
                                theme::chip(ui, "F", c.red).on_hover_text("Valeur forcée");
                            }
                            value_label(ui, app.values.get(slot).copied(), ty);
                        });
                    }
                    _ => {
                        ui.label("");
                    }
                }
                let valid = match &resolved {
                    Some((_, ty)) => r.modify.is_empty() || parse_value(&r.modify, ty).is_some(),
                    None => true,
                };
                let mut edit =
                    egui::TextEdit::singleline(&mut r.modify).font(egui::TextStyle::Monospace).hint_text("valeur");
                if !valid {
                    edit = edit.text_color(c.red);
                }
                changed |= ui.add_sized([150.0, 26.0], edit).changed();
                let mut f = r.force;
                if ui.add_enabled_ui(online && resolved.is_some(), |ui| theme::switch(ui, &mut f)).inner {
                    actions.push((k, f));
                }
                if theme::ghost_icon(ui, icon::TRASH, "Retirer la ligne").clicked() {
                    remove = Some(k);
                }
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.new_watch_row)
                    .hint_text("Ajouter une variable : \"Moteur\", Main.tempo.ET, Main.tab[2]…")
                    .desired_width(360.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (theme::ghost_icon(ui, icon::PLUS, "Ajouter").clicked() || enter) && !app.new_watch_row.trim().is_empty()
            {
                let path = std::mem::take(&mut app.new_watch_row);
                app.project.watch_tables[i].rows.push(WatchRow { path: path.trim().into(), ..Default::default() });
                changed = true;
            }
            if let Some(cp) = &app.compiled {
                ui.menu_button(soft(ui, "Choisir dans la liste…"), |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for s in cp.symbols.iter().filter(|s| s.ty.is_scalar()) {
                            if ui.button(&s.path).clicked() {
                                app.project.watch_tables[i]
                                    .rows
                                    .push(WatchRow { path: s.path.clone(), ..Default::default() });
                                changed = true;
                                ui.close_menu();
                            }
                        }
                    });
                });
            }
        });
    });
    for (k, f) in actions {
        set_force(app, i, k, f);
    }
    if let Some(k) = remove {
        app.project.watch_tables[i].rows.remove(k);
        changed = true;
    }
    if changed {
        app.touch();
    }
}

fn modify_once(app: &mut App, i: usize) {
    let Some(c) = app.compiled.clone() else {
        return;
    };
    let rows = app.project.watch_tables[i].rows.clone();
    for r in rows.iter().filter(|r| !r.force && !r.modify.trim().is_empty()) {
        let Some((slot, ty)) = c.resolve(&r.path) else {
            continue;
        };
        match parse_value(&r.modify, &ty) {
            Some(v) => {
                if app.with_target(|t| target::write(t, slot, v)).is_some() {
                    app.log(format!("{} := {}", r.path, r.modify));
                }
            }
            None => app.log(format!("Valeur invalide pour {} : « {} »", r.path, r.modify)),
        }
    }
}

fn set_force(app: &mut App, i: usize, k: usize, force: bool) {
    let Some(c) = app.compiled.clone() else {
        return;
    };
    let row = app.project.watch_tables[i].rows[k].clone();
    let Some((slot, ty)) = c.resolve(&row.path) else {
        return;
    };
    let value = if force {
        match parse_value(&row.modify, &ty) {
            Some(v) => Some(v),
            None => {
                app.log(format!("Indiquez une valeur de forçage valide pour {}", row.path));
                return;
            }
        }
    } else {
        None
    };
    if app.with_target(|t| target::force(t, slot, value)).is_some() {
        app.project.watch_tables[i].rows[k].force = force;
        app.touch();
        app.log(if force {
            format!("Forçage : {} = {}", row.path, row.modify)
        } else {
            format!("Forçage supprimé : {}", row.path)
        });
    }
}

// ---------------------------------------------------------------------------
// Tiroir du bas

pub fn inspector(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let errors = app.diags.iter().filter(|d| d.severity == Severity::Error).count();
        let messages = if errors > 0 { format!("Messages · {errors}") } else { "Messages".into() };
        theme::segmented(
            ui,
            &mut app.bottom,
            &[
                (BottomTab::Info, messages.as_str()),
                (BottomTab::Diagnostic, "Diagnostic"),
                (BottomTab::Simulation, "Simulation"),
                (BottomTab::Journal, "Journal"),
            ],
        );
    });
    ui.add_space(8.0);
    match app.bottom {
        BottomTab::Info => info_tab(app, ui),
        BottomTab::Diagnostic => diagnostic_tab(app, ui),
        BottomTab::Simulation => simulation_tab(app, ui),
        BottomTab::Journal => {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
                for l in &app.journal {
                    ui.label(soft(ui, l).monospace().size(12.5));
                }
            });
        }
    }
}

fn info_tab(app: &mut App, ui: &mut Ui) {
    let c = theme::colors(ui);
    if app.diags.is_empty() {
        ui.label(soft(ui, "Aucun message. Compilez avec Ctrl+B."));
        return;
    }
    let mut jump = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        for d in &app.diags {
            let (ic, color) = match d.severity {
                Severity::Error => (icon::ALERT, c.red),
                Severity::Warning => (icon::WARNING, c.orange),
                Severity::Info => (icon::CHECK, c.green),
            };
            let r = ui
                .horizontal(|ui| {
                    ui.label(theme::icon_text(ic, 15.0).color(color));
                    let place =
                        if d.pos.line > 0 { format!("{} · ligne {}", d.block, d.pos.line) } else { d.block.clone() };
                    ui.label(RichText::new(place).font(FontId::new(13.5, theme::medium())));
                    ui.label(RichText::new(&d.msg).color(c.text));
                })
                .response
                .interact(Sense::click());
            if r.hovered() {
                ui.painter().rect_filled(r.rect.expand(2.0), 6.0, c.hover);
            }
            if r.clicked() {
                jump = Some(d.clone());
            }
        }
    });
    if let Some(d) = jump {
        if d.block == BLOCK_TAGS {
            app.open(View::Tags);
        } else if d.block == BLOCK_DEVICE {
            app.open(View::Device);
        } else if let Some(i) = app.project.blocks.iter().position(|b| b.name == d.block) {
            app.monitor = false;
            app.open(View::Block(i));
            app.goto = Some((i, d.pos.line));
        }
    }
}

/// Tuile de statistique : grande valeur, libellé en dessous.
fn stat(ui: &mut Ui, value: String, label: &str, color: Color32) {
    theme::card(ui, |ui| {
        ui.set_min_width(150.0);
        ui.label(RichText::new(value).font(FontId::new(22.0, theme::semi())).color(color));
        ui.label(soft(ui, label).size(12.5));
    });
}

fn diagnostic_tab(app: &mut App, ui: &mut Ui) {
    let c = theme::colors(ui);
    let Some(st) = app.status else {
        ui.label(soft(ui, "Hors ligne. Connectez-vous pour lire le diagnostic de la CPU."));
        return;
    };
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let (mode, color) = match Mode::from_u8(st.mode) {
                Mode::Run => ("RUN", c.green),
                Mode::Stop => ("STOP", c.orange),
                Mode::Fault => ("DÉFAUT", c.red),
                Mode::NoProgram => ("vide", c.soft),
            };
            let label = takt_vm::exec::Fault::from_u8(st.fault).map(|f| f.describe()).unwrap_or("état de la CPU");
            stat(ui, mode.into(), label, color);
            stat(ui, format!("{} µs", st.wcet_us), "pire cas prouvé", c.text);
            stat(ui, format!("{} µs", st.exec_max_us), &format!("mesuré max (min {} µs)", st.exec_min_us), c.text);
            stat(ui, format!("{} µs", st.late_max_us), "gigue max du départ", c.text);
            let over = if st.overruns > 0 { c.red } else { c.green };
            stat(ui, st.overruns.to_string(), "dépassements", over);
            theme::card(ui, |ui| {
                ui.label(soft(ui, "Temps d'exécution, en % du cycle").size(12.5));
                histogram(ui, &st.hist);
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut flags = Vec::new();
            if st.diag & takt_vm::exec::DIAG_DIV0 != 0 {
                flags.push("division par zéro");
            }
            if st.diag & takt_vm::exec::DIAG_INDEX != 0 {
                flags.push("index de tableau hors bornes");
            }
            ui.label(soft(ui, format!("{} cycles exécutés", st.scans)));
            for f in flags {
                theme::chip(ui, f, c.orange);
            }
            if app.target.as_mut().is_some_and(|t| t.as_sim().is_some()) {
                ui.label(soft(ui, "· simulation : temps mesurés sur le PC, pas sur le RP2350").size(12.5));
            }
            if theme::ghost_icon(ui, icon::REFRESH, "Remettre les statistiques à zéro").clicked() {
                app.with_target(|t| target::call(t, proto::RESET_STATS, &[]));
            }
        });
    });
}

fn histogram(ui: &mut Ui, hist: &[u32; 8]) {
    let c = theme::colors(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(260.0, 64.0), Sense::hover());
    let p = ui.painter_at(rect);
    let max = hist.iter().copied().max().unwrap_or(0).max(1) as f32;
    let w = rect.width() / 8.0;
    for (i, &n) in hist.iter().enumerate() {
        let h = ((n as f32 / max) * (rect.height() - 16.0)).max(if n > 0 { 2.0 } else { 0.0 });
        let x = rect.left() + i as f32 * w;
        let bar =
            egui::Rect::from_min_max(pos2(x + 3.0, rect.bottom() - 14.0 - h), pos2(x + w - 3.0, rect.bottom() - 14.0));
        p.rect_filled(bar, 3.0, if i >= 6 { c.orange } else { c.accent });
        p.text(
            pos2(x + w / 2.0, rect.bottom() - 5.0),
            egui::Align2::CENTER_CENTER,
            format!("{}", (i + 1) * 125 / 10),
            FontId::proportional(9.5),
            c.soft,
        );
    }
}

/// Simulateur : interrupteurs pour les entrées, voyants pour les sorties.
fn simulation_tab(app: &mut App, ui: &mut Ui) {
    let c = theme::colors(ui);
    let Some(sim) = app.target.as_mut().and_then(|t| t.as_sim()) else {
        ui.label(soft(ui, "Choisissez la cible « Simulateur PLCSIM » (onglet Automate) puis passez en ligne."));
        return;
    };
    let shared = sim.shared.clone();
    let io = sim.io();
    if io.is_empty() {
        ui.label(soft(ui, "Aucune entrée/sortie : chargez d'abord le programme (Ctrl+L)."));
        return;
    }
    let names: Vec<(u16, String)> = app
        .compiled
        .as_ref()
        .map(|cp| cp.symbols.iter().filter(|s| s.io.is_some()).map(|s| (s.slot, s.path.clone())).collect())
        .unwrap_or_default();
    let name_of = |slot: u16, pin: u8| {
        names.iter().find(|(s, _)| *s == slot).map(|(_, n)| n.clone()).unwrap_or(format!("GP{pin}"))
    };
    let mut g = shared.lock().unwrap();
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            theme::card(ui, |ui| {
                ui.set_width(560.0);
                caption(ui, "Entrées");
                ui.horizontal_wrapped(|ui| {
                    for e in io.iter().filter(|e| e.kind.is_input()) {
                        let v = g.inputs.entry(e.pin).or_insert(0);
                        let w = if e.kind == IoKind::Di { 176.0 } else { 360.0 };
                        ui.allocate_ui_with_layout(
                            vec2(w, 28.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                if e.kind == IoKind::Di {
                                    let mut on = *v != 0;
                                    if theme::switch(ui, &mut on) {
                                        *v = on as u32;
                                    }
                                } else {
                                    let mut x = *v as i32;
                                    if ui.add(egui::Slider::new(&mut x, 0..=27648).show_value(true)).changed() {
                                        *v = x as u32;
                                    }
                                }
                                ui.label(name_of(e.slot, e.pin));
                                ui.set_min_width(w);
                            },
                        );
                    }
                });
            });
            ui.add_space(12.0);
            theme::card(ui, |ui| {
                ui.set_width(400.0);
                caption(ui, "Sorties");
                ui.horizontal_wrapped(|ui| {
                    for e in io.iter().filter(|e| !e.kind.is_input()) {
                        let v = g.outputs.get(&e.pin).copied().unwrap_or(0);
                        ui.allocate_ui_with_layout(
                            vec2(190.0, 28.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                if e.kind == IoKind::Dq {
                                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                                    if v != 0 {
                                        ui.painter().circle_filled(r.center(), 9.0, c.orange.gamma_multiply(0.25));
                                    }
                                    ui.painter().circle_filled(
                                        r.center(),
                                        6.0,
                                        if v != 0 { c.orange } else { c.very_soft },
                                    );
                                } else {
                                    slim_bar(ui, v as i32 as f32 / 27648.0, c.accent, 120.0);
                                    ui.label(soft(ui, format!("{}", v as i32)));
                                }
                                ui.label(name_of(e.slot, e.pin));
                                ui.set_min_width(190.0);
                            },
                        );
                    }
                });
            });
        });
    });
    drop(g);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
}
