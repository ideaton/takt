//! Compile des programmes et les exécute dans le runtime de la VM, comme le
//! simulateur de l'IDE et le firmware.

use takt_compiler::project::{Block, Device, Project, Tag};
use takt_compiler::types::Ty;
use takt_compiler::{Compiled, Severity, compile};
use takt_vm::image::IoKind;
use takt_vm::runtime::{Mode, Runtime};

type Rt = Runtime<4096, 4096, 40960>;

fn project(tags: &[(&str, &str, &str)], blocks: &[&str], cycle_us: u32) -> Project {
    Project {
        name: "test".into(),
        device: Device { name: "API_1".into(), board: "pico2".into(), cycle_us },
        tags: tags
            .iter()
            .map(|(n, t, a)| Tag {
                name: n.to_string(),
                ty: t.to_string(),
                address: a.to_string(),
                comment: String::new(),
            })
            .collect(),
        blocks: blocks
            .iter()
            .enumerate()
            .map(|(i, s)| Block { name: format!("B{i}"), source: s.to_string() })
            .collect(),
        watch_tables: vec![],
    }
}

fn build(p: &Project) -> Compiled {
    let out = compile(p);
    let errors: Vec<_> = out.diags.iter().filter(|d| d.severity == Severity::Error).collect();
    assert!(errors.is_empty(), "erreurs : {errors:#?}");
    out.compiled.unwrap()
}

fn errors_of(p: &Project) -> Vec<String> {
    compile(p).diags.into_iter().filter(|d| d.severity == Severity::Error).map(|d| d.msg).collect()
}

/// Automate simulé avec des entrées TOR/analogiques pilotées par le test.
struct Sim {
    rt: Box<Rt>,
    c: Compiled,
    inputs: std::collections::HashMap<u8, u32>,
    outputs: std::collections::HashMap<u8, u32>,
    now_ms: u32,
}

impl Sim {
    fn new(c: Compiled) -> Sim {
        let mut rt = Box::new(Rt::new());
        rt.load(&c.image).expect("image valide");
        assert!(rt.start());
        Sim { rt, c, inputs: Default::default(), outputs: Default::default(), now_ms: 0 }
    }

    fn pin_of(&self, tag: &str) -> u8 {
        let (slot, _) = self.c.resolve(tag).unwrap();
        self.c.io.iter().find(|io| io.slot == slot).unwrap().pin
    }

    fn set(&mut self, tag: &str, v: u32) {
        let pin = self.pin_of(tag);
        self.inputs.insert(pin, v);
    }

    fn out(&self, tag: &str) -> u32 {
        *self.outputs.get(&self.pin_of(tag)).unwrap_or(&0)
    }

    fn get(&self, path: &str) -> u32 {
        let (slot, _) = self.c.resolve(path).unwrap_or_else(|| panic!("{path} introuvable"));
        self.rt.memory()[slot as usize]
    }

    fn real(&self, path: &str) -> f32 {
        f32::from_bits(self.get(path))
    }

    /// Fait tourner l'automate pendant `ms` millisecondes, un cycle par ms.
    fn run(&mut self, ms: u32) {
        for _ in 0..ms.max(1) {
            let inputs = &self.inputs;
            let outputs = &mut self.outputs;
            self.rt.cycle(
                self.now_ms,
                |io| *inputs.get(&io.pin).unwrap_or(&0),
                |io, v| {
                    outputs.insert(io.pin, v);
                },
            );
            assert_eq!(self.rt.mode(), Mode::Run, "l'automate est passé en défaut");
            self.now_ms += 1;
        }
    }
}

#[test]
fn convoyeur_exemple() {
    let p = Project::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/convoyeur.takt")))
        .unwrap();
    let c = build(&p);
    assert!(c.wcet.total_us <= c.wcet.cycle_us as u64);
    let mut s = Sim::new(c);
    s.set("BP_Arret", 1); // NF : 1 = pas d'arrêt demandé
    s.run(5);
    assert_eq!(s.out("Moteur_Convoyeur"), 0);

    // Marche : le moteur s'auto-maintient
    s.set("BP_Marche", 1);
    s.run(2);
    s.set("BP_Marche", 0);
    s.set("Retour_Marche", 1);
    s.run(100);
    assert_eq!(s.out("Moteur_Convoyeur"), 1);
    assert_eq!(s.out("Voyant_Defaut"), 0);

    // 10 pièces → lot terminé → arrêt du moteur
    for _ in 0..10 {
        s.set("Capteur_Piece", 1);
        s.run(3);
        s.set("Capteur_Piece", 0);
        s.run(3);
    }
    assert_eq!(s.get("Nb_Pieces"), 10);
    assert_eq!(s.out("Lot_Termine"), 1);
    assert_eq!(s.out("Moteur_Convoyeur"), 0);

    // Reset, redémarrage sans retour de marche → défaut après 5 s exactement
    s.set("BP_Reset", 1);
    s.run(2);
    s.set("BP_Reset", 0);
    s.set("Retour_Marche", 0);
    s.set("BP_Marche", 1);
    s.run(1);
    s.set("BP_Marche", 0);
    s.run(4990);
    assert_eq!(s.out("Voyant_Defaut"), 0);
    s.run(20);
    assert_eq!(s.out("Voyant_Defaut"), 1);
    assert_eq!(s.out("Moteur_Convoyeur"), 0);

    // Niveau analogique : 27648 = 100 %, moyenne sur 8 cycles
    s.set("Niveau_Tremie", 13824);
    s.run(10);
    assert!((s.real("Niveau_Moyen") - 50.0).abs() < 0.01, "{}", s.real("Niveau_Moyen"));
    assert_eq!(s.out("Alarme_Niveau"), 0);
}

#[test]
fn boucles_non_bornees_refusees() {
    let p = project(&[], &["PROGRAM P VAR i : INT; END_VAR WHILE i < 10 DO i := i + 1; END_WHILE; END_PROGRAM"], 1000);
    assert!(errors_of(&p).iter().any(|e| e.contains("WHILE est interdit")));
    let p = project(&[], &["PROGRAM P VAR i, n : INT; END_VAR FOR i := 0 TO n DO END_FOR; END_PROGRAM"], 1000);
    assert!(errors_of(&p).iter().any(|e| e.contains("bornes de FOR doivent être constantes")));
    let p = project(&[], &["PROGRAM P VAR i : INT; END_VAR FOR i := 0 TO 5 DO i := 2; END_FOR; END_PROGRAM"], 1000);
    assert!(errors_of(&p).iter().any(|e| e.contains("ne peut pas être modifiée")));
}

#[test]
fn temps_de_cycle_non_garanti() {
    let src = "PROGRAM P VAR i, j : DINT; s : REAL; END_VAR
        FOR i := 1 TO 1000 DO FOR j := 1 TO 100 DO s := s + 1.5; END_FOR; END_FOR;
        END_PROGRAM";
    let errs = errors_of(&project(&[], &[src], 1000));
    assert!(errs.iter().any(|e| e.contains("NON garanti")), "{errs:?}");
    // Avec un cycle assez long, le même programme passe
    build(&project(&[], &[src], 1_000_000));
}

#[test]
fn wcet_majore_le_reel() {
    // Le nombre d'instructions réellement exécutées ne dépasse jamais la
    // borne : sinon le runtime passerait en défaut StepLimit.
    let src = "PROGRAM P VAR i : INT; t : ARRAY[1..20] OF DINT; x : DINT; END_VAR
        FOR i := 1 TO 20 DO
            IF i MOD 3 = 0 THEN t[i] := i * i; ELSIF i MOD 3 = 1 THEN t[i] := -i; ELSE t[i] := 0; END_IF;
        END_FOR;
        CASE t[3] OF 9: x := 1; 1..5, 10: x := 2; ELSE x := 3; END_CASE;
        END_PROGRAM";
    let c = build(&project(&[], &[src], 1000));
    let mut s = Sim::new(c);
    s.run(3);
    assert_eq!(s.get("P.t[9]"), 81);
    assert_eq!(s.get("P.t[4]") as i32, -4);
    assert_eq!(s.get("P.x"), 1);
}

#[test]
fn arithmetique_deterministe() {
    let src = "PROGRAM P VAR a : INT := 32767; b : INT; d : DINT; z : DINT; r : REAL; idx : INT := 50; t : ARRAY[0..3] OF INT; v : INT; END_VAR
        b := a + 1;                 // débordement INT : -32768
        d := INT_TO_DINT(a) + 1;    // 32768
        d := d / z;                 // division par 0 : 0 + diagnostic
        r := REAL_TO_INT(2.5) * 1.0;
        t[3] := 7;
        v := t[idx];                // index hors bornes : borné à 3
        END_PROGRAM";
    let c = build(&project(&[], &[src], 1000));
    let mut s = Sim::new(c);
    s.run(1);
    assert_eq!(s.get("P.b") as i32, -32768);
    assert_eq!(s.get("P.d"), 0);
    assert_eq!(s.real("P.r"), 3.0);
    assert_eq!(s.get("P.v"), 7);
    let st = s.rt.status();
    assert_eq!(st.diag & takt_vm::exec::DIAG_DIV0, takt_vm::exec::DIAG_DIV0);
    assert_eq!(st.diag & takt_vm::exec::DIAG_INDEX, takt_vm::exec::DIAG_INDEX);
}

#[test]
fn functions_imbriquees_et_recursion() {
    let f = "FUNCTION Double : DINT VAR_INPUT x : DINT; END_VAR Double := x * 2; END_FUNCTION";
    let p = "PROGRAM P VAR r : DINT; END_VAR r := Double(Double(3) + Double(x := 1)); END_PROGRAM";
    let mut s = Sim::new(build(&project(&[], &[p, f], 1000)));
    s.run(1);
    assert_eq!(s.get("P.r"), 16);

    let rec = "FUNCTION Boucle : INT VAR_INPUT n : INT; END_VAR Boucle := Boucle(n); END_FUNCTION";
    let p = "PROGRAM P VAR r : INT; END_VAR r := Boucle(1); END_PROGRAM";
    assert!(errors_of(&project(&[], &[p, rec], 1000)).iter().any(|e| e.contains("récursif")));
}

#[test]
fn fb_utilisateur_instances_independantes() {
    let fb = "FUNCTION_BLOCK Cpt VAR_INPUT inc : BOOL; END_VAR VAR_OUTPUT n : INT; END_VAR VAR f : R_TRIG; END_VAR
        f(CLK := inc); IF f.Q THEN n := n + 1; END_IF; END_FUNCTION_BLOCK";
    let p = "PROGRAM P VAR a, b : Cpt; END_VAR a(inc := \"E1\"); b(inc := \"E1\" AND \"E2\"); END_PROGRAM";
    let tags = [("E1", "BOOL", "%I0.0"), ("E2", "BOOL", "%I0.1")];
    let mut s = Sim::new(build(&project(&tags, &[p, fb], 1000)));
    for k in 0..4 {
        s.set("E2", (k % 2 == 0) as u32);
        s.set("E1", 1);
        s.run(1);
        s.set("E1", 0);
        s.run(1);
    }
    assert_eq!(s.get("P.a.n"), 4);
    assert_eq!(s.get("P.b.n"), 2);
}

#[test]
fn erreurs_de_types_claires() {
    let p = project(&[("X", "INT", "%I0.0")], &["PROGRAM P END_PROGRAM"], 1000);
    assert!(errors_of(&p)[0].contains("ne correspond pas"));
    let p = project(&[], &["PROGRAM P VAR i : INT; r : REAL; END_VAR i := r; END_PROGRAM"], 1000);
    assert!(errors_of(&p)[0].contains("REAL_TO_INT"));
    let p = project(&[], &["PROGRAM P VAR i : INT; END_VAR i := 40000; END_PROGRAM"], 1000);
    assert!(errors_of(&p)[0].contains("hors de la plage INT"));
    let p = project(&[("Q", "BOOL", "%Q9.0")], &["PROGRAM P END_PROGRAM"], 1000);
    assert!(errors_of(&p)[0].contains("n'existe pas"));
}

#[test]
fn symboles_et_entrees_sorties() {
    let c = build(&project(
        &[("Bp", "BOOL", "%I0.3"), ("Mesure", "INT", "%IW2")],
        &["PROGRAM P VAR t : TON; END_VAR END_PROGRAM"],
        1000,
    ));
    assert_eq!(c.resolve("\"Bp\"").unwrap().1, Ty::Bool);
    assert_eq!(c.resolve("p.T.et").unwrap().1, Ty::Time);
    let di = c.io.iter().find(|io| io.kind == IoKind::Di).unwrap();
    assert_eq!(di.pin, 5);
    let ai = c.io.iter().find(|io| io.kind == IoKind::Ai).unwrap();
    assert_eq!(ai.pin, 27);
}

/// Téléchargement par trames, démarrage, lecture et forçage : exactement le
/// chemin suivi par l'IDE avec le simulateur ou le vrai automate.
#[test]
fn protocole_complet() {
    use takt_vm::proto::{self, FrameDecoder, Status};
    let c = build(&project(
        &[("Sortie", "BOOL", "%Q0.0"), ("Entree", "BOOL", "%I0.0")],
        &["PROGRAM P \"Sortie\" := \"Entree\"; END_PROGRAM"],
        1000,
    ));
    let mut rt = Box::new(Rt::new());
    let mut dec = FrameDecoder::new();
    let mut call = |rt: &mut Rt, cmd: u8, payload: &[u8]| -> (u8, Vec<u8>) {
        // Aller-retour complet : encodage, décodage octet par octet, traitement, réponse
        let mut wire = [0u8; proto::MAX_FRAME];
        let n = proto::encode_frame(cmd, payload, &mut wire);
        let frame = wire[..n].iter().find_map(|b| dec.push(*b)).expect("trame décodée");
        let mut resp = [0u8; proto::MAX_PAYLOAD];
        let (rc, len) = rt.handle(frame.cmd, frame.payload(), &mut resp);
        (rc, resp[..len].to_vec())
    };
    assert_eq!(call(&mut rt, proto::PING, &[]).1[..4], *b"TAKT");
    assert_eq!(call(&mut rt, proto::START, &[]).0, proto::ERR);
    call(&mut rt, proto::DL_BEGIN, &(c.image.len() as u32).to_le_bytes());
    for (k, chunk) in c.image.chunks(proto::CHUNK).enumerate() {
        let mut p = ((k * proto::CHUNK) as u32).to_le_bytes().to_vec();
        p.extend_from_slice(chunk);
        assert_eq!(call(&mut rt, proto::DL_CHUNK, &p).0, proto::DL_CHUNK | 0x80);
    }
    let (rc, crc) = call(&mut rt, proto::DL_END, &[]);
    assert_eq!(rc, proto::DL_END | 0x80);
    assert_eq!(u32::from_le_bytes(crc.try_into().unwrap()), c.crc);
    assert_eq!(call(&mut rt, proto::START, &[]).0, proto::START | 0x80);
    // Téléchargement refusé en RUN
    assert_eq!(call(&mut rt, proto::DL_BEGIN, &[0, 0, 0, 0]).1[0], proto::E_NOT_STOPPED);

    let (entree, _) = c.resolve("Entree").unwrap();
    let (sortie, _) = c.resolve("Sortie").unwrap();
    let mut force = entree.to_le_bytes().to_vec();
    force.push(1);
    force.extend_from_slice(&1u32.to_le_bytes());
    call(&mut rt, proto::FORCE, &force);
    let mut pin_out = 0;
    rt.cycle(0, |_| 0, |_, v| pin_out = v);
    rt.record_timing(12, 1);
    assert_eq!(pin_out, 1, "l'entrée forcée à 1 doit recopier la sortie");
    let mut read = vec![2u8];
    read.extend_from_slice(&entree.to_le_bytes());
    read.extend_from_slice(&sortie.to_le_bytes());
    assert_eq!(call(&mut rt, proto::READ, &read).1, vec![1, 0, 0, 0, 1, 0, 0, 0]);
    let st = Status::decode(&call(&mut rt, proto::STATUS, &[]).1).unwrap();
    assert_eq!((st.mode, st.scans, st.n_forces, st.exec_max_us, st.program_crc), (2, 1, 1, 12, c.crc));
}
