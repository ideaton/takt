# takt-rp2350 — firmware Takt pour RP2350 (Pico 2)

Firmware de l'automate Takt pour le Raspberry Pi RP2350 (cible
`thumbv8m.main-none-eabihf`, coeurs Cortex-M33 à 150 MHz). Il fait tourner
`takt-vm`, exactement le même code que le simulateur de l'IDE.

> **État : compile, jamais testé sur une carte.** Rien de ce qui suit n'a
> encore été mesuré sur du matériel.

## Les deux coeurs

| Coeur | Rôle | Exécute depuis |
|---|---|---|
| **1** | L'automate, et rien d'autre : attente du tick, cycle, statistiques, chien de garde, une trame du protocole au plus | SRAM4-7 (code) + SRAM8-9 (pile) |
| **0** | La communication : USB CDC ↔ décodeur de trames ↔ boîtes aux lettres ; LED d'état | flash (XIP) + SRAM0-3 |

### Coeur 1 (`src/plc.rs`)

Boucle sans fin :

1. **Attente active** sur le compteur 1 MHz de TIMER0 jusqu'à l'instant prévu
   `next` (pas d'interruption, pas de `wfi`).
2. `late_us = début réel − instant prévu`.
3. `Runtime::cycle` : lecture des entrées → programme → écriture des sorties.
   Les GPIO, l'ADC et le PWM sont lus et écrits par accès directs aux
   registres (`src/hw.rs`), sans HAL ni section critique.
4. `exec_us` mesuré, `record_timing(exec_us, late_us)`, mode publié pour la
   LED, chien de garde rechargé.
5. **Au plus une trame** venue du coeur 0 est traitée (`Runtime::handle`,
   travail borné, voir `cost::COMMAND_SERVICE`).
6. `next += cycle_us`.

En STOP (ou sans programme) la boucle continue à la même période : les trames
sont servies et les sorties maintenues à 0.

Après un `DL_END` réussi (forcément en STOP), les broches sont reconfigurées
d'après la nouvelle table d'E/S, le chien de garde et la période sont
recalculés et la grille de temps repart de l'instant présent.

### Coeur 0 (`src/main.rs`)

Initialise les horloges (150 MHz, comme le suppose `takt_vm::cost`), sort les
périphériques de reset, démarre le chien de garde, lance le coeur 1, puis
boucle : `usb_dev.poll` → octets reçus → `FrameDecoder` → case `REQUEST` ;
case `RESPONSE` → `encode_frame` → USB.

Si une trame décodée ne peut pas encore être déposée (le coeur 1 n'a pas pris
la précédente), le coeur 0 cesse de lire l'USB : l'hôte est freiné par le
contrôle de flux USB, rien n'est perdu.

### Boîtes aux lettres (`src/mailbox.rs`)

Deux cases d'une trame chacune (`REQUEST` coeur 0 → coeur 1, `RESPONSE`
coeur 1 → coeur 0), un producteur et un consommateur par case, un drapeau
atomique (`Release`/`Acquire`) qui donne la case à l'un ou à l'autre. Aucun
verrou, aucune attente : le coeur 1 regarde une fois par cycle et passe son
chemin si rien n'est prêt. Le coeur 1 traite la requête **en place** et écrit
la réponse directement dans la case `RESPONSE`.

## Comment le déterminisme est obtenu

### Ce que le coeur 1 ne fait jamais

- **Aucune interruption** n'est activée sur le coeur 1 : rien ne peut
  préempter le cycle.
- **Aucun verrou** (ni spinlock, ni `critical_section`) : le coeur 0 peut
  prendre les siens (le HAL USB en utilise), le coeur 1 ne les attend jamais.
- **Aucune lecture de la flash** pendant la boucle : tout son code et ses
  constantes sont en SRAM (détails ci-dessous). Les défauts de cache XIP et
  les éventuelles écritures en flash n'ont donc aucun effet sur lui.

### Code en SRAM

La section `.plc_ram` (`memory.x`) est stockée en flash et **recopiée en
SRAM4-7 au tout début de `main`** (`copy_plc_ram`, mot par mot en volatile,
avant tout autre code ; le reste de `main` est dans une fonction séparée non
inlinée pour qu'aucun `memcpy`/`memset` ne soit avancé avant la copie). Elle
est placée avant `.text` dans le script, pour que ses motifs passent avant le
`*(.text .text.*)` de cortex-m-rt.

Les motifs portent sur le **nom des sections**, qui contient le symbole mangé
(`-ffunction-sections`, LTO à une seule unité de compilation). Exemple :
`takt_vm::exec::scan` est dans `.text._RNvNtCs…_7takt_vm4exec4scan` (mangling
v0, défaut actuel) ou `.text._ZN7takt_vm4exec4scan…` (ancien mangling) ; les
deux contiennent `takt_vm4exec`.

Contenu de `.plc_ram` (build release actuel, 116 628 octets) :

- **takt_vm** : modules `runtime`, `exec`, `fb`, `cost`, `image`, `instr`, et
  `proto::Status` (l'encodage du statut, utilisé par `handle`). Dans le build
  release, `exec::scan` et `Runtime::cycle` sont inlinés dans `plc::run` ; il
  reste comme fonctions séparées `Runtime::reset_memory`, `ImageView::parse`,
  `ImageView::instr`, `ImageView::init`, `Instr::slot_range`,
  `Status::encode`.
- **firmware** : modules `plc` (`run`, `service_one`, `PinMap`, `MsClock`) et
  `hw` (accès registres).
- **compiler_builtins** utilisés par le code ci-dessus : `memcpy`, `memmove`,
  `__aeabi_memcpy*`, `__aeabi_memclr*`, `__aeabi_uldivmod`,
  `u64_div_rem`, `__udivmoddi4` (la division 64 bits de `record_timing`).
- `core::slice::copy_from_slice` quand il n'est pas inliné (profil dev).
- **Constantes** : `.rodata` de `takt_vm`, `plc`, `hw`, et toutes les
  constantes anonymes (`.Lanon.*`, `.Lswitch.table.*`, `.rodata.cst*`,
  `.rodata.str*`) : tables de décodage des instructions, valeur initiale des
  statistiques (`STATS0`), `b"TAKT"`, emplacements de panique…
- **Données** : le `Runtime<4096, 4096, 40960>` lui-même (94 700 octets :
  code 32 Ko, mémoire 16 Ko, tampon de téléchargement 40 Ko…).

**Ce qui reste en flash et peut être exécuté par le coeur 1 :**

- `core1_startup` du HAL (`rp235x_hal::multicore`), une seule fois au
  démarrage, avant d'entrer dans `plc::run` ;
- les fonctions de **panique** (`panic_bounds_check`, `slice_index_fail`,
  `panic_const_div_by_zero`…) et le gestionnaire de panique : chemins
  terminaux qui ne devraient jamais être pris (la VM valide l'image au
  chargement), et qui redémarrent la puce.

`tools/check-ram.py` le vérifie sur l'ELF : il désassemble `.plc_ram` et
signale tout branchement hors de la section (y compris les « long thunks » vers
la flash) et toute adresse de constante en flash chargée par le code :

```sh
# binutils arm-none-eabi (ici ceux de PlatformIO)
OBJDUMP_PREFIX=~/.platformio/packages/toolchain-gccarmnoneeabi@1.70201.0/bin/arm-none-eabi- \
  python tools/check-ram.py
# … OK : aucune dépendance flash hors panique
```

À relancer après chaque changement de `takt-vm`, de compilateur ou de profil :
le placement dépend de l'inlining.

### Mémoire séparée par coeur

| Zone | Adresses | Bancs | Utilisée par |
|---|---|---|---|
| `RAM` | 0x20000000, 256 Ko | SRAM0-3 (entrelacés) | coeur 0 : `.data`, `.bss`, pile ; cases des boîtes aux lettres |
| `PLCRAM` | 0x20040000, 256 Ko | SRAM4-7 (entrelacés) | coeur 1 : `.plc_ram` (code, constantes, Runtime) |
| `SCRATCH` | 0x20080000, 8 Ko | SRAM8-9 | pile du coeur 1 (garde MSPLIM posée par le HAL) |

Les deux moitiés de la SRAM principale sont entrelacées séparément (SRAM0-3
d'un côté, SRAM4-7 de l'autre, d'après la datasheet RP2350) : pendant le
cycle, le coeur 1 n'accède qu'à ses bancs, à la SIO (privée à chaque coeur) et
aux périphériques APB. En plus, **le coeur 1 a la priorité sur le bus**
(`BUSCTRL.BUS_PRIORITY.PROC1 = 1`) : il gagne tout arbitrage simultané.

### Grille de temps et débordements

- Les instants de démarrage sont sur une **grille fixe** (`next += cycle_us`) :
  aucune dérive, la gigue mesurée (`late_us`) n'est que la latence de sortie
  de l'attente active (quelques cycles d'horloge + une lecture APB).
- `SysTime` (en ms) avance de l'écart exact entre deux instants **prévus**
  (reste en µs conservé) : les temporisations (TON, TOF, TP) voient un temps
  régulier.
- **Débordement** (le cycle et la trame servie dépassent la période) : le cycle
  suivant démarre aussitôt, en retard, et ce retard est compté. Si une période
  entière ou plus est perdue, elle n'est **pas rattrapée en rafale** : `next`
  est recalé sur le dernier point de la grille, et le retard du cycle suivant
  est compté depuis le premier instant raté (il apparaît donc dans
  `late_max_us` et `overruns`).

### Chien de garde

Armé par le coeur 0 au démarrage (500 ms), puis rechargé par le **coeur 1
seul** après chaque cycle, avec `2 × cycle_us + 100 ms` (plafonné à 16,7 s,
maximum du registre). Si le coeur 1 se bloque, la puce redémarre : toutes les
sorties retombent (reset des GPIO). Une panique sur l'un des coeurs redémarre
aussi la puce. Les deux bits « pause en debug » sont actifs (valeur de reset).

## Ce qui est garanti, et ce qui ne l'est pas

Garanti par construction (sous réserve de tests sur carte) :

- pas de préemption, pas d'attente d'un verrou, pas d'accès flash pendant le
  cycle ;
- traitement d'au plus une trame par cycle, à coût borné, en dehors de la
  mesure `exec_us` ;
- pas de dérive de la période, pas de rafale après un débordement.

**Pas garanti / limites connues :**

- **Contention SRAM résiduelle.** Le coeur 0 lit encore des choses placées
  dans les bancs du coeur 1 : `memcpy`/`memset` de compiler_builtins (appelés
  par la pile USB à chaque paquet) et les constantes anonymes (descripteurs
  USB à l'énumération). Un même symbole ne peut pas exister en deux
  exemplaires ; la priorité bus du coeur 1 limite l'effet, mais il n'est pas
  nul en théorie et **n'a pas été mesuré**. De même le coeur 1 touche SRAM0-3
  pour les boîtes aux lettres et le mode publié, mais seulement **après** la
  mesure de `exec_us`.
- **Le temps d'exécution dépend du chemin pris dans le programme** (branches,
  nombre d'instructions exécutées) : il est borné par le WCET calculé par le
  compilateur, pas constant.
- **ADC : environ 2 µs par entrée `Ai`** (conversion unique, 96 cycles à
  48 MHz, attente bornée à 2000 lectures du registre). Ce coût n'est pas dans
  `cost::PER_IO` (40 cycles CPU ≈ 0,27 µs) : le WCET annoncé par la VM sous-
  estime le cycle d'environ 2 µs par `Ai`.
- La table de `takt_vm::cost` suppose code et données en SRAM à 150 MHz :
  c'est le cas ici, mais elle n'a pas été comparée à des mesures.
- Les accès APB (TIMER0, ADC, PWM, chien de garde) passent par le pont APB ;
  le coeur 0 n'y touche plus après l'initialisation (la LED est cadencée par
  le compteur DWT du coeur 0), mais un futur ajout côté coeur 0 pourrait
  réintroduire du partage.
- **Le programme n'est pas conservé** : il vit en SRAM et disparaît à chaque
  reset (y compris par le chien de garde). Pas de démarrage automatique.
- `cycle_us` au-delà d'environ 8 s : le chien de garde (16,7 s maximum) n'est
  plus assez long.
- Les profils autres que `release` ne sont pas visés par la garantie (relancer
  `tools/check-ram.py` sur l'ELF voulu).

## Entrées / sorties

Cible : **RP2350A** (Pico 2), GPIO 0 à 29.

| Type | Broches | Réalisation |
|---|---|---|
| `Di` (BOOL) | GPIO 0-29 | SIO, entrée à trigger de Schmitt, **sans résistance interne** |
| `Dq` (BOOL) | GPIO 0-29 | SIO, sortie 4 mA, à 0 au démarrage et hors RUN |
| `Ai` (INT) | GPIO 26-29 (voies ADC 0-3) | conversion unique 12 bits → `(brut × 27648 + 2047) / 4095` |
| `Aq` (INT) | GPIO 0-29 | PWM : TOP = 27647, diviseur 1 → la valeur INT est directement le rapport cyclique (27648 = 100 %), fréquence 5,43 kHz ; valeurs hors 0..27648 bornées |

- Les entrées TOR n'ont pas de pull-down interne (errata RP2350-E9 : le
  pull-down interne peut laisser l'entrée bloquée vers 2 V) : prévoir une
  résistance externe (≤ 8,2 kΩ) ou un étage d'entrée qui force le niveau.
- Deux sorties PWM sur la même tranche (GPIO 2n et 2n+1, ou à 16 broches
  d'écart) partagent le même diviseur et la même période, ce qui est le cas ici.
- Une broche invalide (numéro ≥ 30, `Ai` hors 26-29) est ignorée : lue à 0,
  écriture sans effet. Une broche déclarée avec deux usages différents est
  laissée inerte. **Rien de cela n'est signalé à l'IDE** (voir « Évolutions
  souhaitées de takt-vm »).
- Si le programme utilise GPIO 25, la LED de la carte lui appartient et le
  coeur 0 n'y touche plus.

## LED d'état (GPIO 25)

| LED | Mode |
|---|---|
| allumée fixe | RUN |
| clignotement lent (1 Hz) | STOP ou pas de programme |
| clignotement rapide (5 Hz) | FAULT |

## USB

Port série CDC-ACM, VID:PID `1209:0001` (PID de test de pid.codes, à remplacer
par un PID attribué), fabricant « Ideaton », produit « Takt PLC ». Le débit
annoncé est ignoré. Trames : voir `takt_vm::proto`.

## Compiler et flasher

```sh
rustup target add thumbv8m.main-none-eabihf   # fait aussi par rust-toolchain.toml
cd firmware/takt-rp2350
cargo build --release
python tools/check-ram.py                      # vérification du placement en SRAM
```

Le crate est exclu de l'espace de travail racine (il ne compile que pour la
cible embarquée) et a son propre `target/`.

**Avec picotool** (carte en BOOTSEL : bouton BOOTSEL maintenu au branchement) :

```sh
cargo run --release
# = picotool load -u -v -x -t elf target/thumbv8m.main-none-eabihf/release/takt-rp2350
```

**Par UF2** (glisser-déposer sur le disque `RP2350` en BOOTSEL) :

```sh
picotool uf2 convert target/thumbv8m.main-none-eabihf/release/takt-rp2350 -t elf takt.uf2 --family rp2350-arm-s
```

`elf2uf2-rs` convient aussi s'il connaît la famille RP2350 (`rp2350-arm-s`,
id 0xe48bff59) ; les anciennes versions ne produisent que des UF2 RP2040, que
la bootrom du RP2350 refuse.

Le bloc `IMAGE_DEF` (exécutable Arm sécurisé, section `.start_block`, dans
les 4 premiers Ko de la flash) est fourni par `rp235x_hal::block::ImageDef`.

## Évolutions souhaitées de takt-vm

- **Coût d'E/S par type** dans `cost` (au moins `Ai` ≈ 2 µs de conversion ADC)
  pour que le WCET couvre le cycle réel.
- **Validation de la table d'E/S par la cible** au `DL_END` (broches
  existantes, ADC possible, pas de double usage), avec un code d'erreur
  `BadIo` renvoyé à l'IDE, ou au minimum un bit de diagnostic posé par le
  firmware (`Runtime::set_diag`).
- **Conservation en flash** du programme et démarrage automatique (hors de la
  VM, mais l'image validée est déjà disponible dans le tampon de staging).
