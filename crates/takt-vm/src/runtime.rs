//! L'automate complet : programme chargé, mémoire, RUN/STOP, forçage,
//! statistiques et serveur du protocole.
//!
//! Le firmware et le simulateur de l'IDE utilisent tous les deux ce type. Le
//! code qui l'appelle ne fait que fournir l'heure, lire/écrire les broches et
//! transporter les trames.

use crate::cost;
use crate::exec::{self, Fault};
use crate::image::{self, Header, ImageView, IoEntry, IoKind, LoadError};
use crate::instr::Instr;
use crate::proto::{self, Status};
use crate::sys;

pub const MAX_IO: usize = 64;
pub const MAX_INIT: usize = 512;
pub const MAX_FORCE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    NoProgram = 0,
    Stop = 1,
    Run = 2,
    Fault = 3,
}

impl Mode {
    pub fn from_u8(v: u8) -> Mode {
        match v {
            1 => Mode::Stop,
            2 => Mode::Run,
            3 => Mode::Fault,
            _ => Mode::NoProgram,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Stats {
    scans: u32,
    exec_last_us: u32,
    exec_min_us: u32,
    exec_max_us: u32,
    late_max_us: u32,
    overruns: u32,
    hist: [u32; 8],
}

const STATS0: Stats = Stats {
    scans: 0,
    exec_last_us: 0,
    exec_min_us: u32::MAX,
    exec_max_us: 0,
    late_max_us: 0,
    overruns: 0,
    hist: [0; 8],
};

/// `CODE` instructions, `MEM` slots mémoire, `IMG` octets d'image au maximum.
pub struct Runtime<const CODE: usize, const MEM: usize, const IMG: usize> {
    code: [Instr; CODE],
    n_code: usize,
    mem: [u32; MEM],
    header: Header,
    io: [IoEntry; MAX_IO],
    n_io: usize,
    init: [(u16, u32); MAX_INIT],
    n_init: usize,
    forces: [(u16, u32); MAX_FORCE],
    n_forces: usize,
    mode: Mode,
    fault: u8,
    diag: u16,
    first_scan: bool,
    program_crc: u32,
    stats: Stats,
    staging: [u8; IMG],
    staging_len: usize,
}

impl<const CODE: usize, const MEM: usize, const IMG: usize> Default for Runtime<CODE, MEM, IMG> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CODE: usize, const MEM: usize, const IMG: usize> Runtime<CODE, MEM, IMG> {
    pub const fn new() -> Self {
        Runtime {
            code: [Instr::Nop; CODE],
            n_code: 0,
            mem: [0; MEM],
            header: Header {
                cycle_us: 10_000,
                mem_slots: 0,
                max_stack: 0,
                wcet_cycles: 0,
                n_io: 0,
                n_init: 0,
                n_code: 0,
            },
            io: [IoEntry::EMPTY; MAX_IO],
            n_io: 0,
            init: [(0, 0); MAX_INIT],
            n_init: 0,
            forces: [(0, 0); MAX_FORCE],
            n_forces: 0,
            mode: Mode::NoProgram,
            fault: 0,
            diag: 0,
            first_scan: true,
            program_crc: 0,
            stats: STATS0,
            staging: [0; IMG],
            staging_len: 0,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn cycle_us(&self) -> u32 {
        self.header.cycle_us
    }

    pub fn io(&self) -> &[IoEntry] {
        &self.io[..self.n_io]
    }

    pub fn memory(&self) -> &[u32] {
        &self.mem[..self.header.mem_slots as usize]
    }

    /// WCET prouvé du cycle complet en µs.
    pub fn wcet_us(&self) -> u32 {
        let c = self.header.wcet_cycles as u64 + cost::overhead(self.io(), MAX_FORCE as u32);
        cost::cycles_to_us(c) as u32
    }

    /// Charge une image complète (utilisé directement par le simulateur et
    /// par `DL_END` pour le firmware).
    pub fn load(&mut self, bytes: &[u8]) -> Result<(), LoadError> {
        let (h, crc) = Self::decode_into(bytes, &mut self.code, &mut self.io, &mut self.init)?;
        self.finish_load(h, crc);
        Ok(())
    }

    /// Valide l'image en entier avant de toucher au programme en place : en
    /// cas d'erreur, l'ancien programme reste intact.
    fn decode_into(
        bytes: &[u8],
        code: &mut [Instr; CODE],
        io: &mut [IoEntry; MAX_IO],
        init: &mut [(u16, u32); MAX_INIT],
    ) -> Result<(Header, u32), LoadError> {
        let view = ImageView::parse(bytes)?;
        let h = view.header;
        if h.n_code as usize > CODE
            || h.mem_slots as usize > MEM
            || h.n_io as usize > MAX_IO
            || h.n_init as usize > MAX_INIT
            || h.mem_slots < sys::SLOTS
        {
            return Err(LoadError::TooLarge);
        }
        for (i, c) in code.iter_mut().take(h.n_code as usize).enumerate() {
            *c = view.instr(i).ok_or(LoadError::BadInstr)?;
        }
        for (i, e) in io.iter_mut().take(h.n_io as usize).enumerate() {
            *e = view.io(i).ok_or(LoadError::BadIo)?;
        }
        for (i, e) in init.iter_mut().take(h.n_init as usize).enumerate() {
            *e = view.init(i);
        }
        Ok((h, view.crc))
    }

    fn finish_load(&mut self, h: Header, crc: u32) {
        self.n_code = h.n_code as usize;
        self.n_io = h.n_io as usize;
        self.n_init = h.n_init as usize;
        self.header = h;
        self.program_crc = crc;
        self.n_forces = 0;
        self.mode = Mode::Stop;
        self.fault = 0;
        self.stats = STATS0;
        self.reset_memory();
    }

    fn reset_memory(&mut self) {
        self.mem = [0; MEM];
        for &(slot, v) in &self.init[..self.n_init] {
            self.mem[slot as usize] = v;
        }
        self.first_scan = true;
        self.diag = 0;
    }

    /// Passage en RUN : démarrage à froid (mémoire réinitialisée).
    pub fn start(&mut self) -> bool {
        if self.mode == Mode::NoProgram {
            return false;
        }
        self.reset_memory();
        self.stats = STATS0;
        self.fault = 0;
        self.mode = Mode::Run;
        true
    }

    pub fn stop(&mut self) {
        if self.mode != Mode::NoProgram {
            self.mode = Mode::Stop;
        }
    }

    fn apply_forces(&mut self) {
        for &(slot, v) in &self.forces[..self.n_forces] {
            self.mem[slot as usize] = v;
        }
    }

    /// Un cycle automate : lecture des entrées, forçage, programme, forçage,
    /// écriture des sorties. Hors RUN, les sorties sont mises à 0.
    pub fn cycle(&mut self, now_ms: u32, mut read: impl FnMut(IoEntry) -> u32, mut write: impl FnMut(IoEntry, u32)) {
        if self.mode != Mode::Run {
            for &io in &self.io[..self.n_io] {
                if !io.kind.is_input() {
                    write(io, 0);
                }
            }
            return;
        }
        for &io in &self.io[..self.n_io] {
            if io.kind.is_input() {
                self.mem[io.slot as usize] = read(io);
            }
        }
        self.mem[sys::FIRST_SCAN as usize] = self.first_scan as u32;
        self.mem[sys::SYS_TIME as usize] = now_ms;
        self.apply_forces();

        let max_steps = cost::max_steps(self.header.wcet_cycles);
        let slots = self.header.mem_slots as usize;
        match exec::scan(&self.code[..self.n_code], &mut self.mem[..slots], now_ms, max_steps) {
            Ok(ok) => self.diag |= ok.diag,
            Err(f) => {
                self.enter_fault(f);
                for &io in &self.io[..self.n_io] {
                    if !io.kind.is_input() {
                        write(io, 0);
                    }
                }
                return;
            }
        }
        self.apply_forces();
        for &io in &self.io[..self.n_io] {
            if !io.kind.is_input() {
                write(io, self.mem[io.slot as usize]);
            }
        }
        self.first_scan = false;
    }

    fn enter_fault(&mut self, f: Fault) {
        self.mode = Mode::Fault;
        self.fault = f as u8;
    }

    /// Enregistre la durée du cycle qui vient de s'exécuter et son retard de
    /// démarrage par rapport à l'instant prévu.
    pub fn record_timing(&mut self, exec_us: u32, late_us: u32) {
        if self.mode != Mode::Run {
            return;
        }
        let s = &mut self.stats;
        s.scans = s.scans.wrapping_add(1);
        s.exec_last_us = exec_us;
        s.exec_min_us = s.exec_min_us.min(exec_us);
        s.exec_max_us = s.exec_max_us.max(exec_us);
        s.late_max_us = s.late_max_us.max(late_us);
        let cycle = self.header.cycle_us.max(1);
        if exec_us + late_us > cycle {
            s.overruns += 1;
        }
        let bucket = ((exec_us as u64 * 8) / cycle as u64).min(7) as usize;
        s.hist[bucket] += 1;
    }

    pub fn status(&self) -> Status {
        let s = &self.stats;
        Status {
            mode: self.mode as u8,
            fault: self.fault,
            diag: self.diag,
            cycle_us: self.header.cycle_us,
            wcet_us: if self.mode == Mode::NoProgram { 0 } else { self.wcet_us() },
            program_crc: self.program_crc,
            scans: s.scans,
            exec_last_us: s.exec_last_us,
            exec_min_us: if s.scans == 0 { 0 } else { s.exec_min_us },
            exec_max_us: s.exec_max_us,
            late_max_us: s.late_max_us,
            overruns: s.overruns,
            n_forces: self.n_forces as u32,
            hist: s.hist,
        }
    }

    /// Traite une requête du protocole. Écrit la réponse (données seules) dans
    /// `resp` et renvoie `(commande de réponse, longueur)`. Le temps de
    /// traitement est borné (voir [`cost::COMMAND_SERVICE`]), sauf `DL_END`,
    /// qui n'est accepté qu'en STOP.
    pub fn handle(&mut self, cmd: u8, p: &[u8], resp: &mut [u8; proto::MAX_PAYLOAD]) -> (u8, usize) {
        let err = |resp: &mut [u8; proto::MAX_PAYLOAD], code: u8, sub: u8| {
            resp[0] = code;
            resp[1] = sub;
            (proto::ERR, 2)
        };
        let ok = cmd | 0x80;
        let u16_at = |i: usize| u16::from_le_bytes([p[i], p[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
        let stopped = matches!(self.mode, Mode::NoProgram | Mode::Stop | Mode::Fault);
        match cmd {
            proto::PING => {
                resp[0..4].copy_from_slice(b"TAKT");
                resp[4..6].copy_from_slice(&image::VERSION.to_le_bytes());
                resp[6..10].copy_from_slice(&(CODE as u32).to_le_bytes());
                resp[10..14].copy_from_slice(&(MEM as u32).to_le_bytes());
                resp[14..18].copy_from_slice(&(IMG as u32).to_le_bytes());
                (ok, 18)
            }
            proto::STOP => {
                self.stop();
                (ok, 0)
            }
            proto::START => {
                if self.start() {
                    (ok, 0)
                } else {
                    err(resp, proto::E_NO_PROGRAM, 0)
                }
            }
            proto::DL_BEGIN => {
                if !stopped {
                    return err(resp, proto::E_NOT_STOPPED, 0);
                }
                if p.len() != 4 {
                    return err(resp, proto::E_PAYLOAD, 0);
                }
                let len = u32_at(0) as usize;
                if len > IMG {
                    return err(resp, proto::E_LOAD, LoadError::TooLarge.code());
                }
                self.staging_len = len;
                (ok, 0)
            }
            proto::DL_CHUNK => {
                if !stopped {
                    return err(resp, proto::E_NOT_STOPPED, 0);
                }
                if p.len() < 4 {
                    return err(resp, proto::E_PAYLOAD, 0);
                }
                let off = u32_at(0) as usize;
                let data = &p[4..];
                if off + data.len() > self.staging_len {
                    return err(resp, proto::E_PAYLOAD, 0);
                }
                self.staging[off..off + data.len()].copy_from_slice(data);
                (ok, 0)
            }
            proto::DL_END => {
                if !stopped {
                    return err(resp, proto::E_NOT_STOPPED, 0);
                }
                let bytes = &self.staging[..self.staging_len];
                match Self::decode_into(bytes, &mut self.code, &mut self.io, &mut self.init) {
                    Ok((h, crc)) => {
                        self.finish_load(h, crc);
                        resp[0..4].copy_from_slice(&self.program_crc.to_le_bytes());
                        (ok, 4)
                    }
                    Err(e) => err(resp, proto::E_LOAD, e.code()),
                }
            }
            proto::READ => {
                let n = *p.first().unwrap_or(&0) as usize;
                if n > proto::MAX_READ || p.len() != 1 + 2 * n {
                    return err(resp, proto::E_PAYLOAD, 0);
                }
                let slots = self.header.mem_slots as usize;
                for i in 0..n {
                    let s = u16_at(1 + 2 * i) as usize;
                    let v = if s < slots { self.mem[s] } else { 0 };
                    resp[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
                }
                (ok, 4 * n)
            }
            proto::WRITE => {
                if p.len() != 6 {
                    return err(resp, proto::E_PAYLOAD, 0);
                }
                let s = u16_at(0);
                if s as usize >= self.header.mem_slots as usize {
                    return err(resp, proto::E_SLOT, 0);
                }
                self.mem[s as usize] = u32_at(2);
                (ok, 0)
            }
            proto::FORCE => {
                if p.len() != 7 {
                    return err(resp, proto::E_PAYLOAD, 0);
                }
                let s = u16_at(0);
                if s as usize >= self.header.mem_slots as usize {
                    return err(resp, proto::E_SLOT, 0);
                }
                let active = p[2] != 0;
                let v = u32_at(3);
                let pos = self.forces[..self.n_forces].iter().position(|f| f.0 == s);
                match (active, pos) {
                    (true, Some(i)) => self.forces[i].1 = v,
                    (true, None) => {
                        if self.n_forces == MAX_FORCE {
                            return err(resp, proto::E_FORCE_FULL, 0);
                        }
                        self.forces[self.n_forces] = (s, v);
                        self.n_forces += 1;
                    }
                    (false, Some(i)) => {
                        self.forces[i] = self.forces[self.n_forces - 1];
                        self.n_forces -= 1;
                    }
                    (false, None) => {}
                }
                (ok, 0)
            }
            proto::UNFORCE_ALL => {
                self.n_forces = 0;
                (ok, 0)
            }
            proto::STATUS => {
                let n = self.status().encode(&mut resp[..]);
                (ok, n)
            }
            proto::RESET_STATS => {
                self.stats = STATS0;
                self.diag = 0;
                (ok, 0)
            }
            _ => err(resp, proto::E_UNKNOWN, 0),
        }
    }

    /// Pour les entrées simulées : écrit directement une valeur en mémoire.
    pub fn poke(&mut self, slot: u16, v: u32) {
        if let Some(m) = self.mem.get_mut(slot as usize) {
            *m = v;
        }
    }

    pub fn io_kind_of(&self, slot: u16) -> Option<IoKind> {
        self.io().iter().find(|io| io.slot == slot).map(|io| io.kind)
    }
}
