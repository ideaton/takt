#!/usr/bin/env python3
"""Vérifie que le code du coeur 1 (section .plc_ram) ne dépend pas de la flash.

Usage : python tools/check-ram.py [ELF] [préfixe des binutils]
  ELF     défaut : target/thumbv8m.main-none-eabihf/release/takt-rp2350
  préfixe défaut : variable OBJDUMP_PREFIX, sinon « arm-none-eabi- »

Désassemble .plc_ram et signale :
  - chaque branchement dont la cible est hors de .plc_ram (y compris les
    « long thunks » insérés par l'éditeur de liens vers la flash) ;
  - chaque constante d'adresse flash chargée par le code (mot de « literal
    pool » ou paire movw/movt) qui tombe dans une section chargée en flash ;
    une valeur movw/movt utilisée ensuite comme multiplicateur (division par
    une constante) n'est pas une adresse et est ignorée.
Les appels vers les fonctions de panique sont listés à part : ce sont des
chemins terminaux (le gestionnaire de panique redémarre la puce).
Code de sortie 1 si une dépendance hors panique est trouvée.
"""
import os
import re
import subprocess
import sys

ELF = sys.argv[1] if len(sys.argv) > 1 else "target/thumbv8m.main-none-eabihf/release/takt-rp2350"
PREFIX = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("OBJDUMP_PREFIX", "arm-none-eabi-")
PANIC = re.compile(r"panic|_fail|unwrap_failed|expect_failed|assert_failed")


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def main():
    lo = hi = None
    flash = []
    for line in run(PREFIX + "objdump", "-h", ELF).splitlines():
        f = line.split()
        if len(f) > 4 and re.fullmatch(r"[0-9a-f]{8}", f[2] or "") and f[1].startswith("."):
            size, lma = int(f[2], 16), int(f[4], 16)
            if 0x10000000 <= lma < 0x11000000 and size:
                flash.append((lma, lma + size))
            if f[1] == ".plc_ram":
                lo = int(f[3], 16)
                hi = lo + size
    if lo is None:
        sys.exit("section .plc_ram introuvable")

    thunks = {}
    for line in run(PREFIX + "nm", "-n", ELF).splitlines():
        f = line.split()
        if len(f) == 3 and "LongThunk_" in f[2]:
            thunks[int(f[0], 16)] = f[2].split("LongThunk_", 1)[1]

    func = "?"
    bad, panics = [], set()
    movw = {}
    pending = {}  # registre -> (valeur, fonction) d'un movt pas encore jugé

    def in_flash(v):
        return any(a <= v < b for a, b in flash)
    for line in run(PREFIX + "objdump", "-d", "--no-show-raw-insn", "-j", ".plc_ram", ELF).splitlines():
        m = re.match(r"^([0-9a-f]+) <(.+)>:$", line)
        if m:
            func = m.group(2)
            continue
        m = re.match(r"^\s*([0-9a-f]+):\s+(\S+)\s*(.*)$", line)
        if not m:
            continue
        op, args = m.group(2), m.group(3)
        if "LongThunk" in func:
            continue
        for reg in list(pending):
            if re.search(r"\b" + reg + r"\b", args):
                v, fn = pending.pop(reg)  # registre relu : mul = constante, sinon adresse
                if not re.match(r"^(u|s)?(mull|mul|mla|mlal|smmul|smlabb)", op):
                    bad.append(f"{fn}: adresse flash {v:#010x} (movw/movt)")
        b = re.match(r"^(b|bl|blx|b\.w|b[a-z]{2}(\.w|\.n)?|cbn?z)$", op)
        if b:
            t = re.search(r"\b([0-9a-f]{6,8}) <", args)
            if t:
                addr = int(t.group(1), 16)
                target = thunks.get(addr)
                if target is not None or not lo <= addr < hi:
                    name = target or args
                    (panics.add(name) if PANIC.search(name) else bad.append(f"{func}: {op} {name}"))
        if op == ".word":
            v = int(args.split()[0], 16)
            if in_flash(v):
                bad.append(f"{func}: constante flash {v:#010x}")
        mw = re.match(r"^(r\d+|ip|lr), #(\d+)", args)
        if op.startswith("movw") and mw:
            movw[mw.group(1)] = int(mw.group(2))
        if op.startswith("movt") and mw:
            v = (int(mw.group(2)) << 16) | movw.get(mw.group(1), 0)
            if in_flash(v):
                pending[mw.group(1)] = (v, func)
        elif mw and mw.group(1) in movw and not op.startswith("movw"):
            movw.pop(mw.group(1), None)
    for reg, (v, fn) in pending.items():
        bad.append(f"{fn}: adresse flash {v:#010x} (movw/movt)")

    print(f".plc_ram : {lo:#010x}..{hi:#010x} ({hi - lo} octets)")
    for p in sorted(panics):
        print(f"  panique (terminal, toléré) : {p}")
    for b in bad:
        print(f"  DEPENDANCE FLASH : {b}")
    print("OK : aucune dépendance flash hors panique" if not bad else f"{len(bad)} dépendance(s) flash")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
