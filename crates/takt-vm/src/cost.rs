//! Coût de chaque instruction, en cycles CPU du RP2350 (Cortex-M33 à 150 MHz,
//! code et données en SRAM, donc sans cache ni état d'attente flash).
//!
//! Ces valeurs sont des majorants estimés pour l'interpréteur : boucle de
//! dispatch + opération. Le firmware mesure le temps réel de chaque cycle avec
//! le compteur matériel et l'IDE affiche les deux côte à côte ; si une mesure
//! dépasse un jour l'estimation, c'est cette table qu'il faut corriger.

use crate::fb::FbKind;
use crate::image::{IoEntry, IoKind};
use crate::instr::Instr;

pub const CPU_HZ: u64 = 150_000_000;

/// Coût minimal d'une instruction : sert à déduire du WCET le nombre maximal
/// d'instructions exécutables, que le runtime fait respecter.
pub const MIN_COST: u32 = 6;

/// Partie fixe d'un cycle : temps système, premier cycle, boucle d'appel.
pub const SCAN_FIXED: u32 = 400;
/// Lecture ou écriture d'une voie TOR de la mémoire image.
pub const PER_IO: u32 = 40;
/// Conversion ADC d'une entrée analogique (environ 2 µs sur le RP2350, avec marge).
pub const PER_AI: u32 = 400;
/// Mise à jour du rapport cyclique PWM d'une sortie analogique.
pub const PER_AQ: u32 = 60;
/// Application d'un forçage (deux fois par cycle).
pub const PER_FORCE: u32 = 20;
/// Traitement d'une trame du protocole entre deux cycles (pire cas : lecture
/// de 60 variables ou copie d'un bloc de 240 octets).
pub const COMMAND_SERVICE: u32 = 6_000;

pub fn cost(i: &Instr) -> u32 {
    use Instr::*;
    match i {
        Nop | Halt => 6,
        PushI(_) | PushF(_) => 8,
        Load(_) | Store(_) => 10,
        LoadIdx { .. } | StoreIdx { .. } => 16,
        AddI | SubI | MulI | NegI | AbsI | MinI | MaxI => 10,
        DivI | ModI => 22,
        AddF | SubF | MulF | NegF | AbsF | MinF | MaxF => 12,
        DivF => 28,
        EqI | NeI | LtI | LeI | GtI | GeI | EqF | NeF | LtF | LeF | GtF | GeF => 12,
        And | Or | Xor | Not | ToBool | WrapI16 => 10,
        I2F | F2IRound | F2ITrunc => 14,
        Jmp(_) => 8,
        Jz(_) => 12,
        Fb { kind, .. } => fb_cost(*kind),
    }
}

pub fn fb_cost(kind: FbKind) -> u32 {
    match kind {
        FbKind::Ton | FbKind::Tof | FbKind::Tp => 60,
        FbKind::RTrig | FbKind::FTrig => 24,
        FbKind::Ctu | FbKind::Ctd => 36,
        FbKind::Sr | FbKind::Rs => 20,
    }
}

/// Partie du cycle hors programme utilisateur.
pub fn overhead(io: &[IoEntry], max_forces: u32) -> u64 {
    let io_cycles: u64 = io.iter().map(|e| io_cost(e.kind) as u64).sum();
    SCAN_FIXED as u64 + io_cycles + 2 * PER_FORCE as u64 * max_forces as u64 + COMMAND_SERVICE as u64
}

pub fn io_cost(kind: IoKind) -> u32 {
    match kind {
        IoKind::Di | IoKind::Dq => PER_IO,
        IoKind::Ai => PER_AI,
        IoKind::Aq => PER_AQ,
    }
}

/// Conversion cycles → µs, arrondie au supérieur.
pub fn cycles_to_us(c: u64) -> u64 {
    (c * 1_000_000).div_ceil(CPU_HZ)
}

/// Nombre maximal d'instructions exécutables pour un WCET donné.
pub fn max_steps(wcet_cycles: u32) -> u32 {
    wcet_cycles / MIN_COST + 1
}
