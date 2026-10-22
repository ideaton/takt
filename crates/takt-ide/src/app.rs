//! Fenêtre principale : état du projet, actions (compiler, charger, en ligne)
//! et disposition façon TIA Portal.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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

/// Couleur « pétrole » de l'interface.
pub const ACCENT: Color32 = Color32::from_rgb(0, 128, 128);
pub const GREEN: Color32 = Color32::from_rgb(0, 160, 70);
pub const ORANGE: Color32 = Color32::from_rgb(230, 140, 0);
pub const RED: Color32 = Color32::from_rgb(210, 40, 40);

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
    pub tabs: Vec<View>,
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
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>) -> App {
        App::create(&cc.egui_ctx, path)
    }

    pub fn create(ctx: &egui::Context, path: Option<PathBuf>) -> App {
        setup_style(ctx);
        let mut app = App {
            project: serde_json_example(),
            path: None,
            rev: 1,
            saved_rev: 1,
            tabs: vec![View::Block(0)],
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
        self.tabs.clear();
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
        if !self.tabs.contains(&v) {
            self.tabs.push(v);
        }
        self.view = Some(v);
    }

    fn close_tab(&mut self, v: View) {
        self.tabs.retain(|t| *t != v);
        if self.view == Some(v) {
            self.view = self.tabs.last().copied();
        }
    }

    pub fn view_title(&mut self, v: View) -> String {
        match v {
            View::Device => format!("{} [{}]", self.project.device.name, views::board_name(&self.project.device.board)),
            View::Tags => "Variables API".into(),
            View::Block(i) => {
                let kind = self.block_kind(i);
                let b = &self.project.blocks[i];
                format!("{} [{}]", b.name, kind_short(kind))
            }
            View::Watch(i) => self.project.watch_tables[i].name.clone(),
        }
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
        self.tabs.retain(|t| *t != View::Block(i));
        for t in self.tabs.iter_mut() {
            if let View::Block(k) = t
                && *k > i
            {
                *k -= 1;
            }
        }
        if let Some(View::Block(k)) = self.view {
            if k == i {
                self.view = self.tabs.last().copied();
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

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("Projet", |ui| {
                if ui.button("Nouveau").clicked() {
                    self.request(Pending::New);
                    ui.close_menu();
                }
                if ui.button("Ouvrir…").clicked() {
                    self.request(Pending::Open);
                    ui.close_menu();
                }
                if ui.button("Ouvrir l'exemple « Convoyeur »").clicked() {
                    self.request(Pending::Example);
                    ui.close_menu();
                }
                ui.separator();
                if ui.add(egui::Button::new("Enregistrer").shortcut_text("Ctrl+S")).clicked() {
                    self.save(false);
                    ui.close_menu();
                }
                if ui.button("Enregistrer sous…").clicked() {
                    self.save(true);
                    ui.close_menu();
                }
            });
            ui.menu_button("Édition", |ui| {
                if ui.button("Ajouter un bloc…").clicked() {
                    self.modal = Some(Modal::NewBlock { name: String::new(), kind: PouKind::FunctionBlock });
                    ui.close_menu();
                }
                if ui.button("Ajouter une table de visualisation").clicked() {
                    self.add_watch_table();
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Compiler").shortcut_text("Ctrl+B")).clicked() {
                    self.compile();
                    ui.close_menu();
                }
            });
            ui.menu_button("En ligne", |ui| {
                if ui.add(egui::Button::new("Charger dans l'appareil").shortcut_text("Ctrl+L")).clicked() {
                    self.download();
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Liaison en ligne").shortcut_text("Ctrl+K")).clicked() {
                    self.connect();
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Liaison hors ligne").shortcut_text("Ctrl+M")).clicked() {
                    self.disconnect();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Démarrer la CPU (RUN)").clicked() {
                    self.run();
                    ui.close_menu();
                }
                if ui.button("Arrêter la CPU (STOP)").clicked() {
                    self.stop();
                    ui.close_menu();
                }
                if ui.button("Remettre à zéro les statistiques").clicked() {
                    self.with_target(|t| target::call(t, proto::RESET_STATS, &[]));
                    ui.close_menu();
                }
            });
            ui.menu_button("Affichage", |ui| {
                let mut dark = ui.ctx().style().visuals.dark_mode;
                if ui.checkbox(&mut dark, "Thème sombre").changed() {
                    let ctx = ui.ctx().clone();
                    ctx.set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
                    setup_style(&ctx);
                }
            });
            ui.menu_button("Aide", |ui| {
                if ui.button("À propos de Takt Studio").clicked() {
                    self.modal = Some(Modal::About);
                    ui.close_menu();
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let b = |ui: &mut egui::Ui, icon: &str, tip: &str| {
                ui.add(egui::Button::new(RichText::new(icon).size(16.0))).on_hover_text(tip)
            };
            if b(ui, "🗋", "Nouveau projet").clicked() {
                self.request(Pending::New);
            }
            if b(ui, "📂", "Ouvrir un projet").clicked() {
                self.request(Pending::Open);
            }
            if b(ui, "💾", "Enregistrer (Ctrl+S)").clicked() {
                self.save(false);
            }
            ui.separator();
            if b(ui, "⚙ Compiler", "Compiler et vérifier le temps de cycle (Ctrl+B)").clicked() {
                self.compile();
            }
            if b(ui, "⬇ Charger", "Charger dans l'appareil (Ctrl+L)").clicked() {
                self.download();
            }
            ui.separator();
            ui.label("Cible :");
            let label = match &self.conn {
                Conn::Sim => "PLCSIM (simulation)".to_string(),
                Conn::Serial(p) => format!("RP2350 sur {p}"),
            };
            ui.add_enabled_ui(!self.online(), |ui| {
                egui::ComboBox::from_id_salt("conn").selected_text(label).width(170.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.conn, Conn::Sim, "PLCSIM (simulation)");
                    for p in self.ports.clone() {
                        ui.selectable_value(&mut self.conn, Conn::Serial(p.clone()), format!("RP2350 sur {p}"));
                    }
                    if ui.button("⟳ Rechercher les ports").clicked() {
                        self.ports = SerialTarget::ports();
                    }
                });
            });
            if self.online() {
                if b(ui, "✂ Hors ligne", "Liaison hors ligne (Ctrl+M)").clicked() {
                    self.disconnect();
                }
            } else if b(ui, "🔗 En ligne", "Liaison en ligne (Ctrl+K)").clicked() {
                self.connect();
            }
            ui.add_enabled_ui(self.online(), |ui| {
                if b(ui, "▶", "Démarrer la CPU (RUN)").clicked() {
                    self.run();
                }
                if b(ui, "■", "Arrêter la CPU (STOP)").clicked() {
                    self.stop();
                }
            });
            let glasses = if self.monitor {
                RichText::new("👓 Visualisation").color(Color32::WHITE)
            } else {
                RichText::new("👓 Visualisation")
            };
            let btn = egui::Button::new(glasses).fill(if self.monitor {
                ACCENT
            } else {
                ui.visuals().widgets.inactive.weak_bg_fill
            });
            if ui.add(btn).on_hover_text("Activer/désactiver la visualisation (Ctrl+T)").clicked() {
                self.toggle_monitor();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.cpu_badges(ui);
            });
        });
    }

    fn cpu_badges(&mut self, ui: &mut egui::Ui) {
        let badge = |ui: &mut egui::Ui, text: &str, color: Color32| {
            egui::Frame::new().fill(color).corner_radius(3.0).inner_margin(egui::Margin::symmetric(6, 2)).show(
                ui,
                |ui| {
                    ui.label(RichText::new(text).color(Color32::WHITE).strong());
                },
            );
        };
        let Some(st) = &self.status else {
            badge(ui, "HORS LIGNE", Color32::from_gray(140));
            return;
        };
        if st.n_forces > 0 {
            badge(ui, "FORCE", RED);
        }
        match self.program_identical() {
            Some(true) => badge(ui, "✔ identique", GREEN),
            _ => badge(ui, "≠ différent", ORANGE),
        }
        let (text, color) = match Mode::from_u8(st.mode) {
            Mode::Run => ("RUN", GREEN),
            Mode::Stop => ("STOP", ORANGE),
            Mode::Fault => ("DÉFAUT", RED),
            Mode::NoProgram => ("VIDE", Color32::from_gray(140)),
        };
        badge(ui, text, color);
    }

    fn navigator(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("Navigateur du projet").strong().color(ACCENT));
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let root = format!("📁 {}", self.project.name);
            egui::CollapsingHeader::new(root).default_open(true).show(ui, |ui| {
                let dev = format!("🖥 {} [{}]", self.project.device.name, views::board_name(&self.project.device.board));
                egui::CollapsingHeader::new(dev).default_open(true).show(ui, |ui| {
                    if ui.selectable_label(self.view == Some(View::Device), "⚙ Configuration de l'appareil").clicked()
                    {
                        self.open(View::Device);
                    }
                    egui::CollapsingHeader::new("📂 Blocs de programme").default_open(true).show(ui, |ui| {
                        if ui.selectable_label(false, "➕ Ajouter nouveau bloc").clicked() {
                            self.modal = Some(Modal::NewBlock { name: String::new(), kind: PouKind::FunctionBlock });
                        }
                        for i in 0..self.project.blocks.len() {
                            let kind = self.block_kind(i);
                            let has_err = self
                                .diags
                                .iter()
                                .any(|d| d.severity == Severity::Error && d.block == self.project.blocks[i].name);
                            let icon = match kind {
                                Some(PouKind::Program) => "🟦",
                                Some(PouKind::FunctionBlock) => "🟩",
                                Some(PouKind::Function) => "🟨",
                                None => "⬜",
                            };
                            let mut text =
                                RichText::new(format!("{icon} {} [{}]", self.project.blocks[i].name, kind_short(kind)));
                            if has_err {
                                text = text.color(RED);
                            }
                            let r = ui.selectable_label(self.view == Some(View::Block(i)), text);
                            if r.clicked() {
                                self.open(View::Block(i));
                            }
                            r.context_menu(|ui| {
                                if ui.button("Supprimer").clicked() {
                                    self.modal = Some(Modal::DeleteBlock(i));
                                    ui.close_menu();
                                }
                            });
                        }
                    });
                    egui::CollapsingHeader::new("📂 Variables API").default_open(true).show(ui, |ui| {
                        if ui
                            .selectable_label(self.view == Some(View::Tags), "📋 Table de variables standard")
                            .clicked()
                        {
                            self.open(View::Tags);
                        }
                    });
                    egui::CollapsingHeader::new("📂 Tables de visualisation et de forçage").default_open(true).show(
                        ui,
                        |ui| {
                            if ui.selectable_label(false, "➕ Ajouter une table").clicked() {
                                self.add_watch_table();
                            }
                            for i in 0..self.project.watch_tables.len() {
                                let name = format!("👁 {}", self.project.watch_tables[i].name);
                                if ui.selectable_label(self.view == Some(View::Watch(i)), name).clicked() {
                                    self.open(View::Watch(i));
                                }
                            }
                        },
                    );
                });
            });
        });
    }

    fn instructions_card(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("Instructions").strong().color(ACCENT));
        ui.separator();
        let editing = match self.view {
            Some(View::Block(i)) if !self.monitor => Some(i),
            _ => None,
        };
        if editing.is_none() {
            ui.label(RichText::new("Ouvrez un bloc hors visualisation pour insérer des instructions.").weak().small());
        }
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for g in instructions::CATALOG {
                egui::CollapsingHeader::new(g.name).default_open(g.name.starts_with("Tempo")).show(ui, |ui| {
                    for it in g.items {
                        let r = ui
                            .add_enabled(editing.is_some(), egui::Button::new(it.label).frame(false))
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
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        // Onglets des éditeurs ouverts
        ui.horizontal_wrapped(|ui| {
            for v in self.tabs.clone() {
                let title = self.view_title(v);
                let selected = self.view == Some(v);
                egui::Frame::new()
                    .fill(if selected { ui.visuals().extreme_bg_color } else { ui.visuals().faint_bg_color })
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        if selected { ACCENT } else { ui.visuals().widgets.noninteractive.bg_stroke.color },
                    ))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        if ui.selectable_label(selected, title).clicked() {
                            self.view = Some(v);
                        }
                        if ui.small_button("✕").clicked() {
                            self.close_tab(v);
                        }
                    });
            }
        });
        ui.separator();
        match self.view {
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Ouvrez un élément dans le navigateur du projet.").weak());
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
                ui.label(
                    RichText::new("👓 Visualisation en cours — édition verrouillée (Ctrl+T pour quitter)")
                        .color(ACCENT),
                );
                if self.program_identical() == Some(false) {
                    ui.label(RichText::new("⚠ programme différent dans l'automate").color(ORANGE));
                }
            });
            let src = self.project.blocks[i].source.clone();
            editor::monitor_view(ui, editor_id(i), &src, &lookup);
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
        let changed = editor::code_editor(ui, editor_id(i), &mut self.project.blocks[i].source, &marks, &mut goto);
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
        self.shortcuts(ctx);
        self.poll();

        let title = format!(
            "Takt Studio — {}{}",
            self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| self.project.name.clone()),
            if self.dirty() { " *" } else { "" }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu(ui));
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(2.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });
        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let last = self.journal.last().cloned().unwrap_or_default();
                ui.label(RichText::new(last).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(c) = &self.compiled {
                        let color = if c.wcet.ratio() > takt_compiler::WCET_WARN_RATIO { ORANGE } else { GREEN };
                        ui.label(
                            RichText::new(format!("⏱ cycle garanti {} / {} µs", c.wcet.total_us, c.wcet.cycle_us))
                                .small()
                                .color(color),
                        );
                    }
                });
            });
        });
        egui::TopBottomPanel::bottom("inspector")
            .resizable(true)
            .default_height(200.0)
            .show(ctx, |ui| views::inspector(self, ui));
        egui::SidePanel::left("navigator").resizable(true).default_width(250.0).show(ctx, |ui| self.navigator(ui));
        egui::SidePanel::right("instructions")
            .resizable(true)
            .default_width(230.0)
            .show(ctx, |ui| self.instructions_card(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.central(ui));
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

fn setup_style(ctx: &egui::Context) {
    ctx.style_mut(|s| {
        s.visuals.selection.bg_fill = ACCENT;
        s.visuals.selection.stroke.color = Color32::WHITE;
        s.visuals.hyperlink_color = ACCENT;
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
    });
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
}
