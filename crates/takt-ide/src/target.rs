//! Liaison avec un automate : simulateur intégré (façon PLCSIM) ou RP2350
//! sur port série USB. Les deux parlent le même protocole et le simulateur
//! exécute exactement le runtime du firmware.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use takt_vm::image::{IoEntry, LoadError};
use takt_vm::proto::{self, FrameDecoder, Status};
use takt_vm::runtime::Runtime;

pub type SimRuntime = Runtime<4096, 4096, 40960>;

pub trait Target {
    fn name(&self) -> String;
    /// Envoie une requête et attend la réponse `(commande, données)`.
    fn request(&mut self, cmd: u8, payload: &[u8]) -> Result<(u8, Vec<u8>), String>;
    fn as_sim(&mut self) -> Option<&mut SimTarget> {
        None
    }
}

/// Requête qui doit réussir ; transforme `ERR` en message lisible.
pub fn call(t: &mut dyn Target, cmd: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
    let (rc, data) = t.request(cmd, payload)?;
    if rc == proto::ERR {
        let code = data.first().copied().unwrap_or(0);
        let mut msg = proto::describe_error(code).to_string();
        if code == proto::E_LOAD
            && let Some(e) = data.get(1).and_then(|c| LoadError::from_code(*c))
        {
            msg = format!("{msg} : {}", e.describe());
        }
        return Err(msg);
    }
    if rc != cmd | 0x80 {
        return Err(format!("réponse inattendue 0x{rc:02X}"));
    }
    Ok(data)
}

pub fn status(t: &mut dyn Target) -> Result<Status, String> {
    Status::decode(&call(t, proto::STATUS, &[])?).ok_or_else(|| "état illisible".into())
}

/// Télécharge une image (l'automate doit être en STOP) et renvoie son CRC.
pub fn download(t: &mut dyn Target, image: &[u8]) -> Result<u32, String> {
    call(t, proto::DL_BEGIN, &(image.len() as u32).to_le_bytes())?;
    for (k, chunk) in image.chunks(proto::CHUNK).enumerate() {
        let mut p = ((k * proto::CHUNK) as u32).to_le_bytes().to_vec();
        p.extend_from_slice(chunk);
        call(t, proto::DL_CHUNK, &p)?;
    }
    let crc = call(t, proto::DL_END, &[])?;
    Ok(u32::from_le_bytes(crc.get(..4).ok_or("réponse courte")?.try_into().unwrap()))
}

/// Lit des slots mémoire par paquets.
pub fn read(t: &mut dyn Target, slots: &[u16]) -> Result<HashMap<u16, u32>, String> {
    let mut out = HashMap::new();
    for part in slots.chunks(proto::MAX_READ) {
        let mut p = vec![part.len() as u8];
        for s in part {
            p.extend_from_slice(&s.to_le_bytes());
        }
        let data = call(t, proto::READ, &p)?;
        for (i, s) in part.iter().enumerate() {
            let b = data.get(4 * i..4 * i + 4).ok_or("réponse courte")?;
            out.insert(*s, u32::from_le_bytes(b.try_into().unwrap()));
        }
    }
    Ok(out)
}

pub fn write(t: &mut dyn Target, slot: u16, v: u32) -> Result<(), String> {
    let mut p = slot.to_le_bytes().to_vec();
    p.extend_from_slice(&v.to_le_bytes());
    call(t, proto::WRITE, &p).map(|_| ())
}

pub fn force(t: &mut dyn Target, slot: u16, v: Option<u32>) -> Result<(), String> {
    let mut p = slot.to_le_bytes().to_vec();
    p.push(v.is_some() as u8);
    p.extend_from_slice(&v.unwrap_or(0).to_le_bytes());
    call(t, proto::FORCE, &p).map(|_| ())
}

// ---------------------------------------------------------------------------
// Simulateur

/// État partagé entre l'IDE et le fil d'exécution du simulateur.
pub struct SimShared {
    pub rt: Box<SimRuntime>,
    /// Valeurs des broches d'entrée, réglées depuis le panneau de simulation.
    pub inputs: HashMap<u8, u32>,
    /// Dernières valeurs écrites sur les broches de sortie.
    pub outputs: HashMap<u8, u32>,
}

pub struct SimTarget {
    pub shared: Arc<Mutex<SimShared>>,
    quit: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SimTarget {
    pub fn start() -> SimTarget {
        let shared = Arc::new(Mutex::new(SimShared {
            rt: Box::new(SimRuntime::new()),
            inputs: HashMap::new(),
            outputs: HashMap::new(),
        }));
        let quit = Arc::new(AtomicBool::new(false));
        let thread = {
            let shared = shared.clone();
            let quit = quit.clone();
            std::thread::Builder::new()
                .name("takt-plcsim".into())
                .spawn(move || sim_loop(shared, quit))
                .expect("fil du simulateur")
        };
        SimTarget { shared, quit, thread: Some(thread) }
    }

    pub fn io(&self) -> Vec<IoEntry> {
        self.shared.lock().unwrap().rt.io().to_vec()
    }
}

impl Drop for SimTarget {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Boucle du simulateur : même séquence que le cœur 1 du RP2350, cadencée
/// par l'horloge du PC (donc avec la gigue de Windows).
fn sim_loop(shared: Arc<Mutex<SimShared>>, quit: Arc<AtomicBool>) {
    let t0 = Instant::now();
    let mut next = Duration::ZERO;
    while !quit.load(Ordering::Relaxed) {
        let cycle = Duration::from_micros(shared.lock().unwrap().rt.cycle_us().max(100) as u64);
        // Attente : sommeil grossier puis attente active pour la fin
        loop {
            let now = t0.elapsed();
            if now >= next || quit.load(Ordering::Relaxed) {
                break;
            }
            let left = next - now;
            if left > Duration::from_millis(2) {
                std::thread::sleep(left - Duration::from_millis(1));
            } else {
                std::thread::yield_now();
            }
        }
        let start = t0.elapsed();
        let late = start.saturating_sub(next);
        let mut g = shared.lock().unwrap();
        let SimShared { rt, inputs, outputs } = &mut *g;
        let begin = Instant::now();
        rt.cycle(
            start.as_millis() as u32,
            |io| *inputs.get(&io.pin).unwrap_or(&0),
            |io, v| {
                outputs.insert(io.pin, v);
            },
        );
        let exec = begin.elapsed();
        rt.record_timing(exec.as_micros() as u32, late.as_micros() as u32);
        drop(g);
        next += cycle;
        // Après un gros retard (PC occupé), on se recale au lieu de rattraper en rafale
        if t0.elapsed() > next + cycle * 10 {
            next = t0.elapsed();
        }
    }
}

impl Target for SimTarget {
    fn name(&self) -> String {
        "PLCSIM Takt (simulation)".into()
    }

    fn request(&mut self, cmd: u8, payload: &[u8]) -> Result<(u8, Vec<u8>), String> {
        let mut resp = [0u8; proto::MAX_PAYLOAD];
        let (rc, len) = self.shared.lock().unwrap().rt.handle(cmd, payload, &mut resp);
        Ok((rc, resp[..len].to_vec()))
    }

    fn as_sim(&mut self) -> Option<&mut SimTarget> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Port série USB

pub struct SerialTarget {
    port: Box<dyn serialport::SerialPort>,
    name: String,
    decoder: FrameDecoder,
}

impl SerialTarget {
    pub fn open(name: &str) -> Result<SerialTarget, String> {
        let mut port = serialport::new(name, 115_200)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| format!("ouverture de {name} : {e}"))?;
        let _ = port.write_data_terminal_ready(true);
        let mut t = SerialTarget { port, name: name.into(), decoder: FrameDecoder::new() };
        let ping = call(&mut t, proto::PING, &[])?;
        if ping.get(..4) != Some(b"TAKT") {
            return Err(format!("{name} ne répond pas comme un automate Takt"));
        }
        Ok(t)
    }

    /// Ports série présents (les plus probables en premier).
    pub fn ports() -> Vec<String> {
        let mut v: Vec<(bool, String)> = serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (matches!(p.port_type, serialport::SerialPortType::UsbPort(_)), p.port_name))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        v.into_iter().map(|(_, n)| n).collect()
    }
}

impl Target for SerialTarget {
    fn name(&self) -> String {
        format!("RP2350 sur {}", self.name)
    }

    fn request(&mut self, cmd: u8, payload: &[u8]) -> Result<(u8, Vec<u8>), String> {
        let mut frame = [0u8; proto::MAX_FRAME];
        let n = proto::encode_frame(cmd, payload, &mut frame);
        let _ = self.port.clear(serialport::ClearBuffer::Input);
        self.decoder = FrameDecoder::new();
        self.port.write_all(&frame[..n]).map_err(|e| format!("envoi : {e}"))?;
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut buf = [0u8; 512];
        while Instant::now() < deadline {
            match self.port.read(&mut buf) {
                Ok(0) => {}
                Ok(k) => {
                    for &b in &buf[..k] {
                        if let Some(f) = self.decoder.push(b) {
                            return Ok((f.cmd, f.payload().to_vec()));
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(format!("réception : {e}")),
            }
        }
        Err("l'automate ne répond pas (délai dépassé)".into())
    }
}
