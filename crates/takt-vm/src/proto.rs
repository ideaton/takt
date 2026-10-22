//! Protocole série IDE ↔ automate.
//!
//! Trame : `0xA5 | cmd u8 | len u16 | données[len] | crc8`. Le CRC-8
//! (polynôme 0x07) couvre `cmd`, `len` et les données. L'automate répond à
//! chaque trame par `cmd | 0x80`, ou par [`ERR`] suivi d'un code d'erreur.

pub const SYNC: u8 = 0xA5;
pub const MAX_PAYLOAD: usize = 256;
pub const MAX_FRAME: usize = MAX_PAYLOAD + 5;
/// Taille utile d'un bloc de téléchargement.
pub const CHUNK: usize = 240;
/// Nombre maximal de variables par lecture.
pub const MAX_READ: usize = 60;

pub const PING: u8 = 0x01;
pub const STOP: u8 = 0x02;
pub const START: u8 = 0x03;
/// `taille u32` — passe en réception d'un programme (automate en STOP).
pub const DL_BEGIN: u8 = 0x04;
/// `offset u32 | octets`
pub const DL_CHUNK: u8 = 0x05;
/// Vérifie et charge le programme reçu.
pub const DL_END: u8 = 0x06;
/// `n u8 | slot u16 × n` → `valeur u32 × n`
pub const READ: u8 = 0x07;
/// `slot u16 | valeur u32` — modification ponctuelle (« Modifier » dans TIA).
pub const WRITE: u8 = 0x08;
/// `slot u16 | actif u8 | valeur u32`
pub const FORCE: u8 = 0x09;
pub const UNFORCE_ALL: u8 = 0x0A;
pub const STATUS: u8 = 0x0B;
pub const RESET_STATS: u8 = 0x0C;
pub const ERR: u8 = 0xFF;

pub const E_UNKNOWN: u8 = 1;
pub const E_PAYLOAD: u8 = 2;
pub const E_NOT_STOPPED: u8 = 3;
pub const E_NO_PROGRAM: u8 = 4;
/// Suivi du code de [`crate::image::LoadError`].
pub const E_LOAD: u8 = 5;
pub const E_FORCE_FULL: u8 = 6;
pub const E_SLOT: u8 = 7;

pub fn describe_error(code: u8) -> &'static str {
    match code {
        E_UNKNOWN => "commande inconnue",
        E_PAYLOAD => "trame mal formée",
        E_NOT_STOPPED => "l'automate doit être en STOP",
        E_NO_PROGRAM => "aucun programme chargé",
        E_LOAD => "programme refusé",
        E_FORCE_FULL => "table de forçage pleine",
        E_SLOT => "adresse hors programme",
        _ => "erreur inconnue",
    }
}

pub fn crc8(data: &[u8]) -> u8 {
    crc8_continue(0, data)
}

/// Écrit une trame dans `out` et renvoie sa longueur.
pub fn encode_frame(cmd: u8, payload: &[u8], out: &mut [u8]) -> usize {
    let n = payload.len().min(MAX_PAYLOAD);
    out[0] = SYNC;
    out[1] = cmd;
    out[2..4].copy_from_slice(&(n as u16).to_le_bytes());
    out[4..4 + n].copy_from_slice(&payload[..n]);
    out[4 + n] = crc8(&out[1..4 + n]);
    5 + n
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub cmd: u8,
    pub len: usize,
    pub data: [u8; MAX_PAYLOAD],
}

impl Frame {
    pub fn payload(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DecState {
    Sync,
    Cmd,
    Len0,
    Len1,
    Data,
    Crc,
}

/// Décodeur octet par octet, sans allocation. Une trame corrompue est
/// ignorée et le décodeur se resynchronise sur le prochain `0xA5`.
pub struct FrameDecoder {
    state: DecState,
    frame: Frame,
    pos: usize,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    pub const fn new() -> Self {
        FrameDecoder { state: DecState::Sync, frame: Frame { cmd: 0, len: 0, data: [0; MAX_PAYLOAD] }, pos: 0 }
    }

    pub fn push(&mut self, b: u8) -> Option<Frame> {
        match self.state {
            DecState::Sync => {
                if b == SYNC {
                    self.state = DecState::Cmd;
                }
            }
            DecState::Cmd => {
                self.frame.cmd = b;
                self.state = DecState::Len0;
            }
            DecState::Len0 => {
                self.frame.len = b as usize;
                self.state = DecState::Len1;
            }
            DecState::Len1 => {
                self.frame.len |= (b as usize) << 8;
                self.pos = 0;
                self.state = if self.frame.len > MAX_PAYLOAD {
                    DecState::Sync
                } else if self.frame.len == 0 {
                    DecState::Crc
                } else {
                    DecState::Data
                };
            }
            DecState::Data => {
                self.frame.data[self.pos] = b;
                self.pos += 1;
                if self.pos == self.frame.len {
                    self.state = DecState::Crc;
                }
            }
            DecState::Crc => {
                self.state = DecState::Sync;
                let len = (self.frame.len as u16).to_le_bytes();
                let mut crc_in = [0u8; 3];
                crc_in[0] = self.frame.cmd;
                crc_in[1..3].copy_from_slice(&len);
                let mut crc = crc8_continue(0, &crc_in);
                crc = crc8_continue(crc, self.frame.payload());
                if crc == b {
                    return Some(self.frame);
                }
            }
        }
        None
    }
}

fn crc8_continue(mut crc: u8, data: &[u8]) -> u8 {
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

/// Réponse à [`STATUS`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub mode: u8,
    pub fault: u8,
    pub diag: u16,
    pub cycle_us: u32,
    /// WCET prouvé du cycle complet (code + partie fixe), en µs.
    pub wcet_us: u32,
    pub program_crc: u32,
    pub scans: u32,
    pub exec_last_us: u32,
    pub exec_min_us: u32,
    pub exec_max_us: u32,
    /// Retard maximal du début de cycle par rapport à l'instant prévu (gigue).
    pub late_max_us: u32,
    pub overruns: u32,
    pub n_forces: u32,
    /// Histogramme du temps d'exécution en huitièmes du temps de cycle.
    pub hist: [u32; 8],
}

impl Status {
    pub const SIZE: usize = 4 + 4 * 10 + 4 * 8;

    pub fn encode(&self, out: &mut [u8]) -> usize {
        out[0] = self.mode;
        out[1] = self.fault;
        out[2..4].copy_from_slice(&self.diag.to_le_bytes());
        let words = [
            self.cycle_us,
            self.wcet_us,
            self.program_crc,
            self.scans,
            self.exec_last_us,
            self.exec_min_us,
            self.exec_max_us,
            self.late_max_us,
            self.overruns,
            self.n_forces,
        ];
        let mut o = 4;
        for w in words.iter().chain(self.hist.iter()) {
            out[o..o + 4].copy_from_slice(&w.to_le_bytes());
            o += 4;
        }
        o
    }

    pub fn decode(b: &[u8]) -> Option<Status> {
        if b.len() < Self::SIZE {
            return None;
        }
        let w = |i: usize| u32::from_le_bytes([b[4 + 4 * i], b[5 + 4 * i], b[6 + 4 * i], b[7 + 4 * i]]);
        let mut hist = [0u32; 8];
        for (i, h) in hist.iter_mut().enumerate() {
            *h = w(10 + i);
        }
        Some(Status {
            mode: b[0],
            fault: b[1],
            diag: u16::from_le_bytes([b[2], b[3]]),
            cycle_us: w(0),
            wcet_us: w(1),
            program_crc: w(2),
            scans: w(3),
            exec_last_us: w(4),
            exec_min_us: w(5),
            exec_max_us: w(6),
            late_max_us: w(7),
            overruns: w(8),
            n_forces: w(9),
            hist,
        })
    }
}
