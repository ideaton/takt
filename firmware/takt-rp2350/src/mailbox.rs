//! Boîtes aux lettres entre les deux coeurs : une case chacune, un seul
//! producteur et un seul consommateur, sans verrou.
//!
//! - [`REQUEST`] : coeur 0 (USB) → coeur 1 (automate) ;
//! - [`RESPONSE`] : coeur 1 → coeur 0.
//!
//! Le drapeau `full` donne la case à l'un ou à l'autre : le producteur n'écrit
//! que si elle est vide, le consommateur ne lit que si elle est pleine, et
//! chacun la rend avec une écriture `Release` lue en `Acquire` de l'autre
//! côté. Aucun des deux coeurs n'attend jamais l'autre : le coeur 1 regarde la
//! case une fois par cycle et passe son chemin si elle n'est pas prête.
//!
//! Les méthodes sont `inline(always)` : chaque coeur exécute sa copie depuis
//! sa propre mémoire (flash pour le coeur 0, SRAM4-7 pour le coeur 1). Les
//! cases elles-mêmes sont en SRAM0-3 (`.bss` ordinaire).

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use takt_vm::proto::MAX_PAYLOAD;

pub struct Slot {
    full: AtomicBool,
    cmd: UnsafeCell<u8>,
    len: UnsafeCell<usize>,
    data: UnsafeCell<[u8; MAX_PAYLOAD]>,
}

// SAFETY: l'accès aux champs non atomiques est réservé, à tout instant, au
// seul coeur désigné par `full` (producteur si faux, consommateur si vrai).
unsafe impl Sync for Slot {}

impl Slot {
    pub const fn new() -> Self {
        Slot {
            full: AtomicBool::new(false),
            cmd: UnsafeCell::new(0),
            len: UnsafeCell::new(0),
            data: UnsafeCell::new([0; MAX_PAYLOAD]),
        }
    }

    /// Côté consommateur : un message attend.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.full.load(Ordering::Acquire)
    }

    /// Côté producteur : écrit un message avec `f(tampon) -> (cmd, len)` si la
    /// case est libre. Renvoie `None` sans rien faire sinon.
    #[inline(always)]
    pub fn produce(&self, f: impl FnOnce(&mut [u8; MAX_PAYLOAD]) -> (u8, usize)) -> Option<(u8, usize)> {
        if self.full.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: case vide, donc réservée au producteur (voir `Sync`).
        let (cmd, len) = unsafe { f(&mut *self.data.get()) };
        let len = len.min(MAX_PAYLOAD);
        unsafe {
            *self.cmd.get() = cmd;
            *self.len.get() = len;
        }
        self.full.store(true, Ordering::Release);
        Some((cmd, len))
    }

    /// Côté consommateur : lit le message en place avec `f(cmd, données)`,
    /// puis libère la case. Renvoie `None` si elle est vide.
    #[inline(always)]
    pub fn consume<R>(&self, f: impl FnOnce(u8, &[u8]) -> R) -> Option<R> {
        if !self.full.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: case pleine, donc réservée au consommateur (voir `Sync`).
        let r = unsafe {
            let data = &*self.data.get();
            f(*self.cmd.get(), &data[..*self.len.get()])
        };
        self.full.store(false, Ordering::Release);
        Some(r)
    }
}

pub static REQUEST: Slot = Slot::new();
pub static RESPONSE: Slot = Slot::new();
