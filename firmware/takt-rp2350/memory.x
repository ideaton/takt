/* Carte mémoire de Takt sur RP2350 (Pico 2).
 *
 * La SRAM principale est découpée par moitié pour que les deux coeurs ne se
 * partagent pas les mêmes bancs pendant le cycle automate :
 *   - RAM    : SRAM0-3 (0x20000000, 256 Ko, entrelacée sur 4 bancs) -> coeur 0
 *              (.data/.bss/.uninit de cortex-m-rt, pile du coeur 0, boîtes
 *              aux lettres entre coeurs) ;
 *   - PLCRAM : SRAM4-7 (0x20040000, 256 Ko, entrelacée sur 4 bancs) -> coeur 1
 *              (code de la boucle automate et de takt_vm, Runtime) ;
 *   - SCRATCH: SRAM8-9 (0x20080000, 2 x 4 Ko, non entrelacée) -> pile du coeur 1.
 */
MEMORY {
    FLASH   : ORIGIN = 0x10000000, LENGTH = 2048K
    RAM     : ORIGIN = 0x20000000, LENGTH = 256K
    PLCRAM  : ORIGIN = 0x20040000, LENGTH = 256K
    SCRATCH : ORIGIN = 0x20080000, LENGTH = 8K
}

SECTIONS {
    /* Bloc IMAGE_DEF lu par la bootrom : doit être dans les 4 premiers Ko. */
    .start_block : ALIGN(4)
    {
        __start_block_addr = .;
        KEEP(*(.start_block));
        KEEP(*(.boot_info));
    } > FLASH

    /* Tout ce que le coeur 1 exécute ou lit pendant le cycle : stocké en
     * flash, exécuté en SRAM4-7 après recopie par `main` (voir
     * `copy_plc_ram`). Placée AVANT .text dans le script pour que ces motifs
     * gagnent sur le `*(.text .text.*)` générique de cortex-m-rt.
     *
     * Les motifs portent sur le nom de section, qui contient le symbole mangé
     * (`-ffunction-sections`) : avec le mangling v0 (défaut de rustc stable
     * actuel), `takt_vm::exec::scan` est dans
     * `.text._RNvNtCs<hash>_7takt_vm4exec4scan`, et avec l'ancien mangling
     * dans `.text._ZN7takt_vm4exec4scan17h<hash>E` : les deux contiennent
     * `takt_vm4exec`. Ça marche avec LTO, où il n'y a plus qu'un seul
     * fichier objet. `tools/check-ram.py` vérifie le résultat sur l'ELF. */
    .plc_ram : ALIGN(8)
    {
        __plc_ram_start = .;
        /* takt_vm : interpréteur, blocs standard, runtime, chargement, statut.
         * FrameDecoder et encode_frame (utilisés par le coeur 0) restent en flash. */
        *(.text.*takt_vm7runtime* .text.*takt_vm4exec* .text.*takt_vm2fb*)
        *(.text.*takt_vm4cost* .text.*takt_vm5image* .text.*takt_vm5instr*)
        *(.text.*takt_vm5proto*Status*)
        /* Boucle du coeur 1 et accès matériels (modules plc et hw du firmware). */
        *(.text.*takt_rp23503plc* .text.*takt_rp23502hw*)
        /* Fonctions de compiler_builtins appelées par le code ci-dessus
         * (memcpy/memset, division 64 bits...). */
        *(.text.memcpy .text.memset .text.memmove .text.memcmp)
        *(.text.__aeabi_* .text.*compiler_builtins*)
        *(.text.__udiv* .text.__umod* .text.__div* .text.__mod*)
        /* Fonctions génériques de core parfois non inlinées (profil dev). */
        *(.text.*4core5slice*copy_from_slice*)
        *(.plc_text .plc_text.*)
        /* Constantes et données. Les constantes anonymes (`.Lanon.*` : tables
         * de décodage, valeurs initiales comme STATS0, emplacements de
         * panique) n'ont pas de nom de crate : elles viennent toutes ici. Le
         * coeur 0 n'en lit qu'à l'énumération USB ou en cas de panique.
         * Idem pour les constantes fusionnables (`.rodata.cst*`, `.rodata.str*`).
         * `.L.crctable` (table du CRC-8 générée par LLVM, lue par le décodeur
         * de trames du coeur 0) reste en flash. */
        *(.rodata.*takt_vm* .rodata.*takt_rp23503plc* .rodata.*takt_rp23502hw*)
        *(.rodata..Lanon.* .rodata..Lswitch.table.* .rodata.cst* .rodata.str*)
        *(.plc_data .plc_data.*)
        . = ALIGN(4);
        __plc_ram_end = .;
    } > PLCRAM AT > FLASH
    __plc_ram_load = LOADADDR(.plc_ram);
} INSERT AFTER .vector_table;

/* .text commence après la copie en flash de .plc_ram. */
_stext = ALIGN(LOADADDR(.plc_ram) + SIZEOF(.plc_ram), ALIGNOF(.text));

/* Pile du coeur 1 : SRAM8 + SRAM9, bancs que le coeur 0 n'utilise pas. */
__core1_stack_bottom = ORIGIN(SCRATCH);
__core1_stack_top = ORIGIN(SCRATCH) + LENGTH(SCRATCH);
