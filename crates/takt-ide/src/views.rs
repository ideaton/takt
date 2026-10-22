//! Vues du projet : appareil, variables API, tables de visualisation et
//! inspecteur (Info, Diagnostic, Simulation, Journal).

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use takt_compiler::board::{self, BOARDS, Board};
use takt_compiler::project::{Tag, WatchRow};
use takt_compiler::types::{Ty, format_value, parse_value};
use takt_compiler::{BLOCK_DEVICE, BLOCK_TAGS, Severity};
use takt_vm::image::IoKind;
use takt_vm::proto;
use takt_vm::runtime::Mode;

use crate::app::{ACCENT, App, BottomTab, GREEN, ORANGE, RED, View};
use crate::target;

pub fn board_name(id: &str) -> &'static str {
    board::find(id).map(|b| b.name).unwrap_or("carte inconnue")
}

fn kind_color(k: IoKind) -> Color32 {
    match k {
        IoKind::Di => Color32::from_rgb(40, 150, 60),
        IoKind::Dq => Color32::from_rgb(220, 120, 0),
        IoKind::Ai => Color32::from_rgb(40, 110, 200),
        IoKind::Aq => Color32::from_rgb(140, 70, 180),
    }
}

fn kind_name(k: IoKind) -> &'static str {
    match k {
        IoKind::Di => "Entrée TOR",
        IoKind::Dq => "Sortie TOR",
        IoKind::Ai => "Entrée analogique",
        IoKind::Aq => "Sortie analogique (PWM)",
    }
}

/// Variable API reliée à une voie, d'après la table du projet.
fn tag_at(tags: &[Tag], kind: IoKind, channel: u16) -> Option<&Tag> {
    tags.iter().find(|t| {
        matches!(board::parse_address(&t.address), Ok((board::Address::Io { kind: k, channel: c }, _)) if k == kind && c == channel)
    })
}

// ---------------------------------------------------------------------------
// Configuration de l'appareil

pub fn device_view(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.heading(format!("Vue des appareils — {}", app.project.device.name));
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            let board = board::find(&app.project.device.board).unwrap_or(&BOARDS[0]);
            draw_board(ui, board, &app.project.tags);
            ui.add_space(16.0);
            ui.vertical(|ui| {
                device_properties(app, ui);
            });
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Vue d'ensemble des voies").strong());
        let board = board::find(&app.project.device.board).unwrap_or(&BOARDS[0]);
        egui::Grid::new("io_overview").striped(true).num_columns(5).show(ui, |ui| {
            for h in ["Adresse", "Type", "GPIO", "Variable", "Commentaire"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for kind in [IoKind::Di, IoKind::Dq, IoKind::Ai, IoKind::Aq] {
                for (ch, pin) in board.pins(kind).iter().enumerate() {
                    let tag = tag_at(&app.project.tags, kind, ch as u16);
                    ui.label(RichText::new(Board::address_of(kind, ch as u16)).monospace());
                    ui.label(RichText::new(kind_name(kind)).color(kind_color(kind)));
                    ui.label(format!("GP{pin}"));
                    ui.label(tag.map(|t| t.name.as_str()).unwrap_or("—"));
                    ui.label(RichText::new(tag.map(|t| t.comment.as_str()).unwrap_or("")).weak());
                    ui.end_row();
                }
            }
        });
    });
}

fn device_properties(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    egui::Grid::new("device_props").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        ui.label("Nom de l'appareil");
        changed |= ui.text_edit_singleline(&mut app.project.device.name).changed();
        ui.end_row();
        ui.label("Carte");
        let current = board_name(&app.project.device.board);
        egui::ComboBox::from_id_salt("board").selected_text(current).show_ui(ui, |ui| {
            for b in BOARDS {
                changed |= ui.selectable_value(&mut app.project.device.board, b.id.to_string(), b.name).changed();
            }
        });
        ui.end_row();
        ui.label("");
        ui.label(RichText::new(board::find(&app.project.device.board).map(|b| b.description).unwrap_or("")).weak());
        ui.end_row();
        ui.label("Temps de cycle");
        changed |= ui
            .add(egui::DragValue::new(&mut app.project.device.cycle_us).range(100..=1_000_000).speed(10).suffix(" µs"))
            .on_hover_text("Période fixe du cycle automate. Le compilateur prouve que le programme tient dedans.")
            .changed();
        ui.end_row();
    });
    if changed {
        app.touch();
    }
    ui.add_space(12.0);
    ui.label(RichText::new("Temps de cycle garanti").strong());
    match (&app.compiled, app.up_to_date()) {
        (Some(c), true) => {
            let w = &c.wcet;
            let ratio = w.ratio() as f32;
            let color = if ratio > 1.0 {
                RED
            } else if ratio > takt_compiler::WCET_WARN_RATIO as f32 {
                ORANGE
            } else {
                GREEN
            };
            ui.add(egui::ProgressBar::new(ratio.min(1.0)).desired_width(320.0).fill(color).text(format!(
                "pire cas {} µs / cycle {} µs ({:.0} %)",
                w.total_us,
                w.cycle_us,
                ratio * 100.0
            )));
            egui::Grid::new("wcet").num_columns(3).striped(true).show(ui, |ui| {
                for b in &w.blocks {
                    ui.label(&b.name);
                    ui.label(format!("{} cycles", b.cycles));
                    ui.label(format!("{} µs", takt_vm::cost::cycles_to_us(b.cycles)));
                    ui.end_row();
                }
                ui.label("Partie fixe (E/S, forçage, communication)");
                ui.label(format!("{} cycles", w.overhead_cycles));
                ui.label(format!("{} µs", takt_vm::cost::cycles_to_us(w.overhead_cycles)));
                ui.end_row();
            });
        }
        _ => {
            ui.label(RichText::new("Compilez (Ctrl+B) pour calculer le pire temps de cycle.").weak());
            if ui.button("⚙ Compiler").clicked() {
                app.compile();
            }
        }
    }
}

/// Dessin schématique de la carte, avec les voies utilisées.
fn draw_board(ui: &mut Ui, board: &Board, tags: &[Tag]) {
    let pin_h = 18.0;
    let n_pins = 20.0;
    let size = vec2(360.0, n_pins * pin_h + 60.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    let pcb = egui::Rect::from_center_size(rect.center(), vec2(110.0, n_pins * pin_h + 40.0));
    p.rect_filled(pcb, 6.0, Color32::from_rgb(20, 110, 60));
    p.rect_stroke(pcb, 6.0, Stroke::new(1.0_f32, Color32::from_rgb(10, 70, 40)), StrokeKind::Inside);
    let chip = egui::Rect::from_center_size(pcb.center(), vec2(46.0, 46.0));
    p.rect_filled(chip, 2.0, Color32::from_gray(30));
    p.text(chip.center(), egui::Align2::CENTER_CENTER, "RP2350", egui::FontId::proportional(10.0), Color32::WHITE);
    let usb = egui::Rect::from_center_size(pos2(pcb.center().x, pcb.top() + 4.0), vec2(30.0, 14.0));
    p.rect_filled(usb, 2.0, Color32::from_gray(180));

    // Broches utilisées, réparties sur les deux côtés
    let mut used = Vec::new();
    for kind in [IoKind::Di, IoKind::Dq, IoKind::Ai, IoKind::Aq] {
        for (ch, pin) in board.pins(kind).iter().enumerate() {
            let tag = tag_at(tags, kind, ch as u16).map(|t| t.name.clone());
            used.push((*pin, kind, ch as u16, tag));
        }
    }
    used.sort_by_key(|u| u.0);
    let half = used.len().div_ceil(2);
    for (i, (pin, kind, ch, tag)) in used.iter().enumerate() {
        let left = i < half;
        let row = if left { i } else { i - half } as f32;
        let y = pcb.top() + 30.0 + row * pin_h;
        let x = if left { pcb.left() } else { pcb.right() };
        let c = kind_color(*kind);
        p.circle_filled(pos2(x, y), 5.0, if tag.is_some() { c } else { Color32::from_gray(200) });
        let label = format!("GP{pin} {}", Board::address_of(*kind, *ch));
        let text = match tag {
            Some(t) => format!("{label}  {t}"),
            None => label,
        };
        let (anchor, tx) =
            if left { (egui::Align2::RIGHT_CENTER, x - 9.0) } else { (egui::Align2::LEFT_CENTER, x + 9.0) };
        let color = if tag.is_some() { c } else { ui.visuals().weak_text_color() };
        p.text(pos2(tx, y), anchor, text, egui::FontId::monospace(10.5), color);
    }
}

// ---------------------------------------------------------------------------
// Variables API

pub fn tags_view(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.heading("Variables API — table standard");
        if ui.button("➕ Ajouter").clicked() {
            app.project.tags.push(Tag {
                name: format!("Variable_{}", app.project.tags.len() + 1),
                ty: "BOOL".into(),
                ..Default::default()
            });
            app.touch();
        }
    });
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
        egui::Grid::new("tags").striped(true).num_columns(if online { 7 } else { 6 }).spacing(vec2(8.0, 4.0)).show(
            ui,
            |ui| {
                ui.label("");
                for h in ["Nom", "Type", "Adresse", "Commentaire"] {
                    ui.label(RichText::new(h).strong());
                }
                if online {
                    ui.label(RichText::new("Valeur de visualisation").strong());
                }
                ui.label("");
                ui.end_row();
                for (i, t) in app.project.tags.iter_mut().enumerate() {
                    let err = errors.iter().find(|(l, _)| *l == i as u32 + 1);
                    match err {
                        Some((_, m)) => {
                            ui.label(RichText::new("●").color(RED)).on_hover_text(m);
                        }
                        None => {
                            ui.label(RichText::new(icon_for(&t.address)).weak());
                        }
                    }
                    changed |= ui.add(egui::TextEdit::singleline(&mut t.name).desired_width(150.0)).changed();
                    ui.horizontal(|ui| {
                        changed |= ui.add(egui::TextEdit::singleline(&mut t.ty).desired_width(110.0)).changed();
                        ui.menu_button("▾", |ui| {
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
                        .add(
                            egui::TextEdit::singleline(&mut t.address)
                                .desired_width(70.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    changed |= ui.add(egui::TextEdit::singleline(&mut t.comment).desired_width(240.0)).changed();
                    if online {
                        let c = app.compiled.as_ref().unwrap();
                        match c.resolve(&t.name) {
                            Some((slot, ty)) => value_label(ui, app.values.get(&slot).copied(), &ty),
                            None => {
                                ui.label("");
                            }
                        }
                    }
                    if ui.small_button("🗑").on_hover_text("Supprimer la variable").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            },
        );
    });
    if let Some(i) = remove {
        app.project.tags.remove(i);
        changed = true;
    }
    if changed {
        app.touch();
    }
}

fn icon_for(address: &str) -> &'static str {
    match board::parse_address(address) {
        Ok((board::Address::Io { kind: IoKind::Di | IoKind::Ai, .. }, _)) => "⮊",
        Ok((board::Address::Io { .. }, _)) => "⮈",
        _ => "▫",
    }
}

fn value_label(ui: &mut Ui, v: Option<u32>, ty: &Ty) {
    match v {
        Some(v) => {
            let text = format_value(v, ty);
            let color = match ty {
                Ty::Bool if v != 0 => GREEN,
                Ty::Bool => Color32::from_gray(130),
                _ => ui.visuals().text_color(),
            };
            ui.label(RichText::new(text).monospace().color(color));
        }
        None => {
            ui.label(RichText::new("…").weak());
        }
    }
}

// ---------------------------------------------------------------------------
// Tables de visualisation et de forçage

pub fn watch_view(app: &mut App, ui: &mut Ui, i: usize) {
    let online = app.online();
    ui.horizontal(|ui| {
        let mut name = app.project.watch_tables[i].name.clone();
        if ui.add(egui::TextEdit::singleline(&mut name).font(egui::TextStyle::Heading).desired_width(320.0)).changed() {
            app.project.watch_tables[i].name = name;
            app.touch();
        }
    });
    ui.horizontal(|ui| {
        ui.add_enabled_ui(online, |ui| {
            if ui
                .button("⚡ Modifier une fois")
                .on_hover_text("Écrit les valeurs de modification non forcées")
                .clicked()
            {
                modify_once(app, i);
            }
            if ui.button("🔓 Tout déforcer").clicked()
                && app.with_target(|t| target::call(t, proto::UNFORCE_ALL, &[])).is_some()
            {
                for r in &mut app.project.watch_tables[i].rows {
                    r.force = false;
                }
                app.log("Tous les forçages sont supprimés.");
            }
        });
        if !online {
            ui.label(RichText::new("Passez en ligne (Ctrl+K) pour visualiser et forcer.").weak());
        } else if !app.up_to_date() {
            ui.label(RichText::new("⚠ Projet modifié depuis la compilation : chemins non résolus.").color(ORANGE));
        }
    });
    ui.separator();

    let mut actions: Vec<(usize, bool)> = Vec::new();
    let mut remove = None;
    let mut changed = false;
    egui::Grid::new(("watch", i)).striped(true).num_columns(6).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
        for h in ["Nom", "Type", "Valeur de visualisation", "Valeur de modification", "Forcer", ""] {
            ui.label(RichText::new(h).strong());
        }
        ui.end_row();
        let compiled = app.compiled.as_ref();
        for (k, r) in app.project.watch_tables[i].rows.iter_mut().enumerate() {
            let resolved = compiled.and_then(|c| c.resolve(&r.path));
            changed |= ui
                .add(egui::TextEdit::singleline(&mut r.path).desired_width(220.0).font(egui::TextStyle::Monospace))
                .changed();
            match &resolved {
                Some((_, ty)) => ui.label(ty.name()),
                None if compiled.is_some() => ui.label(RichText::new("introuvable").color(RED)),
                None => ui.label(""),
            };
            match &resolved {
                Some((slot, ty)) if online => {
                    ui.horizontal(|ui| {
                        if r.force {
                            ui.label(RichText::new("F").strong().color(Color32::WHITE).background_color(RED));
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
                egui::TextEdit::singleline(&mut r.modify).desired_width(120.0).font(egui::TextStyle::Monospace);
            if !valid {
                edit = edit.text_color(RED);
            }
            changed |= ui.add(edit).changed();
            let mut f = r.force;
            if ui.add_enabled(online && resolved.is_some(), egui::Checkbox::without_text(&mut f)).changed() {
                actions.push((k, f));
            }
            if ui.small_button("🗑").clicked() {
                remove = Some(k);
            }
            ui.end_row();
        }
    });
    for (k, f) in actions {
        set_force(app, i, k, f);
    }
    if let Some(k) = remove {
        app.project.watch_tables[i].rows.remove(k);
        changed = true;
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut app.new_watch_row)
                .hint_text("Ajouter : \"Variable\", Main.tempo.ET, Main.tab[2]…")
                .desired_width(320.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("➕").clicked() || enter) && !app.new_watch_row.trim().is_empty() {
            let path = std::mem::take(&mut app.new_watch_row);
            app.project.watch_tables[i].rows.push(WatchRow { path: path.trim().into(), ..Default::default() });
            changed = true;
        }
        if let Some(c) = &app.compiled {
            ui.menu_button("Choisir…", |ui| {
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for s in c.symbols.iter().filter(|s| s.ty.is_scalar()) {
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
// Inspecteur

pub fn inspector(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let errors = app.diags.iter().filter(|d| d.severity == Severity::Error).count();
        let info = if errors > 0 { format!("ℹ Info ({errors} erreur(s))") } else { "ℹ Info".into() };
        ui.selectable_value(&mut app.bottom, BottomTab::Info, info);
        ui.selectable_value(&mut app.bottom, BottomTab::Diagnostic, "🩺 Diagnostic");
        ui.selectable_value(&mut app.bottom, BottomTab::Simulation, "🧪 Simulation");
        ui.selectable_value(&mut app.bottom, BottomTab::Journal, "📜 Journal");
    });
    ui.separator();
    match app.bottom {
        BottomTab::Info => info_tab(app, ui),
        BottomTab::Diagnostic => diagnostic_tab(app, ui),
        BottomTab::Simulation => simulation_tab(app, ui),
        BottomTab::Journal => {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
                for l in &app.journal {
                    ui.label(RichText::new(l).monospace().small());
                }
            });
        }
    }
}

fn info_tab(app: &mut App, ui: &mut Ui) {
    if app.diags.is_empty() {
        ui.label(RichText::new("Aucun message. Compilez avec Ctrl+B.").weak());
        return;
    }
    let mut jump = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        egui::Grid::new("diags").striped(true).num_columns(4).spacing(vec2(12.0, 3.0)).show(ui, |ui| {
            for d in &app.diags {
                let (icon, color) = match d.severity {
                    Severity::Error => ("⛔", RED),
                    Severity::Warning => ("⚠", ORANGE),
                    Severity::Info => ("✔", GREEN),
                };
                ui.label(RichText::new(icon).color(color));
                ui.label(&d.block);
                ui.label(if d.pos.line > 0 { format!("ligne {}", d.pos.line) } else { String::new() });
                let r = ui.add(egui::Label::new(&d.msg).sense(Sense::click()));
                if r.clicked() {
                    jump = Some(d.clone());
                }
                if d.pos.line > 0 {
                    r.on_hover_text("Cliquer pour aller à la ligne");
                }
                ui.end_row();
            }
        });
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

fn diagnostic_tab(app: &mut App, ui: &mut Ui) {
    let Some(st) = app.status else {
        ui.label(RichText::new("Hors ligne. Passez en ligne (Ctrl+K) pour lire le diagnostic de la CPU.").weak());
        return;
    };
    ui.horizontal_top(|ui| {
        egui::Grid::new("diag_grid").num_columns(2).spacing(vec2(14.0, 3.0)).show(ui, |ui| {
            let mode = match Mode::from_u8(st.mode) {
                Mode::Run => RichText::new("RUN").color(GREEN).strong(),
                Mode::Stop => RichText::new("STOP").color(ORANGE).strong(),
                Mode::Fault => RichText::new("DÉFAUT").color(RED).strong(),
                Mode::NoProgram => RichText::new("aucun programme").weak(),
            };
            ui.label("État");
            ui.label(mode);
            ui.end_row();
            if let Some(f) = takt_vm::exec::Fault::from_u8(st.fault) {
                ui.label("Cause du défaut");
                ui.label(RichText::new(f.describe()).color(RED));
                ui.end_row();
            }
            ui.label("Cible");
            ui.label(app.target.as_ref().map(|t| t.name()).unwrap_or_default());
            ui.end_row();
            ui.label("Temps de cycle configuré");
            ui.label(format!("{} µs", st.cycle_us));
            ui.end_row();
            ui.label("Pire cas prouvé (compilateur)");
            ui.label(RichText::new(format!("{} µs", st.wcet_us)).strong());
            ui.end_row();
            ui.label("Exécution mesurée min / dernier / max");
            ui.label(format!("{} / {} / {} µs", st.exec_min_us, st.exec_last_us, st.exec_max_us));
            ui.end_row();
            ui.label("Retard max du début de cycle (gigue)");
            ui.label(format!("{} µs", st.late_max_us));
            ui.end_row();
            ui.label("Dépassements du temps de cycle");
            ui.label(RichText::new(st.overruns.to_string()).color(if st.overruns > 0 { RED } else { GREEN }));
            ui.end_row();
            ui.label("Cycles exécutés");
            ui.label(st.scans.to_string());
            ui.end_row();
            let mut flags = Vec::new();
            if st.diag & takt_vm::exec::DIAG_DIV0 != 0 {
                flags.push("division par zéro");
            }
            if st.diag & takt_vm::exec::DIAG_INDEX != 0 {
                flags.push("index de tableau hors bornes");
            }
            ui.label("Avertissements d'exécution");
            ui.label(if flags.is_empty() { "aucun".to_string() } else { flags.join(", ") });
            ui.end_row();
        });
        ui.add_space(24.0);
        ui.vertical(|ui| {
            ui.label(RichText::new("Répartition du temps d'exécution (en % du cycle)").strong());
            histogram(ui, &st.hist);
            if app.target.as_mut().is_some_and(|t| t.as_sim().is_some()) {
                ui.label(
                    RichText::new("Simulation : temps mesurés sur le PC (Windows), pas représentatifs du RP2350.")
                        .small()
                        .weak(),
                );
            }
            if ui.button("Remettre à zéro").clicked() {
                app.with_target(|t| target::call(t, proto::RESET_STATS, &[]));
            }
        });
    });
}

fn histogram(ui: &mut Ui, hist: &[u32; 8]) {
    let (rect, _) = ui.allocate_exact_size(vec2(360.0, 110.0), Sense::hover());
    let p = ui.painter_at(rect);
    let max = hist.iter().copied().max().unwrap_or(0).max(1) as f32;
    let w = rect.width() / 8.0;
    for (i, &n) in hist.iter().enumerate() {
        let h = (n as f32 / max) * (rect.height() - 18.0);
        let x = rect.left() + i as f32 * w;
        let bar =
            egui::Rect::from_min_max(pos2(x + 3.0, rect.bottom() - 16.0 - h), pos2(x + w - 3.0, rect.bottom() - 16.0));
        let color = if i >= 6 { ORANGE } else { ACCENT };
        p.rect_filled(bar, 2.0, color);
        p.text(
            pos2(x + w / 2.0, rect.bottom() - 8.0),
            egui::Align2::CENTER_CENTER,
            format!("{}%", (i + 1) * 125 / 10),
            egui::FontId::proportional(9.5),
            ui.visuals().weak_text_color(),
        );
    }
}

/// Panneau PLCSIM : boutons pour les entrées, voyants pour les sorties.
fn simulation_tab(app: &mut App, ui: &mut Ui) {
    let Some(sim) = app.target.as_mut().and_then(|t| t.as_sim()) else {
        ui.label(RichText::new("Sélectionnez la cible « PLCSIM » et passez en ligne pour simuler l'automate.").weak());
        return;
    };
    let shared = sim.shared.clone();
    let io = sim.io();
    if io.is_empty() {
        ui.label(RichText::new("Aucune entrée/sortie : chargez un programme (Ctrl+L).").weak());
        return;
    }
    let names: Vec<(u16, String)> = app
        .compiled
        .as_ref()
        .map(|c| c.symbols.iter().filter(|s| s.io.is_some()).map(|s| (s.slot, s.path.clone())).collect())
        .unwrap_or_default();
    let name_of = |slot: u16, pin: u8| {
        names.iter().find(|(s, _)| *s == slot).map(|(_, n)| n.clone()).unwrap_or(format!("GP{pin}"))
    };
    let mut g = shared.lock().unwrap();
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Entrées").strong());
                for e in io.iter().filter(|e| e.kind.is_input()) {
                    let v = g.inputs.entry(e.pin).or_insert(0);
                    ui.horizontal(|ui| {
                        if e.kind == IoKind::Di {
                            let on = *v != 0;
                            let btn = egui::Button::new(
                                RichText::new(if on { "1" } else { "0" }).monospace().color(Color32::WHITE),
                            )
                            .fill(if on { GREEN } else { Color32::from_gray(150) })
                            .min_size(vec2(26.0, 18.0));
                            if ui.add(btn).on_hover_text("Cliquer pour basculer").clicked() {
                                *v = (!on) as u32;
                            }
                        } else {
                            let mut x = *v as i32;
                            if ui.add(egui::Slider::new(&mut x, 0..=27648).show_value(true)).changed() {
                                *v = x as u32;
                            }
                        }
                        ui.label(name_of(e.slot, e.pin));
                    });
                }
            });
            ui.add_space(30.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Sorties").strong());
                for e in io.iter().filter(|e| !e.kind.is_input()) {
                    let v = g.outputs.get(&e.pin).copied().unwrap_or(0);
                    ui.horizontal(|ui| {
                        if e.kind == IoKind::Dq {
                            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                            ui.painter().circle_filled(
                                r.center(),
                                7.0,
                                if v != 0 { ORANGE } else { Color32::from_gray(170) },
                            );
                        } else {
                            ui.add(
                                egui::ProgressBar::new(v as i32 as f32 / 27648.0)
                                    .desired_width(120.0)
                                    .text(format!("{}", v as i32)),
                            );
                        }
                        ui.label(name_of(e.slot, e.pin));
                    });
                }
            });
        });
    });
    drop(g);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
}
