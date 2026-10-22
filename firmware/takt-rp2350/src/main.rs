//! Firmware Takt pour RP2350 (Raspberry Pi Pico 2).
//!
//! - Coeur 1 : automate seul, cycle déterministe (voir [`plc`]).
//! - Coeur 0 : communication seule : USB CDC ↔ trames du protocole ↔ boîtes
//!   aux lettres ([`mailbox`]), et LED d'état.
//!
//! Voir README.md pour ce qui est garanti et ce qui ne l'est pas.
#![no_std]
#![no_main]

mod hw;
mod mailbox;
mod plc;

use core::sync::atomic::Ordering;

use cortex_m::peripheral::DWT;
use rp235x_hal as hal;
use hal::multicore::{Multicore, StackAllocation};
use hal::pac;
use takt_vm::proto::{self, FrameDecoder, Frame};
use takt_vm::runtime::Mode;
use usb_device::bus::UsbBusAllocator;
use usb_device::device::{StringDescriptors, UsbDeviceBuilder, UsbDeviceState, UsbVidPid};
use usbd_serial::SerialPort;

use mailbox::{REQUEST, RESPONSE};

/// Bloc IMAGE_DEF : indique à la bootrom un exécutable Arm sécurisé.
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

const XTAL_FREQ_HZ: u32 = 12_000_000;
/// Fréquence CPU fixée par `init_clocks_and_plls`, celle de `takt_vm::cost`.
const CPU_HZ: u32 = 150_000_000;

/// VID/PID de test de pid.codes (à remplacer par un PID attribué).
const USB_VID_PID: UsbVidPid = UsbVidPid(0x1209, 0x0001);

/// Chien de garde au démarrage, avant que le coeur 1 ne le règle d'après le
/// temps de cycle.
const WATCHDOG_BOOT_MS: u32 = 500;

unsafe extern "C" {
    static __plc_ram_load: u32;
    static mut __plc_ram_start: u32;
    static mut __plc_ram_end: u32;
    static mut __core1_stack_bottom: usize;
    static mut __core1_stack_top: usize;
}

/// Recopie `.plc_ram` (code et données du coeur 1) de la flash vers SRAM4-7.
///
/// Mot par mot et en volatile : surtout pas de `memcpy`, qui fait lui-même
/// partie de ce qui est recopié.
///
/// `inline(never)`, comme [`core0`] : la frontière d'appel empêche le
/// compilateur d'avancer avant la copie un `memset`/`memcpy` du reste de
/// `main` (initialisation de variables locales, par exemple).
///
/// # Safety
/// À appeler une seule fois, avant toute exécution de code de `.plc_ram`.
#[inline(never)]
unsafe fn copy_plc_ram() {
    unsafe {
        let mut src = &raw const __plc_ram_load;
        let mut dst = &raw mut __plc_ram_start;
        let end = &raw mut __plc_ram_end;
        while dst < end {
            dst.write_volatile(src.read_volatile());
            dst = dst.add(1);
            src = src.add(1);
        }
    }
    cortex_m::asm::dsb();
    cortex_m::asm::isb();
}

#[hal::entry]
fn main() -> ! {
    // SAFETY: premier appel, rien de `.plc_ram` n'a encore été exécuté.
    unsafe { copy_plc_ram() };
    core0()
}

/// Initialisation, lancement du coeur 1, puis boucle de communication.
#[inline(never)]
fn core0() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let mut core = cortex_m::Peripherals::take().unwrap();

    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let Ok(clocks) = hal::clocks::init_clocks_and_plls(
        XTAL_FREQ_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    ) else {
        panic!("horloges");
    };

    // Périphériques utilisés par le coeur 1 : sortis de reset ici, puis
    // pilotés en accès direct par `hw`.
    let _timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);
    let mut sio = hal::Sio::new(pac.SIO);
    let _pins = hal::gpio::Pins::new(pac.IO_BANK0, pac.PADS_BANK0, sio.gpio_bank0, &mut pac.RESETS);
    let _pwm = hal::pwm::Slices::new(pac.PWM, &mut pac.RESETS);
    let _adc = hal::Adc::new(pac.ADC, &mut pac.RESETS);
    hw::check_register_map();

    // Le coeur 1 gagne les arbitrages de bus contre le coeur 0.
    pac.BUSCTRL.bus_priority().write(|w| w.proc1().set_bit());

    hw::led_init();
    core.DCB.enable_trace();
    core.DWT.enable_cycle_counter();

    let usb_bus = UsbBusAllocator::new(hal::usb::UsbBus::new(
        pac.USB,
        pac.USB_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));
    let mut serial = SerialPort::new(&usb_bus);
    let Ok(builder) = UsbDeviceBuilder::new(&usb_bus, USB_VID_PID).strings(&[StringDescriptors::default()
        .manufacturer("Ideaton")
        .product("Takt PLC")
        .serial_number("TAKT-RP2350")])
    else {
        panic!("descripteurs USB");
    };
    let mut usb_dev = builder.device_class(usbd_serial::USB_CLASS_CDC).build();

    // Chien de garde : armé ici, rechargé ensuite par le coeur 1 seul.
    watchdog.start(hal::fugit::MicrosDurationU32::millis(WATCHDOG_BOOT_MS));

    let mut mc = Multicore::new(&mut pac.PSM, &mut pac.PPB, &mut sio.fifo);
    let core1 = &mut mc.cores()[1];
    // SAFETY: SRAM8-9 est réservée à cette pile par memory.x, rien d'autre n'y
    // est placé.
    let stack = unsafe {
        StackAllocation::from_raw_parts(&raw mut __core1_stack_bottom, &raw mut __core1_stack_top)
    };
    if core1.spawn(stack, || plc::run()).is_err() {
        panic!("coeur 1");
    }

    let mut link = Link::new();
    let mut led = Led::new();
    loop {
        usb_dev.poll(&mut [&mut serial]);
        let configured = usb_dev.state() == UsbDeviceState::Configured;
        link.pump(&mut serial, configured);
        led.update(Mode::from_u8(plc::MODE.load(Ordering::Relaxed)));
    }
}

/// Transport des trames entre l'USB et les boîtes aux lettres (coeur 0).
struct Link {
    decoder: FrameDecoder,
    /// Trame décodée qui attend que la case REQUEST se libère.
    pending: Option<Frame>,
    rx: [u8; 64],
    rx_pos: usize,
    rx_len: usize,
    tx: [u8; proto::MAX_FRAME],
    tx_pos: usize,
    tx_len: usize,
}

impl Link {
    fn new() -> Self {
        Link {
            decoder: FrameDecoder::new(),
            pending: None,
            rx: [0; 64],
            rx_pos: 0,
            rx_len: 0,
            tx: [0; proto::MAX_FRAME],
            tx_pos: 0,
            tx_len: 0,
        }
    }

    fn pump<B: usb_device::bus::UsbBus>(&mut self, serial: &mut SerialPort<'_, B>, configured: bool) {
        // Réponse du coeur 1 → trame à émettre.
        if self.tx_pos == self.tx_len {
            let tx = &mut self.tx;
            if let Some(n) = RESPONSE.consume(|cmd, payload| proto::encode_frame(cmd, payload, tx)) {
                self.tx_pos = 0;
                self.tx_len = if configured { n } else { 0 };
            }
        }
        if !configured {
            self.tx_pos = self.tx_len;
        } else if self.tx_pos < self.tx_len
            && let Ok(n) = serial.write(&self.tx[self.tx_pos..self.tx_len])
        {
            self.tx_pos += n;
        }

        // Requête décodée → coeur 1, dès que la case est libre.
        if let Some(f) = &self.pending {
            let posted = REQUEST.produce(|buf| {
                buf[..f.len].copy_from_slice(f.payload());
                (f.cmd, f.len)
            });
            if posted.is_some() {
                self.pending = None;
            }
        }

        // Octets reçus → décodeur, tant qu'aucune trame n'attend (sinon on
        // laisse l'USB refuser les paquets suivants : contrôle de flux naturel).
        while self.pending.is_none() && self.rx_pos < self.rx_len {
            self.pending = self.decoder.push(self.rx[self.rx_pos]);
            self.rx_pos += 1;
        }
        if self.rx_pos == self.rx_len {
            self.rx_pos = 0;
            self.rx_len = serial.read(&mut self.rx).unwrap_or(0);
        }
    }
}

/// LED d'état (coeur 0) : allumée = RUN, clignotement lent = STOP ou pas de
/// programme, clignotement rapide = FAULT. Rythme pris sur le compteur de
/// cycles du coeur 0 (DWT), pour ne pas toucher au bus APB que le coeur 1
/// utilise.
struct Led {
    last_toggle: u32,
    on: bool,
    owned: bool,
}

impl Led {
    fn new() -> Self {
        Led { last_toggle: DWT::cycle_count(), on: false, owned: true }
    }

    fn update(&mut self, mode: Mode) {
        let free = plc::LED_FREE.load(Ordering::Acquire);
        if !free {
            self.owned = false;
            return;
        }
        if !self.owned {
            hw::led_init();
            self.owned = true;
        }
        let half_period_ms = match mode {
            Mode::Run => {
                self.on = true;
                hw::led_set(true);
                return;
            }
            Mode::Fault => 100,
            Mode::Stop | Mode::NoProgram => 500,
        };
        let now = DWT::cycle_count();
        if now.wrapping_sub(self.last_toggle) >= half_period_ms * (CPU_HZ / 1000) {
            self.last_toggle = now;
            self.on = !self.on;
            hw::led_set(self.on);
        }
    }
}

/// Toute panique (qui ne devrait pas arriver) redémarre la puce par le chien
/// de garde : sorties remises à zéro, automate en « pas de programme ».
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    hal::reset()
}
