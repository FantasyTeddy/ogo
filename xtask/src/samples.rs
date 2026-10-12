//! `cargo xtask samples`: fake games for testing the launcher's game list, written to an SD card.
//!
//! They aren't runnable: `game.bin` is a placeholder. Two entries are broken on purpose, to show how
//! the launcher reports problems: one needs a newer SDK, one has a `game.toml` that doesn't parse.

use std::{fs, path::Path};

use anyhow::{ensure, Context};
use ogo_format::game::{Manifest, ICON_SIZE, SDK_VERSION};
use sha2::{Digest, Sha256};

use crate::hex;

#[derive(Clone, Copy)]
enum Pattern {
    Rings,
    Bricks,
    Checker,
    Stripes,
    Dots,
    Grid,
    Diamond,
    Gradient,
}

struct Sample {
    id: &'static str,
    name: &'static str,
    author: &'static str,
    version: &'static str,
    icon: Option<(Pattern, [u8; 3])>,
}

const SAMPLES: [Sample; 13] = [
    Sample { id: "snake", name: "Snake", author: "Matthias", version: "1.2.0", icon: Some((Pattern::Rings, [40, 200, 80])) },
    Sample { id: "breakout", name: "Breakout", author: "Matthias", version: "0.9.1", icon: Some((Pattern::Bricks, [230, 90, 40])) },
    Sample { id: "blocks", name: "Falling Blocks", author: "ogo examples", version: "2.0.0", icon: Some((Pattern::Checker, [80, 120, 240])) },
    Sample { id: "pong", name: "Pong", author: "ogo examples", version: "1.0.0", icon: Some((Pattern::Stripes, [240, 240, 240])) },
    Sample { id: "rocks", name: "Space Rocks", author: "Someone with a rather long name", version: "0.3.0", icon: Some((Pattern::Dots, [160, 160, 180])) },
    Sample { id: "mines", name: "Mines", author: "", version: "1.1.0", icon: Some((Pattern::Grid, [200, 200, 60])) },
    Sample { id: "2048", name: "2048", author: "ogo examples", version: "1.0.0", icon: Some((Pattern::Diamond, [240, 180, 60])) },
    Sample { id: "sokoban", name: "Sokoban", author: "ogo examples", version: "1.4.2", icon: Some((Pattern::Bricks, [160, 110, 60])) },
    Sample { id: "flappy", name: "Flappy Bit", author: "ogo examples", version: "0.1.0", icon: Some((Pattern::Gradient, [90, 200, 240])) },
    Sample { id: "lander", name: "Lunar Lander", author: "ogo examples", version: "1.0.0", icon: Some((Pattern::Rings, [200, 200, 200])) },
    Sample { id: "solitaire", name: "Solitaire", author: "ogo examples", version: "3.1.0", icon: Some((Pattern::Diamond, [40, 140, 60])) },
    Sample {
        id: "long-name",
        name: "The Incredibly Long Named Adventure of Rusty the Crab",
        author: "Ferris",
        version: "0.0.1",
        icon: Some((Pattern::Gradient, [230, 70, 30])),
    },
    Sample { id: "no-icon", name: "No Icon Game", author: "ogo examples", version: "1.0.0", icon: None },
];

/// Writes the samples into `<sd_root>/ogo/games/`, skipping folders that exist unless `force`.
pub fn write(sd_root: &Path, force: bool) -> anyhow::Result<()> {
    ensure!(sd_root.is_dir(), "{} is not a folder", sd_root.display());
    let games = sd_root.join("ogo").join("games");
    let mut written = Vec::new();

    for sample in &SAMPLES {
        written.push(write_folder(&games.join(sample.id), force, |dir| write_game(dir, sample, SDK_VERSION))?);
    }
    // Broken on purpose: needs a newer SDK, and a game.toml that doesn't parse.
    let future = Sample {
        id: "future-game",
        name: "From The Future",
        author: "ogo examples",
        version: "9.0.0",
        icon: Some((Pattern::Rings, [180, 60, 220])),
    };
    written.push(write_folder(&games.join(future.id), force, |dir| write_game(dir, &future, SDK_VERSION + 1))?);
    written.push(write_folder(&games.join("bad-toml"), force, |dir| {
        fs::write(dir.join("game.toml"), "id = \"bad-toml\"\nname = \"Missing quote\n")?;
        Ok(())
    })?);

    let count = written.iter().filter(|&&w| w).count();
    println!("{count} written, {} skipped", written.len() - count);
    Ok(())
}

/// Creates `dir` and fills it with `write`; returns `false` if it existed and `force` is off.
fn write_folder(dir: &Path, force: bool, write: impl FnOnce(&Path) -> anyhow::Result<()>) -> anyhow::Result<bool> {
    if dir.exists() && !force {
        println!("skip   {} (exists, use --force to overwrite)", dir.display());
        return Ok(false);
    }
    fs::create_dir_all(dir)?;
    write(dir).with_context(|| format!("can't write {}", dir.display()))?;
    println!("write  {}", dir.display());
    Ok(true)
}

fn write_game(dir: &Path, sample: &Sample, sdk_version: u32) -> anyhow::Result<()> {
    let bin = format!("placeholder for {}, not a real firmware image\n", sample.id).repeat(64).into_bytes();
    let manifest = Manifest {
        id: sample.id.to_owned(),
        name: sample.name.to_owned(),
        version: sample.version.to_owned(),
        author: sample.author.to_owned(),
        sdk_version,
        sha256: hex(&Sha256::digest(&bin)),
    };
    fs::write(dir.join("game.bin"), &bin)?;
    fs::write(dir.join("game.toml"), manifest.to_toml())?;
    if let Some((pattern, base)) = sample.icon {
        fs::write(dir.join("icon.rgb565"), icon(pattern, base))?;
    }
    Ok(())
}

/// A 64x64 RGB565 icon: `pattern` in `base`, on a darker shade of it.
fn icon(pattern: Pattern, base: [u8; 3]) -> Vec<u8> {
    let dark = base.map(|c| c / 4);
    let size = ICON_SIZE as i32;
    let mut out = Vec::with_capacity((size * size * 2) as usize);
    for y in 0..size {
        for x in 0..size {
            let (cx, cy) = (x as f32 - 31.5, y as f32 - 31.5);
            let on = match pattern {
                Pattern::Rings => (cx.hypot(cy) as i32) / 6 % 2 == 0,
                Pattern::Bricks => (x + if y / 8 % 2 == 1 { 8 } else { 0 }) % 16 != 0 && y % 8 != 0,
                Pattern::Checker => (x / 8 + y / 8) % 2 == 0,
                Pattern::Stripes => (x + y) / 6 % 2 == 0,
                Pattern::Dots => ((x % 12) as f32 - 5.5).hypot((y % 12) as f32 - 5.5) < 4.0,
                Pattern::Grid => x % 10 > 1 && y % 10 > 1,
                Pattern::Diamond => cx.abs() + cy.abs() < 28.0,
                Pattern::Gradient => {
                    let t = y as f32 / (size - 1) as f32;
                    let color: [u8; 3] = std::array::from_fn(|i| (dark[i] as f32 + (base[i] as f32 - dark[i] as f32) * t) as u8);
                    out.extend_from_slice(&rgb565(color));
                    continue;
                }
            };
            out.extend_from_slice(&rgb565(if on { base } else { dark }));
        }
    }
    out
}

fn rgb565([r, g, b]: [u8; 3]) -> [u8; 2] {
    let value = (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
    value.to_le_bytes()
}
