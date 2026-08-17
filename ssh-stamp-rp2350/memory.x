/* SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
 * SPDX-FileCopyrightText: The Embassy contributors
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * RP2350A layout for the W6300-EVB-Pico2 (Pico 2 form factor).
 * Section layout follows the upstream embassy `examples/rp235x/memory.x`:
 * the boot ROM needs `.start_block` inside the first 4 KiB of flash.
 */

MEMORY {
    /*
     * Pico 2 (and this board) ship 4 MiB. Larger parts simply leave the
     * remainder unused. The last 4 KiB sector holds the ssh-stamp config
     * (see flash.rs::CONFIG_WINDOW), so keep the image below that.
     */
    FLASH : ORIGIN = 0x10000000, LENGTH = 4096K
    /*
     * RAM banks SRAM0-SRAM7, striped for even load.
     */
    RAM : ORIGIN = 0x20000000, LENGTH = 512K
    /*
     * Banks 8 and 9, directly mapped.
     */
    SRAM8 : ORIGIN = 0x20080000, LENGTH = 4K
    SRAM9 : ORIGIN = 0x20081000, LENGTH = 4K
}

SECTIONS {
    /* ### Boot ROM info
     *
     * Goes after .vector_table, to keep it in the first 4K of flash
     * where the Boot ROM (and picotool) can find it
     */
    .start_block : ALIGN(4)
    {
        __start_block_addr = .;
        KEEP(*(.start_block));
        KEEP(*(.boot_info));
    } > FLASH

} INSERT AFTER .vector_table;

/* move .text to start /after/ the boot info */
_stext = ADDR(.start_block) + SIZEOF(.start_block);

SECTIONS {
    /* ### Picotool 'Binary Info' Entries
     *
     * Picotool looks through this block (as we have pointers to it in our
     * header) to find interesting information.
     */
    .bi_entries : ALIGN(4)
    {
        /* We put this in the header */
        __bi_entries_start = .;
        /* Here are the entries */
        KEEP(*(.bi_entries));
        /* Keep this block a nice round size */
        . = ALIGN(4);
        /* We put this in the header */
        __bi_entries_end = .;
    } > FLASH
} INSERT AFTER .text;

SECTIONS {
    /* ### Boot ROM extra info
     *
     * Goes after everything in our program, so it can contain a signature.
     */
    .end_block : ALIGN(4)
    {
        __end_block_addr = .;
        KEEP(*(.end_block));
    } > FLASH

} INSERT AFTER .uninit;

PROVIDE(start_to_end = __end_block_addr - __start_block_addr);
PROVIDE(end_to_start = __start_block_addr - __end_block_addr);
