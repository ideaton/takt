//! Habillage de la fenêtre façon atelier d'automatisme (UperLogic, TIA) :
//! ruban à onglets, panneaux ancrés avec barre de titre, onglets de
//! documents, boîte à outils par catégories, barre d'état.

use eframe::egui::{self, Color32, FontId, Rect, Response, RichText, Sense, Stroke, Ui, pos2, vec2};
use takt_compiler::Severity;
use takt_compiler::ast::PouKind;
use takt_compiler::lang::{BlockType, Language};
use takt_vm::proto;
use takt_vm::runtime::Mode;

use super::{App, BottomTab, Conn, Modal, Pending, View, editor_id, short_board};
use crate::target::{self, SerialTarget};
use crate::theme::{self, icon};
use crate::{editor, instructions, ladder_ui, sfc_ui, update, views};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    Projet,
    Automate,
    Affichage,
    Aide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftTab {
    Projet,
    EnLigne,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolboxTab {
    Normal,
    Favoris,
}

/// Grand bouton du ruban : icône au-dessus, libellé en dessous.
fn big_button(ui: &mut Ui, ic: char, label: &str, active: bool, enabled: bool) -> Response {
    let c = theme::colors(ui);
    let font = FontId::new(12.0, egui::FontFamily::Proportional);
    let text_w = label
        .lines()
        .map(|l| ui.fonts(|f| f.layout_no_wrap(l.to_string(), font.clone(), c.text).size().x))
        .fold(0.0, f32::max);
    let w = (text_w + 16.0).max(56.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 66.0), sense);
    if active {
        ui.painter().rect_filled(rect, 8.0, c.accent_soft);
    } else if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, 8.0, c.hover);
    }
    let fg = if !enabled {
        c.very_soft
    } else if active {
        c.accent
    } else {
        c.text
    };
    let icon_fg = if !enabled { c.very_soft } else { c.accent };
    ui.painter().text(
        pos2(rect.center().x, rect.top() + 18.0),
        egui::Align2::CENTER_CENTER,
        ic,
        FontId::new(22.0, theme::icons()),
        icon_fg,
    );
    for (k, line) in label.lines().enumerate() {
        ui.painter().text(
            pos2(rect.center().x, rect.top() + 42.0 + k as f32 * 14.0),
            egui::Align2::CENTER_CENTER,
            line,
            font.clone(),
            fg,
        );
    }
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// Groupe du ruban : boutons, puis légende centrée en dessous, puis séparateur.
fn group(ui: &mut Ui, caption: &str, add: impl FnOnce(&mut Ui)) {
    let c = theme::colors(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
        let r = ui.horizontal(add).response.rect;
        let (cap, _) = ui.allocate_exact_size(vec2(r.width(), 16.0), Sense::hover());
        ui.painter().text(cap.center(), egui::Align2::CENTER_CENTER, caption, FontId::proportional(11.0), c.soft);
    });
    let (sep, _) = ui.allocate_exact_size(vec2(13.0, 82.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(sep.center().x, sep.top() + 6.0), pos2(sep.center().x, sep.bottom() - 4.0)],
        Stroke::new(1.0_f32, c.very_soft.gamma_multiply(0.6)),
    );
}

/// Barre de titre d'un panneau ancré. Renvoie vrai si on a cliqué sur la croix.
pub fn dock_header(ui: &mut Ui, ic: char, title: &str) -> bool {
    let c = theme::colors(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().text(
        rect.left_center() + vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        ic,
        FontId::new(14.0, theme::icons()),
        c.accent,
    );
    ui.painter().text(
        rect.left_center() + vec2(30.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::new(13.0, theme::semi()),
        c.text,
    );
    let close = Rect::from_center_size(rect.right_center() - vec2(16.0, 0.0), vec2(24.0, 24.0));
    let resp = ui.interact(close, ui.id().with(("close", title)), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(close, 6.0, c.hover);
    }
    ui.painter().text(close.center(), egui::Align2::CENTER_CENTER, icon::X, FontId::new(13.0, theme::icons()), c.soft);
    ui.painter()
        .line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, c.very_soft.gamma_multiply(0.5)));
    resp.on_hover_text("Masquer (menu Affichage pour le rouvrir)").clicked()
}

/// Onglet texte avec soulignement bleu quand il est actif.
fn text_tab(ui: &mut Ui, label: &str, active: bool, size: f32) -> Response {
    let c = theme::colors(ui);
    let font = FontId::new(size, if active { theme::semi() } else { theme::medium() });
    let w = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), c.text).size().x) + 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    if resp.hovered() && !active {
        ui.painter().rect_filled(rect.shrink2(vec2(2.0, 3.0)), 6.0, c.hover);
    }
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, if active { c.accent } else { c.text });
    if active {
        let y = rect.bottom() - 1.5;
        ui.painter()
            .line_segment([pos2(rect.left() + 8.0, y), pos2(rect.right() - 8.0, y)], Stroke::new(2.5_f32, c.accent));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn small_icon(ui: &mut Ui, ic: char, tip: &str) -> bool {
    theme::ghost_icon(ui, ic, tip).clicked()
}

impl App {
    // ---------- ruban ----------

    pub(super) fn ribbon(&mut self, ui: &mut Ui) {
        let c = theme::colors(ui);
        // Barre d'accès rapide + onglets + état
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(6.0);
            let logo = self.logo.get_or_insert_with(|| theme::logo_texture(ui.ctx())).clone();
            ui.add(egui::Image::new(&logo).fit_to_exact_size(vec2(22.0, 22.0)).corner_radius(5.0));
            ui.add_space(4.0);
            if small_icon(ui, icon::FILE_PLUS, "Nouveau projet") {
                self.request(Pending::New);
            }
            if small_icon(ui, icon::FOLDER, "Ouvrir…") {
                self.request(Pending::Open);
            }
            if small_icon(ui, icon::SAVE, "Enregistrer (Ctrl+S)") {
                self.save(false);
            }
            if small_icon(ui, icon::HAMMER, "Compiler (Ctrl+B)") {
                self.compile();
            }
            if small_icon(ui, icon::DOWNLOAD, "Charger dans l'appareil (Ctrl+L)") {
                self.download();
            }
            ui.add_space(14.0);
            for (tab, label) in [
                (RibbonTab::Projet, "Projet"),
                (RibbonTab::Automate, "Automate"),
                (RibbonTab::Affichage, "Affichage"),
                (RibbonTab::Aide, "Aide"),
            ] {
                if text_tab(ui, label, self.ribbon_tab == tab, 13.5).clicked() {
                    self.ribbon_tab = tab;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                let (text, color) = self.mode_label();
                egui::Frame::new()
                    .fill(color.gamma_multiply(0.14))
                    .corner_radius(7.0)
                    .inner_margin(egui::Margin::symmetric(12, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).font(FontId::new(13.5, theme::semi())).color(color));
                    });
                ui.add_space(6.0);
                let dirty = if self.dirty() { " •" } else { "" };
                ui.label(RichText::new(format!("{}{dirty}", self.project.name)).color(c.soft));
            });
        });
        // Corps du ruban
        egui::Frame::new().fill(c.bg).corner_radius(10.0).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::horizontal().id_salt("ribbon").show(ui, |ui| {
                ui.horizontal(|ui| match self.ribbon_tab {
                    RibbonTab::Projet => self.ribbon_project(ui),
                    RibbonTab::Automate => self.ribbon_plc(ui),
                    RibbonTab::Affichage => self.ribbon_view(ui),
                    RibbonTab::Aide => self.ribbon_help(ui),
                });
            });
        });
        ui.add_space(4.0);
    }

    /// Mode de l'atelier, comme « Offline Edit » dans UperLogic.
    fn mode_label(&self) -> (String, Color32) {
        let c = if self.dark { theme::DARK } else { theme::LIGHT };
        match (&self.status, self.online()) {
            (Some(st), true) => match Mode::from_u8(st.mode) {
                Mode::Run if self.monitor => ("En ligne · Visualisation".into(), c.green),
                Mode::Run => ("En ligne · RUN".into(), c.green),
                Mode::Stop => ("En ligne · STOP".into(), c.orange),
                Mode::Fault => ("En ligne · DÉFAUT".into(), c.red),
                Mode::NoProgram => ("En ligne · vide".into(), c.soft),
            },
            (_, true) => ("Connexion…".into(), c.soft),
            _ => ("Édition hors ligne".into(), c.accent),
        }
    }

    fn new_block(&mut self, kind: BlockType) {
        let lang = if kind == BlockType::Fc { Language::St } else { Language::Ladder };
        self.modal = Some(Modal::NewBlock { name: String::new(), kind, lang });
    }

    fn ribbon_project(&mut self, ui: &mut Ui) {
        group(ui, "Fichier", |ui| {
            if big_button(ui, icon::FILE_PLUS, "Nouveau", false, true).clicked() {
                self.request(Pending::New);
            }
            if big_button(ui, icon::FOLDER, "Ouvrir", false, true).clicked() {
                self.request(Pending::Open);
            }
            if big_button(ui, icon::SAVE, "Enregistrer", false, true).clicked() {
                self.save(false);
            }
            if big_button(ui, icon::SAVE, "Enregistrer\nsous…", false, true).clicked() {
                self.save(true);
            }
        });
        group(ui, "Configuration système", |ui| {
            if big_button(ui, icon::CPU, "Appareil", self.view == Some(View::Device), true).clicked() {
                self.open(View::Device);
            }
            if big_button(ui, icon::TAG, "Variables\nAPI", self.view == Some(View::Tags), true).clicked() {
                self.open(View::Tags);
            }
        });
        group(ui, "Blocs de programme", |ui| {
            if big_button(ui, icon::CODE, "Bloc\nd'organisation", false, true).clicked() {
                self.new_block(BlockType::Ob);
            }
            if big_button(ui, icon::BOX, "Bloc\nfonctionnel", false, true).clicked() {
                self.new_block(BlockType::Fb);
            }
            if big_button(ui, icon::FUNCTION, "Fonction", false, true).clicked() {
                self.new_block(BlockType::Fc);
            }
        });
        group(ui, "Tables", |ui| {
            if big_button(ui, icon::EYE, "Table de\nvisualisation", false, true).clicked() {
                self.add_watch_table();
            }
        });
        group(ui, "Exemples", |ui| {
            if big_button(ui, icon::BOOK, "Convoyeur", false, true).clicked() {
                self.request(Pending::Example);
            }
            let recent = self.config.existing_recent();
            let r = big_button(ui, icon::FOLDER_CLOSED, "Récents", false, !recent.is_empty());
            egui::popup::popup_below_widget(
                ui,
                egui::Id::new("recents"),
                &r,
                egui::PopupCloseBehavior::CloseOnClick,
                |ui| {
                    ui.set_min_width(260.0);
                    for p in recent {
                        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        if ui.button(name).on_hover_text(p.display().to_string()).clicked() {
                            self.request(Pending::Recent(p));
                        }
                    }
                },
            );
            if r.clicked() {
                ui.memory_mut(|m| m.toggle_popup(egui::Id::new("recents")));
            }
        });
    }

    fn ribbon_plc(&mut self, ui: &mut Ui) {
        let online = self.online();
        group(ui, "Programme", |ui| {
            if big_button(ui, icon::HAMMER, "Compiler", false, true).on_hover_text("Ctrl+B").clicked() {
                self.compile();
            }
            if big_button(ui, icon::DOWNLOAD, "Charger dans\nl'appareil", false, true).on_hover_text("Ctrl+L").clicked()
            {
                self.download();
            }
        });
        group(ui, "Liaison", |ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);
                let label = match &self.conn {
                    Conn::Sim => "Simulateur PLCSIM".to_string(),
                    Conn::Serial(p) => format!("RP2350 · {p}"),
                };
                ui.label(RichText::new("Cible").size(11.5).color(theme::colors(ui).soft));
                ui.add_enabled_ui(!online, |ui| {
                    egui::ComboBox::from_id_salt("conn").selected_text(label).width(170.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.conn, Conn::Sim, "Simulateur PLCSIM");
                        for p in self.ports.clone() {
                            ui.selectable_value(&mut self.conn, Conn::Serial(p.clone()), format!("RP2350 · {p}"));
                        }
                        if ui.button("Rechercher les ports").clicked() {
                            self.ports = SerialTarget::ports();
                        }
                    });
                });
            });
            if online {
                if big_button(ui, icon::UNPLUG, "Hors ligne", false, true).on_hover_text("Ctrl+M").clicked() {
                    self.disconnect();
                }
            } else if big_button(ui, icon::PLUG, "En ligne", false, true).on_hover_text("Ctrl+K").clicked() {
                self.connect();
            }
        });
        let mode = self.status.map(|s| Mode::from_u8(s.mode));
        group(ui, "CPU", |ui| {
            if big_button(ui, icon::PLAY, "RUN", mode == Some(Mode::Run), online).clicked() {
                self.run();
            }
            if big_button(ui, icon::SQUARE, "STOP", mode == Some(Mode::Stop), online).clicked() {
                self.stop();
            }
        });
        group(ui, "Mise au point", |ui| {
            if big_button(ui, icon::EYE, "Visualiser", self.monitor, true).on_hover_text("Ctrl+T").clicked() {
                self.toggle_monitor();
            }
            if big_button(ui, icon::LOCK_OPEN, "Tout\ndéforcer", false, online).clicked()
                && self.with_target(|t| target::call(t, proto::UNFORCE_ALL, &[])).is_some()
            {
                for t in &mut self.project.watch_tables {
                    for r in &mut t.rows {
                        r.force = false;
                    }
                }
                self.log("Tous les forçages sont supprimés.");
            }
        });
        group(ui, "Outils en ligne", |ui| {
            if big_button(ui, icon::ACTIVITY, "Diagnostic", self.drawer && self.bottom == BottomTab::Diagnostic, true)
                .clicked()
            {
                self.drawer = true;
                self.bottom = BottomTab::Diagnostic;
            }
            if big_button(ui, icon::FLASK, "Simulation", self.drawer && self.bottom == BottomTab::Simulation, true)
                .clicked()
            {
                self.drawer = true;
                self.bottom = BottomTab::Simulation;
            }
            if big_button(ui, icon::REFRESH, "RAZ des\nstatistiques", false, online).clicked() {
                self.with_target(|t| target::call(t, proto::RESET_STATS, &[]));
            }
        });
    }

    fn ribbon_view(&mut self, ui: &mut Ui) {
        group(ui, "Panneaux", |ui| {
            if big_button(ui, icon::FOLDER_CLOSED, "Gestion\nde projet", self.show_tree, true).clicked() {
                self.show_tree = !self.show_tree;
            }
            if big_button(ui, icon::BLOCKS, "Boîte\nà outils", self.show_instructions, true).clicked() {
                self.show_instructions = !self.show_instructions;
            }
            if big_button(ui, icon::SCROLL, "Fenêtre\nd'état", self.drawer, true).clicked() {
                self.drawer = !self.drawer;
            }
        });
        group(ui, "Éditeur", |ui| {
            if big_button(ui, icon::CODE, "ST généré", self.show_st, true)
                .on_hover_text("Afficher le Structured Text compilé sous les blocs CONT, GRAPH et BASIC")
                .clicked()
            {
                self.show_st = !self.show_st;
            }
        });
        group(ui, "Thème", |ui| {
            if big_button(ui, icon::SUN, "Clair", !self.dark, true).clicked() && self.dark {
                self.set_dark(ui.ctx(), false);
            }
            if big_button(ui, icon::MOON, "Sombre", self.dark, true).clicked() && !self.dark {
                self.set_dark(ui.ctx(), true);
            }
        });
    }

    fn ribbon_help(&mut self, ui: &mut Ui) {
        group(ui, "Takt Studio", |ui| {
            if big_button(ui, icon::REFRESH, "Mise à jour", false, true).clicked() {
                self.update = update::start_check();
                self.log("Recherche d'une mise à jour…");
            }
            if big_button(ui, icon::INFO, "À propos", false, true).clicked() {
                self.modal = Some(Modal::About);
            }
            if big_button(ui, icon::BOOK, "Documentation", false, true)
                .on_hover_text("github.com/ideaton/takt")
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab("https://github.com/ideaton/takt#readme"));
            }
        });
    }

    pub(super) fn set_dark(&mut self, ctx: &egui::Context, dark: bool) {
        self.dark = dark;
        theme::apply(ctx, dark);
        self.config.dark = dark;
        self.config.save();
    }

    // ---------- panneau de gauche ----------

    pub(super) fn left_dock(&mut self, ui: &mut Ui) {
        if dock_header(ui, icon::FOLDER_CLOSED, "Gestion de projet") {
            self.show_tree = false;
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            // Onglets du bas, comme « Project Man… / Memory… »
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (tab, label) in [(LeftTab::Projet, "Projet"), (LeftTab::EnLigne, "En ligne")] {
                    if text_tab(ui, label, self.left_tab == tab, 12.5).clicked() {
                        self.left_tab = tab;
                    }
                }
            });
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| match self.left_tab {
                LeftTab::Projet => {
                    ui.add_space(4.0);
                    self.sidebar(ui);
                }
                LeftTab::EnLigne => {
                    ui.add_space(8.0);
                    self.update_banner(ui);
                    self.connection_box(ui);
                    ui.add_space(8.0);
                    if let Some(st) = self.status {
                        let c = theme::colors(ui);
                        egui::Grid::new("online_summary").num_columns(2).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                            ui.label(RichText::new("Pire cas prouvé").color(c.soft));
                            ui.label(format!("{} µs", st.wcet_us));
                            ui.end_row();
                            ui.label(RichText::new("Mesuré max").color(c.soft));
                            ui.label(format!("{} µs", st.exec_max_us));
                            ui.end_row();
                            ui.label(RichText::new("Cycles").color(c.soft));
                            ui.label(st.scans.to_string());
                            ui.end_row();
                        });
                    }
                }
            });
        });
    }

    // ---------- onglets de documents ----------

    fn view_icon(&mut self, v: View) -> char {
        match v {
            View::Device => icon::CPU,
            View::Tags => icon::TAG,
            View::Watch(_) => icon::EYE,
            View::Block(i) => match self.block_kind(i) {
                Some(PouKind::Program) => icon::CODE,
                Some(PouKind::FunctionBlock) => icon::BOX,
                _ => icon::FUNCTION,
            },
        }
    }

    fn view_title(&mut self, v: View) -> String {
        match v {
            View::Device => {
                format!("{} [{}]", self.project.device.name, short_board(views::board_name(&self.project.device.board)))
            }
            View::Tags => "Variables API".into(),
            View::Watch(i) => self.project.watch_tables.get(i).map(|t| t.name.clone()).unwrap_or_default(),
            View::Block(i) => {
                let n = self.block_number(i);
                format!("{} [{n}]", self.project.blocks.get(i).map(|b| b.name.as_str()).unwrap_or(""))
            }
        }
    }

    pub(super) fn doc_tabs(&mut self, ui: &mut Ui) {
        let c = theme::colors(ui);
        let mut close = None;
        egui::ScrollArea::horizontal().id_salt("doc_tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for v in self.tabs.clone() {
                    let active = self.view == Some(v);
                    let ic = self.view_icon(v);
                    let title = self.view_title(v);
                    let lang = match v {
                        View::Block(i) => self.project.blocks.get(i).map(|b| b.language.label()),
                        _ => None,
                    };
                    let font = FontId::new(13.0, if active { theme::semi() } else { theme::medium() });
                    let tw = ui.fonts(|f| f.layout_no_wrap(title.clone(), font.clone(), c.text).size().x);
                    let lw = lang
                        .map(|l| {
                            ui.fonts(|f| f.layout_no_wrap(l.to_string(), FontId::proportional(10.5), c.text).size().x)
                                + 12.0
                        })
                        .unwrap_or(0.0);
                    let (rect, resp) = ui.allocate_exact_size(vec2(tw + lw + 66.0, 32.0), Sense::click());
                    let fill = if active {
                        c.bg
                    } else if resp.hovered() {
                        c.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, fill);
                    if active {
                        ui.painter().line_segment(
                            [rect.left_top() + vec2(6.0, 0.5), rect.right_top() + vec2(-6.0, 0.5)],
                            Stroke::new(2.5_f32, c.accent),
                        );
                    }
                    ui.painter().text(
                        rect.left_center() + vec2(12.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        ic,
                        FontId::new(13.0, theme::icons()),
                        if active { c.accent } else { c.soft },
                    );
                    ui.painter().text(
                        rect.left_center() + vec2(32.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &title,
                        font,
                        c.text,
                    );
                    if let Some(l) = lang {
                        let lr = Rect::from_min_size(
                            pos2(rect.left() + 38.0 + tw, rect.center().y - 8.0),
                            vec2(lw - 6.0, 16.0),
                        );
                        ui.painter().rect_filled(lr, 4.0, c.accent.gamma_multiply(0.12));
                        ui.painter().text(
                            lr.center(),
                            egui::Align2::CENTER_CENTER,
                            l,
                            FontId::new(10.5, theme::semi()),
                            c.accent,
                        );
                    }
                    let x = Rect::from_center_size(rect.right_center() - vec2(16.0, 0.0), vec2(20.0, 20.0));
                    let xr = ui.interact(x, ui.id().with(("close_tab", format!("{v:?}"))), Sense::click());
                    if xr.hovered() {
                        ui.painter().rect_filled(x, 5.0, c.hover);
                    }
                    if active || resp.hovered() || xr.hovered() {
                        ui.painter().text(
                            x.center(),
                            egui::Align2::CENTER_CENTER,
                            icon::X,
                            FontId::new(11.0, theme::icons()),
                            c.soft,
                        );
                    }
                    if xr.clicked() {
                        close = Some(v);
                    } else if resp.clicked() {
                        self.view = Some(v);
                    } else if resp.middle_clicked() {
                        close = Some(v);
                    }
                }
            });
        });
        if let Some(v) = close {
            self.close_tab(v);
        }
    }

    /// Fil d'Ariane sous les onglets, et à droite la position ou le langage.
    pub(super) fn breadcrumb(&mut self, ui: &mut Ui) {
        let c = theme::colors(ui);
        let Some(v) = self.view else { return };
        let mut parts = vec![self.project.name.clone(), self.project.device.name.clone()];
        match v {
            View::Device => parts.push("Configuration de l'appareil".into()),
            View::Tags => parts.extend(["Variables API".to_string(), "Table de variables standard".to_string()]),
            View::Watch(_) => parts.extend(["Tables de visualisation".to_string(), self.view_title(v)]),
            View::Block(_) => parts.extend(["Blocs de programme".to_string(), self.view_title(v)]),
        }
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            for (k, p) in parts.iter().enumerate() {
                if k > 0 {
                    ui.label(
                        RichText::new(icon::CHEVRON_RIGHT.to_string())
                            .font(FontId::new(11.0, theme::icons()))
                            .color(c.very_soft),
                    );
                }
                let last = k + 1 == parts.len();
                ui.label(RichText::new(p).size(12.0).color(if last { c.text } else { c.soft }));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let View::Block(i) = v {
                    let b = &self.project.blocks[i];
                    let detail = match b.block_type() {
                        Some(BlockType::Ob) => "exécuté à chaque cycle",
                        Some(BlockType::Fb) => "avec mémoire d'instance",
                        _ => "sans mémoire",
                    };
                    ui.label(
                        RichText::new(format!("{} · {detail}", b.language.description())).size(12.0).color(c.soft),
                    );
                }
            });
        });
    }

    // ---------- boîte à outils ----------

    pub(super) fn toolbox(&mut self, ui: &mut Ui) {
        let c = theme::colors(ui);
        if dock_header(ui, icon::BLOCKS, "Boîte à outils") {
            self.show_instructions = false;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (tab, label) in [(ToolboxTab::Normal, "Normal"), (ToolboxTab::Favoris, "Favoris")] {
                if text_tab(ui, label, self.toolbox_tab == tab, 12.5).clicked() {
                    self.toolbox_tab = tab;
                }
            }
        });
        ui.add_space(4.0);
        let editing = match self.view {
            Some(View::Block(i)) if !self.monitor => Some(i),
            _ => None,
        };
        let Some(i) = editing else {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Ouvrez un bloc de programme (hors visualisation) pour insérer des instructions.")
                    .size(12.0)
                    .color(c.soft),
            );
            return;
        };
        let lang = self.project.blocks[i].language;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            match lang {
                Language::Ladder => {
                    let groups: Vec<(&str, Vec<ladder_ui::Tool>)> = if self.toolbox_tab == ToolboxTab::Favoris {
                        vec![("Favoris", ladder_ui::FAVORITES.to_vec())]
                    } else {
                        ladder_ui::CATALOG.iter().map(|(g, t)| (*g, t.to_vec())).collect()
                    };
                    for (g, tools) in groups {
                        if self.category(ui, g) {
                            for t in tools {
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    if ladder_ui::tool_button(ui, t, true) {
                                        ladder_ui::apply(&mut self.project.blocks[i], &mut self.ld_sel, t);
                                        self.touch();
                                    }
                                });
                            }
                        }
                    }
                }
                Language::Sfc => {
                    if self.category(ui, "Étapes et transitions") {
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            if theme::secondary_button(ui, icon::PLUS, "Ajouter une étape").clicked() {
                                sfc_ui::add_step(&mut self.project.blocks[i], &mut self.sfc_sel);
                                self.touch();
                            }
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Sélectionnez une étape pour ajouter des transitions et des actions dans le panneau de propriétés. Conditions : T_Etape >= T#2s, \"Capteur\", #Entree…")
                                .size(11.5)
                                .color(c.soft),
                        );
                    }
                }
                Language::Basic => {
                    const SNIPPETS: &[(&str, &str, &str)] = &[
                        ("Contrôle", "IF … THEN … END IF", "IF  THEN\n    \nEND IF\n"),
                        ("Contrôle", "IF … ELSE", "IF  THEN\n    \nELSE\n    \nEND IF\n"),
                        ("Contrôle", "FOR … NEXT", "FOR i = 0 TO 9\n    \nNEXT i\n"),
                        ("Contrôle", "SELECT CASE", "SELECT CASE x\n    CASE 0 : \n    CASE 1, 2 : \n    CASE ELSE : \nEND SELECT\n"),
                        ("Bits", "SET / RESET", "SET \"Sortie\"\n"),
                        ("Bits", "TOGGLE", "TOGGLE \"Sortie\"\n"),
                        ("Temporisations", "CALL tempo", "CALL Tempo(IN := \"Entree\", PT := T#2s)\n"),
                        ("Déclarations", "DIM", "DIM compteur AS INT = 0\n"),
                        ("Déclarations", "Commentaire", "' \n"),
                    ];
                    let mut last = "";
                    let mut open = false;
                    for (g, label, snippet) in SNIPPETS {
                        if *g != last {
                            last = g;
                            open = self.category(ui, g);
                        }
                        if open && self.snippet_row(ui, label, "").clicked() {
                            editor::insert_snippet(ui.ctx(), editor_id(i), &mut self.project.blocks[i].source, snippet);
                            self.touch();
                        }
                    }
                }
                Language::St => {
                    for g in instructions::CATALOG {
                        if !self.category(ui, g.name) {
                            continue;
                        }
                        for it in g.items {
                            let (name, rest) = it.label.split_once("  ").unwrap_or((it.label, ""));
                            if self.snippet_row(ui, name, rest).on_hover_text(it.help).clicked() {
                                let src = &mut self.project.blocks[i].source;
                                let snippet = match it.instance {
                                    Some((ty, prefix)) => {
                                        let n = instructions::declare_instance(src, ty, prefix);
                                        it.snippet.replace("{n}", &n)
                                    }
                                    None => it.snippet.to_string(),
                                };
                                editor::insert_snippet(ui.ctx(), editor_id(i), src, &snippet);
                                self.touch();
                            }
                        }
                    }
                }
            }
            ui.add_space(12.0);
        });
    }

    /// Ligne de catégorie dépliable de la boîte à outils. Renvoie vrai si ouverte.
    fn category(&mut self, ui: &mut Ui, name: &str) -> bool {
        let c = theme::colors(ui);
        let key = format!("{:?}/{name}", self.toolbox_tab);
        let open = !self.toolbox_closed.contains(&key);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 6.0, c.hover);
        }
        let ch = if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT };
        ui.painter().text(
            rect.left_center() + vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            ch,
            FontId::new(12.0, theme::icons()),
            c.soft,
        );
        ui.painter().text(
            rect.left_center() + vec2(26.0, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            FontId::new(13.0, theme::medium()),
            c.text,
        );
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0_f32, c.very_soft.gamma_multiply(0.35)),
        );
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            if open {
                self.toolbox_closed.insert(key);
            } else {
                self.toolbox_closed.remove(&key);
            }
        }
        open
    }

    fn snippet_row(&self, ui: &mut Ui, label: &str, detail: &str) -> Response {
        let c = theme::colors(ui);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 6.0, c.hover);
        }
        ui.painter().text(
            rect.left_center() + vec2(26.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::new(13.0, theme::medium()),
            c.text,
        );
        if !detail.is_empty() {
            ui.painter().text(
                rect.right_center() - vec2(8.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                detail,
                FontId::proportional(11.5),
                c.soft,
            );
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    // ---------- fenêtre d'état et barre d'état ----------

    pub(super) fn bottom_dock(&mut self, ui: &mut Ui) {
        if dock_header(ui, icon::SCROLL, "Fenêtre d'état") {
            self.drawer = false;
        }
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 4 }).show(ui, |ui| {
            views::inspector(self, ui);
        });
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        let c = theme::colors(ui);
        let sep = |ui: &mut Ui| {
            let (r, _) = ui.allocate_exact_size(vec2(15.0, 16.0), Sense::hover());
            ui.painter().line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0_f32, c.very_soft));
        };
        let item = |ui: &mut Ui, ic: char, text: &str, color: Color32| {
            ui.label(theme::rich(&format!("{ic}  {text}"), 12.0, egui::FontFamily::Proportional, color));
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(6.0);
            let (mode, color) = self.mode_label();
            item(ui, icon::CPU, &mode, color);
            sep(ui);
            if let Some((line, col)) = ui.ctx().data(|d| d.get_temp::<(usize, usize)>(egui::Id::new("takt_cursor")))
                && matches!(self.view, Some(View::Block(_)))
            {
                item(ui, icon::CODE, &format!("Ligne {line}, Col {col}"), c.soft);
                sep(ui);
            }
            let errors = self.diags.iter().filter(|d| d.severity == Severity::Error).count();
            match (&self.compiled, self.up_to_date()) {
                (Some(cp), true) => {
                    let col = if cp.wcet.ratio() > takt_compiler::WCET_WARN_RATIO { c.orange } else { c.green };
                    item(
                        ui,
                        icon::TIMER,
                        &format!("Cycle garanti {} / {} µs", cp.wcet.total_us, cp.wcet.cycle_us),
                        col,
                    );
                }
                _ if errors > 0 && self.compiled_rev == Some(self.rev) => {
                    if ui
                        .add(
                            egui::Label::new(theme::rich(
                                &format!("{}  {errors} erreur(s)", icon::ALERT),
                                12.0,
                                egui::FontFamily::Proportional,
                                c.red,
                            ))
                            .sense(Sense::click()),
                        )
                        .clicked()
                    {
                        self.drawer = true;
                        self.bottom = BottomTab::Info;
                    }
                }
                _ => item(ui, icon::HAMMER, "Non compilé", c.soft),
            }
            if let Some(st) = &self.status {
                sep(ui);
                match self.program_identical() {
                    Some(true) => item(ui, icon::CHECK, "Programme identique", c.green),
                    _ => item(ui, icon::WARNING, "Programme différent", c.orange),
                }
                if st.n_forces > 0 {
                    sep(ui);
                    item(ui, icon::ZAP, &format!("{} forçage(s)", st.n_forces), c.red);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("v{}", update::VERSION)).size(11.5).color(c.soft));
                sep(ui);
                let target = match (&self.conn, self.online()) {
                    (Conn::Sim, _) => "PLCSIM".to_string(),
                    (Conn::Serial(p), _) => format!("USB {p}"),
                };
                item(ui, if self.online() { icon::PLUG } else { icon::UNPLUG }, &target, c.soft);
                sep(ui);
                let last = self
                    .journal
                    .last()
                    .map(|l| l.split_once("] ").map(|(_, m)| m.to_string()).unwrap_or_default())
                    .unwrap_or_default();
                ui.add(egui::Label::new(RichText::new(last).size(11.5).color(c.soft)).truncate());
            });
        });
    }
}
