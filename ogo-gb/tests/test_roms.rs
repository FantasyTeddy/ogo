//! Runs public test ROMs from the collection at https://github.com/c-sp/game-boy-test-roms (v7.0),
//! unpacked into `ogo-gb/test-roms/` or the folder in `OGO_GB_TEST_ROMS`. Without them the tests
//! are skipped, except in CI (`CI` set), which downloads them.

use std::path::PathBuf;

use ogo_gb::{Cartridge, GameBoy, CLOCK_HZ};

fn rom_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("OGO_GB_TEST_ROMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-roms"));
    if dir.is_dir() {
        Some(dir)
    } else if std::env::var_os("CI").is_some() {
        panic!("test ROMs missing in {}", dir.display());
    } else {
        eprintln!("skipped: no test ROMs in {}", dir.display());
        None
    }
}

fn load(dir: &std::path::Path, name: &str) -> GameBoy {
    let rom = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let cart = Cartridge::new(rom).unwrap_or_else(|e| panic!("{name}: {e}"));
    GameBoy::new(cart)
}

/// Runs every ROM with `check`, which returns `Err` on failure; reports all failures together.
fn run_all(names: &[&str], check: fn(&std::path::Path, &str) -> Result<(), String>) {
    let Some(dir) = rom_dir() else { return };
    let failures: Vec<String> = names
        .iter()
        .filter_map(|name| check(&dir, name).err().map(|e| format!("{name}: {e}")))
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Blargg's ROMs print their result over the serial port, ending with "Passed" or "Failed".
fn blargg(dir: &std::path::Path, name: &str) -> Result<(), String> {
    let mut gb = load(dir, name);
    gb.capture_serial();
    let mut cycles = 0u64;
    while cycles < 120 * CLOCK_HZ as u64 {
        cycles += gb.step() as u64;
        let output = String::from_utf8_lossy(gb.serial_output());
        if output.contains("Passed") {
            return Ok(());
        }
        if output.contains("Failed") {
            return Err(output.trim().replace('\n', " | "));
        }
    }
    Err(format!(
        "timed out: {}",
        String::from_utf8_lossy(gb.serial_output()).trim()
    ))
}

/// Mooneye's ROMs end with `LD B,B`, with B, C, D, E, H, L = 3, 5, 8, 13, 21, 34 on success.
fn mooneye(dir: &std::path::Path, name: &str) -> Result<(), String> {
    let mut gb = load(dir, name);
    let mut cycles = 0u64;
    while cycles < 30 * CLOCK_HZ as u64 {
        if gb.peek(gb.registers().pc) == 0x40 {
            let r = gb.registers();
            let regs = [r.b, r.c, r.d, r.e, r.h, r.l];
            return if regs == [3, 5, 8, 13, 21, 34] {
                Ok(())
            } else {
                Err(format!("failed, registers {regs:?}"))
            };
        }
        cycles += gb.step() as u64;
    }
    Err("timed out".into())
}

#[test]
fn blargg_cpu() {
    run_all(
        &[
            "blargg/cpu_instrs/individual/01-special.gb",
            "blargg/cpu_instrs/individual/02-interrupts.gb",
            "blargg/cpu_instrs/individual/03-op sp,hl.gb",
            "blargg/cpu_instrs/individual/04-op r,imm.gb",
            "blargg/cpu_instrs/individual/05-op rp.gb",
            "blargg/cpu_instrs/individual/06-ld r,r.gb",
            "blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb",
            "blargg/cpu_instrs/individual/08-misc instrs.gb",
            "blargg/cpu_instrs/individual/09-op r,r.gb",
            "blargg/cpu_instrs/individual/10-bit ops.gb",
            "blargg/cpu_instrs/individual/11-op a,(hl).gb",
            "blargg/cpu_instrs/cpu_instrs.gb",
            "blargg/instr_timing/instr_timing.gb",
        ],
        blargg,
    );
}

#[test]
fn mooneye_cpu_and_timer() {
    run_all(
        &[
            "mooneye-test-suite/acceptance/bits/reg_f.gb",
            "mooneye-test-suite/acceptance/instr/daa.gb",
            "mooneye-test-suite/acceptance/if_ie_registers.gb",
            "mooneye-test-suite/acceptance/timer/div_write.gb",
            "mooneye-test-suite/acceptance/timer/tim00.gb",
            "mooneye-test-suite/acceptance/timer/tim01.gb",
            "mooneye-test-suite/acceptance/timer/tim10.gb",
            "mooneye-test-suite/acceptance/timer/tim11.gb",
        ],
        mooneye,
    );
}

#[test]
fn mooneye_mbc() {
    run_all(
        &[
            "mooneye-test-suite/emulator-only/mbc1/bits_bank1.gb",
            "mooneye-test-suite/emulator-only/mbc1/bits_bank2.gb",
            "mooneye-test-suite/emulator-only/mbc1/bits_mode.gb",
            "mooneye-test-suite/emulator-only/mbc1/bits_ramg.gb",
            "mooneye-test-suite/emulator-only/mbc1/ram_64kb.gb",
            "mooneye-test-suite/emulator-only/mbc1/ram_256kb.gb",
            "mooneye-test-suite/emulator-only/mbc1/rom_512kb.gb",
            "mooneye-test-suite/emulator-only/mbc1/rom_1Mb.gb",
            "mooneye-test-suite/emulator-only/mbc1/rom_2Mb.gb",
            "mooneye-test-suite/emulator-only/mbc1/rom_4Mb.gb",
            "mooneye-test-suite/emulator-only/mbc1/rom_8Mb.gb",
            "mooneye-test-suite/emulator-only/mbc1/rom_16Mb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_512kb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_1Mb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_2Mb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_4Mb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_8Mb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_16Mb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_32Mb.gb",
            "mooneye-test-suite/emulator-only/mbc5/rom_64Mb.gb",
        ],
        mooneye,
    );
}
