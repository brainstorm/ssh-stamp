// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! List known boards and chips.

use crate::board;

pub fn run() {
    println!("Boards (firmware binary):");
    for board in board::BOARDS {
        println!(
            "    {:<38} {}  (+{})",
            board.name, board.target, board.toolchain
        );
    }

    println!();
    println!("Chips (library-only build):");
    for chip in board::CHIPS {
        println!(
            "    {:<38} {}  (+{})",
            chip.name, chip.target, chip.toolchain
        );
    }
}
