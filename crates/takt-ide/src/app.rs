//! Fenêtre principale : état du projet, actions (compiler, charger, en ligne)
//! et vues. L'habillage de la fenêtre (ruban, panneaux ancrés, barre d'état)
//! est dans `shell`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::theme::{self, icon};
use eframe::egui::{self, Color32, Key, KeyboardShortcut, Modifiers, RichText};
use takt_compiler::ast::PouKind;
use takt_compiler::lang::{BlockType, Language};
use takt_compiler::project::{Block, Project, WatchTable};
use takt_compiler::{Compiled, Diag, Severity, compile};
use takt_vm::proto::{self, Status};
use takt_vm::runtime::Mode;

use crate::editor::{self, LineMark};
use crate::target::{self, SerialTarget, SimTarget, Target};
use crate::views;
mod shell;

use crate::config::Config;
use crate::update;
use crate::{iface_ui, ladder_ui, sfc_ui};
use shell::{LeftTab, RibbonTab, ToolboxTab};

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
    NewBlock { name: String, kind: BlockType, lang: Language },
    DeleteBlock(usize),
    Discard(Pending),
    About,
}

#[derive(Clone)]
pub enum Pending {
    New,
    Open,
    Example,
    Recent(PathBuf),
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
    pub config: Config,
    pub update: update::State,
    /// Documents ouverts en onglets.
    pub tabs: Vec<View>,
    pub ribbon_tab: RibbonTab,
    pub left_tab: LeftTab,
    pub toolbox_tab: ToolboxTab,
    /// Catégories repliées de la boîte à outils.
    toolbox_closed: std::collections::HashSet<String>,
    pub show_tree: bool,
    pub ld_sel: Option<ladder_ui::LdSel>,
    pub sfc_sel: Option<sfc_ui::SfcSel>,
    pub iface_open: bool,
    pub show_st: bool,
    /// Nœuds repliés de l'arborescence du projet.
    tree_closed: std::collections::HashSet<&'static str>,
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
            config: Config::load(),
            update: update::State::Idle,
            tabs: vec![View::Block(0)],
            ribbon_tab: RibbonTab::Projet,
            left_tab: LeftTab::Projet,
            toolbox_tab: ToolboxTab::Normal,
            toolbox_closed: Default::default(),
            show_tree: true,
            ld_sel: None,
            sfc_sel: None,
            iface_open: false,
            show_st: false,
            tree_closed: Default::default(),
        };
        app.log(format!("Takt Studio {} prêt.", update::VERSION));
        if app.config.dark {
            app.dark = true;
            theme::apply(ctx, true);
        }
        // Projet passé en argument (double-clic sur un .takt), sinon le dernier ouvert
        match path.or_else(|| app.config.existing_recent().into_iter().next()) {
            Some(p) => app.load_from(p),
            None => app.log("Projet d'exemple « Convoyeur » chargé."),
        }
        if app.config.check_updates {
            app.update = update::start_check();
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
            self.kinds =
                (self.rev, self.project.blocks.iter().map(|b| b.block_type().map(BlockType::pou_kind)).collect());
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
        self.tabs.clear();
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
            Pending::Recent(p) => self.load_from(p),
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
                self.config.add_recent(&p);
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
                self.config.add_recent(&path);
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
        let pos = self.tabs.iter().position(|t| *t == v);
        self.tabs.retain(|t| *t != v);
        if self.view == Some(v) {
            self.view = pos.and_then(|p| self.tabs.get(p.saturating_sub(1)).or(self.tabs.first()).copied());
        }
    }

    pub fn add_block(&mut self, name: &str, kind: BlockType, lang: Language) {
        self.project.blocks.push(Block::new(name, kind, lang));
        self.touch();
        self.ld_sel = None;
        self.sfc_sel = None;
        self.open(View::Block(self.project.blocks.len() - 1));
    }

    /// Numéro de bloc façon TIA : OB1, FB1, FB2, FC1…
    pub fn block_number(&mut self, i: usize) -> String {
        let kind = self.block_kind(i);
        let n = (0..=i).filter(|k| self.block_kind(*k) == kind).count();
        format!("{}{n}", kind_short(kind))
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
                    set.extend(c.slots_of_block(&b.name));
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
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing
            && ctx.input(|i| i.key_pressed(Key::Delete))
            && let Some(View::Block(i)) = self.view
            && self.project.blocks.get(i).is_some_and(|b| b.language == Language::Ladder)
            && ladder_ui::delete(&mut self.project.blocks[i], &mut self.ld_sel)
        {
            self.touch();
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

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let c = theme::colors(ui);
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let open = |s: &App, k: &str| !s.tree_closed.contains(k);
            let toggle = |s: &mut App, k: &'static str| {
                if !s.tree_closed.remove(k) {
                    s.tree_closed.insert(k);
                }
            };
            let project = self.project.name.clone();
            let (r, t) = theme::tree_item(ui, 0, Some(open(self, "projet")), icon::FOLDER, &project, false, None, None);
            if t || r.double_clicked() {
                toggle(self, "projet");
            }
            if !open(self, "projet") {
                return;
            }
            let board = views::board_name(&self.project.device.board);
            let dev = format!("{} [{}]", self.project.device.name, short_board(board));
            let (r, t) = theme::tree_item(ui, 1, Some(open(self, "api")), icon::CPU, &dev, false, None, None);
            if t || r.double_clicked() {
                toggle(self, "api");
            }
            if !open(self, "api") {
                return;
            }
            if theme::tree_item(
                ui,
                2,
                None,
                icon::SETTINGS,
                "Configuration de l'appareil",
                self.view == Some(View::Device),
                None,
                None,
            )
            .0
            .clicked()
            {
                self.open(View::Device);
            }
            if theme::tree_item(ui, 2, None, icon::ACTIVITY, "En ligne et diagnostic", false, None, None).0.clicked() {
                self.drawer = true;
                self.bottom = BottomTab::Diagnostic;
                self.connect();
            }

            let (r, t) = theme::tree_item(
                ui,
                2,
                Some(open(self, "blocs")),
                icon::FOLDER_CLOSED,
                "Blocs de programme",
                false,
                None,
                None,
            );
            if t || r.double_clicked() {
                toggle(self, "blocs");
            }
            if open(self, "blocs") {
                if theme::tree_item(ui, 3, None, icon::PLUS, "Ajouter nouveau bloc", false, None, Some(c.accent))
                    .0
                    .clicked()
                {
                    self.modal =
                        Some(Modal::NewBlock { name: String::new(), kind: BlockType::Fb, lang: Language::Ladder });
                }
                for i in 0..self.project.blocks.len() {
                    let kind = self.block_kind(i);
                    let number = self.block_number(i);
                    let b = &self.project.blocks[i];
                    let name = b.name.clone();
                    let lang = b.language.label();
                    let has_err = self.diags.iter().any(|d| d.severity == Severity::Error && d.block == name);
                    let ic = match kind {
                        Some(PouKind::Program) => icon::CODE,
                        Some(PouKind::FunctionBlock) => icon::BOX,
                        _ => icon::FUNCTION,
                    };
                    let trailing = if has_err { ("●".to_string(), c.red) } else { (lang.to_string(), c.soft) };
                    let label = format!("{name} [{number}]");
                    let (r, _) = theme::tree_item(
                        ui,
                        3,
                        None,
                        ic,
                        &label,
                        self.view == Some(View::Block(i)),
                        Some(trailing),
                        None,
                    );
                    if r.clicked() {
                        if self.view != Some(View::Block(i)) {
                            self.ld_sel = None;
                            self.sfc_sel = None;
                        }
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
            }

            let (r, t) = theme::tree_item(
                ui,
                2,
                Some(open(self, "vars")),
                icon::FOLDER_CLOSED,
                "Variables API",
                false,
                None,
                None,
            );
            if t || r.double_clicked() {
                toggle(self, "vars");
            }
            if open(self, "vars") {
                let n = (self.project.tags.len().to_string(), c.soft);
                if theme::tree_item(
                    ui,
                    3,
                    None,
                    icon::TAG,
                    "Table de variables standard",
                    self.view == Some(View::Tags),
                    Some(n),
                    None,
                )
                .0
                .clicked()
                {
                    self.open(View::Tags);
                }
            }

            let (r, t) = theme::tree_item(
                ui,
                2,
                Some(open(self, "tables")),
                icon::FOLDER_CLOSED,
                "Tables de visualisation et de forçage",
                false,
                None,
                None,
            );
            if t || r.double_clicked() {
                toggle(self, "tables");
            }
            if open(self, "tables") {
                if theme::tree_item(ui, 3, None, icon::PLUS, "Ajouter une table", false, None, Some(c.accent))
                    .0
                    .clicked()
                {
                    self.add_watch_table();
                }
                for i in 0..self.project.watch_tables.len() {
                    let name = self.project.watch_tables[i].name.clone();
                    let n = (self.project.watch_tables[i].rows.len().to_string(), c.soft);
                    if theme::tree_item(ui, 3, None, icon::EYE, &name, self.view == Some(View::Watch(i)), Some(n), None)
                        .0
                        .clicked()
                    {
                        self.open(View::Watch(i));
                    }
                }
            }
        });
    }

    /// Bandeau « Version X disponible », comme dans Sura Novi.
    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let c = theme::colors(ui);
        let (title, detail, button) = match &self.update {
            update::State::Available(r) => {
                (format!("Version {} disponible", r.version), r.notes.lines().next().unwrap_or("").to_string(), true)
            }
            update::State::Downloading(_, r) => (format!("Téléchargement de la {}…", r.version), String::new(), false),
            update::State::Failed(e) => ("Mise à jour impossible".to_string(), e.clone(), false),
            _ => return,
        };
        egui::Frame::new().fill(c.accent_soft).corner_radius(theme::RADIUS).inner_margin(egui::Margin::same(12)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&title).font(egui::FontId::new(13.5, theme::semi())));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::ghost_icon(ui, icon::X, "Plus tard").clicked() {
                            self.update = update::State::Idle;
                        }
                    });
                });
                if !detail.is_empty() {
                    ui.label(RichText::new(detail).size(12.0).color(c.soft));
                }
                if button {
                    let b = egui::Button::new(theme::rich(
                        "Installer et redémarrer",
                        13.5,
                        theme::medium(),
                        Color32::WHITE,
                    ))
                    .fill(c.accent);
                    if ui.add(b).clicked()
                        && let update::State::Available(r) = std::mem::replace(&mut self.update, update::State::Idle)
                    {
                        if self.dirty() {
                            self.save(false);
                        }
                        self.update = update::start_download(r);
                    }
                }
            },
        );
        ui.add_space(6.0);
    }

    /// Onglet « En ligne » de la gestion de projet : cible et état de la liaison.
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

    fn central(&mut self, ui: &mut egui::Ui) {
        self.doc_tabs(ui);
        let c = theme::colors(ui);
        let page = egui::Frame::new()
            .fill(c.bg)
            .corner_radius(egui::CornerRadius { nw: 0, ne: 10, sw: 10, se: 10 })
            .inner_margin(egui::Margin::symmetric(16, 10));
        page.show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.breadcrumb(ui);
            ui.add_space(8.0);
            self.document(ui);
        });
    }

    fn document(&mut self, ui: &mut egui::Ui) {
        match self.view {
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new("Ouvrez un élément dans « Gestion de projet ».").color(theme::colors(ui).soft),
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
        let Some(block) = self.project.blocks.get(i) else {
            return;
        };
        let name = block.name.clone();
        let lang = block.language;
        let monitoring = self.monitor && self.compiled.is_some() && self.online();
        let errors: Vec<(u32, String)> = self
            .diags
            .iter()
            .filter(|d| d.block == name && d.severity == Severity::Error)
            .map(|d| (d.pos.line, d.msg.clone()))
            .collect();
        let values = self.values.clone();
        let compiled = self.compiled.clone();
        let get = |op: &str| -> Option<(u32, takt_compiler::types::Ty)> {
            let c = compiled.as_ref()?;
            let (slot, ty) = c.lookup_in_block(&name, op)?;
            Some((*values.get(&slot)?, ty))
        };
        let text_value = |n: &str| -> Option<editor::MonitorValue> {
            let (v, ty) = get(n)?;
            let state = (ty == takt_compiler::types::Ty::Bool).then_some(v != 0);
            Some((takt_compiler::types::format_value(v, &ty), state))
        };
        if monitoring {
            ui.horizontal(|ui| {
                let text = format!("{}  visualisation en cours (Ctrl+T pour quitter)", icon::EYE);
                theme::chip(ui, &text, theme::colors(ui).accent);
            });
            ui.add_space(8.0);
        }

        if lang == Language::St {
            if monitoring {
                let src = self.project.blocks[i].source.clone();
                theme::card(ui, |ui| editor::monitor_view(ui, editor_id(i), &src, &text_value, false));
                return;
            }
            let marks = self.marks(&name);
            let mut goto = self.take_goto(i);
            let source = &mut self.project.blocks[i].source;
            let changed = theme::card(ui, |ui| editor::code_editor(ui, editor_id(i), source, &marks, &mut goto, false));
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
            return;
        }

        if lang == Language::Ladder && !monitoring {
            // Barre des favoris, comme dans TIA
            let c = theme::colors(ui);
            egui::Frame::new().fill(c.group).corner_radius(10.0).inner_margin(egui::Margin::symmetric(8, 4)).show(
                ui,
                |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for t in ladder_ui::FAVORITES {
                            if ladder_ui::tool_button(ui, t, false) {
                                ladder_ui::apply(&mut self.project.blocks[i], &mut self.ld_sel, t);
                                self.touch();
                            }
                        }
                        ui.separator();
                        if theme::ghost_icon(ui, icon::TRASH, "Supprimer l'élément sélectionné (Suppr)").clicked()
                            && ladder_ui::delete(&mut self.project.blocks[i], &mut self.ld_sel)
                        {
                            self.touch();
                        }
                    });
                },
            );
            ui.add_space(8.0);
        }

        let mut goto = self.take_goto(i);
        let marks = self.marks(&name);
        let mut changed = false;
        let scroll = egui::ScrollArea::vertical().id_salt(("block", i)).auto_shrink([false; 2]);
        scroll.show(ui, |ui| {
            let block = &mut self.project.blocks[i];
            if !monitoring {
                changed |= iface_ui::interface(ui, block, &mut self.iface_open);
                ui.add_space(10.0);
            }
            match lang {
                Language::Ladder => {
                    let mon = ladder_ui::Monitor { get: &get };
                    changed |= ladder_ui::editor(ui, block, &mut self.ld_sel, monitoring.then_some(&mon), &errors);
                }
                Language::Sfc => {
                    let mon = sfc_ui::SfcMonitor {
                        active: get(takt_compiler::lang::sfc::STEP_VAR).map(|(v, _)| v as u16),
                        time: text_value(takt_compiler::lang::sfc::STEP_TIME_VAR).map(|(t, _)| t),
                    };
                    changed |= sfc_ui::editor(ui, block, &mut self.sfc_sel, monitoring.then_some(&mon), &errors);
                }
                _ => {
                    if monitoring {
                        let src = block.source.clone();
                        theme::card(ui, |ui| editor::monitor_view(ui, editor_id(i), &src, &text_value, true));
                    } else {
                        changed |= theme::card(ui, |ui| {
                            editor::code_editor(ui, editor_id(i), &mut block.source, &marks, &mut goto, true)
                        });
                    }
                }
            }
            ui.add_space(12.0);
            let c = theme::colors(ui);
            let label = theme::rich(
                &format!("{}  ST généré", if self.show_st { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT }),
                13.5,
                theme::medium(),
                c.soft,
            );
            if ui
                .add(egui::Button::new(label).frame(false))
                .on_hover_text("Le code Structured Text réellement compilé")
                .clicked()
            {
                self.show_st = !self.show_st;
            }
            if self.show_st {
                let st = self.project.blocks[i].to_st().source();
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let font = egui::FontId::monospace(12.5);
                    let pal = editor::palette(ui.visuals().dark_mode);
                    ui.add(egui::Label::new(editor::highlight(&st, &font, &pal, false)).extend());
                });
            }
            ui.add_space(24.0);
        });
        if changed {
            self.touch();
        }
    }

    fn marks(&self, block: &str) -> Vec<LineMark> {
        self.diags
            .iter()
            .filter(|d| d.block == block && d.pos.line > 0)
            .map(|d| LineMark { line: d.pos.line, severity: d.severity, msg: d.msg.clone() })
            .collect()
    }

    fn take_goto(&mut self, i: usize) -> Option<u32> {
        match self.goto {
            Some((b, line)) if b == i => {
                self.goto = None;
                Some(line)
            }
            _ => None,
        }
    }

    fn modals(&mut self, ctx: &egui::Context) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        if let Modal::NewBlock { mut name, mut kind, mut lang } = modal {
            let r = egui::Modal::new(egui::Id::new("newblock")).show(ctx, |ui| {
                let c = theme::colors(ui);
                ui.set_width(460.0);
                ui.heading("Ajouter nouveau bloc");
                ui.add_space(10.0);
                ui.label(RichText::new("Nom").color(c.soft));
                ui.add_sized([440.0, 28.0], egui::TextEdit::singleline(&mut name).hint_text("ex. Pompe_Remplissage"))
                    .request_focus();
                ui.add_space(10.0);
                ui.label(RichText::new("Type").color(c.soft));
                theme::segmented(
                    ui,
                    &mut kind,
                    &[
                        (BlockType::Ob, "Bloc d'organisation (OB)"),
                        (BlockType::Fb, "Bloc fonctionnel (FB)"),
                        (BlockType::Fc, "Fonction (FC)"),
                    ],
                );
                if kind == BlockType::Fc && lang == Language::Sfc {
                    lang = Language::Ladder;
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Langage").color(c.soft));
                for l in Language::ALL {
                    let enabled = !(l == Language::Sfc && kind == BlockType::Fc);
                    let selected = lang == l;
                    let text = theme::rich(
                        &format!("{}   {}", l.label(), l.description()),
                        14.0,
                        if selected { theme::medium() } else { egui::FontFamily::Proportional },
                        if selected { c.accent } else { c.text },
                    );
                    let b = egui::Button::new(text)
                        .fill(if selected { c.accent_soft } else { c.group })
                        .min_size(egui::vec2(440.0, 32.0));
                    let r = ui.add_enabled(enabled, b);
                    let r = if enabled {
                        r
                    } else {
                        r.on_disabled_hover_text("GRAPH a besoin de mémoire : choisissez un OB ou un FB")
                    };
                    if r.clicked() {
                        lang = l;
                    }
                }
                ui.add_space(12.0);
                let valid = !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !name.starts_with(|c: char| c.is_ascii_digit());
                let mut action = 0;
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            valid,
                            egui::Button::new(theme::rich("OK", 14.0, theme::medium(), Color32::WHITE))
                                .fill(c.accent)
                                .min_size(egui::vec2(90.0, 32.0)),
                        )
                        .clicked()
                        || (valid && ui.input(|i| i.key_pressed(Key::Enter)))
                    {
                        action = 1;
                    }
                    if ui.add(egui::Button::new("Annuler").min_size(egui::vec2(90.0, 32.0))).clicked() {
                        action = 2;
                    }
                });
                action
            });
            match r.inner {
                1 => self.add_block(&name, kind, lang),
                2 => {}
                _ if r.should_close() => {}
                _ => self.modal = Some(Modal::NewBlock { name, kind, lang }),
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
                    ui.horizontal(|ui| {
                        let logo = self.logo.get_or_insert_with(|| theme::logo_texture(ui.ctx())).clone();
                        ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(56.0, 56.0)).corner_radius(12.0));
                        ui.vertical(|ui| {
                            ui.heading("Takt Studio");
                            ui.label(RichText::new(format!("Version {}", update::VERSION)).color(theme::colors(ui).soft));
                        });
                    });
                    ui.add_space(8.0);
                    ui.label("Atelier de programmation d'automates façon TIA Portal, pour RP2350, au temps de cycle garanti.");
                    ui.label("CONT, SCL, GRAPH et BASIC. Simulateur intégré.");
                    ui.hyperlink_to("github.com/ideaton/takt", "https://github.com/ideaton/takt");
                    ui.label(RichText::new("Licence GPL-3.0 · police Inter (OFL) · icônes Lucide (ISC)").size(12.0).color(theme::colors(ui).soft));
                    ui.add_space(6.0);
                    let mut auto = self.config.check_updates;
                    if ui.checkbox(&mut auto, "Rechercher les mises à jour au lancement").changed() {
                        self.config.check_updates = auto;
                        self.config.save();
                    }
                    ui.add_space(6.0);
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
        if let Some(setup) = update::poll(&mut self.update) {
            match update::install(&setup) {
                Ok(()) => {
                    self.log("Installation de la mise à jour : Takt Studio va redémarrer.");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(e) => self.update = update::State::Failed(format!("lancement de l'installateur : {e}")),
            }
        }
        if matches!(self.update, update::State::Checking(_) | update::State::Downloading(..)) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        let title = format!(
            "Takt Studio — {}{}",
            self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| self.project.name.clone()),
            if self.dirty() { " *" } else { "" }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let c = if self.dark { theme::DARK } else { theme::LIGHT };
        let dock = egui::Frame::new().fill(c.bg).corner_radius(10.0).inner_margin(egui::Margin::same(4));
        egui::TopBottomPanel::top("ribbon")
            .frame(egui::Frame::new().fill(c.bar).inner_margin(egui::Margin { left: 6, right: 6, top: 6, bottom: 2 }))
            .show_separator_line(false)
            .show(ctx, |ui| self.ribbon(ui));
        egui::TopBottomPanel::bottom("statusbar")
            .exact_height(26.0)
            .frame(egui::Frame::new().fill(c.bar).inner_margin(egui::Margin::symmetric(6, 3)))
            .show_separator_line(false)
            .show(ctx, |ui| self.status_bar(ui));
        if self.show_tree {
            egui::SidePanel::left("tree")
                .resizable(true)
                .default_width(290.0)
                .width_range(220.0..=460.0)
                .frame(egui::Frame::new().fill(c.bar).inner_margin(egui::Margin {
                    left: 6,
                    right: 0,
                    top: 2,
                    bottom: 6,
                }))
                .show_separator_line(false)
                .show(ctx, |ui| {
                    dock.show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.left_dock(ui);
                    });
                });
        }
        if self.show_instructions {
            egui::SidePanel::right("toolbox")
                .resizable(true)
                .default_width(270.0)
                .width_range(200.0..=420.0)
                .frame(egui::Frame::new().fill(c.bar).inner_margin(egui::Margin {
                    left: 0,
                    right: 6,
                    top: 2,
                    bottom: 6,
                }))
                .show_separator_line(false)
                .show(ctx, |ui| {
                    dock.show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.toolbox(ui);
                    });
                });
        }
        if self.drawer {
            egui::TopBottomPanel::bottom("drawer")
                .resizable(true)
                .default_height(220.0)
                .min_height(110.0)
                .frame(egui::Frame::new().fill(c.bar).inner_margin(egui::Margin {
                    left: 6,
                    right: 6,
                    top: 0,
                    bottom: 6,
                }))
                .show_separator_line(false)
                .show(ctx, |ui| {
                    dock.show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.bottom_dock(ui);
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(c.bar).inner_margin(egui::Margin { left: 6, right: 6, top: 2, bottom: 6 }))
            .show(ctx, |ui| self.central(ui));
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
        let step = |h: &mut Harness<Option<App>>, f: &dyn Fn(&mut App)| f(h.state_mut().as_mut().unwrap());
        shot(&mut h, "1-editeur-scl");
        step(&mut h, &|a| a.open(View::Block(1)));
        shot(&mut h, "2-cont");
        step(&mut h, &|a| {
            a.open(View::Block(3));
            a.sfc_sel = Some(sfc_ui::SfcSel::Step(1));
        });
        shot(&mut h, "3-graph");
        step(&mut h, &|a| a.open(View::Block(2)));
        shot(&mut h, "4-basic");
        step(&mut h, &|a| {
            a.modal = Some(Modal::NewBlock { name: "Pompe".into(), kind: BlockType::Fb, lang: Language::Ladder })
        });
        shot(&mut h, "5-nouveau-bloc");
        step(&mut h, &|a| {
            a.modal = None;
            a.compile();
            a.open(View::Device);
        });
        shot(&mut h, "6-appareil");
        step(&mut h, &|a| {
            a.download();
            a.modal = None;
            a.run();
            a.open(View::Block(1));
            a.toggle_monitor();
            a.ribbon_tab = RibbonTab::Automate;
        });
        std::thread::sleep(Duration::from_millis(300));
        shot(&mut h, "7-cont-visualisation");
        step(&mut h, &|a| a.open(View::Block(0)));
        std::thread::sleep(Duration::from_millis(200));
        shot(&mut h, "8-scl-visualisation");
        step(&mut h, &|a| a.open(View::Block(3)));
        std::thread::sleep(Duration::from_millis(200));
        shot(&mut h, "9-graph-visualisation");
        step(&mut h, &|a| {
            a.monitor = false;
            a.open(View::Watch(0));
        });
        std::thread::sleep(Duration::from_millis(200));
        shot(&mut h, "10-table");
        step(&mut h, &|a| {
            a.disconnect();
            a.dark = true;
            a.open(View::Block(1));
        });
        let ctx = h.ctx.clone();
        theme::apply(&ctx, true);
        shot(&mut h, "11-sombre");
    }
}
