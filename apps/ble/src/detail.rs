//! One device in detail: everything it advertised, and its signal over the last minute; and what
//! it says about itself when connected (the read screen).

use std::time::Duration;

use embedded_graphics::{
    mono_font::ascii::{FONT_6X10, FONT_8X13},
    primitives::{PrimitiveStyle, Rectangle},
    text::Alignment,
};
use ogo_sdk::{prelude::*, ui};

use crate::{
    advert, companies,
    gatt::Info,
    scanner::{Device, Reading, Status, HISTORY_SECONDS},
    uuids,
};

const TITLE_HEIGHT: i32 = 24;
const FOOTER_HEIGHT: i32 = 16;
const LINE_HEIGHT: i32 = 12;
const LABEL_X: i32 = 8;
const VALUE_X: i32 = 68;
/// FONT_6X10 characters between `VALUE_X` and the right edge.
const VALUE_CHARS: usize = ((SCREEN_WIDTH as i32 - VALUE_X - 8) / 6) as usize;
/// Lines of values above the graph.
const MAX_LINES: usize = 10;
/// Lines of the read screen, which has no graph.
const READ_LINES: usize =
    ((SCREEN_HEIGHT as i32 - TITLE_HEIGHT - FOOTER_HEIGHT - 10) / LINE_HEIGHT) as usize;
/// The graph: one bar per second, from `GRAPH_FLOOR` to `GRAPH_CEILING` dBm.
const GRAPH_TOP: i32 = 166;
const GRAPH_HEIGHT: i32 = 50;
const GRAPH_FLOOR: i32 = -100;
const GRAPH_CEILING: i32 = -30;
const BAR_WIDTH: i32 = 5;
/// After this long without a packet, "Heard" says so instead of the packet rate.
const SILENT_AFTER: Duration = Duration::from_secs(5);

/// How often to beep while hunting for the device: faster the stronger its signal, from 1.5 s at
/// -100 dBm to 0.1 s at -40 dBm.
pub fn beep_interval(rssi: i32) -> Duration {
    let closeness = (rssi.clamp(-100, -40) + 100) as u64; // 0 far .. 60 near
    Duration::from_millis(1500 - closeness * 1400 / 60)
}

/// The beep's pitch rises with the signal too, from 600 to 1800 Hz.
pub fn beep(rssi: i32) -> Note {
    let closeness = (rssi.clamp(-100, -40) + 100) as u32;
    Note::new(600 + closeness * 20, 30)
}

pub fn draw(fb: &mut Framebuffer, device: &Device, beeping: bool, gone: bool) {
    draw_frame(fb, device, gone);
    draw_lines(fb, &lines(device), MAX_LINES);
    draw_graph(fb, device);
    let beep = if beeping {
        "A stop beeping"
    } else {
        "A beep by signal"
    };
    let controls = if device.connectable == Some(true) {
        format!("{beep}  START read  B back")
    } else {
        format!("{beep}   B back")
    };
    draw_footer(fb, &controls);
}

/// The device's own answers (see [`crate::gatt`]): while connecting and reading, what it does; then
/// the values and services, or why it didn't work.
/// `scroll`: lines scrolled down, up to [`read_scroll_max`].
pub fn draw_read(
    fb: &mut Framebuffer,
    device: &Device,
    reading: Option<&Reading>,
    gone: bool,
    scroll: usize,
) {
    draw_frame(fb, device, gone);
    let center = Point::new(SCREEN_WIDTH as i32 / 2, SCREEN_HEIGHT as i32 / 2);
    match reading.map(|reading| &reading.status) {
        None | Some(Status::Busy(_)) => {
            let step = match reading.map(|reading| &reading.status) {
                Some(Status::Busy(step)) => step,
                _ => "Waiting...",
            };
            ui::text(fb, step, center, &FONT_8X13, ui::DIM, Alignment::Center);
            draw_footer(fb, "B back");
        }
        Some(Status::Failed(message)) => {
            ui::text(
                fb,
                message,
                center,
                &FONT_8X13,
                ui::WARNING,
                Alignment::Center,
            );
            draw_footer(fb, "START try again   B back");
        }
        Some(Status::Done(info)) => {
            let lines = read_lines(info);
            let scroll = scroll.min(lines.len().saturating_sub(READ_LINES));
            draw_lines(fb, &lines[scroll..], READ_LINES);
            draw_footer(
                fb,
                if lines.len() > READ_LINES {
                    "Up/Down scroll  START read again  B back"
                } else {
                    "START read again   B back"
                },
            );
        }
    }
}

/// How far the read screen scrolls: the lines that don't fit.
pub fn read_scroll_max(reading: Option<&Reading>) -> usize {
    match reading.map(|reading| &reading.status) {
        Some(Status::Done(info)) => read_lines(info).len().saturating_sub(READ_LINES),
        _ => 0,
    }
}

/// The read screen's lines: the values, then the services, one per line.
fn read_lines(info: &Info) -> Vec<(&'static str, String)> {
    let mut lines: Vec<(&'static str, String)> = Vec::new();
    if info.values.is_empty() {
        lines.push(("", "No standard values to read.".to_owned()));
    }
    lines.extend(info.values.iter().cloned());
    let services = info
        .services
        .iter()
        .map(|uuid| match uuids::service_name(uuid) {
            Some(name) => name.to_owned(),
            None => uuid.to_string(),
        })
        .collect();
    list_into(&mut lines, "Services", services, None);
    lines
}

/// Title bar with the device's label and its signal, or "gone".
fn draw_frame(fb: &mut Framebuffer, device: &Device, gone: bool) {
    fb.clear(ui::BACKGROUND).ok();
    Rectangle::new(Point::zero(), Size::new(SCREEN_WIDTH, TITLE_HEIGHT as u32))
        .into_styled(PrimitiveStyle::with_fill(ui::BAR))
        .draw(fb)
        .ok();
    let label = device.label().unwrap_or_else(|| device.address.clone());
    let title: String = label.chars().take(26).collect();
    ui::text(
        fb,
        &title,
        Point::new(12, TITLE_HEIGHT / 2),
        &FONT_8X13,
        ui::TEXT,
        Alignment::Left,
    );
    let signal = if gone {
        "gone".to_owned()
    } else {
        format!("{} dBm", device.rssi())
    };
    ui::text(
        fb,
        &signal,
        Point::new(SCREEN_WIDTH as i32 - 12, TITLE_HEIGHT / 2),
        &FONT_8X13,
        if gone { ui::WARNING } else { ui::TEXT },
        Alignment::Right,
    );
}

/// Label and value lines below the title bar, at most `max`.
fn draw_lines(fb: &mut Framebuffer, lines: &[(&'static str, String)], max: usize) {
    let mut y = TITLE_HEIGHT + 10;
    for (label, value) in lines.iter().take(max) {
        if !label.is_empty() {
            ui::text(
                fb,
                label,
                Point::new(LABEL_X, y),
                &FONT_6X10,
                ui::DIM,
                Alignment::Left,
            );
        }
        let value: String = value.chars().take(VALUE_CHARS).collect();
        ui::text(
            fb,
            &value,
            Point::new(VALUE_X, y),
            &FONT_6X10,
            ui::TEXT,
            Alignment::Left,
        );
        y += LINE_HEIGHT;
    }
}

fn draw_footer(fb: &mut Framebuffer, controls: &str) {
    ui::text(
        fb,
        controls,
        Point::new(
            SCREEN_WIDTH as i32 / 2,
            SCREEN_HEIGHT as i32 - FOOTER_HEIGHT / 2,
        ),
        &FONT_6X10,
        ui::DIM,
        Alignment::Center,
    );
}

/// Adds `items` under `label`, one per line (the label on the first). With `max`, at most that
/// many lines: the last says how many more there are.
fn list_into(
    lines: &mut Vec<(&'static str, String)>,
    label: &'static str,
    items: Vec<String>,
    max: Option<usize>,
) {
    let shown = match max {
        Some(max) if items.len() > max => max - 1,
        _ => items.len(),
    };
    let more = items.len() - shown;
    let mut items: Vec<String> = items.into_iter().take(shown).collect();
    if more > 0 {
        items.push(format!("and {more} more"));
    }
    for (i, item) in items.into_iter().enumerate() {
        lines.push((if i == 0 { label } else { "" }, item));
    }
}

/// Label and value per line; a value too long for one line continues on the next, without label.
fn lines(device: &Device) -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    let mut add = |label: &'static str, value: String, max_lines: usize| {
        let chars: Vec<char> = value.chars().collect();
        for (i, chunk) in chars.chunks(VALUE_CHARS).take(max_lines).enumerate() {
            lines.push((if i == 0 { label } else { "" }, chunk.iter().collect()));
        }
    };

    add(
        "Address",
        format!("{}  {}", device.address, device.address_kind),
        1,
    );
    if let Some(name) = &device.name {
        add("Name", name.clone(), 1);
    }
    if let Some((id, _)) = &device.manufacturer {
        let maker = match companies::name(*id) {
            Some(name) => format!("{name} ({id:04X})"),
            None => format!("{id:04X}, unknown"),
        };
        add("Maker", maker, 1);
    }
    let mut kind = match device.connectable {
        Some(true) => "connectable".to_owned(),
        Some(false) => "not connectable".to_owned(),
        None => "?".to_owned(),
    };
    if let Some(power) = device.tx_power {
        kind += &format!(", sends at {power} dBm");
    }
    add("Type", kind, 1);
    // Many devices send only every second or two: silence counts after a few seconds
    let silent = device.last_seen.elapsed();
    let heard = if silent >= SILENT_AFTER {
        format!("not for {}", duration(silent))
    } else {
        format!("{:.1} packets/s", device.packets_per_second())
    };
    add(
        "Heard",
        format!(
            "{heard}, for {}",
            duration(device.last_seen - device.first_seen)
        ),
        1,
    );
    let messages = advert::messages(device.manufacturer.as_ref(), device.service_data.as_ref());
    if !messages.is_empty() {
        add("Says", messages.join(", "), 3);
    }
    let services: Vec<String> = device
        .services
        .iter()
        .map(|uuid| match uuids::service_name(uuid) {
            Some(name) => format!("{name} ({uuid})"),
            None => uuid.to_string(),
        })
        .collect();
    if !services.is_empty() {
        // Below the rest: what doesn't fit above the graph is cut off
        list_into(&mut lines, "Services", services, Some(3));
    }
    lines
}

/// The signal over the last minute, one bar per second, the newest on the right.
fn draw_graph(fb: &mut Framebuffer, device: &Device) {
    let width = SCREEN_WIDTH as i32;
    let left = width - 8 - HISTORY_SECONDS as i32 * BAR_WIDTH;
    let bottom = GRAPH_TOP + GRAPH_HEIGHT;
    Rectangle::new(
        Point::new(left, GRAPH_TOP),
        Size::new(
            (HISTORY_SECONDS as i32 * BAR_WIDTH) as u32,
            GRAPH_HEIGHT as u32,
        ),
    )
    .into_styled(PrimitiveStyle::with_fill(ui::BAR))
    .draw(fb)
    .ok();
    ui::text(
        fb,
        "Signal, last minute",
        Point::new(left, GRAPH_TOP - 7),
        &FONT_6X10,
        ui::DIM,
        Alignment::Left,
    );
    let start = HISTORY_SECONDS - device.history.len();
    for (i, second) in device.history.iter().enumerate() {
        let Some(rssi) = second.signal else {
            continue;
        };
        let level = (i32::from(rssi).clamp(GRAPH_FLOOR, GRAPH_CEILING) - GRAPH_FLOOR)
            * GRAPH_HEIGHT
            / (GRAPH_CEILING - GRAPH_FLOOR);
        Rectangle::new(
            Point::new(left + (start + i) as i32 * BAR_WIDTH, bottom - level),
            Size::new((BAR_WIDTH - 1) as u32, level.max(1) as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(ui::SUCCESS))
        .draw(fb)
        .ok();
    }
}

/// E.g. "12 s" or "3 min".
fn duration(duration: Duration) -> String {
    match duration.as_secs() {
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}
