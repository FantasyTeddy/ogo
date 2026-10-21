//! Bluetooth: lists the BLE devices nearby with their name (or what they are), maker and signal
//! strength.
//!
//! The list keeps its order, new devices at the end, so rows don't move while the signal varies;
//! SELECT sorts it by signal strength once.

mod advert;
mod companies;
mod scanner;

use std::{
    cmp::Reverse,
    collections::HashMap,
    time::{Duration, Instant},
};

use embedded_graphics::{
    mono_font::ascii::{FONT_6X10, FONT_8X13},
    primitives::{PrimitiveStyle, Rectangle},
    text::Alignment,
};
use ogo_sdk::{prelude::*, ui};
use scanner::{Device, Scanner};

const TITLE_HEIGHT: i32 = 24;
const FOOTER_HEIGHT: i32 = 16;
const ROW_HEIGHT: i32 = 26;
const VISIBLE_ROWS: usize =
    ((SCREEN_HEIGHT as i32 - TITLE_HEIGHT - FOOTER_HEIGHT - 4) / ROW_HEIGHT) as usize;
/// Columns: name (FONT_8X13) from 12 to 172, maker (FONT_6X10) to 230, bars to 254, then the dBm
/// right-aligned (up to "-100 dBm", from 260).
const NAME_CHARS: usize = 20;
const MAKER_CHARS: usize = 9;
const MAKER_X: i32 = 176;
const BARS_X: i32 = 236;
/// Devices not heard for this long are shown dimmed.
const DIM_AFTER: Duration = Duration::from_secs(15);
/// How often the rows take the scanner's latest values.
const REFRESH_EVERY: Duration = Duration::from_millis(500);

struct Ble {
    scanner: Result<Scanner, String>,
    /// In the order shown: as first heard, or by signal since SELECT, new ones at the end.
    devices: Vec<Device>,
    /// The selected device's address, so the selection stays with it.
    selected: Option<String>,
    /// The topmost visible row.
    first: usize,
    refreshed_at: Option<Instant>,
}

impl App for Ble {
    const FPS: u32 = 10;

    fn new(_ctx: &mut Ctx) -> Self {
        let scanner = Scanner::start().map_err(|e| {
            log::error!("Bluetooth didn't start: {e:?}");
            format!("Bluetooth didn't start: {e}")
        });
        Ble {
            scanner,
            devices: Vec::new(),
            selected: None,
            first: 0,
            refreshed_at: None,
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if self
            .refreshed_at
            .is_none_or(|at| at.elapsed() >= REFRESH_EVERY)
        {
            self.refresh();
        }
        if self.devices.is_empty() {
            self.selected = None;
            return;
        }

        let len = self.devices.len();
        let mut index = self.selected_index().unwrap_or(0);
        let input = ctx.input();
        if input.pressed(Button::Up) {
            index = index.checked_sub(1).unwrap_or(len - 1);
        } else if input.pressed(Button::Down) {
            index = (index + 1) % len;
        } else if input.pressed(Button::Left) {
            index = index.saturating_sub(VISIBLE_ROWS);
        } else if input.pressed(Button::Right) {
            index = (index + VISIBLE_ROWS).min(len - 1);
        } else if input.pressed(Button::Select) {
            // Strongest first; the selection stays on its device, wherever that is now
            self.devices.sort_by(|a, b| b.signal.total_cmp(&a.signal));
            index = self.selected_index().unwrap_or(0);
            self.first = 0;
        }
        self.selected = Some(self.devices[index].address.clone());
        if index < self.first {
            self.first = index;
        } else if index >= self.first + VISIBLE_ROWS {
            self.first = index + 1 - VISIBLE_ROWS;
        }
        self.first = self.first.min(len.saturating_sub(VISIBLE_ROWS));
    }

    fn draw(&self, fb: &mut Framebuffer) {
        fb.clear(ui::BACKGROUND).ok();
        let width = SCREEN_WIDTH as i32;
        let height = SCREEN_HEIGHT as i32;
        Rectangle::new(Point::zero(), Size::new(SCREEN_WIDTH, TITLE_HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(ui::BAR))
            .draw(fb)
            .ok();
        ui::text(
            fb,
            "Bluetooth",
            Point::new(12, TITLE_HEIGHT / 2),
            &FONT_8X13,
            ui::TEXT,
            Alignment::Left,
        );

        let devices = match &self.scanner {
            Err(message) => return message_line(fb, message, ui::WARNING),
            Ok(_) if self.devices.is_empty() => {
                return message_line(fb, "Scanning...", ui::DIM);
            }
            Ok(_) => &self.devices,
        };
        let count = match devices.len() {
            1 => "1 device".to_owned(),
            n => format!("{n} devices"),
        };
        ui::text(
            fb,
            &count,
            Point::new(width - 12, TITLE_HEIGHT / 2),
            &FONT_6X10,
            ui::DIM,
            Alignment::Right,
        );
        ui::text(
            fb,
            "SELECT sort by signal",
            Point::new(width / 2, height - FOOTER_HEIGHT / 2),
            &FONT_6X10,
            ui::DIM,
            Alignment::Center,
        );

        let selected = self.selected_index();
        for (row, device) in devices
            .iter()
            .enumerate()
            .skip(self.first)
            .take(VISIBLE_ROWS)
        {
            let top = TITLE_HEIGHT + 2 + (row - self.first) as i32 * ROW_HEIGHT;
            let is_selected = Some(row) == selected;
            if is_selected {
                ui::selection(
                    fb,
                    Rectangle::new(
                        Point::new(4, top),
                        Size::new(SCREEN_WIDTH - 8, ROW_HEIGHT as u32),
                    ),
                );
            }
            draw_row(fb, device, top + ROW_HEIGHT / 2, is_selected);
        }
    }
}

impl Ble {
    /// Takes the scanner's latest values, keeping the order: devices gone are dropped, new ones
    /// added at the end in the order they were first heard.
    fn refresh(&mut self) {
        let Ok(scanner) = &self.scanner else {
            return;
        };
        let mut latest: HashMap<String, Device> = scanner
            .devices()
            .into_iter()
            .map(|device| (device.address.clone(), device))
            .collect();
        let mut devices: Vec<Device> = self
            .devices
            .iter()
            .filter_map(|device| latest.remove(&device.address))
            .collect();
        let mut new: Vec<Device> = latest.into_values().collect();
        new.sort_by_key(|device| (device.first_seen, Reverse(device.rssi())));
        devices.extend(new);
        self.devices = devices;
        self.refreshed_at = Some(Instant::now());
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.devices
            .iter()
            .position(|device| device.address == *selected)
    }
}

/// One device, vertically centred on `y`: its name (or what it is, or its address), maker, signal
/// bars and dBm. On the selection's blue everything is white, so it stays readable.
fn draw_row(fb: &mut Framebuffer, device: &Device, y: i32, selected: bool) {
    let stale = device.last_seen.elapsed() >= DIM_AFTER;
    let color = |normal| if selected { ui::TEXT } else { normal };
    let (label, label_color) = match (&device.name, device.description) {
        (Some(name), _) => (name.as_str(), if stale { ui::DIM } else { ui::TEXT }),
        (None, Some(description)) => (description, if stale { ui::DIM } else { ui::TEXT }),
        (None, None) => (device.address.as_str(), ui::DIM),
    };
    let label: String = label.chars().take(NAME_CHARS).collect();
    ui::text(
        fb,
        &label,
        Point::new(12, y),
        &FONT_8X13,
        color(label_color),
        Alignment::Left,
    );

    if let Some(id) = device.company {
        let maker = companies::name(id).map_or_else(|| format!("{id:04X}"), str::to_owned);
        let maker: String = maker.chars().take(MAKER_CHARS).collect();
        ui::text(
            fb,
            &maker,
            Point::new(MAKER_X, y),
            &FONT_6X10,
            color(ui::DIM),
            Alignment::Left,
        );
    }

    // Four bars, filled by signal strength
    let rssi = device.rssi();
    let filled = match rssi {
        -60.. => 4,
        -70..=-61 => 3,
        -80..=-71 => 2,
        -90..=-81 => 1,
        _ => 0,
    };
    let bar_color = color(if stale { ui::DIM } else { ui::SUCCESS });
    for bar in 0..4 {
        let height = 3 + bar * 3;
        Rectangle::new(
            Point::new(BARS_X + bar * 5, y + 6 - height),
            Size::new(3, height as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(if bar < filled {
            bar_color
        } else {
            ui::BAR
        }))
        .draw(fb)
        .ok();
    }
    ui::text(
        fb,
        &format!("{rssi} dBm"),
        Point::new(SCREEN_WIDTH as i32 - 12, y),
        &FONT_6X10,
        color(ui::DIM),
        Alignment::Right,
    );
}

/// A line in the middle of the list area.
fn message_line(fb: &mut Framebuffer, message: &str, color: Rgb565) {
    ui::text(
        fb,
        message,
        Point::new(SCREEN_WIDTH as i32 / 2, SCREEN_HEIGHT as i32 / 2),
        &FONT_8X13,
        color,
        Alignment::Center,
    );
}

ogo_sdk::main!(Ble);
