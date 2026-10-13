//! Plays Game Boy ROMs from the SD card at their original size, with the `ogo-gb` core.
//!
//! The ROMs are `.gb` files in `/roms/gb/`. A cartridge with a battery keeps its RAM in the saves,
//! under the ROM's file name: saved when the pause menu opens, and a few seconds after it changed.

use std::{
    cell::Cell,
    fs,
    time::{Duration, Instant},
};

use embedded_graphics::{
    mono_font::ascii::{FONT_6X10, FONT_8X13},
    primitives::Rectangle,
    text::Alignment,
};
use esp_idf_svc::sys;
use ogo_gb::{Buttons, Cartridge, GameBoy, CLOCK_HZ, CYCLES_PER_FRAME};
use ogo_sdk::{prelude::*, ui};

/// Where the stock firmware and other ODROID-GO emulators keep Game Boy ROMs too.
const ROM_DIR: &str = "/sd/roms/gb";
/// The Game Boy's 160x144 pixels, in the middle of the screen.
const PICTURE: Rectangle = Rectangle::new(Point::new(80, 48), Size::new(160, 144));
/// The four shades, white to black.
const SHADES: [Rgb565; 4] = [
    Rgb565::new(31, 63, 31),
    Rgb565::new(21, 42, 21),
    Rgb565::new(10, 21, 10),
    Rgb565::new(0, 0, 0),
];
/// A Game Boy frame in real time: about 59.7 per second.
const FRAME_NANOS: u128 = 1_000_000_000 * CYCLES_PER_FRAME as u128 / CLOCK_HZ as u128;
/// Frames run without drawing per update at most, to catch up when emulating falls behind.
const MAX_SKIPPED: u64 = 2;
/// Further behind than this (e.g. after the pause menu), the clock starts over instead.
const MAX_BEHIND: u64 = 30;
/// How often the cartridge RAM is checked for changes to save.
const SAVE_EVERY: u64 = 120;
/// How often the speed is logged.
const LOG_EVERY: u64 = 600;
const BUTTONS: [(Button, u8); 8] = [
    (Button::Right, Buttons::RIGHT),
    (Button::Left, Buttons::LEFT),
    (Button::Up, Buttons::UP),
    (Button::Down, Buttons::DOWN),
    (Button::A, Buttons::A),
    (Button::B, Buttons::B),
    (Button::Select, Buttons::SELECT),
    (Button::Start, Buttons::START),
];

// The ROM list
const TITLE_HEIGHT: i32 = 32;
const ROW_HEIGHT: i32 = 20;
const VISIBLE_ROWS: usize = 9;
const NAME_CHARS: usize = 36;

enum Gb {
    List(List),
    Playing(Box<Playing>),
}

struct List {
    /// File names in `ROM_DIR`, sorted.
    roms: Vec<String>,
    selected: usize,
    /// Why there's nothing to choose, or why the last ROM didn't start.
    message: Option<String>,
}

struct Playing {
    gb: GameBoy,
    /// The save name for the cartridge RAM; `None` without a battery.
    save: Option<String>,
    saved_ram: Vec<u8>,
    /// Frames run since `clock_start`, to keep the emulation in step with real time.
    frames: u64,
    clock_start: Instant,
    skipped: u64,
    log_start: Instant,
    /// Time spent emulating and copying the picture since `log_start`, for the speed log.
    emulating: Duration,
    copying: Cell<Duration>,
}

impl Game for Gb {
    const FPS: u32 = 60;

    fn new(ctx: &mut Ctx) -> Self {
        let list = List::scan(ctx);
        Gb::List(list)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        match self {
            Gb::List(list) => {
                if let Some(playing) = list.update(ctx) {
                    *self = Gb::Playing(playing);
                }
            }
            Gb::Playing(playing) => playing.update(ctx),
        }
    }

    fn draw(&self, fb: &mut Framebuffer) {
        match self {
            Gb::List(list) => list.draw(fb),
            Gb::Playing(playing) => playing.draw(fb),
        }
    }

    fn area(&self) -> Option<Rectangle> {
        matches!(self, Gb::Playing(_)).then_some(PICTURE)
    }

    fn paused(&mut self, ctx: &mut Ctx) {
        if let Gb::Playing(playing) = self {
            playing.save(ctx);
        }
    }
}

impl List {
    fn scan(ctx: &Ctx) -> Self {
        let mut roms: Vec<String> = fs::read_dir(ROM_DIR)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                    .filter(|name| name.to_ascii_lowercase().ends_with(".gb"))
                    .collect()
            })
            .unwrap_or_default();
        roms.sort_by_key(|name| name.to_ascii_lowercase());
        let last: Option<String> = ctx.saves().read("last");
        let selected = last
            .and_then(|last| roms.iter().position(|name| *name == last))
            .unwrap_or(0);
        let message = roms
            .is_empty()
            .then(|| "Put .gb files into /roms/gb/ on the SD card".to_string());
        List {
            roms,
            selected,
            message,
        }
    }

    fn update(&mut self, ctx: &mut Ctx) -> Option<Box<Playing>> {
        if self.roms.is_empty() {
            return None;
        }
        let input = ctx.input();
        if input.pressed(Button::Up) {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.roms.len() - 1);
        } else if input.pressed(Button::Down) {
            self.selected = (self.selected + 1) % self.roms.len();
        } else if input.pressed(Button::A) {
            let name = self.roms[self.selected].clone();
            match Playing::start(ctx, &name) {
                Ok(playing) => {
                    ctx.saves().write("last", &name).ok();
                    return Some(Box::new(playing));
                }
                Err(e) => {
                    log::warn!("{name}: {e:?}");
                    self.message = Some(format!("{e}"));
                }
            }
        }
        None
    }

    fn draw(&self, fb: &mut Framebuffer) {
        fb.clear(ui::BACKGROUND).ok();
        let width = SCREEN_WIDTH as i32;
        Rectangle::new(Point::zero(), Size::new(SCREEN_WIDTH, TITLE_HEIGHT as u32))
            .into_styled(embedded_graphics::primitives::PrimitiveStyle::with_fill(
                ui::BAR,
            ))
            .draw(fb)
            .ok();
        ui::text(
            fb,
            "Choose a game",
            Point::new(width / 2, TITLE_HEIGHT / 2),
            &FONT_8X13,
            ui::TEXT,
            Alignment::Center,
        );

        let first = self
            .selected
            .saturating_sub(VISIBLE_ROWS - 1)
            .min(self.roms.len().saturating_sub(VISIBLE_ROWS));
        for (row, name) in self.roms.iter().enumerate().skip(first).take(VISIBLE_ROWS) {
            let y = TITLE_HEIGHT + 4 + (row - first) as i32 * ROW_HEIGHT;
            if row == self.selected {
                ui::selection(
                    fb,
                    Rectangle::new(
                        Point::new(4, y),
                        Size::new(SCREEN_WIDTH - 8, ROW_HEIGHT as u32),
                    ),
                );
            }
            let stem = name
                .rsplit_once('.')
                .map_or(name.as_str(), |(stem, _)| stem);
            let shown: String = stem.chars().take(NAME_CHARS).collect();
            ui::text(
                fb,
                &shown,
                Point::new(12, y + ROW_HEIGHT / 2),
                &FONT_8X13,
                ui::TEXT,
                Alignment::Left,
            );
        }

        if let Some(message) = &self.message {
            ui::text(
                fb,
                message,
                Point::new(width / 2, SCREEN_HEIGHT as i32 - 14),
                &FONT_6X10,
                ui::WARNING,
                Alignment::Center,
            );
        }
    }
}

impl Playing {
    fn start(ctx: &mut Ctx, name: &str) -> anyhow::Result<Self> {
        let rom = fs::read(format!("{ROM_DIR}/{name}"))?;
        let cart = Cartridge::new(rom).map_err(|e| anyhow::anyhow!("{e}"))?;
        log::info!("Starting {name}: \"{}\"", cart.title());

        // The emulator's memory is used for every instruction and pixel: keep all of it in
        // internal RAM, which is faster than PSRAM. The ROM stays in PSRAM.
        unsafe { sys::heap_caps_malloc_extmem_enable(64 * 1024) };
        let mut gb = GameBoy::new(cart);
        unsafe { sys::heap_caps_malloc_extmem_enable(16 * 1024) };

        let save = gb
            .cartridge()
            .has_battery()
            .then(|| save_name(name))
            .filter(|_| !gb.cartridge().ram().is_empty());
        if let Some(save) = &save {
            if let Some(data) = ctx.saves().read_bytes(save) {
                let ram = gb.cartridge_mut().ram_mut();
                if data.len() == ram.len() {
                    ram.copy_from_slice(&data);
                } else {
                    log::warn!(
                        "Ignoring the save {save}: {} bytes, not {}",
                        data.len(),
                        ram.len()
                    );
                }
            }
        }
        let saved_ram = gb.cartridge().ram().to_vec();

        if let Err(e) = ctx.set_cpu_mhz(240) {
            log::warn!("{e:?}");
        }
        let now = Instant::now();
        Ok(Playing {
            gb,
            save,
            saved_ram,
            frames: 0,
            clock_start: now,
            skipped: 0,
            log_start: now,
            emulating: Duration::ZERO,
            copying: Cell::new(Duration::ZERO),
        })
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let input = ctx.input();
        let buttons = BUTTONS
            .iter()
            .filter(|(button, _)| input.held(*button))
            .fold(0, |bits, (_, bit)| bits | bit);
        self.gb.set_buttons(Buttons(buttons));

        // Frames that should be done once this one is
        let due = (self.clock_start.elapsed().as_nanos() / FRAME_NANOS) as u64 + 1;
        let mut behind = due.saturating_sub(self.frames + 1);
        if behind > MAX_BEHIND {
            self.clock_start = Instant::now();
            self.frames = 0;
            behind = 0;
        }
        let started = Instant::now();
        let skip = behind.min(MAX_SKIPPED);
        self.gb.set_drawing(false);
        for _ in 0..skip {
            self.gb.run_frame();
        }
        self.gb.set_drawing(true);
        self.gb.run_frame();
        self.frames += skip + 1;
        self.skipped += skip;
        self.emulating += started.elapsed();

        if ctx.frame().is_multiple_of(SAVE_EVERY) {
            self.save(ctx);
        }
        if ctx.frame().is_multiple_of(LOG_EVERY) {
            let seconds = self.log_start.elapsed().as_secs_f32();
            let per_update = |time: Duration| time.as_secs_f32() * 1000.0 / LOG_EVERY as f32;
            log::info!(
                "{:.1} updates/s, {} of {} frames skipped; per update {:.1} ms emulating, {:.1} ms copying",
                LOG_EVERY as f32 / seconds,
                self.skipped,
                self.skipped + LOG_EVERY,
                per_update(self.emulating),
                per_update(self.copying.get()),
            );
            self.log_start = Instant::now();
            self.skipped = 0;
            self.emulating = Duration::ZERO;
            self.copying.set(Duration::ZERO);
        }
    }

    /// Saves the cartridge RAM if it changed since the last save.
    fn save(&mut self, ctx: &Ctx) {
        let Some(save) = &self.save else {
            return;
        };
        let ram = self.gb.cartridge().ram();
        if ram == self.saved_ram {
            return;
        }
        match ctx.saves().write_bytes(save, ram) {
            Ok(()) => self.saved_ram.copy_from_slice(ram),
            Err(e) => log::warn!("Saving {save} failed: {e:?}"),
        }
    }

    fn draw(&self, fb: &mut Framebuffer) {
        let started = Instant::now();
        let pixels = self
            .gb
            .frame()
            .iter()
            .map(|&shade| SHADES[shade as usize & 3]);
        fb.fill_contiguous(&PICTURE, pixels).ok();
        self.copying.set(self.copying.get() + started.elapsed());
    }
}

/// The ROM's file name as a save name (letters, digits, - and _), e.g. "Tetris (World).gb" ->
/// "Tetris__World_".
fn save_name(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    stem.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

ogo_sdk::main!(Gb);
