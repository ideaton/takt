<p align="center"><img src="docs/logo.svg" width="96" alt="Logo Takt"></p>

# Takt

**Un automate programmable à 5 €, avec un temps de cycle prouvé avant le téléchargement.**

Takt, c'est un IDE façon TIA Portal (Takt Studio) et un runtime pour le
Raspberry Pi RP2350 (Pico 2 et cartes compatibles, Waveshare…). Il est écrit
entièrement en Rust. On programme en Structured Text (IEC 61131-3, syntaxe SCL
de Siemens acceptée). Le compilateur **refuse tout programme dont il ne peut
pas prouver qu'il tient dans le temps de cycle**.

> **État :** prototype. Le compilateur, la VM, le simulateur et l'IDE sont
> testés sur PC. Le firmware RP2350 compile, mais **n'a encore jamais tourné
> sur une vraie carte**.

## Pourquoi « déterministe » ?

Un ESP32 ou un Arduino exécute le code depuis la flash, à travers un cache, au
milieu des interruptions Wi-Fi : le temps de cycle varie et personne ne sait
dire combien il vaut au pire. Takt attaque le problème à trois niveaux.

1. **Le langage.** Pas de `WHILE`/`REPEAT`, des `FOR` à bornes constantes,
   pas de récursion, pas d'allocation. Division par zéro et index hors bornes
   donnent un résultat défini (et un drapeau de diagnostic), jamais un plantage.
2. **Le compilateur.** Il calcule le **WCET** (temps d'exécution au pire cas)
   de chaque bloc à partir du coût de chaque instruction. Il ajoute la partie
   fixe du cycle (E/S, forçage, communication) et compare le total au temps de
   cycle configuré. Si ça ne tient pas, la compilation échoue avec un message
   clair. Le runtime fait respecter cette borne : un programme qui la
   dépasserait passe en défaut au lieu de déborder.
3. **Le matériel.** Sur le RP2350, le cœur 1 ne fait **que** l'automate. Son
   code est exécuté depuis la SRAM (pas de cache flash) et il est cadencé par
   un timer matériel en attente active. Le cœur 0 s'occupe de l'USB. Voir
   [firmware/takt-rp2350/README.md](firmware/takt-rp2350/README.md) pour le
   détail et les limites.

```
$ taktc examples/convoyeur.takt
info [Appareil] compilation réussie : 137 instructions, 48 mots mémoire, temps de cycle garanti 76 µs ≤ 1000 µs

WCET par bloc :
  Main                         3564 cycles      24 µs
  partie fixe du cycle         7800 cycles      52 µs
  total garanti : 76 µs / cycle 1000 µs (8 %)
```

## Takt Studio (l'IDE)

![Éditeur de Takt Studio](docs/editeur.png)

Les notions et les raccourcis de TIA Portal, dans une interface épurée façon
Apple (police Inter, icônes Lucide, thème clair et sombre) :

- **Barre latérale** : appareil, variables API, blocs de programme (OB / FB / FC),
  tables de visualisation et de forçage, état de la liaison.
- **Configuration de l'appareil** : choix de la carte, schéma des broches,
  temps de cycle, jauge « temps de cycle garanti ».
- **Éditeur ST** : coloration, erreurs dans la marge, carte « Instructions »
  qui insère TON/CTU/R_TRIG… et déclare l'instance (multi-instance).
- **Variables API** : `%I0.0`, `%Q0.1`, `%IW0`, `%QW2`, `%M`, ou sans adresse.
- **En ligne** : chargement, RUN/STOP, visualisation des valeurs dans le code
  (lunettes), tables de visualisation, modification et forçage, badge
  « programme identique / différent ».
- **Diagnostic** : WCET prouvé et temps mesuré (min/max), gigue, dépassements,
  histogramme.
- **PLCSIM intégré** : le simulateur exécute exactement le runtime du
  firmware, avec des interrupteurs pour les entrées et des voyants pour les sorties.

![Visualisation en ligne avec le simulateur](docs/visualisation.png)

| Raccourci | Action |
|---|---|
| Ctrl+B | Compiler |
| Ctrl+L | Charger dans l'appareil |
| Ctrl+K / Ctrl+M | Liaison en ligne / hors ligne |
| Ctrl+T | Visualisation on/off |
| Ctrl+S | Enregistrer |

## Langage supporté

- Blocs : `ORGANIZATION_BLOCK` (ou `PROGRAM`), `FUNCTION_BLOCK`, `FUNCTION`.
- Types : `BOOL`, `INT` (16 bits), `DINT`, `REAL`, `TIME`, `ARRAY[a..b] OF …`.
- Instructions : `:=`, `IF/ELSIF/ELSE`, `CASE`, `FOR … BY`, `RETURN`.
- Blocs standard : `TON`, `TOF`, `TP`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`,
  `SR`, `RS`.
- Fonctions : `ABS`, `MIN`, `MAX`, `LIMIT`, `SEL`, `TRUNC`, `ROUND`,
  `NORM_X`, `SCALE_X`, conversions `X_TO_Y`.
- Syntaxe Siemens : `#local`, `"Variable API"`, `{ attributs }` ignorés.
- Variables système : `FirstScan`, `SysTime`.

Les FB et FC utilisateur sont développés en ligne à chaque appel, ce qui rend
le pire cas exact et interdit la récursion par construction.

## Organisation du dépôt

| Dossier | Contenu |
|---|---|
| `crates/takt-vm` | VM `no_std` : bytecode, blocs standard, runtime, protocole série. Partagée par le firmware et le simulateur. |
| `crates/takt-compiler` | Lexer, parser, analyse, génération du bytecode, calcul du WCET ; outil en ligne de commande `taktc`. |
| `crates/takt-ide` | Takt Studio (egui). |
| `firmware/takt-rp2350` | Firmware RP2350 (deux cœurs, exécution en SRAM). |
| `examples/` | Projet d'exemple : convoyeur avec moteur, comptage et niveau analogique. |
| `tools/logo.py` | Génère le logo (SVG, PNG, ICO). |

## Compiler et lancer

```bash
cargo run --release -p takt-ide --bin takt-studio
```

```bash
cargo run -p takt-compiler --bin taktc -- examples/convoyeur.takt
```

```bash
cargo test --workspace
```

Firmware (voir son README pour le flashage) :

```bash
cd firmware/takt-rp2350 && cargo build --release
```

## Limites connues

- Les coûts d'instruction (`crates/takt-vm/src/cost.rs`) sont des majorants
  **estimés**. Il faut les calibrer sur carte : le firmware mesure chaque cycle
  et l'IDE affiche le prouvé à côté du mesuré.
- Le programme téléchargé n'est pas encore sauvegardé en flash : il est perdu
  à la mise hors tension.
- Le firmware ne vérifie pas encore la table d'E/S reçue (broche inexistante
  ou utilisée deux fois).
- Pas encore de CONT/LOG (LD/FBD), de `VAR_IN_OUT`, de chaînes, ni de
  Modbus/Ethernet.
- L'identifiant USB est celui de test de pid.codes (1209:0001).

Captures d'écran de l'interface, rendues hors écran dans `target/captures` :

```bash
cargo test -p takt-ide captures -- --ignored
```

## Licence

GPL-3.0, voir [LICENSE](LICENSE). Police Inter (SIL OFL 1.1) et icônes Lucide
(ISC), voir `crates/takt-ide/assets/fonts`.
