//! Records a ROM's sound into a WAV file, to listen to on the PC:
//! `cargo run --release --example record -- <rom> <seconds> <out.wav> [buttons...]`.
//!
//! Buttons (`start`, `a`, `b`, `select`, `up`, ...) are each pressed for a moment, one per second,
//! e.g. `start start` to get past a title screen.

use std::io::Write;

use ogo_gb::{Buttons, Cartridge, GameBoy};

const RATE: u32 = 32_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: record <rom> <seconds> <out.wav> [buttons...]";
    let rom = std::fs::read(args.next().expect(usage)).expect("read ROM");
    let seconds: u32 = args.next().expect(usage).parse().expect("seconds");
    let out = args.next().expect(usage);
    let presses: Vec<u8> = args
        .map(|name| match name.as_str() {
            "right" => Buttons::RIGHT,
            "left" => Buttons::LEFT,
            "up" => Buttons::UP,
            "down" => Buttons::DOWN,
            "a" => Buttons::A,
            "b" => Buttons::B,
            "select" => Buttons::SELECT,
            "start" => Buttons::START,
            _ => panic!("unknown button {name}"),
        })
        .collect();

    let mut gb = GameBoy::new(Cartridge::new(rom).expect("cartridge"));
    gb.set_sample_rate(RATE);
    let mut samples = Vec::new();
    // About 60 frames per second
    for frame in 0..seconds * 60 {
        let press = presses.get(frame as usize / 60).filter(|_| frame % 60 < 6);
        gb.set_buttons(Buttons(press.copied().unwrap_or(0)));
        gb.run_frame();
        gb.take_samples(&mut samples);
    }

    let data_len = samples.len() as u32 * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in &samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::File::create(&out)
        .and_then(|mut file| file.write_all(&wav))
        .expect("write WAV");
    println!(
        "{} samples, {:.1} s, into {out}",
        samples.len(),
        samples.len() as f32 / RATE as f32
    );
}
