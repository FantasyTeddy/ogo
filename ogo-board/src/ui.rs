//! The shared look: colours and small drawing helpers used by the launcher, recovery and the SDK's
//! overlays. Apps get them as `ogo_sdk::ui`.
//!
//! The helpers draw onto any embedded-graphics target in RGB565 (a [`crate::Canvas`] or the
//! display) and ignore drawing errors, which a canvas can't have.

use embedded_graphics::{
    mono_font::{MonoFont, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};

pub const BACKGROUND: Rgb565 = Rgb565::BLACK;
/// Top bar, footer and panels.
pub const BAR: Rgb565 = Rgb565::new(4, 8, 4);
/// Behind the selected row.
pub const SELECTION: Rgb565 = Rgb565::CSS_STEEL_BLUE;
pub const TEXT: Rgb565 = Rgb565::WHITE;
/// Secondary text: hints, details, controls.
pub const DIM: Rgb565 = Rgb565::CSS_GRAY;
pub const WARNING: Rgb565 = Rgb565::CSS_ORANGE;
pub const SUCCESS: Rgb565 = Rgb565::CSS_LIGHT_GREEN;
/// Values to notice, e.g. the address and PIN on the WiFi screen.
pub const ACCENT: Rgb565 = Rgb565::new(30, 27, 8);

/// Draws one line of text, vertically centred on `position`; `alignment` places it horizontally.
pub fn text<D: DrawTarget<Color = Rgb565>>(
    target: &mut D,
    text: &str,
    position: Point,
    font: &MonoFont<'_>,
    color: Rgb565,
    alignment: Alignment,
) {
    let style = TextStyleBuilder::new()
        .alignment(alignment)
        .baseline(Baseline::Middle)
        .build();
    Text::with_text_style(text, position, MonoTextStyle::new(font, color), style)
        .draw(target)
        .ok();
}

/// The highlight behind a selected row.
pub fn selection<D: DrawTarget<Color = Rgb565>>(target: &mut D, area: Rectangle) {
    area.into_styled(PrimitiveStyle::with_fill(SELECTION))
        .draw(target)
        .ok();
}

/// A box with a thin border, e.g. for a menu over an app.
pub fn panel<D: DrawTarget<Color = Rgb565>>(target: &mut D, area: Rectangle) {
    let style = PrimitiveStyleBuilder::new()
        .fill_color(BAR)
        .stroke_color(TEXT)
        .stroke_width(1)
        .build();
    area.into_styled(style).draw(target).ok();
}

/// A progress bar: an outline in `frame`, filled to `fraction` (0.0 to 1.0) with a 2-pixel gap.
pub fn progress_bar<D: DrawTarget<Color = Rgb565>>(
    target: &mut D,
    area: Rectangle,
    fraction: f32,
    frame: Rgb565,
) {
    area.into_styled(PrimitiveStyle::with_stroke(frame, 1))
        .draw(target)
        .ok();
    let inner = area.size.saturating_sub(Size::new(4, 4));
    let filled = (inner.width as f32 * fraction.clamp(0.0, 1.0)) as u32;
    Rectangle::new(
        area.top_left + Point::new(2, 2),
        Size::new(filled, inner.height),
    )
    .into_styled(PrimitiveStyle::with_fill(SELECTION))
    .draw(target)
    .ok();
}
