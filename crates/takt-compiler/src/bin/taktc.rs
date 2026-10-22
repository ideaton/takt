//! Compilateur en ligne de commande : `taktc projet.takt [-o image.bin] [--listing]`.

use std::path::PathBuf;
use std::process::ExitCode;

use takt_compiler::{Severity, compile, project::Project};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut output = None;
    let mut listing = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = args.next().map(PathBuf::from),
            "--listing" => listing = true,
            "-h" | "--help" => {
                println!("usage : taktc projet.takt [-o image.bin] [--listing]");
                return ExitCode::SUCCESS;
            }
            _ => input = Some(PathBuf::from(a)),
        }
    }
    let Some(input) = input else {
        eprintln!("usage : taktc projet.takt [-o image.bin] [--listing]");
        return ExitCode::from(2);
    };
    let project = match Project::load(&input) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let out = compile(&project);
    for d in &out.diags {
        let level = match d.severity {
            Severity::Error => "erreur",
            Severity::Warning => "avertissement",
            Severity::Info => "info",
        };
        if d.pos.line > 0 {
            println!("{level} [{}:{}:{}] {}", d.block, d.pos.line, d.pos.col, d.msg);
        } else {
            println!("{level} [{}] {}", d.block, d.msg);
        }
    }
    let Some(c) = out.compiled else {
        return ExitCode::FAILURE;
    };
    println!();
    println!("WCET par bloc :");
    for b in &c.wcet.blocks {
        println!("  {:<24} {:>8} cycles  {:>6} µs", b.name, b.cycles, takt_vm::cost::cycles_to_us(b.cycles));
    }
    println!(
        "  {:<24} {:>8} cycles  {:>6} µs",
        "partie fixe du cycle",
        c.wcet.overhead_cycles,
        takt_vm::cost::cycles_to_us(c.wcet.overhead_cycles)
    );
    println!(
        "  total garanti : {} µs / cycle {} µs ({:.0} %)",
        c.wcet.total_us,
        c.wcet.cycle_us,
        c.wcet.ratio() * 100.0
    );
    if listing {
        println!();
        for (i, ins) in c.code.iter().enumerate() {
            println!("{i:5}  {ins:?}");
        }
    }
    if let Some(path) = output {
        if let Err(e) = std::fs::write(&path, &c.image) {
            eprintln!("écriture de {} : {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("image écrite : {} ({} octets, CRC {:08X})", path.display(), c.image.len(), c.crc);
    }
    ExitCode::SUCCESS
}
