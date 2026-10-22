use takt_vm::exec::{DIAG_DIV0, Fault, scan};
use takt_vm::fb::{ALL, FbKind};
use takt_vm::{Instr, cost};

#[test]
fn encodage_aller_retour() {
    let mut all = vec![
        Instr::PushI(-5),
        Instr::PushF(2.5),
        Instr::Load(7),
        Instr::Store(65535),
        Instr::LoadIdx { base: 3, len: 10 },
        Instr::StoreIdx { base: 4, len: 2 },
        Instr::Jmp(12),
        Instr::Jz(0),
    ];
    all.extend(ALL.iter().map(|&kind| Instr::Fb { kind, base: 9 }));
    use Instr::*;
    all.extend([
        Nop, AddI, SubI, MulI, DivI, ModI, NegI, AbsI, MinI, MaxI, AddF, SubF, MulF, DivF, NegF, AbsF, MinF, MaxF, EqI,
        NeI, LtI, LeI, GtI, GeI, EqF, NeF, LtF, LeF, GtF, GeF, And, Or, Xor, Not, I2F, F2IRound, F2ITrunc, WrapI16,
        ToBool, Halt,
    ]);
    for i in all {
        assert_eq!(Instr::decode(&i.encode()), Some(i), "{i:?}");
        assert!(cost::cost(&i) >= cost::MIN_COST, "{i:?} sous le coût minimal");
    }
    assert_eq!(Instr::decode(&[200, 0, 0, 0, 0, 0, 0, 0]), None);
}

#[test]
fn ton_au_cycle_pres() {
    // IN, PT, Q, ET, _START, _STATE
    let mut m = [1, 100, 0, 0, 0, 0];
    FbKind::Ton.exec(&mut m, 1000);
    assert_eq!((m[2], m[3]), (0, 0));
    FbKind::Ton.exec(&mut m, 1099);
    assert_eq!((m[2], m[3]), (0, 99));
    FbKind::Ton.exec(&mut m, 1100);
    assert_eq!((m[2], m[3]), (1, 100));
    m[0] = 0;
    FbKind::Ton.exec(&mut m, 1101);
    assert_eq!((m[2], m[3]), (0, 0));
    // Passage du compteur de ms par 2^32 (49 jours)
    let mut m = [1, 50, 0, 0, 0, 0];
    FbKind::Ton.exec(&mut m, u32::MAX - 10);
    FbKind::Ton.exec(&mut m, 39);
    assert_eq!(m[2], 1);
}

#[test]
fn borne_dexecution_et_division() {
    // Boucle infinie : stoppée par la borne au lieu de bloquer l'automate
    let code = [Instr::Jmp(0)];
    let mut mem = [0u32; 4];
    assert_eq!(scan(&code, &mut mem, 0, 100), Err(Fault::StepLimit));
    let code = [Instr::PushI(7), Instr::PushI(0), Instr::DivI, Instr::Store(2), Instr::Halt];
    let ok = scan(&code, &mut mem, 0, 100).unwrap();
    assert_eq!((mem[2], ok.diag, ok.steps), (0, DIAG_DIV0, 5));
}
