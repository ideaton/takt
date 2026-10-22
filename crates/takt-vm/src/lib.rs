//! Machine virtuelle déterministe de Takt.
//!
//! Ce crate est `no_std` et n'alloue jamais : il tourne tel quel sur le
//! RP2350 (firmware) et dans le simulateur de l'IDE. Les deux exécutent donc
//! exactement le même code.
//!
//! - [`instr`] : jeu d'instructions (machine à pile, 8 octets par instruction)
//! - [`fb`] : blocs standard IEC 61131-3 (TON, TOF, TP, R_TRIG, CTU…)
//! - [`exec`] : interpréteur d'un cycle
//! - [`cost`] : coût de chaque instruction en cycles CPU, base du calcul du WCET
//! - [`image`] : format binaire du programme téléchargé dans l'automate
//! - [`runtime`] : automate complet (RUN/STOP, mémoire image, forçage, stats)
//! - [`proto`] : protocole série entre l'IDE et l'automate
#![no_std]

pub mod cost;
pub mod exec;
pub mod fb;
pub mod image;
pub mod instr;
pub mod proto;
pub mod runtime;

pub use fb::FbKind;
pub use instr::Instr;

/// Slots mémoire réservés au système, remplis par le runtime avant chaque cycle.
pub mod sys {
    /// `FirstScan` (BOOL) : vrai pendant le premier cycle après le passage en RUN.
    pub const FIRST_SCAN: u16 = 0;
    /// `SysTime` (TIME) : temps système en ms, figé au début du cycle.
    pub const SYS_TIME: u16 = 1;
    /// Nombre de slots réservés en tête de mémoire.
    pub const SLOTS: u16 = 2;
}
