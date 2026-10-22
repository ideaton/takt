//! Fenêtre principale : état du projet, actions (compiler, charger, en ligne)
//! et disposition épurée façon Sura Novi (barre latérale, titre de page, tiroir).

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::theme::{self, icon};
use eframe::egui::{self, Color32, Key, KeyboardShortcut, Modifiers, RichText};
use takt_compiler::ast::PouKind;
use takt_compiler::project::{Block, Project, WatchTable};
use takt_compiler::{Compiled, Diag, Severity, compile};
use takt_vm::proto::{self, Status};
use takt_vm::runtime::Mode;

use crate::editor::{self, LineMark};
use crate::instructions;
use crate::target::{self, SerialTarget, SimTarget, Target};
use crate::views;

pub const EXAMPLE: &str = include_str!("../../../examples/convoyeur.takt");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Device,
    Tags,
    Block(usize),
    Watch(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conn {
    Sim,
    Serial(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomTab {
    Info,
    Diagnostic,
    Simulation,
    Journal,
}

pub enum Modal {
    StopForDownload,
    StartAfterDownload,
    NewBlock { name: String, kind: PouKind },
    DeleteBlock(usize),
    Discard(Pending),
    About,
}

#[derive(Clone, Copy)]
pub enum Pending {
    New,
    Open,
    Example,
}

pub struct App {
    pub project: Project,
    pub path: Option<PathBuf>,
    /// Incrémenté à chaque modification du projet.
    pub rev: u64,
    saved_rev: u64,
    pub view: Option<View>,
    pub compiled: Option<Compiled>,
    pub compiled_rev: Option<u64>,
    pub diags: Vec<Diag>,
    pub target: Option<Box<dyn Target>>,
    pub conn: Conn,
    pub ports: Vec<String>,
    pub status: Option<Status>,
    pub values: HashMap<u16, u32>,
    last_poll: Instant,
    pub monitor: bool,
    pub bottom: BottomTab,
    pub journal: Vec<String>,
    started: Instant,
    pub modal: Option<Modal>,
    pub goto: Option<(usize, u32)>,
    kinds: (u64, Vec<Option<PouKind>>),
    pub new_watch_row: String,
    pub dark: bool,
    /// Tiroir du bas (messages, diagnostic, simulation, journal) ouvert.
    pub drawer: bool,
    pub show_instructions: bool,
    logo: Option<egui::TextureHandle>,
    /// Les polices installées par `theme::install` ne sont prises en compte
    /// qu'à l'image suivante : la première image n'affiche rien.
    first_frame: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>) -> App {
        App::create(&cc.egui_ctx, path)
    }

    pub fn create(ctx: &egui::Context, path: Option<PathBuf>) -> App {
        theme::install(ctx);
        let mut app = App {
            project: serde_json_example(),
            path: None,
            rev: 1,
            saved_rev: 1,
            view: Some(View::Block(0)),
            compiled: None,
            compiled_rev: None,
            diags: Vec::new(),
            target: None,
            conn: Conn::Sim,
            ports: SerialTarget::ports(),
            status: None,
            values: HashMap::new(),
            last_poll: Instant::now(),
            monitor: false,
            bottom: BottomTab::Info,
            journal: Vec::new(),
            started: Instant::now(),
            modal: None,
            goto: None,
            kinds: (0, Vec::new()),
            new_watch_row: String::new(),
            dark: false,
            drawer: true,
            show_instructions: true,
            logo: None,
            first_frame: true,
        };
        app.log("Takt Studio prêt. Projet d'exemple « Convoyeur » chargé.");
        if let Some(p) = path {
            app.load_from(p);
        }
        app
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        let t = self.started.elapsed().as_secs_f32();
        self.journal.push(format!("[{t:>7.1} s] {}", msg.into()));
    }

    pub fn touch(&mut self) {
        self.rev += 1;
    }

    pub fn dirty(&self) -> bool {
        self.rev != self.saved_rev
    }

    pub fn up_to_date(&self) -> bool {
        self.compiled_rev == Some(self.rev) && self.compiled.is_some()
    }

    pub fn online(&self) -> bool {
        self.target.is_some()
    }

    /// Le programme dans l'automate est-il celui compilé à partir du projet actuel ?
    pub fn program_identical(&self) -> Option<bool> {
        let st = self.status.as_ref()?;
        if st.mode == Mode::NoProgram as u8 {
            return Some(false);
        }
        Some(self.up_to_date() && self.compiled.as_ref().is_some_and(|c| c.crc == st.program_crc))
    }

    pub fn block_kind(&mut self, i: usize) -> Option<PouKind> {
        if self.kinds.0 != self.rev {
            self.kinds = (self.rev, self.project.blocks.iter().map(|b| takt_compiler::block_kind(&b.source)).collect());
        }
        self.kinds.1.get(i).copied().flatten()
    }

    // ---------- fichiers ----------

    fn set_project(&mut self, p: Project, path: Option<PathBuf>) {
        self.disconnect();
        self.project = p;
        self.path = path;
        self.rev += 1;
        self.saved_rev = self.rev;
        self.compiled = None;
        self.compiled_rev = None;
        self.diags.clear();
        self.view = None;
        if !self.project.blocks.is_empty() {
            self.open(View::Block(0));
        }
    }

    fn request(&mut self, action: Pending) {
        if self.dirty() {
            self.modal = Some(Modal::Discard(action));
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: Pending) {
        match action {
            Pending::New => {
                self.set_project(Project::new_empty("Nouveau projet"), None);
                self.log("Nouveau projet.");
            }
            Pending::Example => {
                self.set_project(serde_json_example(), None);
                self.log("Projet d'exemple « Convoyeur » chargé.");
            }
            Pending::Open => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Projet Takt", &["takt"]).pick_file() {
                    self.load_from(p);
                }
            }
        }
    }

    fn load_from(&mut self, p: PathBuf) {
        match Project::load(&p) {
            Ok(proj) => {
                self.log(format!("Projet ouvert : {}", p.display()));
                self.set_project(proj, Some(p));
            }
            Err(e) => self.log(format!("Erreur : {e}")),
        }
    }

    pub fn save(&mut self, as_new: bool) {
        let path = match (&self.path, as_new) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .add_filter("Projet Takt", &["takt"])
                .set_file_name(format!("{}.takt", self.project.name))
                .save_file(),
        };
        let Some(path) = path else { return };
        match self.project.save(&path) {
            Ok(()) => {
                self.saved_rev = self.rev;
                self.log(format!("Projet enregistré : {}", path.display()));
                self.path = Some(path);
            }
            Err(e) => self.log(format!("Erreur : {e}")),
        }
    }

    // ---------- vues ----------

    pub fn open(&mut self, v: View) {
        self.view = Some(v);
    }

    pub fn add_block(&mut self, name: &str, kind: PouKind) {
        let source = match kind {
            PouKind::Program => {
                format!("ORGANIZATION_BLOCK {name}\n// Bloc cyclique\nVAR\nEND_VAR\nBEGIN\n\nEND_ORGANIZATION_BLOCK\n")
            }
            PouKind::FunctionBlock => format!(
                "FUNCTION_BLOCK {name}\nVAR_INPUT\nEND_VAR\nVAR_OUTPUT\nEND_VAR\nVAR\nEND_VAR\nBEGIN\n\nEND_FUNCTION_BLOCK\n"
            ),
            PouKind::Function => {
                format!("FUNCTION {name} : INT\nVAR_INPUT\nEND_VAR\nBEGIN\n    {name} := 0;\nEND_FUNCTION\n")
            }
        };
        self.project.blocks.push(Block { name: name.into(), source });
        self.touch();
        self.open(View::Block(self.project.blocks.len() - 1));
    }

    fn delete_block(&mut self, i: usize) {
        self.project.blocks.remove(i);
        if let Some(View::Block(k)) = self.view {
            if k == i {
                self.view = (!self.project.blocks.is_empty()).then_some(View::Block(0));
            } else if k > i {
                self.view = Some(View::Block(k - 1));
            }
        }
        self.touch();
    }

    pub fn add_watch_table(&mut self) {
        let n = self.project.watch_tables.len() + 1;
        self.project.watch_tables.push(WatchTable { name: format!("Table de visualisation_{n}"), rows: vec![] });
        self.touch();
        self.open(View::Watch(self.project.watch_tables.len() - 1));
    }

    // ---------- compilation et automate ----------

    pub fn compile(&mut self) -> bool {
        let out = compile(&self.project);
        let errors = out.errors();
        let warnings = out.warnings();
        self.diags = out.diags;
        self.compiled = out.compiled;
        self.compiled_rev = Some(self.rev);
        self.bottom = BottomTab::Info;
        if self.diags.iter().any(|d| d.severity != Severity::Info) {
            self.drawer = true;
        }
        match &self.compiled {
            Some(c) => {
                let msg = format!(
                    "Compilation terminée : 0 erreur, {warnings} avertissement(s). Temps de cycle garanti {} µs / {} µs.",
                    c.wcet.total_us, c.wcet.cycle_us
                );
                self.log(msg);
                true
            }
            None => {
                self.log(format!("Compilation échouée : {errors} erreur(s), {warnings} avertissement(s)."));
                false
            }
        }
    }

    fn ensure_compiled(&mut self) -> bool {
        self.up_to_date() || self.compile()
    }

    pub fn connect(&mut self) {
        if self.target.is_some() {
            return;
        }
        let r: Result<Box<dyn Target>, String> = match &self.conn {
            Conn::Sim => Ok(Box::new(SimTarget::start())),
            Conn::Serial(port) => SerialTarget::open(port).map(|t| Box::new(t) as Box<dyn Target>),
        };
        match r {
            Ok(t) => {
                self.log(format!("Liaison en ligne établie avec {}.", t.name()));
                if matches!(self.conn, Conn::Sim) {
                    self.bottom = BottomTab::Simulation;
                    self.drawer = true;
                }
                self.target = Some(t);
                self.last_poll = Instant::now() - Duration::from_secs(1);
            }
            Err(e) => self.log(format!("Erreur de liaison : {e}")),
        }
    }

    pub fn disconnect(&mut self) {
        if self.target.take().is_some() {
            self.log("Liaison en ligne coupée.");
        }
        self.status = None;
        self.values.clear();
        self.monitor = false;
    }

    /// Exécute une opération sur l'automate ; une erreur de liaison coupe la liaison.
    pub fn with_target<T>(&mut self, f: impl FnOnce(&mut dyn Target) -> Result<T, String>) -> Option<T> {
        let t = self.target.as_mut()?;
        match f(t.as_mut()) {
            Ok(v) => Some(v),
            Err(e) => {
                self.log(format!("Erreur automate : {e}"));
                if e.contains("délai") || e.contains("envoi") || e.contains("réception") {
                    self.disconnect();
                }
                None
            }
        }
    }

    pub fn download(&mut self) {
        if !self.ensure_compiled() {
            return;
        }
        if self.target.is_none() {
            self.connect();
        }
        let Some(st) = self.with_target(target::status) else {
            return;
        };
        if st.mode == Mode::Run as u8 {
            self.modal = Some(Modal::StopForDownload);
        } else {
            self.do_download();
        }
    }

    fn do_download(&mut self) {
        let Some(c) = &self.compiled else { return };
        let image = c.image.clone();
        let crc = c.crc;
        let size = image.len();
        if self.with_target(|t| target::call(t, proto::STOP, &[])).is_none() {
            return;
        }
        let started = Instant::now();
        match self.with_target(|t| target::download(t, &image)) {
            Some(got) if got == crc => {
                self.log(format!(
                    "Chargement terminé : {size} octets en {} ms, CRC {crc:08X} vérifié.",
                    started.elapsed().as_millis()
                ));
                self.modal = Some(Modal::StartAfterDownload);
            }
            Some(got) => self.log(format!("Chargement : CRC différent ({got:08X} au lieu de {crc:08X})")),
            None => {}
        }
    }

    pub fn run(&mut self) {
        if self.with_target(|t| target::call(t, proto::START, &[])).is_some() {
            self.log("Automate en RUN.");
        }
    }

    pub fn stop(&mut self) {
        if self.with_target(|t| target::call(t, proto::STOP, &[])).is_some() {
            self.log("Automate en STOP.");
        }
    }

    /// Variables à lire pour la vue affichée.
    fn wanted_slots(&self) -> Vec<u16> {
        let Some(c) = &self.compiled else {
            return vec![];
        };
        let mut set = BTreeSet::new();
        match self.view {
            Some(View::Block(i)) if self.monitor => {
                if let Some(b) = self.project.blocks.get(i) {
                    for s in c.block_symbols(&b.name) {
                        if s.ty.is_scalar() {
                            set.insert(s.slot);
                        }
                    }
                }
                // Variables appelées depuis ce bloc mais déclarées ailleurs (FB, globales)
                for s in &c.symbols {
                    if s.block.is_none() && s.ty.is_scalar() {
                        set.insert(s.slot);
                    }
                }
                if let Some(b) = self.project.blocks.get(i) {
                    // Pour un FB : toutes ses instances
                    for s in &c.symbols {
                        if s.path.split('.').any(|p| p.eq_ignore_ascii_case(&b.name)) && s.ty.is_scalar() {
                            set.insert(s.slot);
                        }
                    }
                }
            }
            Some(View::Tags) | Some(View::Device) => {
                for s in c.symbols.iter().filter(|s| s.block.is_none()) {
                    if let takt_compiler::types::Ty::Array { len, .. } = s.ty {
                        set.extend(s.slot..s.slot + len);
                    } else {
                        set.insert(s.slot);
                    }
                }
            }
            Some(View::Watch(i)) => {
                if let Some(t) = self.project.watch_tables.get(i) {
                    for r in &t.rows {
                        if let Some((slot, _)) = c.resolve(&r.path) {
                            set.insert(slot);
                        }
                    }
                }
            }
            _ => {}
        }
        if self.bottom == BottomTab::Simulation {
            set.extend(c.io.iter().map(|io| io.slot));
        }
        set.into_iter().collect()
    }

    fn poll(&mut self) {
        if self.target.is_none() || self.last_poll.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.last_poll = Instant::now();
        let slots = self.wanted_slots();
        let r = self.with_target(|t| {
            let st = target::status(t)?;
            let vals = if slots.is_empty() { HashMap::new() } else { target::read(t, &slots)? };
            Ok((st, vals))
        });
        if let Some((st, vals)) = r {
            if let Some(old) = &self.status
                && old.mode != st.mode
                && st.mode == Mode::Fault as u8
            {
                let f = takt_vm::exec::Fault::from_u8(st.fault).map(|f| f.describe()).unwrap_or("?");
                self.log(format!("L'automate est passé en DÉFAUT : {f}."));
            }
            self.status = Some(st);
            self.values = vals;
        }
    }

    // ---------- interface ----------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let sc = |k: Key| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if pressed(sc(Key::S)) {
            self.save(false);
        }
        if pressed(sc(Key::B)) {
            self.compile();
        }
        if pressed(sc(Key::L)) {
            self.download();
        }
        if pressed(sc(Key::K)) {
            self.connect();
        }
        if pressed(sc(Key::M)) {
            self.disconnect();
        }
        if pressed(sc(Key::T)) {
            self.toggle_monitor();
        }
    }

    pub fn toggle_monitor(&mut self) {
        if self.monitor {
            self.monitor = false;
            return;
        }
        if !self.up_to_date() && !self.compile() {
            return;
        }
        if self.target.is_none() {
            self.connect();
        }
        self.monitor = self.target.is_some();
        if self.monitor && self.program_identical() == Some(false) {
            self.log("Attention : le programme dans l'automate est différent du projet. Chargez-le (Ctrl+L).");
        }
    }

    /// Menu « … » du projet, en bas du titre de la barre latérale.
    fn project_menu(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, ic: char, label: &str, shortcut: &str| {
            let text =
                theme::rich(&format!("{ic}   {label}"), 14.0, egui::FontFamily::Proportional, theme::colors(ui).text);
            ui.add(egui::Button::new(text).frame(false).shortcut_text(shortcut).min_size(egui::vec2(220.0, 28.0)))
                .clicked()
        };
        if item(ui, icon::FILE_PLUS, "Nouveau projet", "") {
            self.request(Pending::New);
            ui.close_menu();
        }
        if item(ui, icon::FOLDER, "Ouvrir…", "") {
            self.request(Pending::Open);
            ui.close_menu();
        }
        if item(ui, icon::BOOK, "Exemple « Convoyeur »", "") {
            self.request(Pending::Example);
            ui.close_menu();
        }
        ui.separator();
        if item(ui, icon::SAVE, "Enregistrer", "Ctrl+S") {
            self.save(false);
            ui.close_menu();
        }
        if item(ui, icon::SAVE, "Enregistrer sous…", "") {
            self.save(true);
            ui.close_menu();
        }
        ui.separator();
        let (ic, label) = if self.dark { (icon::SUN, "Thème clair") } else { (icon::MOON, "Thème sombre") };
        if item(ui, ic, label, "") {
            self.dark = !self.dark;
            theme::apply(ui.ctx(), self.dark);
            ui.close_menu();
        }
        if item(ui, icon::INFO, "À propos de Takt Studio", "") {
            self.modal = Some(Modal::About);
            ui.close_menu();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let c = theme::colors(ui);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let logo = self.logo.get_or_insert_with(|| theme::logo_texture(ui.ctx())).clone();
            ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(28.0, 28.0)).corner_radius(7.0));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("Takt Studio").font(egui::FontId::new(17.0, theme::semi())).color(c.text));
                let dirty = if self.dirty() { " ·  modifié" } else { "" };
                ui.label(RichText::new(format!("{}{dirty}", self.project.name)).size(12.0).color(c.soft));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(theme::icon_text(icon::MORE, 15.0).color(c.soft), |ui| self.project_menu(ui))
                    .response
                    .on_hover_text("Projet");
            });
        });
        ui.add_space(14.0);

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let board = views::board_name(&self.project.device.board);
            if theme::nav_item(
                ui,
                icon::CPU,
                &self.project.device.name,
                self.view == Some(View::Device),
                Some((short_board(board), c.soft)),
            )
            .on_hover_text(board)
            .clicked()
            {
                self.open(View::Device);
            }
            let tag_count = (self.project.tags.len().to_string(), c.soft);
            if theme::nav_item(ui, icon::TAG, "Variables API", self.view == Some(View::Tags), Some(tag_count)).clicked()
            {
                self.open(View::Tags);
            }

            if theme::nav_header(ui, "Blocs de programme", Some("Ajouter un bloc")) {
                self.modal = Some(Modal::NewBlock { name: String::new(), kind: PouKind::FunctionBlock });
            }
            for i in 0..self.project.blocks.len() {
                let kind = self.block_kind(i);
                let name = self.project.blocks[i].name.clone();
                let has_err = self.diags.iter().any(|d| d.severity == Severity::Error && d.block == name);
                let ic = match kind {
                    Some(PouKind::Program) => icon::CODE,
                    Some(PouKind::FunctionBlock) => icon::BOX,
                    _ => icon::FUNCTION,
                };
                let trailing =
                    if has_err { ("●".to_string(), c.red) } else { (kind_short(kind).to_string(), c.soft) };
                let r = theme::nav_item(ui, ic, &name, self.view == Some(View::Block(i)), Some(trailing));
                if r.clicked() {
                    self.open(View::Block(i));
                }
                r.context_menu(|ui| {
                    let text = theme::rich(
                        &format!("{}   Supprimer", icon::TRASH),
                        14.0,
                        egui::FontFamily::Proportional,
                        c.red,
                    );
                    if ui.button(text).clicked() {
                        self.modal = Some(Modal::DeleteBlock(i));
                        ui.close_menu();
                    }
                });
            }

            if theme::nav_header(ui, "Visualisation et forçage", Some("Ajouter une table")) {
                self.add_watch_table();
            }
            for i in 0..self.project.watch_tables.len() {
                let name = self.project.watch_tables[i].name.clone();
                let n = (self.project.watch_tables[i].rows.len().to_string(), c.soft);
                if theme::nav_item(ui, icon::EYE, &name, self.view == Some(View::Watch(i)), Some(n)).clicked() {
                    self.open(View::Watch(i));
                }
            }
        });
    }

    /// Bas de la barre latérale : cible et état de la liaison.
    fn connection_box(&mut self, ui: &mut egui::Ui) {
        let c = theme::colors(ui);
        egui::Frame::new().fill(c.bg).corner_radius(theme::RADIUS).inner_margin(egui::Margin::same(12)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                let (dot, text) = match (&self.status, self.online()) {
                    (Some(st), true) => match Mode::from_u8(st.mode) {
                        Mode::Run => (c.green, "En ligne · RUN"),
                        Mode::Stop => (c.orange, "En ligne · STOP"),
                        Mode::Fault => (c.red, "En ligne · DÉFAUT"),
                        Mode::NoProgram => (c.soft, "En ligne · vide"),
                    },
                    (_, true) => (c.soft, "Connexion…"),
                    _ => (c.very_soft, "Hors ligne"),
                };
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, dot);
                    ui.label(RichText::new(text).font(egui::FontId::new(13.5, theme::medium())).color(c.text));
                });
                let label = match &self.conn {
                    Conn::Sim => "Simulateur PLCSIM".to_string(),
                    Conn::Serial(p) => format!("RP2350 · {p}"),
                };
                ui.add_enabled_ui(!self.online(), |ui| {
                    egui::ComboBox::from_id_salt("conn")
                        .selected_text(RichText::new(label).size(13.0))
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.conn, Conn::Sim, "Simulateur PLCSIM");
                            for p in self.ports.clone() {
                                ui.selectable_value(&mut self.conn, Conn::Serial(p.clone()), format!("RP2350 · {p}"));
                            }
                            let text = theme::rich(
                                &format!("{}  Rechercher les ports", icon::REFRESH),
                                14.0,
                                egui::FontFamily::Proportional,
                                c.text,
                            );
                            if ui.button(text).clicked() {
                                self.ports = SerialTarget::ports();
                            }
                        });
                });
                ui.add_space(2.0);
                let w = ui.available_width();
                if self.online() {
                    let b = egui::Button::new(theme::rich(
                        &format!("{}  Se déconnecter", icon::UNPLUG),
                        14.0,
                        theme::medium(),
                        c.text,
                    ))
                    .fill(c.group);
                    if ui.add_sized([w, 30.0], b).on_hover_text("Ctrl+M").clicked() {
                        self.disconnect();
                    }
                } else {
                    let b = egui::Button::new(theme::rich(
                        &format!("{}  Se connecter", icon::PLUG),
                        14.0,
                        theme::medium(),
                        Color32::WHITE,
                    ))
                    .fill(c.accent);
                    if ui.add_sized([w, 30.0], b).on_hover_text("Ctrl+K").clicked() {
                        self.connect();
                    }
                }
            },
        );
    }

    /// Pastilles d'état sous le titre : temps de cycle garanti, programme, forçage.
    fn status_chips(&mut self, ui: &mut egui::Ui) {
        let c = theme::colors(ui);
        ui.horizontal_wrapped(|ui| {
            match (&self.compiled, self.up_to_date()) {
                (Some(cp), true) => {
                    let color = if cp.wcet.ratio() > takt_compiler::WCET_WARN_RATIO { c.orange } else { c.green };
                    theme::chip(
                        ui,
                        &format!("{}  cycle garanti {} / {} µs", icon::TIMER, cp.wcet.total_us, cp.wcet.cycle_us),
                        color,
                    )
                    .on_hover_text("Pire cas prouvé par le compilateur, comparé au temps de cycle configuré");
                }
                _ if self.diags.iter().any(|d| d.severity == Severity::Error)
                    && self.compiled_rev == Some(self.rev) =>
                {
                    let n = self.diags.iter().filter(|d| d.severity == Severity::Error).count();
                    if theme::chip(ui, &format!("{}  {n} erreur(s)", icon::ALERT), c.red)
                        .interact(egui::Sense::click())
                        .clicked()
                    {
                        self.drawer = true;
                        self.bottom = BottomTab::Info;
                    }
                }
                _ => {
                    theme::chip(ui, &format!("{}  non compilé", icon::HAMMER), c.soft);
                }
            }
            if let Some(st) = &self.status {
                match self.program_identical() {
                    Some(true) => theme::chip(ui, &format!("{}  programme identique", icon::CHECK), c.green),
                    _ => theme::chip(ui, &format!("{}  programme différent", icon::WARNING), c.orange),
                };
                if st.n_forces > 0 {
                    theme::chip(ui, &format!("{}  {} forçage(s)", icon::ZAP, st.n_forces), c.red);
                }
                if st.overruns > 0 {
                    theme::chip(ui, &format!("{}  {} dépassement(s)", icon::ALERT, st.overruns), c.red);
                }
            }
        });
    }

    /// En-tête de page : titre, sous-titre, actions.
    fn header(&mut self, ui: &mut egui::Ui) {
        let (title, subtitle) = match self.view {
            Some(View::Device) => (
                self.project.device.name.clone(),
                format!(
                    "{} · cycle de {} µs",
                    views::board_name(&self.project.device.board),
                    self.project.device.cycle_us
                ),
            ),
            Some(View::Tags) => ("Variables API".into(), "Entrées, sorties et mémentos de l'automate".into()),
            Some(View::Block(i)) => {
                let kind = self.block_kind(i);
                let sub = match kind {
                    Some(PouKind::Program) => "Bloc d'organisation · exécuté à chaque cycle",
                    Some(PouKind::FunctionBlock) => "Bloc fonctionnel · avec mémoire d'instance",
                    Some(PouKind::Function) => "Fonction · sans mémoire",
                    None => "Bloc",
                };
                (self.project.blocks[i].name.clone(), sub.to_string())
            }
            Some(View::Watch(i)) => {
                (self.project.watch_tables[i].name.clone(), "Visualiser, modifier et forcer des variables".into())
            }
            None => (self.project.name.clone(), String::new()),
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| theme::page_title(ui, &title, &subtitle));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::primary_button(ui, icon::DOWNLOAD, "Charger")
                    .on_hover_text("Charger dans l'appareil (Ctrl+L)")
                    .clicked()
                {
                    self.download();
                }
                if theme::secondary_button(ui, icon::HAMMER, "Compiler")
                    .on_hover_text("Compiler et prouver le temps de cycle (Ctrl+B)")
                    .clicked()
                {
                    self.compile();
                }
                if self.online() {
                    let mut mode = self.status.map(|s| Mode::from_u8(s.mode) == Mode::Run).unwrap_or(false);
                    if theme::segmented(ui, &mut mode, &[(false, "STOP"), (true, "RUN")]) {
                        if mode { self.run() } else { self.stop() }
                    }
                }
                if theme::toggle_button(ui, icon::EYE, "Visualiser", self.monitor)
                    .on_hover_text("Valeurs en ligne dans le code (Ctrl+T)")
                    .clicked()
                {
                    self.toggle_monitor();
                }
            });
        });
        ui.add_space(10.0);
        self.status_chips(ui);
        ui.add_space(14.0);
    }

    fn instructions_card(&mut self, ui: &mut egui::Ui) {
        let c = theme::colors(ui);
        let editing = match self.view {
            Some(View::Block(i)) if !self.monitor => Some(i),
            _ => None,
        };
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            theme::section_title(ui, "Instructions");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::ghost_icon(ui, icon::X, "Masquer").clicked() {
                    self.show_instructions = false;
                }
            });
        });
        ui.label(
            RichText::new("Cliquez pour insérer au curseur. Les blocs standard sont déclarés automatiquement.")
                .size(12.0)
                .color(c.soft),
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for g in instructions::CATALOG {
                ui.add_space(6.0);
                ui.label(RichText::new(g.name).size(12.5).color(c.soft));
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for it in g.items {
                        let (name, rest) = it.label.split_once("  ").unwrap_or((it.label, ""));
                        let r = ui
                            .add_enabled(
                                editing.is_some(),
                                egui::Button::new(
                                    RichText::new(format!("{name}  "))
                                        .font(egui::FontId::new(13.5, theme::medium()))
                                        .color(c.text),
                                )
                                .frame(false)
                                .shortcut_text(RichText::new(rest).size(12.0).color(c.soft))
                                .min_size(egui::vec2(ui.available_width(), 28.0)),
                            )
                            .on_hover_text(it.help);
                        if r.clicked()
                            && let Some(i) = editing
                        {
                            let id = editor_id(i);
                            let src = &mut self.project.blocks[i].source;
                            let snippet = match it.instance {
                                Some((ty, prefix)) => {
                                    let name = instructions::declare_instance(src, ty, prefix);
                                    it.snippet.replace("{n}", &name)
                                }
                                None => it.snippet.to_string(),
                            };
                            editor::insert_snippet(ui.ctx(), id, src, &snippet);
                            self.touch();
                        }
                    }
                });
            }
            ui.add_space(12.0);
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        self.header(ui);
        match self.view {
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new("Choisissez un élément dans la barre latérale.").color(theme::colors(ui).soft),
                    );
                });
            }
            Some(View::Device) => views::device_view(self, ui),
            Some(View::Tags) => views::tags_view(self, ui),
            Some(View::Watch(i)) => views::watch_view(self, ui, i),
            Some(View::Block(i)) => self.block_view(ui, i),
        }
    }

    fn block_view(&mut self, ui: &mut egui::Ui, i: usize) {
        let Some(name) = self.project.blocks.get(i).map(|b| b.name.clone()) else {
            return;
        };
        let monitoring = self.monitor && self.compiled.is_some() && self.online();
        if monitoring {
            let c = self.compiled.as_ref().unwrap();
            let values = &self.values;
            let lookup = |n: &str| -> Option<(String, Option<bool>)> {
                let key = n.trim_start_matches('#');
                let sym = c
                    .block_symbols(&name)
                    .find(|s| s.local_name().eq_ignore_ascii_case(key))
                    .map(|s| (s.slot, s.ty.clone()))
                    .or_else(|| c.resolve(&format!("{name}.{key}")))
                    .or_else(|| c.resolve(key))
                    .or_else(|| {
                        // Dans un FB : première instance trouvée
                        c.symbols
                            .iter()
                            .find(|s| {
                                s.path.to_ascii_uppercase().ends_with(&format!(".{}", key.to_ascii_uppercase()))
                                    && s.path.split('.').any(|p| p.eq_ignore_ascii_case(&name))
                            })
                            .map(|s| (s.slot, s.ty.clone()))
                    })?;
                let v = *values.get(&sym.0)?;
                if !sym.1.is_scalar() {
                    return None;
                }
                let state = (sym.1 == takt_compiler::types::Ty::Bool).then_some(v != 0);
                Some((takt_compiler::types::format_value(v, &sym.1), state))
            };
            ui.horizontal(|ui| {
                let text = format!("{}  visualisation en cours · édition verrouillée (Ctrl+T pour quitter)", icon::EYE);
                theme::chip(ui, &text, theme::colors(ui).accent);
            });
            ui.add_space(8.0);
            let src = self.project.blocks[i].source.clone();
            theme::card(ui, |ui| editor::monitor_view(ui, editor_id(i), &src, &lookup));
            return;
        }
        let marks: Vec<LineMark> = self
            .diags
            .iter()
            .filter(|d| d.block == name && d.pos.line > 0)
            .map(|d| LineMark { line: d.pos.line, severity: d.severity, msg: d.msg.clone() })
            .collect();
        let mut goto = match self.goto {
            Some((b, line)) if b == i => {
                self.goto = None;
                Some(line)
            }
            _ => None,
        };
        let source = &mut self.project.blocks[i].source;
        let changed = theme::card(ui, |ui| editor::code_editor(ui, editor_id(i), source, &marks, &mut goto));
        if changed {
            self.touch();
            // Le nom du bloc suit l'en-tête de la source
            if let Ok(pous) = takt_compiler::parser::parse(&self.project.blocks[i].source)
                && let Some(p) = pous.first()
                && p.name != self.project.blocks[i].name
            {
                self.project.blocks[i].name = p.name.clone();
            }
        }
    }

    fn modals(&mut self, ctx: &egui::Context) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        if let Modal::NewBlock { mut name, mut kind } = modal {
            // Formulaire rendu séparément pour pouvoir modifier ses champs
            let r = egui::Modal::new(egui::Id::new("newblock")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.heading("Ajouter nouveau bloc");
                ui.horizontal(|ui| {
                    ui.label("Nom :");
                    ui.text_edit_singleline(&mut name).request_focus();
                });
                ui.radio_value(&mut kind, PouKind::Program, "Bloc d'organisation (OB) — cyclique");
                ui.radio_value(&mut kind, PouKind::FunctionBlock, "Bloc fonctionnel (FB) — avec mémoire");
                ui.radio_value(&mut kind, PouKind::Function, "Fonction (FC) — sans mémoire");
                ui.add_space(8.0);
                let valid = !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !name.starts_with(|c: char| c.is_ascii_digit());
                let mut action = 0;
                ui.horizontal(|ui| {
                    if ui.add_enabled(valid, egui::Button::new("OK")).clicked()
                        || (valid && ui.input(|i| i.key_pressed(Key::Enter)))
                    {
                        action = 1;
                    }
                    if ui.button("Annuler").clicked() {
                        action = 2;
                    }
                });
                action
            });
            match r.inner {
                1 => self.add_block(&name, kind),
                2 => {}
                _ if r.should_close() => {}
                _ => self.modal = Some(Modal::NewBlock { name, kind }),
            }
            return;
        }
        let mut keep = true;
        let mut next = None;
        let resp = egui::Modal::new(egui::Id::new("modal")).show(ctx, |ui| {
            ui.set_width(380.0);
            match &modal {
                Modal::StopForDownload => {
                    ui.heading("Charger dans l'appareil");
                    ui.label("L'automate est en RUN. Il doit passer en STOP pour recevoir le programme.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Passer en STOP et charger").clicked() {
                            next = Some(1);
                        }
                        if ui.button("Annuler").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::StartAfterDownload => {
                    ui.heading("Chargement terminé");
                    ui.label("Démarrer l'automate (RUN) ?");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("▶ Démarrer").clicked() {
                            next = Some(2);
                        }
                        if ui.button("Rester en STOP").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::NewBlock { .. } => unreachable!(),
                Modal::DeleteBlock(i) => {
                    ui.heading("Supprimer le bloc");
                    ui.label(format!(
                        "Supprimer « {} » ? Cette action ne peut pas être annulée.",
                        self.project.blocks[*i].name
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Supprimer").clicked() {
                            next = Some(3);
                        }
                        if ui.button("Annuler").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::Discard(_) => {
                    ui.heading("Modifications non enregistrées");
                    ui.label("Le projet a été modifié. Abandonner les modifications ?");
                    ui.horizontal(|ui| {
                        if ui.button("Enregistrer d'abord").clicked() {
                            next = Some(4);
                        }
                        if ui.button("Abandonner").clicked() {
                            next = Some(5);
                        }
                        if ui.button("Annuler").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::About => {
                    ui.heading("Takt Studio");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.label("IDE façon TIA Portal pour automates RP2350 au temps de cycle garanti.");
                    ui.label("Licence GPL-3.0 — https://github.com/ideaton/takt");
                    if ui.button("Fermer").clicked() {
                        keep = false;
                    }
                }
            }
        });
        if resp.should_close() {
            keep = false;
        }
        match next {
            Some(1) => {
                self.do_download();
                if !matches!(self.modal, Some(Modal::StartAfterDownload)) {
                    self.modal = None;
                }
                return;
            }
            Some(2) => {
                self.run();
                return;
            }
            Some(3) => {
                if let Modal::DeleteBlock(i) = modal {
                    self.delete_block(i);
                }
                return;
            }
            Some(4) => {
                self.save(false);
                if let Modal::Discard(a) = modal
                    && !self.dirty()
                {
                    self.perform(a);
                }
                return;
            }
            Some(5) => {
                if let Modal::Discard(a) = modal {
                    self.saved_rev = self.rev;
                    self.perform(a);
                }
                return;
            }
            _ => {}
        }
        if keep {
            self.modal = Some(modal);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

impl App {
    pub fn ui(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.first_frame) {
            ctx.request_repaint();
            return;
        }
        self.shortcuts(ctx);
        self.poll();

        let title = format!(
            "Takt Studio — {}{}",
            self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| self.project.name.clone()),
            if self.dirty() { " *" } else { "" }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let c = if self.dark { theme::DARK } else { theme::LIGHT };
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(250.0)
            .frame(egui::Frame::new().fill(c.bar).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(12.0);
                    self.connection_box(ui);
                    ui.add_space(6.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| self.sidebar(ui));
                });
            });
        if self.drawer {
            egui::TopBottomPanel::bottom("drawer")
                .resizable(true)
                .default_height(230.0)
                .min_height(120.0)
                .frame(egui::Frame::new().fill(c.bg).inner_margin(egui::Margin {
                    left: 32,
                    right: 32,
                    top: 10,
                    bottom: 10,
                }))
                .show(ctx, |ui| views::inspector(self, ui));
        }
        let block_view = matches!(self.view, Some(View::Block(_))) && !self.monitor;
        if block_view && self.show_instructions {
            egui::SidePanel::right("instructions")
                .resizable(false)
                .exact_width(270.0)
                .frame(egui::Frame::new().fill(c.bg).inner_margin(egui::Margin {
                    left: 16,
                    right: 24,
                    top: 8,
                    bottom: 0,
                }))
                .show(ctx, |ui| self.instructions_card(ui));
        }
        let page =
            egui::Frame::new().fill(c.bg).inner_margin(egui::Margin { left: 32, right: 32, top: 26, bottom: 12 });
        egui::CentralPanel::default().frame(page).show(ctx, |ui| {
            // Boutons pour rouvrir ce qui a été masqué
            if !self.drawer || (block_view && !self.show_instructions) {
                egui::Area::new(egui::Id::new("reopen"))
                    .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-24.0, -16.0))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            if block_view
                                && !self.show_instructions
                                && theme::secondary_button(ui, icon::BLOCKS, "Instructions").clicked()
                            {
                                self.show_instructions = true;
                            }
                            if !self.drawer && theme::secondary_button(ui, icon::SCROLL, "Messages").clicked() {
                                self.drawer = true;
                            }
                        });
                    });
            }
            self.central(ui);
        });
        self.modals(ctx);

        if self.online() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

pub fn editor_id(block: usize) -> egui::Id {
    egui::Id::new(("editor", block))
}

pub fn kind_short(k: Option<PouKind>) -> &'static str {
    match k {
        Some(PouKind::Program) => "OB",
        Some(PouKind::FunctionBlock) => "FB",
        Some(PouKind::Function) => "FC",
        None => "?",
    }
}

fn serde_json_example() -> Project {
    // L'exemple est embarqué et couvert par les tests du compilateur.
    Project::from_json(EXAMPLE).unwrap_or_else(|_| Project::new_empty("Nouveau projet"))
}

/// « Raspberry Pi Pico 2 » → « Pico 2 », pour la barre latérale.
fn short_board(name: &str) -> String {
    name.trim_start_matches("Raspberry Pi ").trim_start_matches("Waveshare ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fait tourner l'interface sans écran : chaque vue, la simulation, le
    /// chargement, le RUN et la visualisation en ligne.
    #[test]
    fn interface_complete_sans_ecran() {
        let ctx = egui::Context::default();
        let mut app = App::create(&ctx, None);
        let frame = |app: &mut App| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.ui(ctx));
        };
        for v in [View::Device, View::Tags, View::Block(0), View::Block(1), View::Block(2), View::Watch(0)] {
            app.open(v);
            frame(&mut app);
        }
        assert!(app.compile(), "{:#?}", app.diags);
        app.open(View::Device);
        frame(&mut app);

        app.download();
        assert!(matches!(app.modal, Some(Modal::StartAfterDownload)), "le simulateur vide se charge sans STOP");
        app.modal = None;
        app.run();
        app.bottom = BottomTab::Simulation;
        for tab in [BottomTab::Simulation, BottomTab::Diagnostic, BottomTab::Info, BottomTab::Journal] {
            app.bottom = tab;
            frame(&mut app);
        }

        app.open(View::Block(0));
        app.toggle_monitor();
        assert!(app.monitor);
        std::thread::sleep(Duration::from_millis(150));
        frame(&mut app);
        let st = app.status.expect("état lu");
        assert_eq!(st.mode, Mode::Run as u8);
        assert!(st.scans > 10, "le simulateur tourne ({} cycles)", st.scans);
        assert_eq!(app.program_identical(), Some(true));
        assert!(!app.values.is_empty());
        app.open(View::Block(1));
        frame(&mut app);

        // Rechargement en RUN : demande de passage en STOP
        app.download();
        assert!(matches!(app.modal, Some(Modal::StopForDownload)));
        frame(&mut app);

        // Projet modifié : le programme en ligne n'est plus identique
        app.project.device.cycle_us = 2000;
        app.touch();
        assert_eq!(app.program_identical(), Some(false));
        app.disconnect();
        frame(&mut app);
    }

    /// Captures d'écran de l'interface, rendues hors écran avec wgpu.
    /// `cargo test -p takt-ide captures -- --ignored` ; images dans `target/captures`.
    #[test]
    #[ignore]
    fn captures() {
        use egui_kittest::Harness;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/captures");
        std::fs::create_dir_all(&dir).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .build_state(|ctx, app: &mut Option<App>| app.get_or_insert_with(|| App::create(ctx, None)).ui(ctx), None);
        let shot = |h: &mut Harness<Option<App>>, name: &str| {
            for _ in 0..4 {
                h.run_steps(1);
            }
            h.render().expect("rendu").save(dir.join(format!("{name}.png"))).unwrap();
        };
        h.run_steps(2);
        shot(&mut h, "1-editeur");
        {
            let app = h.state_mut().as_mut().unwrap();
            app.compile();
            app.open(View::Device);
        }
        shot(&mut h, "2-appareil");
        {
            let app = h.state_mut().as_mut().unwrap();
            app.download();
            app.modal = None;
            app.run();
            app.open(View::Block(0));
            app.toggle_monitor();
        }
        std::thread::sleep(Duration::from_millis(300));
        shot(&mut h, "3-visualisation");
        {
            let app = h.state_mut().as_mut().unwrap();
            app.monitor = false;
            app.open(View::Watch(0));
        }
        std::thread::sleep(Duration::from_millis(200));
        shot(&mut h, "4-table");
        {
            let app = h.state_mut().as_mut().unwrap();
            app.open(View::Tags);
            app.bottom = BottomTab::Diagnostic;
        }
        std::thread::sleep(Duration::from_millis(200));
        shot(&mut h, "5-variables-diagnostic");
        {
            let app = h.state_mut().as_mut().unwrap();
            app.disconnect();
            app.dark = true;
            app.open(View::Block(1));
            let ctx = h.ctx.clone();
            theme::apply(&ctx, true);
        }
        shot(&mut h, "6-sombre");
    }
}
