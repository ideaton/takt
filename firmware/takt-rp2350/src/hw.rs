//! Accès matériels bruts utilisés par la boucle automate (coeur 1).
//!
//! Pas de HAL ici : uniquement des lectures/écritures volatiles à des adresses
//! fixes, pour que le temps de chaque accès ne dépende que du périphérique
//! (aucune section critique, aucun verrou, aucune fonction en flash). Tout ce
//! module est placé en SRAM par `memory.x` (motif `*takt_rp23502hw*`).
//!
//! Les adresses sont vérifiées au démarrage contre la PAC par
//! [`check_register_map`].
//!
//! Cible : RP2350A (QFN-60, Pico 2), GPIO 0 à 29, ADC sur GPIO 26 à 29.

use core::ptr::{read_volatile, write_volatile};

use rp235x_hal::pac;

/// Nombre de GPIO utilisables sur le RP2350A.
pub const N_GPIO: usize = 30;
/// LED de la carte Pico 2.
pub const LED_PIN: usize = 25;
/// Pleine échelle analogique (INT 0..27648, comme chez Siemens).
pub const ANALOG_FULL: u32 = 27648;
/// Premier GPIO relié à l'ADC (voie 0) sur le RP2350A.
const ADC_FIRST_PIN: u8 = 26;
const ADC_CHANNELS: u8 = 4;

const SIO: usize = 0xd000_0000;
const SIO_GPIO_IN: usize = SIO + 0x004;
const SIO_GPIO_OUT_SET: usize = SIO + 0x018;
const SIO_GPIO_OUT_CLR: usize = SIO + 0x020;
const SIO_GPIO_OE_SET: usize = SIO + 0x038;
const SIO_GPIO_OE_CLR: usize = SIO + 0x040;

const IO_BANK0: usize = 0x4002_8000;
const PADS_BANK0: usize = 0x4003_8000;
const TIMER0_TIMERAWL: usize = 0x400b_0000 + 0x28;
const WATCHDOG_LOAD: usize = 0x400d_8000 + 0x04;
const ADC_CS: usize = 0x400a_0000;
const ADC_RESULT: usize = 0x400a_0000 + 0x04;
const PWM: usize = 0x400a_8000;
const PWM_SLICE_STRIDE: usize = 0x14;
/// Tranches PWM accessibles depuis les GPIO 0 à 29.
const PWM_SLICES: usize = 8;

const FUNCSEL_PWM: u32 = 4;
const FUNCSEL_SIO: u32 = 5;
const FUNCSEL_NULL: u32 = 31;

const PAD_SCHMITT: u32 = 1 << 1;
const PAD_DRIVE_4MA: u32 = 1 << 4;
const PAD_IE: u32 = 1 << 6;
const PAD_OD: u32 = 1 << 7;
/// Isolation des pads (propre au RP2350, à 1 après reset) : doit être effacé
/// pour que le pad suive le périphérique.
const PAD_ISO: u32 = 1 << 8;

const ADC_CS_EN: u32 = 1 << 0;
const ADC_CS_START_ONCE: u32 = 1 << 2;
const ADC_CS_READY: u32 = 1 << 8;
const ADC_CS_AINSEL_SHIFT: u32 = 12;
/// Borne de l'attente d'une conversion (≈ 2 µs en pratique) : le cycle reste
/// borné même si l'ADC ne répond plus.
const ADC_TIMEOUT_POLLS: u32 = 2_000;

#[inline(always)]
fn rd(addr: usize) -> u32 {
    // SAFETY: adresse de registre périphérique valide (vérifiée au démarrage).
    unsafe { read_volatile(addr as *const u32) }
}

#[inline(always)]
fn wr(addr: usize, v: u32) {
    // SAFETY: adresse de registre périphérique valide (vérifiée au démarrage).
    unsafe { write_volatile(addr as *mut u32, v) }
}

#[inline(always)]
fn ctrl(pin: usize) -> usize {
    IO_BANK0 + 0x004 + 8 * pin
}

#[inline(always)]
fn pad(pin: usize) -> usize {
    PADS_BANK0 + 0x004 + 4 * pin
}

/// Compteur 1 MHz de TIMER0 (32 bits bas, sans effet de bord).
#[inline(always)]
pub fn now_us() -> u32 {
    rd(TIMER0_TIMERAWL)
}

/// Recharge le chien de garde (décompte en µs sur le RP2350).
#[inline(always)]
pub fn watchdog_feed(load_us: u32) {
    wr(WATCHDOG_LOAD, load_us);
}

/// Voie ADC d'un GPIO, si ce GPIO en a une.
pub fn adc_channel(pin: u8) -> Option<u8> {
    let ch = pin.wrapping_sub(ADC_FIRST_PIN);
    (ch < ADC_CHANNELS).then_some(ch)
}

// ---------------------------------------------------------------------------
// Configuration des broches (en STOP uniquement, hors cycle)
// ---------------------------------------------------------------------------

fn set_function(pin: usize, funcsel: u32) {
    wr(ctrl(pin), funcsel);
}

/// Broche rendue inerte : fonction nulle, pad isolé, ni entrée ni sortie.
pub fn release(pin: usize) {
    let mask = 1u32 << pin;
    wr(SIO_GPIO_OE_CLR, mask);
    wr(SIO_GPIO_OUT_CLR, mask);
    set_function(pin, FUNCSEL_NULL);
    wr(pad(pin), PAD_ISO | PAD_OD);
}

/// Entrée TOR : sans résistance interne (voir README, errata RP2350-E9).
pub fn config_input(pin: usize) {
    let mask = 1u32 << pin;
    wr(SIO_GPIO_OE_CLR, mask);
    wr(pad(pin), PAD_SCHMITT | PAD_IE | PAD_DRIVE_4MA);
    set_function(pin, FUNCSEL_SIO);
}

/// Sortie TOR, à 0 au départ.
pub fn config_output(pin: usize) {
    let mask = 1u32 << pin;
    wr(SIO_GPIO_OUT_CLR, mask);
    wr(SIO_GPIO_OE_SET, mask);
    wr(pad(pin), PAD_SCHMITT | PAD_IE | PAD_DRIVE_4MA);
    set_function(pin, FUNCSEL_SIO);
}

/// Entrée analogique : pad entièrement numérique coupé.
pub fn config_adc(pin: usize) {
    let mask = 1u32 << pin;
    wr(SIO_GPIO_OE_CLR, mask);
    set_function(pin, FUNCSEL_NULL);
    wr(pad(pin), PAD_OD);
}

#[inline(always)]
fn pwm_reg(slice: usize, off: usize) -> usize {
    PWM + slice * PWM_SLICE_STRIDE + off
}

#[inline(always)]
fn pwm_slice(pin: usize) -> usize {
    (pin >> 1) % PWM_SLICES
}

/// Arrête toutes les tranches PWM (avant reconfiguration).
pub fn pwm_reset_all() {
    for s in 0..PWM_SLICES {
        wr(pwm_reg(s, 0x00), 0);
        wr(pwm_reg(s, 0x0c), 0);
        wr(pwm_reg(s, 0x08), 0);
    }
}

/// Sortie analogique en PWM : TOP = 27647 et diviseur 1, donc le rapport
/// cyclique vaut directement la valeur INT (27648 = 100 %), sans calcul. À
/// 150 MHz, la fréquence PWM est de 5,43 kHz.
pub fn config_pwm(pin: usize) {
    let s = pwm_slice(pin);
    wr(pwm_reg(s, 0x04), 1 << 4); // DIV : entier 1, fraction 0
    wr(pwm_reg(s, 0x10), ANALOG_FULL - 1); // TOP
    wr(pwm_reg(s, 0x00), 1); // CSR : EN
    wr(pad(pin), PAD_DRIVE_4MA);
    set_function(pin, FUNCSEL_PWM);
}

// ---------------------------------------------------------------------------
// Accès pendant le cycle
// ---------------------------------------------------------------------------

/// Entrée TOR (`pin < N_GPIO`, vérifié par l'appelant).
#[inline(always)]
pub fn read_di(pin: usize) -> u32 {
    (rd(SIO_GPIO_IN) >> pin) & 1
}

/// Sortie TOR (`pin < N_GPIO`, vérifié par l'appelant).
#[inline(always)]
pub fn write_dq(pin: usize, v: u32) {
    let mask = 1u32 << pin;
    wr(if v != 0 { SIO_GPIO_OUT_SET } else { SIO_GPIO_OUT_CLR }, mask);
}

/// Conversion ADC unique, mise à l'échelle 12 bits → 0..27648.
pub fn read_ai(pin: u8) -> u32 {
    let Some(ch) = adc_channel(pin) else { return 0 };
    wr(ADC_CS, ADC_CS_EN | ((ch as u32) << ADC_CS_AINSEL_SHIFT));
    wr(ADC_CS, ADC_CS_EN | ((ch as u32) << ADC_CS_AINSEL_SHIFT) | ADC_CS_START_ONCE);
    let mut n = ADC_TIMEOUT_POLLS;
    while rd(ADC_CS) & ADC_CS_READY == 0 {
        n -= 1;
        if n == 0 {
            return 0;
        }
    }
    let raw = rd(ADC_RESULT) & 0xfff;
    (raw * ANALOG_FULL + 2047) / 4095
}

/// Sortie PWM : valeur INT bornée à 0..27648 (`pin < N_GPIO`).
pub fn write_aq(pin: usize, v: u32) {
    let duty = (v as i32).clamp(0, ANALOG_FULL as i32) as u32;
    let cc = pwm_reg(pwm_slice(pin), 0x0c);
    let old = rd(cc);
    let new = if pin & 1 == 0 { (old & 0xffff_0000) | duty } else { (old & 0x0000_ffff) | (duty << 16) };
    wr(cc, new);
}

// ---------------------------------------------------------------------------
// LED (coeur 0) : fonctions inline, pour ne pas exécuter de code du coeur 0
// dans les bancs SRAM du coeur 1.
// ---------------------------------------------------------------------------

#[inline(always)]
pub fn led_init() {
    let mask = 1u32 << LED_PIN;
    wr(SIO_GPIO_OUT_CLR, mask);
    wr(SIO_GPIO_OE_SET, mask);
    wr(pad(LED_PIN), PAD_IE | PAD_DRIVE_4MA);
    wr(ctrl(LED_PIN), FUNCSEL_SIO);
}

#[inline(always)]
pub fn led_set(on: bool) {
    write_dq(LED_PIN, on as u32);
}

/// Compare les adresses ci-dessus à celles de la PAC (au démarrage, coeur 0).
#[inline(never)]
pub fn check_register_map() {
    // SAFETY: lecture d'adresses uniquement, aucun accès aux registres.
    unsafe {
        let sio = &*pac::SIO::ptr();
        assert_eq!(sio.gpio_in().as_ptr() as usize, SIO_GPIO_IN);
        assert_eq!(sio.gpio_out_set().as_ptr() as usize, SIO_GPIO_OUT_SET);
        assert_eq!(sio.gpio_out_clr().as_ptr() as usize, SIO_GPIO_OUT_CLR);
        assert_eq!(sio.gpio_oe_set().as_ptr() as usize, SIO_GPIO_OE_SET);
        assert_eq!(sio.gpio_oe_clr().as_ptr() as usize, SIO_GPIO_OE_CLR);
        let io = &*pac::IO_BANK0::ptr();
        let pads = &*pac::PADS_BANK0::ptr();
        for pin in [0, 1, LED_PIN, N_GPIO - 1] {
            assert_eq!(io.gpio(pin).gpio_ctrl().as_ptr() as usize, ctrl(pin));
            assert_eq!(pads.gpio(pin).as_ptr() as usize, pad(pin));
        }
        assert_eq!((*pac::TIMER0::ptr()).timerawl().as_ptr() as usize, TIMER0_TIMERAWL);
        assert_eq!((*pac::WATCHDOG::ptr()).load().as_ptr() as usize, WATCHDOG_LOAD);
        let adc = &*pac::ADC::ptr();
        assert_eq!(adc.cs().as_ptr() as usize, ADC_CS);
        assert_eq!(adc.result().as_ptr() as usize, ADC_RESULT);
        let pwm = &*pac::PWM::ptr();
        for s in [0, PWM_SLICES - 1] {
            assert_eq!(pwm.ch(s).csr().as_ptr() as usize, pwm_reg(s, 0x00));
            assert_eq!(pwm.ch(s).div().as_ptr() as usize, pwm_reg(s, 0x04));
            assert_eq!(pwm.ch(s).ctr().as_ptr() as usize, pwm_reg(s, 0x08));
            assert_eq!(pwm.ch(s).cc().as_ptr() as usize, pwm_reg(s, 0x0c));
            assert_eq!(pwm.ch(s).top().as_ptr() as usize, pwm_reg(s, 0x10));
        }
    }
}
