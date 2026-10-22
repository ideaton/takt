//! CONT, GRAPH et BASIC : traduits en ST, compilés et exécutés dans le
//! runtime, comme dans le simulateur.

use takt_compiler::lang::ladder::{CmpOp, Coil, CoilKind, ContactKind, CounterKind, Elem, Network, TimerKind};
use takt_compiler::lang::sfc::{Action, Qualifier, Sfc, Step, Transition};
use takt_compiler::lang::{BlockType, IfaceVar, Language, Section};
use takt_compiler::project::{Block, Device, Project, Tag};
use takt_compiler::{Compiled, Severity, compile};
use takt_vm::runtime::{Mode, Runtime};

type Rt = Runtime<4096, 4096, 40960>;

fn project(tags: &[(&str, &str, &str)], blocks: Vec<Block>) -> Project {
    Project {
        name: "test".into(),
        device: Device { name: "API_1".into(), board: "pico2".into(), cycle_us: 1000 },
        tags: tags
            .iter()
            .map(|(n, t, a)| Tag {
                name: n.to_string(),
                ty: t.to_string(),
                address: a.to_string(),
                comment: String::new(),
            })
            .collect(),
        blocks,
        watch_tables: vec![],
    }
}

fn build(p: &Project) -> Compiled {
    let out = compile(p);
    let errors: Vec<_> = out.diags.iter().filter(|d| d.severity == Severity::Error).collect();
    if !errors.is_empty() {
        for b in &p.blocks {
            eprintln!("--- {} ---\n{}", b.name, b.to_st().source());
        }
        panic!("erreurs : {errors:#?}");
    }
    out.compiled.unwrap()
}

fn errors(p: &Project) -> Vec<(String, u32, String)> {
    compile(p)
        .diags
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| (d.block, d.pos.line, d.msg))
        .collect()
}

struct Sim {
    rt: Box<Rt>,
    c: Compiled,
    now: u32,
}

impl Sim {
    fn new(c: Compiled) -> Sim {
        let mut rt = Box::new(Rt::new());
        rt.load(&c.image).unwrap();
        rt.start();
        Sim { rt, c, now: 0 }
    }

    fn slot(&self, path: &str) -> u16 {
        self.c.resolve(path).unwrap_or_else(|| panic!("{path} introuvable")).0
    }

    fn set(&mut self, path: &str, v: u32) {
        let s = self.slot(path);
        // Les entrées sont relues à chaque cycle : on passe par un forçage
        let mut p = s.to_le_bytes().to_vec();
        p.push(1);
        p.extend_from_slice(&v.to_le_bytes());
        let mut resp = [0u8; takt_vm::proto::MAX_PAYLOAD];
        self.rt.handle(takt_vm::proto::FORCE, &p, &mut resp);
    }

    fn get(&self, path: &str) -> u32 {
        self.rt.memory()[self.slot(path) as usize]
    }

    fn run(&mut self, ms: u32) {
        for _ in 0..ms {
            self.rt.cycle(self.now, |_| 0, |_, _| {});
            assert_eq!(self.rt.mode(), Mode::Run);
            self.now += 1;
        }
    }
}

fn contact(op: &str) -> Elem {
    Elem::Contact { kind: ContactKind::No, operand: op.into() }
}

fn nc(op: &str) -> Elem {
    Elem::Contact { kind: ContactKind::Nc, operand: op.into() }
}

fn coil(kind: CoilKind, op: &str) -> Coil {
    Coil { kind, operand: op.into(), value: String::new() }
}

#[test]
fn cont_auto_maintien_tempo_compteur() {
    let mut b = Block::new("Main", BlockType::Ob, Language::Ladder);
    b.interface = vec![IfaceVar::new(Section::Static, "Tempo", "TON"), IfaceVar::new(Section::Static, "Niveau", "INT")];
    b.networks = vec![
        // Marche/arrêt avec auto-maintien
        Network {
            title: "Moteur".into(),
            comment: String::new(),
            rung: vec![
                Elem::Parallel { branches: vec![vec![contact("\"Marche\"")], vec![contact("\"Moteur\"")]] },
                nc("\"Arret\""),
            ],
            coils: vec![coil(CoilKind::Assign, "\"Moteur\"")],
        },
        // Voyant 2 s après le démarrage
        Network {
            title: "Tempo".into(),
            comment: String::new(),
            rung: vec![
                contact("\"Moteur\""),
                Elem::Timer { kind: TimerKind::Ton, instance: "Tempo".into(), pt: "T#2s".into() },
            ],
            coils: vec![coil(CoilKind::Assign, "\"Voyant\"")],
        },
        // Comptage des fronts de "Piece", mise à 1 de "Plein" à 3 pièces
        Network {
            title: "Comptage".into(),
            comment: String::new(),
            rung: vec![
                Elem::Contact { kind: ContactKind::Pos, operand: "\"Piece\"".into() },
                Elem::Counter {
                    kind: CounterKind::Ctu,
                    instance: "Compteur".into(),
                    reset: "\"Arret\"".into(),
                    pv: "3".into(),
                },
            ],
            coils: vec![coil(CoilKind::Set, "\"Plein\"")],
        },
        // Comparateur et MOVE
        Network {
            title: String::new(),
            comment: String::new(),
            rung: vec![Elem::Compare { op: CmpOp::Ge, a: "#Compteur.CV".into(), b: "2".into() }],
            coils: vec![Coil { kind: CoilKind::Move, operand: "#Niveau".into(), value: "100".into() }],
        },
    ];
    let tags = [
        ("Marche", "BOOL", ""),
        ("Arret", "BOOL", ""),
        ("Moteur", "BOOL", ""),
        ("Voyant", "BOOL", ""),
        ("Piece", "BOOL", ""),
        ("Plein", "BOOL", ""),
    ];
    let mut s = Sim::new(build(&project(&tags, vec![b])));
    s.set("Marche", 1);
    s.run(2);
    s.set("Marche", 0);
    s.run(10);
    assert_eq!(s.get("Moteur"), 1, "auto-maintien");
    s.run(1985);
    assert_eq!(s.get("Voyant"), 0);
    s.run(10);
    assert_eq!(s.get("Voyant"), 1, "TON de 2 s");
    for _ in 0..3 {
        s.set("Piece", 1);
        s.run(2);
        s.set("Piece", 0);
        s.run(2);
    }
    assert_eq!(s.get("Main.Compteur.CV"), 3);
    assert_eq!(s.get("Plein"), 1);
    assert_eq!(s.get("Main.Niveau"), 100);
    s.set("Arret", 1);
    s.run(2);
    assert_eq!(s.get("Moteur"), 0);
    assert_eq!(s.get("Main.Compteur.CV"), 0, "reset du compteur");
}

#[test]
fn graph_feux_tricolores() {
    let mut b = Block::new("Feux", BlockType::Fb, Language::Sfc);
    b.interface = vec![IfaceVar::new(Section::Static, "Passages", "INT")];
    b.sfc = Sfc {
        steps: vec![
            Step {
                number: 1,
                name: "Rouge".into(),
                actions: vec![Action { qualifier: Qualifier::N, operand: "\"Rouge\"".into() }],
            },
            Step {
                number: 2,
                name: "Vert".into(),
                actions: vec![
                    Action { qualifier: Qualifier::N, operand: "\"Vert\"".into() },
                    Action {
                        qualifier: Qualifier::Code,
                        operand: "IF T_Etape = T#0ms THEN #Passages := #Passages + 1; END_IF".into(),
                    },
                ],
            },
            Step {
                number: 3,
                name: "Orange".into(),
                actions: vec![Action { qualifier: Qualifier::N, operand: "\"Orange\"".into() }],
            },
        ],
        transitions: vec![
            Transition { from: 1, to: 2, condition: "T_Etape >= T#3s".into() },
            Transition { from: 2, to: 3, condition: "T_Etape >= T#2s OR \"Urgence\"".into() },
            Transition { from: 3, to: 1, condition: "T_Etape >= T#1s".into() },
        ],
    };
    let ob = Block::st("Main", "ORGANIZATION_BLOCK Main VAR feux : Feux; END_VAR feux(); END_ORGANIZATION_BLOCK");
    let tags = [("Rouge", "BOOL", ""), ("Vert", "BOOL", ""), ("Orange", "BOOL", ""), ("Urgence", "BOOL", "")];
    let mut s = Sim::new(build(&project(&tags, vec![ob, b])));
    s.run(10);
    assert_eq!((s.get("Rouge"), s.get("Vert"), s.get("Orange")), (1, 0, 0));
    assert_eq!(s.get("Main.feux.Etape"), 1);
    s.run(3000);
    assert_eq!((s.get("Rouge"), s.get("Vert")), (0, 1), "vert après 3 s de rouge");
    s.set("Urgence", 1);
    s.run(3);
    assert_eq!(s.get("Orange"), 1, "transition par condition");
    s.set("Urgence", 0);
    s.run(1005);
    assert_eq!(s.get("Rouge"), 1, "retour au rouge");
    assert_eq!(s.get("Main.feux.Passages"), 1);
}

#[test]
fn basic_complet() {
    let mut b = Block::new("Calcul", BlockType::Fc, Language::Basic);
    b.return_type = "INT".into();
    b.interface = vec![IfaceVar::new(Section::Input, "n", "INT")];
    b.source = "' Somme de 1 à 5, puis choix selon n
DIM i, total AS INT
total = 0
FOR i = 1 TO 5
    total = total + i   ' 15 au final
NEXT i
SELECT CASE n
    CASE 1 : total = total * 2
    CASE 2, 3 TO 4 : total = 0
    CASE ELSE : total = total + n
END SELECT
IF n = 7 THEN SET \"Sept\" ELSE RESET \"Sept\"
Calcul = total
"
    .into();
    let ob = Block::st(
        "Main",
        "ORGANIZATION_BLOCK Main VAR a, b2, c, d : INT; END_VAR a := Calcul(1); b2 := Calcul(3); c := Calcul(7); d := Calcul(n := 10); END_ORGANIZATION_BLOCK",
    );
    let mut s = Sim::new(build(&project(&[("Sept", "BOOL", "")], vec![ob, b])));
    s.run(1);
    assert_eq!(s.get("Main.a"), 30);
    assert_eq!(s.get("Main.b2"), 0);
    assert_eq!(s.get("Main.c"), 22);
    assert_eq!(s.get("Main.d"), 25);
    assert_eq!(s.get("Sept"), 0, "le dernier appel (n = 10) remet Sept à 0");
}

#[test]
fn erreurs_renvoyees_a_la_source() {
    // CONT : opérande inconnu → réseau 2
    let mut b = Block::new("Main", BlockType::Ob, Language::Ladder);
    b.networks = vec![
        Network::default(),
        Network {
            title: "Pompe".into(),
            comment: String::new(),
            rung: vec![contact("Inconnu")],
            coils: vec![coil(CoilKind::Assign, "\"Q\"")],
        },
    ];
    let errs = errors(&project(&[("Q", "BOOL", "")], vec![b]));
    assert!(
        errs.iter().any(|(blk, line, m)| blk == "Main" && *line == 2 && m.starts_with("réseau 2 « Pompe »")),
        "{errs:?}"
    );

    // BASIC : WHILE refusé, à la bonne ligne
    let mut b = Block::new("Main", BlockType::Ob, Language::Basic);
    b.source = "x = 1\nWHILE x < 10\n  x = x + 1\nWEND\n".into();
    b.interface = vec![IfaceVar::new(Section::Static, "x", "INT")];
    let errs = errors(&project(&[], vec![b]));
    assert!(errs.iter().any(|(_, line, m)| *line == 2 && m.contains("WHILE est interdit")), "{errs:?}");

    // GRAPH : transition vers une étape inexistante
    let mut b = Block::new("Seq", BlockType::Fb, Language::Sfc);
    b.sfc.transitions.push(Transition { from: 2, to: 9, condition: "TRUE".into() });
    let errs = errors(&project(&[], vec![Block::st("Main", "PROGRAM Main END_PROGRAM"), b]));
    assert!(errs.iter().any(|(_, _, m)| m.contains("S9 n'existe pas")), "{errs:?}");
}
