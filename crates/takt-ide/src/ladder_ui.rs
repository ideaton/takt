//! Éditeur CONT (ladder) façon TIA Portal : réseaux, barre d'alimentation,
//! contacts, branches, boîtes et bobines. En visualisation, le courant qui
//! passe est dessiné en vert.

use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use takt_compiler::lang::ladder::{CmpOp, Coil, CoilKind, ContactKind, CounterKind, Elem, Network, TimerKind};
use takt_compiler::lang::{IfaceVar, Section};
use takt_compiler::project::Block;
use takt_compiler::types::{Ty, format_value, parse_value};

use crate::theme::{self, Colors, icon};

/// Hauteur du fil par rapport au haut d'un élément : l'opérande est au-dessus.
const WIRE_Y: f32 = 34.0;
const W_CONTACT: f32 = 112.0;
const H_CONTACT: f32 = 58.0;
const W_BOX: f32 = 156.0;

#[derive(Clone, Debug, PartialEq)]
pub enum LdSel {
    /// Élément du réseau ; `path` = index dans la série, puis branche et index pour les branches.
    Elem {
        net: usize,
        path: Vec<usize>,
    },
    Coil {
        net: usize,
        idx: usize,
    },
    Network(usize),
}

impl LdSel {
    pub fn net(&self) -> usize {
        match self {
            LdSel::Elem { net, .. } | LdSel::Coil { net, .. } | LdSel::Network(net) => *net,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    Contact(ContactKind),
    Compare,
    Timer(TimerKind),
    Counter(CounterKind),
    Coil(CoilKind),
    Branch,
    Network,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Contact(ContactKind::No) => "Contact à fermeture",
            Tool::Contact(ContactKind::Nc) => "Contact à ouverture",
            Tool::Contact(ContactKind::Pos) => "Front montant (P)",
            Tool::Contact(ContactKind::Neg) => "Front descendant (N)",
            Tool::Compare => "Comparaison (CMP)",
            Tool::Timer(TimerKind::Ton) => "TON  retard à la montée",
            Tool::Timer(TimerKind::Tof) => "TOF  retard à la retombée",
            Tool::Timer(TimerKind::Tp) => "TP  impulsion",
            Tool::Counter(CounterKind::Ctu) => "CTU  compteur",
            Tool::Counter(CounterKind::Ctd) => "CTD  décompteur",
            Tool::Coil(CoilKind::Assign) => "Bobine",
            Tool::Coil(CoilKind::Negate) => "Bobine inversée",
            Tool::Coil(CoilKind::Set) => "Mise à 1 (S)",
            Tool::Coil(CoilKind::Reset) => "Mise à 0 (R)",
            Tool::Coil(CoilKind::Move) => "MOVE  copier une valeur",
            Tool::Branch => "Branche parallèle (OU)",
            Tool::Network => "Nouveau réseau",
        }
    }
}

/// Lecture des valeurs en ligne : `get("#x")` → valeur brute et type.
pub struct Monitor<'a> {
    pub get: &'a dyn Fn(&str) -> Option<(u32, Ty)>,
}

impl Monitor<'_> {
    fn bool(&self, op: &str) -> Option<bool> {
        let t = op.trim();
        if t.eq_ignore_ascii_case("TRUE") {
            return Some(true);
        }
        if t.eq_ignore_ascii_case("FALSE") {
            return Some(false);
        }
        (self.get)(t).map(|(v, _)| v != 0)
    }

    fn num(&self, op: &str) -> Option<f64> {
        let t = op.trim();
        if let Some((v, ty)) = (self.get)(t) {
            return Some(match ty {
                Ty::Real => f32::from_bits(v) as f64,
                _ => v as i32 as f64,
            });
        }
        parse_value(t, &Ty::DInt)
            .map(|v| v as i32 as f64)
            .or_else(|| parse_value(t, &Ty::Real).map(|v| f32::from_bits(v) as f64))
            .or_else(|| parse_value(t, &Ty::Time).map(|v| v as i32 as f64))
    }

    fn text(&self, op: &str) -> Option<String> {
        (self.get)(op.trim()).map(|(v, ty)| format_value(v, &ty))
    }

    fn compare(&self, a: &str, op: CmpOp, b: &str) -> Option<bool> {
        let (a, b) = (self.num(a)?, self.num(b)?);
        Some(match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        })
    }
}

struct Ctx<'a, 'm> {
    sel: &'a mut Option<LdSel>,
    net: usize,
    mon: Option<&'a Monitor<'m>>,
    c: Colors,
    changed: bool,
}

impl Ctx<'_, '_> {
    fn wire(&self, powered: Option<bool>) -> Stroke {
        match (self.mon.is_some(), powered) {
            (true, Some(true)) => Stroke::new(2.5_f32, self.c.green),
            (true, _) => Stroke::new(1.5_f32, Color32::from_rgb(90, 140, 220)),
            _ => Stroke::new(1.5_f32, self.c.text),
        }
    }

    fn selected(&self, s: &LdSel) -> bool {
        self.sel.as_ref() == Some(s)
    }

    /// Champ d'opérande centré au-dessus d'un élément.
    fn operand(&mut self, ui: &mut Ui, rect: Rect, text: &mut String, hint: &str, select: LdSel) {
        let r = ui.put(
            rect,
            egui::TextEdit::singleline(text)
                .hint_text(hint)
                .horizontal_align(egui::Align::Center)
                .font(FontId::proportional(12.5))
                .margin(egui::Margin::symmetric(4, 2)),
        );
        if r.changed() {
            self.changed = true;
        }
        if r.gained_focus() {
            *self.sel = Some(select);
        }
    }
}

fn line(ui: &Ui, a: Pos2, b: Pos2, s: Stroke) {
    ui.painter().line_segment([a, b], s);
}

/// Arc de cercle (pour les bobines).
fn arc(ui: &Ui, center: Pos2, r: f32, from: f32, to: f32, s: Stroke) {
    let pts: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = (from + (to - from) * k as f32 / 12.0).to_radians();
            pos2(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    ui.painter().add(egui::Shape::line(pts, s));
}

fn select_bg(ui: &Ui, rect: Rect, c: &Colors, selected: bool, hovered: bool) {
    if selected {
        ui.painter().rect_filled(rect.shrink(2.0), 8.0, c.accent_soft);
    } else if hovered {
        ui.painter().rect_filled(rect.shrink(2.0), 8.0, c.hover);
    }
}

fn series(ui: &mut Ui, ctx: &mut Ctx, elems: &mut [Elem], prefix: &[usize], power: Option<bool>) -> Option<bool> {
    let mut p = power;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for (i, e) in elems.iter_mut().enumerate() {
            let mut path = prefix.to_vec();
            path.push(i);
            p = element(ui, ctx, e, path, p);
        }
    });
    p
}

fn element(ui: &mut Ui, ctx: &mut Ctx, e: &mut Elem, path: Vec<usize>, power: Option<bool>) -> Option<bool> {
    let me = LdSel::Elem { net: ctx.net, path: path.clone() };
    let c = ctx.c;
    match e {
        Elem::Contact { kind, operand } => {
            let (rect, resp) = ui.allocate_exact_size(vec2(W_CONTACT, H_CONTACT), Sense::click());
            select_bg(ui, rect, &c, ctx.selected(&me), resp.hovered());
            let y = rect.top() + WIRE_Y;
            let cx = rect.center().x;
            let closed = ctx.mon.and_then(|m| match kind {
                ContactKind::No => m.bool(operand),
                ContactKind::Nc => m.bool(operand).map(|v| !v),
                _ => Some(false),
            });
            let out = power.zip(closed).map(|(p, k)| p && k);
            line(ui, pos2(rect.left(), y), pos2(cx - 8.0, y), ctx.wire(power));
            let bars = ctx.wire(out);
            if out == Some(true) {
                ui.painter().rect_filled(
                    Rect::from_min_max(pos2(cx - 8.0, y - 10.0), pos2(cx + 8.0, y + 10.0)),
                    0.0,
                    c.green.gamma_multiply(0.25),
                );
            }
            line(ui, pos2(cx - 8.0, y - 11.0), pos2(cx - 8.0, y + 11.0), bars);
            line(ui, pos2(cx + 8.0, y - 11.0), pos2(cx + 8.0, y + 11.0), bars);
            match kind {
                ContactKind::Nc => line(ui, pos2(cx - 6.0, y + 9.0), pos2(cx + 6.0, y - 9.0), bars),
                ContactKind::Pos | ContactKind::Neg => {
                    let t = if *kind == ContactKind::Pos { "P" } else { "N" };
                    ui.painter().text(
                        pos2(cx, y),
                        egui::Align2::CENTER_CENTER,
                        t,
                        FontId::new(11.0, theme::semi()),
                        c.text,
                    );
                }
                ContactKind::No => {}
            }
            line(ui, pos2(cx + 8.0, y), pos2(rect.right(), y), ctx.wire(out));
            ctx.operand(
                ui,
                Rect::from_center_size(pos2(cx, rect.top() + 12.0), vec2(W_CONTACT - 14.0, 20.0)),
                operand,
                "<??.?>",
                me.clone(),
            );
            if resp.clicked() {
                *ctx.sel = Some(me);
            }
            out
        }
        Elem::Compare { op, a, b } => {
            let (rect, resp) = ui.allocate_exact_size(vec2(W_CONTACT, 86.0), Sense::click());
            select_bg(ui, rect, &c, ctx.selected(&me), resp.hovered());
            let y = rect.top() + WIRE_Y;
            let cx = rect.center().x;
            let res = ctx.mon.and_then(|m| m.compare(a, *op, b));
            let out = power.zip(res).map(|(p, k)| p && k);
            let boxr = Rect::from_center_size(pos2(cx, y + 6.0), vec2(56.0, 34.0));
            line(ui, pos2(rect.left(), y), pos2(boxr.left(), y), ctx.wire(power));
            line(ui, pos2(boxr.right(), y), pos2(rect.right(), y), ctx.wire(out));
            ui.painter().rect(boxr, 4.0, c.bg, Stroke::new(1.2_f32, c.text), egui::StrokeKind::Inside);
            ui.painter().text(
                pos2(cx, boxr.top() + 9.0),
                egui::Align2::CENTER_CENTER,
                "CMP",
                FontId::new(10.0, theme::semi()),
                c.soft,
            );
            let opr = Rect::from_center_size(pos2(cx, boxr.top() + 23.0), vec2(40.0, 16.0));
            let r =
                ui.put(opr, egui::Button::new(RichText::new(op.label()).font(FontId::monospace(12.5))).frame(false));
            if r.clicked() {
                let i = CmpOp::ALL.iter().position(|o| o == op).unwrap_or(0);
                *op = CmpOp::ALL[(i + 1) % CmpOp::ALL.len()];
                ctx.changed = true;
                *ctx.sel = Some(me.clone());
            }
            r.on_hover_text("Cliquer pour changer la comparaison");
            ctx.operand(
                ui,
                Rect::from_center_size(pos2(cx, rect.top() + 12.0), vec2(W_CONTACT - 14.0, 20.0)),
                a,
                "<??.?>",
                me.clone(),
            );
            ctx.operand(
                ui,
                Rect::from_center_size(pos2(cx, rect.bottom() - 12.0), vec2(W_CONTACT - 14.0, 20.0)),
                b,
                "<??.?>",
                me.clone(),
            );
            if resp.clicked() {
                *ctx.sel = Some(me);
            }
            out
        }
        Elem::Timer { kind, instance, pt } => {
            let q = ctx.mon.and_then(|m| m.bool(&format!("{}.Q", instance.trim())));
            let et = ctx.mon.and_then(|m| m.text(&format!("{}.ET", instance.trim())));
            let title = kind.name();
            let mut fields = vec![("PT", pt as &mut String)];
            let (out, clicked_title) =
                fb_box(ui, ctx, me, instance, title, "IN", "Q", &mut fields, power, q, et.map(|t| ("ET", t)));
            if clicked_title {
                *kind = match kind {
                    TimerKind::Ton => TimerKind::Tof,
                    TimerKind::Tof => TimerKind::Tp,
                    TimerKind::Tp => TimerKind::Ton,
                };
                ctx.changed = true;
            }
            out
        }
        Elem::Counter { kind, instance, reset, pv } => {
            let q = ctx.mon.and_then(|m| m.bool(&format!("{}.Q", instance.trim())));
            let cv = ctx.mon.and_then(|m| m.text(&format!("{}.CV", instance.trim())));
            let (inp, rst) = if *kind == CounterKind::Ctu { ("CU", "R") } else { ("CD", "LD") };
            let mut fields = vec![(rst, reset as &mut String), ("PV", pv as &mut String)];
            let (out, clicked_title) =
                fb_box(ui, ctx, me, instance, kind.name(), inp, "Q", &mut fields, power, q, cv.map(|t| ("CV", t)));
            if clicked_title {
                *kind = if *kind == CounterKind::Ctu { CounterKind::Ctd } else { CounterKind::Ctu };
                ctx.changed = true;
            }
            out
        }
        Elem::Parallel { branches } => {
            let mut outs = Vec::new();
            let mut rows = Vec::new();
            let group = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                    for (b, br) in branches.iter_mut().enumerate() {
                        let mut prefix = path.clone();
                        prefix.push(b);
                        let r = ui
                            .horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                                ui.add_space(12.0);
                                outs.push(series(ui, ctx, br, &prefix, power));
                            })
                            .response
                            .rect;
                        rows.push(r);
                    }
                })
                .response
                .rect;
            let (tail, _) = ui.allocate_exact_size(vec2(12.0, group.height()), Sense::hover());
            let out = if ctx.mon.is_some() && power.is_some() && outs.iter().all(|o| o.is_some()) {
                Some(outs.contains(&Some(true)))
            } else {
                None
            };
            let x_left = group.left() + 4.0;
            let x_right = tail.right() - 4.0;
            let (y0, y1) = (rows[0].top() + WIRE_Y, rows.last().unwrap().top() + WIRE_Y);
            line(ui, pos2(group.left(), y0), pos2(x_left, y0), ctx.wire(power));
            line(ui, pos2(x_left, y0), pos2(x_left, y1), ctx.wire(power));
            for (k, r) in rows.iter().enumerate() {
                let y = r.top() + WIRE_Y;
                line(ui, pos2(x_left, y), pos2(r.left() + 12.0, y), ctx.wire(power));
                line(ui, pos2(r.right(), y), pos2(x_right, y), ctx.wire(outs[k]));
            }
            line(ui, pos2(x_right, y0), pos2(x_right, y1), ctx.wire(out));
            line(ui, pos2(x_right, y0), pos2(tail.right(), y0), ctx.wire(out));
            // Poignée de sélection de la branche : la barre de gauche
            let handle = Rect::from_min_max(pos2(group.left(), y0 - 6.0), pos2(group.left() + 10.0, y1 + 6.0));
            let resp = ui.interact(handle, ui.id().with(("par", &path)), Sense::click());
            if ctx.selected(&me) || resp.hovered() {
                ui.painter().rect_filled(handle, 3.0, c.accent.gamma_multiply(0.35));
            }
            if resp.on_hover_text("Branche : cliquer pour la sélectionner").clicked() {
                *ctx.sel = Some(me);
            }
            out
        }
    }
}

/// Boîte de bloc standard (TON, CTU…) avec instance au-dessus et paramètres.
#[allow(clippy::too_many_arguments)]
fn fb_box(
    ui: &mut Ui,
    ctx: &mut Ctx,
    me: LdSel,
    instance: &mut String,
    title: &str,
    input: &str,
    output: &str,
    fields: &mut [(&str, &mut String)],
    power: Option<bool>,
    q: Option<bool>,
    value: Option<(&str, String)>,
) -> (Option<bool>, bool) {
    let c = ctx.c;
    let h = 52.0 + 24.0 * fields.len() as f32 + 18.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(W_BOX, h), Sense::click());
    select_bg(ui, rect, &c, ctx.selected(&me), resp.hovered());
    let y = rect.top() + WIRE_Y;
    let boxr =
        Rect::from_min_max(pos2(rect.left() + 18.0, rect.top() + 24.0), pos2(rect.right() - 18.0, rect.bottom() - 6.0));
    let out = if ctx.mon.is_some() { q } else { None };
    line(ui, pos2(rect.left(), y), pos2(boxr.left(), y), ctx.wire(power));
    line(ui, pos2(boxr.right(), y), pos2(rect.right(), y), ctx.wire(out));
    ui.painter().rect(boxr, 5.0, c.bg, Stroke::new(1.2_f32, c.text), egui::StrokeKind::Inside);
    let tr = Rect::from_center_size(pos2(boxr.center().x, boxr.top() + 2.0), vec2(56.0, 16.0));
    let t = ui.put(
        tr,
        egui::Button::new(RichText::new(title).font(FontId::new(12.5, theme::semi())).color(c.accent)).frame(false),
    );
    let clicked_title = t.clicked();
    t.on_hover_text("Cliquer pour changer de type");
    let small = FontId::proportional(11.0);
    ui.painter().text(pos2(boxr.left() + 5.0, y), egui::Align2::LEFT_CENTER, input, small.clone(), c.soft);
    ui.painter().text(pos2(boxr.right() - 5.0, y), egui::Align2::RIGHT_CENTER, output, small.clone(), c.soft);
    for (k, (label, text)) in fields.iter_mut().enumerate() {
        let fy = y + 24.0 * (k as f32 + 1.0);
        ui.painter().text(pos2(boxr.left() + 5.0, fy), egui::Align2::LEFT_CENTER, *label, small.clone(), c.soft);
        let fr = Rect::from_min_max(pos2(boxr.left() + 26.0, fy - 10.0), pos2(boxr.right() - 6.0, fy + 10.0));
        ctx.operand(ui, fr, text, "<??>", me.clone());
    }
    if let Some((label, v)) = value {
        ui.painter().text(
            pos2(boxr.right() - 5.0, boxr.bottom() - 9.0),
            egui::Align2::RIGHT_CENTER,
            format!("{label} {v}"),
            FontId::monospace(10.5),
            c.accent,
        );
    }
    ctx.operand(
        ui,
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 11.0), vec2(W_BOX - 20.0, 20.0)),
        instance,
        "instance",
        me.clone(),
    );
    if resp.clicked() || clicked_title {
        *ctx.sel = Some(me);
    }
    (out, clicked_title)
}

fn coil(ui: &mut Ui, ctx: &mut Ctx, coil: &mut Coil, idx: usize, power: Option<bool>) -> f32 {
    let me = LdSel::Coil { net: ctx.net, idx };
    let c = ctx.c;
    if coil.kind == CoilKind::Move {
        let mut fields = vec![("IN", &mut coil.value)];
        let (rect_top, _) = (ui.cursor().top(), ());
        let (_, _) = fb_box(ui, ctx, me, &mut coil.operand, "MOVE", "EN", "OUT1", &mut fields, power, power, None);
        return rect_top;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(W_CONTACT, H_CONTACT), Sense::click());
    select_bg(ui, rect, &c, ctx.selected(&me), resp.hovered());
    let y = rect.top() + WIRE_Y;
    let cx = rect.center().x;
    let s = ctx.wire(power);
    line(ui, pos2(rect.left(), y), pos2(cx - 12.0, y), s);
    arc(ui, pos2(cx + 2.0, y), 13.0, 140.0, 220.0, s);
    arc(ui, pos2(cx - 2.0, y), 13.0, -40.0, 40.0, s);
    let letter = coil.kind.symbol().trim();
    if !letter.is_empty() {
        ui.painter().text(pos2(cx, y), egui::Align2::CENTER_CENTER, letter, FontId::new(11.0, theme::semi()), c.text);
    }
    ctx.operand(
        ui,
        Rect::from_center_size(pos2(cx, rect.top() + 12.0), vec2(W_CONTACT - 14.0, 20.0)),
        &mut coil.operand,
        "<??.?>",
        me.clone(),
    );
    if resp.clicked() {
        *ctx.sel = Some(me);
    }
    rect.top()
}

/// Un réseau : barre d'alimentation, série principale, bobines.
fn network_body(ui: &mut Ui, ctx: &mut Ctx, net: &mut Network) {
    let power = ctx.mon.map(|_| true);
    let resp = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(8.0);
        let out = series(ui, ctx, &mut net.rung, &[], power);
        let (gap, _) = ui.allocate_exact_size(vec2(36.0, H_CONTACT), Sense::hover());
        let y = gap.top() + WIRE_Y;
        line(ui, pos2(gap.left(), y), pos2(gap.right(), y), ctx.wire(out));
        let mut tops = Vec::new();
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            if net.coils.is_empty() {
                ui.label(RichText::new("ajoutez une bobine →").size(11.5).color(ctx.c.soft));
            }
            for (k, cl) in net.coils.iter_mut().enumerate() {
                tops.push(coil(ui, ctx, cl, k, out));
            }
        });
        if tops.len() > 1 {
            let x = gap.right();
            line(ui, pos2(x, tops[0] + WIRE_Y), pos2(x, tops.last().unwrap() + WIRE_Y), ctx.wire(out));
        }
    });
    // Barre d'alimentation à gauche
    let r = resp.response.rect;
    let rail = if ctx.mon.is_some() { ctx.c.green } else { ctx.c.text };
    ui.painter().line_segment(
        [pos2(r.left() + 2.0, r.top() + 4.0), pos2(r.left() + 2.0, r.bottom() - 4.0)],
        Stroke::new(3.0_f32, rail),
    );
}

/// Éditeur complet d'un bloc CONT. `errors` : (numéro de réseau, message).
pub fn editor(
    ui: &mut Ui,
    block: &mut Block,
    sel: &mut Option<LdSel>,
    mon: Option<&Monitor>,
    errors: &[(u32, String)],
) -> bool {
    let c = theme::colors(ui);
    let mut changed = false;
    let mut action: Option<(usize, i32)> = None;
    let n_nets = block.networks.len();
    for (n, net) in block.networks.iter_mut().enumerate() {
        let errs: Vec<&String> = errors.iter().filter(|(l, _)| *l == n as u32 + 1).map(|(_, m)| m).collect();
        let selected_net = sel.as_ref().is_some_and(|s| s.net() == n);
        let frame =
            egui::Frame::new().fill(c.group).corner_radius(theme::RADIUS).inner_margin(egui::Margin::same(14)).stroke(
                if !errs.is_empty() {
                    Stroke::new(1.5_f32, c.red)
                } else if selected_net {
                    Stroke::new(1.0_f32, c.accent.gamma_multiply(0.5))
                } else {
                    Stroke::NONE
                },
            );
        let resp = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.visuals_mut().extreme_bg_color = c.bg;
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Réseau {}", n + 1)).font(FontId::new(14.0, theme::semi())));
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut net.title)
                            .hint_text("Titre du réseau")
                            .frame(false)
                            .desired_width(360.0),
                    )
                    .changed();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::ghost_icon(ui, icon::TRASH, "Supprimer le réseau").clicked() {
                        action = Some((n, 0));
                    }
                    if n + 1 < n_nets && theme::ghost_icon(ui, icon::CHEVRON_DOWN, "Descendre").clicked() {
                        action = Some((n, 1));
                    }
                    if n > 0 && theme::ghost_icon(ui, icon::CHEVRON_UP, "Monter").clicked() {
                        action = Some((n, -1));
                    }
                });
            });
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut net.comment)
                        .hint_text("Commentaire")
                        .frame(false)
                        .text_color(c.soft)
                        .desired_width(f32::INFINITY),
                )
                .changed();
            for e in &errs {
                ui.label(theme::rich(&format!("{}  {e}", icon::ALERT), 12.5, egui::FontFamily::Proportional, c.red));
            }
            ui.add_space(4.0);
            egui::ScrollArea::horizontal().id_salt(("net", n)).show(ui, |ui| {
                let mut ctx = Ctx { sel, net: n, mon, c, changed: false };
                network_body(ui, &mut ctx, net);
                changed |= ctx.changed;
            });
        });
        if resp.response.interact(Sense::click()).clicked() {
            *sel = Some(LdSel::Network(n));
        }
        ui.add_space(10.0);
    }
    if let Some((n, a)) = action {
        match a {
            0 => {
                block.networks.remove(n);
                if block.networks.is_empty() {
                    block.networks.push(Network::default());
                }
                *sel = None;
            }
            d => {
                let m = (n as i32 + d) as usize;
                block.networks.swap(n, m);
                *sel = Some(LdSel::Network(m));
            }
        }
        changed = true;
    }
    changed
}

// ---------------------------------------------------------------------------
// Modification de la structure

fn series_at<'a>(rung: &'a mut Vec<Elem>, path: &[usize]) -> Option<&'a mut Vec<Elem>> {
    if path.len() <= 1 {
        return Some(rung);
    }
    match rung.get_mut(path[0])? {
        Elem::Parallel { branches } => series_at(branches.get_mut(path[1])?, &path[2..]),
        _ => None,
    }
}

/// Retire les branches vides et les branches parallèles à une seule voie.
fn cleanup(series: &mut Vec<Elem>) {
    let mut out = Vec::new();
    for mut e in series.drain(..) {
        if let Elem::Parallel { branches } = &mut e {
            for b in branches.iter_mut() {
                cleanup(b);
            }
            branches.retain(|b| !b.is_empty());
            match branches.len() {
                0 => continue,
                1 => {
                    out.append(&mut branches[0]);
                    continue;
                }
                _ => {}
            }
        }
        out.push(e);
    }
    *series = out;
}

fn unique_instance(block: &Block, prefix: &str) -> String {
    (1..).map(|k| format!("{prefix}_{k}")).find(|n| !block.declares(n)).unwrap()
}

/// Insère un élément de la palette selon la sélection, comme dans TIA.
pub fn apply(block: &mut Block, sel: &mut Option<LdSel>, tool: Tool) {
    if block.networks.is_empty() {
        block.networks.push(Network::default());
    }
    let net = sel.as_ref().map(|s| s.net()).unwrap_or(block.networks.len() - 1).min(block.networks.len() - 1);
    let new_elem = match tool {
        Tool::Contact(kind) => Some(Elem::Contact { kind, operand: String::new() }),
        Tool::Compare => Some(Elem::Compare { op: CmpOp::Eq, a: String::new(), b: String::new() }),
        Tool::Timer(kind) => {
            let name = unique_instance(block, "Tempo");
            block.interface.push(IfaceVar::new(Section::Static, &name, kind.name()));
            Some(Elem::Timer { kind, instance: name, pt: "T#1s".into() })
        }
        Tool::Counter(kind) => {
            let name = unique_instance(block, "Compteur");
            block.interface.push(IfaceVar::new(Section::Static, &name, kind.name()));
            Some(Elem::Counter { kind, instance: name, reset: String::new(), pv: "10".into() })
        }
        _ => None,
    };
    match tool {
        Tool::Network => {
            let at = sel.as_ref().map(|s| s.net() + 1).unwrap_or(block.networks.len());
            block.networks.insert(at, Network::default());
            *sel = Some(LdSel::Network(at));
        }
        Tool::Coil(kind) => {
            let coils = &mut block.networks[net].coils;
            let at = match sel {
                Some(LdSel::Coil { idx, .. }) => *idx + 1,
                _ => coils.len(),
            };
            coils.insert(at, Coil { kind, operand: String::new(), value: String::new() });
            *sel = Some(LdSel::Coil { net, idx: at });
        }
        Tool::Branch => {
            let rung = &mut block.networks[net].rung;
            let fresh = || vec![Elem::Contact { kind: ContactKind::No, operand: String::new() }];
            match sel.clone() {
                Some(LdSel::Elem { path, .. }) => {
                    let idx = *path.last().unwrap();
                    let Some(s) = series_at(rung, &path) else { return };
                    match &mut s[idx] {
                        Elem::Parallel { branches } => {
                            branches.push(fresh());
                            let mut p = path.clone();
                            p.extend([branches.len() - 1, 0]);
                            *sel = Some(LdSel::Elem { net, path: p });
                        }
                        other => {
                            let old = other.clone();
                            *other = Elem::Parallel { branches: vec![vec![old], fresh()] };
                            let mut p = path.clone();
                            p.extend([1, 0]);
                            *sel = Some(LdSel::Elem { net, path: p });
                        }
                    }
                }
                _ => {
                    rung.push(Elem::Parallel { branches: vec![fresh(), fresh()] });
                    *sel = Some(LdSel::Elem { net, path: vec![rung.len() - 1, 0, 0] });
                }
            }
        }
        _ => {
            let e = new_elem.unwrap();
            let rung = &mut block.networks[net].rung;
            match sel.clone() {
                Some(LdSel::Elem { path, .. }) => {
                    let idx = *path.last().unwrap();
                    if let Some(s) = series_at(rung, &path) {
                        s.insert(idx + 1, e);
                        let mut p = path.clone();
                        *p.last_mut().unwrap() = idx + 1;
                        *sel = Some(LdSel::Elem { net, path: p });
                    }
                }
                _ => {
                    rung.push(e);
                    *sel = Some(LdSel::Elem { net, path: vec![rung.len() - 1] });
                }
            }
        }
    }
}

/// Supprime l'élément sélectionné.
pub fn delete(block: &mut Block, sel: &mut Option<LdSel>) -> bool {
    match sel.take() {
        Some(LdSel::Elem { net, path }) => {
            let Some(n) = block.networks.get_mut(net) else { return false };
            if let Some(s) = series_at(&mut n.rung, &path) {
                let idx = *path.last().unwrap();
                if idx < s.len() {
                    s.remove(idx);
                }
            }
            cleanup(&mut n.rung);
            *sel = Some(LdSel::Network(net));
            true
        }
        Some(LdSel::Coil { net, idx }) => {
            if let Some(n) = block.networks.get_mut(net)
                && idx < n.coils.len()
            {
                n.coils.remove(idx);
            }
            *sel = Some(LdSel::Network(net));
            true
        }
        other => {
            *sel = other;
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Palette

/// Petit symbole de la palette (barre des favoris et carte « Instructions »).
pub fn mini_symbol(ui: &Ui, rect: Rect, tool: Tool, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    let y = rect.center().y;
    let cx = rect.center().x;
    let p = ui.painter();
    let label =
        |t: &str| p.text(rect.center(), egui::Align2::CENTER_CENTER, t, FontId::new(10.0, theme::semi()), color);
    match tool {
        Tool::Contact(k) => {
            p.line_segment([pos2(rect.left(), y), pos2(cx - 5.0, y)], s);
            p.line_segment([pos2(cx + 5.0, y), pos2(rect.right(), y)], s);
            p.line_segment([pos2(cx - 5.0, y - 7.0), pos2(cx - 5.0, y + 7.0)], s);
            p.line_segment([pos2(cx + 5.0, y - 7.0), pos2(cx + 5.0, y + 7.0)], s);
            match k {
                ContactKind::Nc => {
                    p.line_segment([pos2(cx - 4.0, y + 6.0), pos2(cx + 4.0, y - 6.0)], s);
                }
                ContactKind::Pos => {
                    label("P");
                }
                ContactKind::Neg => {
                    label("N");
                }
                ContactKind::No => {}
            }
        }
        Tool::Coil(CoilKind::Move) => {
            p.rect_stroke(rect.shrink2(vec2(4.0, 3.0)), 2.0, s, egui::StrokeKind::Inside);
            p.text(rect.center(), egui::Align2::CENTER_CENTER, "MOV", FontId::new(8.5, theme::semi()), color);
        }
        Tool::Coil(k) => {
            p.line_segment([pos2(rect.left(), y), pos2(cx - 8.0, y)], s);
            arc(ui, pos2(cx + 1.0, y), 9.0, 140.0, 220.0, s);
            arc(ui, pos2(cx - 1.0, y), 9.0, -40.0, 40.0, s);
            let t = k.symbol().trim();
            if !t.is_empty() {
                p.text(pos2(cx, y), egui::Align2::CENTER_CENTER, t, FontId::new(9.0, theme::semi()), color);
            }
        }
        Tool::Compare | Tool::Timer(_) | Tool::Counter(_) => {
            p.rect_stroke(rect.shrink2(vec2(4.0, 3.0)), 2.0, s, egui::StrokeKind::Inside);
            let t = match tool {
                Tool::Compare => "CMP",
                Tool::Timer(k) => k.name(),
                Tool::Counter(k) => k.name(),
                _ => "",
            };
            p.text(rect.center(), egui::Align2::CENTER_CENTER, t, FontId::new(8.5, theme::semi()), color);
        }
        Tool::Branch => {
            p.line_segment([pos2(rect.left(), y - 5.0), pos2(rect.right(), y - 5.0)], s);
            p.line_segment([pos2(cx - 6.0, y - 5.0), pos2(cx - 6.0, y + 6.0)], s);
            p.line_segment([pos2(cx - 6.0, y + 6.0), pos2(cx + 6.0, y + 6.0)], s);
            p.line_segment([pos2(cx + 6.0, y + 6.0), pos2(cx + 6.0, y - 5.0)], s);
        }
        Tool::Network => {
            p.text(rect.center(), egui::Align2::CENTER_CENTER, icon::PLUS, FontId::new(14.0, theme::icons()), color);
        }
    }
}

/// Bouton de palette : symbole + libellé optionnel.
pub fn tool_button(ui: &mut Ui, tool: Tool, with_label: bool) -> bool {
    let c = theme::colors(ui);
    let w = if with_label { ui.available_width() } else { 38.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 7.0, c.hover);
    }
    let sym = Rect::from_min_size(rect.min + vec2(5.0, 4.0), vec2(28.0, 20.0));
    mini_symbol(ui, sym, tool, c.text);
    if with_label {
        ui.painter().text(
            rect.left_center() + vec2(42.0, 0.0),
            egui::Align2::LEFT_CENTER,
            tool.label(),
            FontId::new(13.0, theme::medium()),
            c.text,
        );
    }
    resp.on_hover_text(tool.label()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Barre des favoris au-dessus des réseaux (comme dans TIA).
pub const FAVORITES: [Tool; 12] = [
    Tool::Contact(ContactKind::No),
    Tool::Contact(ContactKind::Nc),
    Tool::Contact(ContactKind::Pos),
    Tool::Branch,
    Tool::Coil(CoilKind::Assign),
    Tool::Coil(CoilKind::Set),
    Tool::Coil(CoilKind::Reset),
    Tool::Timer(TimerKind::Ton),
    Tool::Counter(CounterKind::Ctu),
    Tool::Compare,
    Tool::Coil(CoilKind::Move),
    Tool::Network,
];

pub const CATALOG: &[(&str, &[Tool])] = &[
    (
        "Opérations logiques sur bits",
        &[
            Tool::Contact(ContactKind::No),
            Tool::Contact(ContactKind::Nc),
            Tool::Contact(ContactKind::Pos),
            Tool::Contact(ContactKind::Neg),
            Tool::Branch,
            Tool::Coil(CoilKind::Assign),
            Tool::Coil(CoilKind::Negate),
            Tool::Coil(CoilKind::Set),
            Tool::Coil(CoilKind::Reset),
        ],
    ),
    ("Temporisations", &[Tool::Timer(TimerKind::Ton), Tool::Timer(TimerKind::Tof), Tool::Timer(TimerKind::Tp)]),
    ("Comptage", &[Tool::Counter(CounterKind::Ctu), Tool::Counter(CounterKind::Ctd)]),
    ("Comparaison et transfert", &[Tool::Compare, Tool::Coil(CoilKind::Move)]),
    ("Réseaux", &[Tool::Network]),
];
