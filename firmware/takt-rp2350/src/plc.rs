//! Coeur 1 : la boucle automate, et rien d'autre.
//!
//! Chaque période :
//! 1. attente active sur le compteur 1 MHz de TIMER0 jusqu'à l'instant prévu ;
//! 2. `Runtime::cycle` (entrées → programme → sorties), GPIO en accès direct ;
//! 3. statistiques (`record_timing`), chien de garde ;
//! 4. au plus UNE trame du protocole venue du coeur 0 (travail borné).
//!
//! Le coeur 1 ne prend aucun verrou, n'a aucune interruption activée et
//! n'exécute que du code placé en SRAM4-7 (motif `*takt_rp23503plc*` de
//! `memory.x`, comme `takt_vm` et `hw`). Sa pile est en SRAM8-9.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use takt_vm::image::{IoEntry, IoKind};
use takt_vm::proto;
use takt_vm::runtime::Runtime;

use crate::hw;
use crate::mailbox::{REQUEST, RESPONSE};

/// 4096 instructions (32 Ko), 4096 slots (16 Ko), image de 40 Ko au plus.
pub type Rt = Runtime<4096, 4096, 40960>;

const _: () = assert!(core::mem::size_of::<Rt>() <= 160 * 1024, "le Runtime doit tenir largement dans SRAM4-7");

/// L'automate. Vit en SRAM4-7 (`.plc_data`, recopié depuis la flash par
/// `main`) et n'est touché que par le coeur 1.
#[unsafe(link_section = ".plc_data.runtime")]
static RT: Core1Only<Rt> = Core1Only(UnsafeCell::new(Rt::new()));

/// Donnée réservée au coeur 1.
struct Core1Only<T>(UnsafeCell<T>);

// SAFETY: seul le coeur 1 y accède (voir `run`).
unsafe impl<T> Sync for Core1Only<T> {}

/// Mode publié pour la LED du coeur 0 (`takt_vm::runtime::Mode as u8`).
pub static MODE: AtomicU8 = AtomicU8::new(0);
/// Faux quand le programme utilise la broche de la LED : le coeur 0 n'y
/// touche plus.
pub static LED_FREE: AtomicBool = AtomicBool::new(true);

/// Marge du chien de garde au-delà de deux périodes.
const WATCHDOG_MARGIN_US: u32 = 100_000;
/// Maximum du registre LOAD (24 bits, ≈ 16,7 s à 1 µs par pas).
const WATCHDOG_MAX_US: u32 = 0x00ff_ffff;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Free,
    Di,
    Dq,
    Ai,
    Aq,
    /// Même broche demandée avec deux usages différents : laissée inerte.
    Conflict,
}

/// Usage courant de chaque GPIO, tenu à jour à chaque changement de programme.
struct PinMap {
    pins: [Use; hw::N_GPIO],
}

impl PinMap {
    const fn new() -> Self {
        PinMap { pins: [Use::Free; hw::N_GPIO] }
    }

    /// Reconfigure les broches d'après `io` (programme en STOP).
    fn configure(&mut self, io: &[IoEntry]) {
        for (pin, u) in self.pins.iter_mut().enumerate() {
            if *u != Use::Free {
                hw::release(pin);
            }
            *u = Use::Free;
        }
        hw::pwm_reset_all();
        for e in io {
            let pin = e.pin as usize;
            let want = match e.kind {
                IoKind::Di => Use::Di,
                IoKind::Dq => Use::Dq,
                IoKind::Ai => Use::Ai,
                IoKind::Aq => Use::Aq,
            };
            let valid = pin < hw::N_GPIO && (e.kind != IoKind::Ai || hw::adc_channel(e.pin).is_some());
            if !valid {
                continue;
            }
            let u = &mut self.pins[pin];
            *u = if *u == Use::Free || *u == want { want } else { Use::Conflict };
        }
        // La LED est rendue au programme avant d'être configurée par lui.
        let led_free = self.pins[hw::LED_PIN] == Use::Free;
        if !led_free {
            LED_FREE.store(false, Ordering::Release);
        }
        for (pin, u) in self.pins.iter().enumerate() {
            match u {
                Use::Di => hw::config_input(pin),
                Use::Dq => hw::config_output(pin),
                Use::Ai => hw::config_adc(pin),
                Use::Aq => hw::config_pwm(pin),
                Use::Free | Use::Conflict => {}
            }
        }
        if led_free {
            LED_FREE.store(true, Ordering::Release);
        }
    }

    #[inline(always)]
    fn read(&self, io: IoEntry) -> u32 {
        let pin = io.pin as usize;
        match (io.kind, self.pins.get(pin)) {
            (IoKind::Di, Some(Use::Di)) => hw::read_di(pin),
            (IoKind::Ai, Some(Use::Ai)) => hw::read_ai(io.pin),
            _ => 0,
        }
    }

    #[inline(always)]
    fn write(&self, io: IoEntry, v: u32) {
        let pin = io.pin as usize;
        match (io.kind, self.pins.get(pin)) {
            (IoKind::Dq, Some(Use::Dq)) => hw::write_dq(pin, v),
            (IoKind::Aq, Some(Use::Aq)) => hw::write_aq(pin, v),
            _ => {}
        }
    }
}

/// Horloge `SysTime` en ms, avancée de l'écart exact entre deux instants de
/// démarrage prévus (reste en µs conservé : aucune dérive).
struct MsClock {
    ms: u32,
    rem_us: u32,
    last_tick: u32,
}

impl MsClock {
    #[inline(always)]
    fn advance_to(&mut self, tick: u32) {
        self.rem_us += tick.wrapping_sub(self.last_tick);
        self.last_tick = tick;
        self.ms = self.ms.wrapping_add(self.rem_us / 1000);
        self.rem_us %= 1000;
    }
}

fn watchdog_load(cycle_us: u32) -> u32 {
    cycle_us.saturating_mul(2).saturating_add(WATCHDOG_MARGIN_US).min(WATCHDOG_MAX_US)
}

/// Sert au plus une trame. Renvoie vrai si un nouveau programme a été chargé.
#[inline(never)]
fn service_one(rt: &mut Rt) -> bool {
    if !REQUEST.is_full() {
        return false;
    }
    // La réponse précédente doit avoir été prise par le coeur 0 ; sinon on
    // réessaiera au cycle suivant (le protocole est requête/réponse, ça
    // n'arrive pas en fonctionnement normal).
    let mut loaded = false;
    let _ = RESPONSE.produce(|resp| {
        REQUEST
            .consume(|cmd, payload| {
                let (rcmd, len) = rt.handle(cmd, payload, resp);
                loaded = cmd == proto::DL_END && rcmd == proto::DL_END | 0x80;
                (rcmd, len)
            })
            .unwrap_or((proto::ERR, 0))
    });
    loaded
}

/// Point d'entrée du coeur 1. Ne rend jamais la main.
pub fn run() -> ! {
    // SAFETY: seul le coeur 1 touche à RT, et seulement à travers cette
    // unique référence.
    let rt: &mut Rt = unsafe { &mut *RT.0.get() };
    let mut map = PinMap::new();
    map.configure(rt.io());

    let mut cycle_us = rt.cycle_us();
    let mut wd_load = watchdog_load(cycle_us);
    let start = hw::now_us();
    let mut clock = MsClock { ms: 0, rem_us: 0, last_tick: start };
    let mut next = start.wrapping_add(cycle_us);
    // Premier instant prévu raté, quand des périodes ont été abandonnées.
    let mut missed_since: Option<u32> = None;

    loop {
        // 1. Attente active jusqu'à l'instant prévu.
        let mut t0 = hw::now_us();
        while (t0.wrapping_sub(next) as i32) < 0 {
            t0 = hw::now_us();
        }
        let late = t0.wrapping_sub(missed_since.take().unwrap_or(next));
        clock.advance_to(next);

        // 2. Le cycle automate.
        rt.cycle(clock.ms, |io| map.read(io), |io, v| map.write(io, v));
        let t1 = hw::now_us();

        // 3. Statistiques, état, chien de garde.
        rt.record_timing(t1.wrapping_sub(t0), late);
        MODE.store(rt.mode() as u8, Ordering::Relaxed);
        hw::watchdog_feed(wd_load);

        // 4. Une trame au plus. DL_END n'est accepté qu'en STOP : la carte des
        // E/S ne change donc jamais pendant que le programme tourne.
        if service_one(rt) {
            map.configure(rt.io());
            cycle_us = rt.cycle_us();
            wd_load = watchdog_load(cycle_us);
            hw::watchdog_feed(wd_load);
            let now = hw::now_us();
            next = now;
            missed_since = None;
        }

        // 5. Instant suivant, sur une grille fixe (pas de dérive). En cas de
        // débordement, le cycle suivant démarre aussitôt, en retard (compté
        // dans les statistiques) ; si une période entière ou plus a été
        // perdue, on ne la rattrape pas en rafale : on se recale sur le
        // dernier point de la grille et le retard est compté depuis le
        // premier instant raté.
        next = next.wrapping_add(cycle_us);
        let behind = hw::now_us().wrapping_sub(next) as i32;
        if behind >= 0 && behind as u32 >= cycle_us {
            missed_since = Some(next);
            next = next.wrapping_add(behind as u32 / cycle_us * cycle_us);
        }
    }
}
