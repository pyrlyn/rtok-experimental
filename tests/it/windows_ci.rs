// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Guards for what keeps the Windows CI job honest (T82, T93, T108).

use std::path::PathBuf;

/// Return the repository root used by each Windows CI guard.
fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// T93: `build.rs` links the bins with an 8 MiB main-thread stack on Windows. Without it
/// the debug `rtok.exe` overflowed its 1 MiB default on nearly every command — this names
/// the cause directly instead of through a hundred crashed e2e tests.
#[cfg(windows)]
#[test]
fn rtok_exe_reserves_an_8_mib_main_thread_stack() {
    let exe = std::fs::read(env!("CARGO_BIN_EXE_rtok")).unwrap();
    let u32_at = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap());
    let pe = u32_at(0x3c) as usize;
    assert_eq!(&exe[pe..pe + 4], b"PE\0\0", "not a PE image");
    // COFF header is 20 bytes; SizeOfStackReserve sits at offset 72 of the optional
    // header in both PE32 (u32) and PE32+ (u64).
    let opt = pe + 4 + 20;
    let reserve = match u16::from_le_bytes([exe[opt], exe[opt + 1]]) {
        0x20b => u64::from_le_bytes(exe[opt + 72..opt + 80].try_into().unwrap()),
        _ => u64::from(u32_at(opt + 72)),
    };
    assert!(reserve >= 8 << 20, "stack reserve is {reserve} bytes");
}

/// T82: files compared byte for byte must reach the working tree as LF on every OS;
/// `.gitattributes` (`eol=lf`) is what beats Windows' `core.autocrlf=true`.
#[test]
fn byte_compared_files_are_lf_in_the_working_tree() {
    let mut crlf = Vec::new();
    for dir in ["tests/trycmd", "skills"] {
        for entry in ignore::WalkBuilder::new(repo().join(dir)).build() {
            let path = entry.unwrap().into_path();
            if path.is_file() && std::fs::read(&path).unwrap().contains(&b'\r') {
                crlf.push(path);
            }
        }
    }
    assert!(crlf.is_empty(), "CR bytes in {crlf:?}");
}
