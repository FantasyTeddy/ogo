//! Emulation speed on the PC: `cargo run --release --example bench -- <rom> [frames]`.

use std::time::Instant;

use ogo_gb::{Cartridge, GameBoy, CYCLES_PER_FRAME};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: bench <rom> [frames]");
    let frames: u32 = args.next().map_or(600, |n| n.parse().expect("frames"));
    let cart = Cartridge::new(std::fs::read(&path).expect("read ROM")).expect("cartridge");
    let mut gb = GameBoy::new(cart);
    let start = Instant::now();
    for _ in 0..frames {
        gb.run_frame();
    }
    let seconds = start.elapsed().as_secs_f64();
    let mhz = frames as f64 * CYCLES_PER_FRAME as f64 / seconds / 1e6;
    println!(
        "{frames} frames in {seconds:.2} s: {:.0} frames/s, {mhz:.1} MHz",
        frames as f64 / seconds
    );
}
