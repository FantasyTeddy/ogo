//! The editor's look: the launcher's colours (black, grey bars, steel-blue selection, orange
//! accent) with round, roomy controls. Fonts: Nunito for text, Silkscreen (the pixel font) for the
//! logo and panel headings only, Phosphor for icons; all three are bundled in `assets/fonts` with
//! their licenses (SIL Open Font License, MIT).

use std::sync::Arc;

use eframe::egui::{
    self, epaint::Shadow, vec2, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions,
    FontFamily, FontId, Frame, Margin, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    TextEdit, TextStyle, Ui,
};

pub const BG: Color32 = Color32::BLACK;
pub const BAR: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
pub const PANEL: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
pub const PANEL2: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1c);
/// The open map, and its tab
pub const SHEET: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
/// The other maps' tabs
pub const TAB_IDLE: Color32 = Color32::from_rgb(0x0d, 0x0d, 0x0d);
pub const LINE: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x2e);
pub const TEXT: Color32 = Color32::WHITE;
pub const MUTED: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
pub const ACCENT: Color32 = Color32::from_rgb(0xf7, 0x6d, 0x42);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xff, 0x87, 0x5e);
pub const ACCENT_DOWN: Color32 = Color32::from_rgb(0xd8, 0x55, 0x2e);
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x1a, 0x08, 0x00);
pub const SELECT: Color32 = Color32::from_rgb(0x46, 0x82, 0xb4);
pub const BUTTON: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const INPUT: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0a);
/// The border of text fields: visible on the panels, lighter when hovered
pub const INPUT_LINE: Color32 = Color32::from_rgb(0x48, 0x48, 0x48);
pub const INPUT_HOVER: Color32 = Color32::from_rgb(0x78, 0x78, 0x78);
/// An idle tab or segment under the mouse
pub const HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const OK: Color32 = Color32::from_rgb(0x90, 0xee, 0x90);
pub const WARN: Color32 = Color32::from_rgb(0xff, 0xa5, 0x00);

pub const PANEL_RADIUS: u8 = 18;
pub const PILL: u8 = 15;

// Phosphor icons (regular), by codepoint
pub const PENCIL: &str = "\u{e3b4}";
pub const FILL: &str = "\u{e392}";
pub const BOX: &str = "\u{e3f0}";
pub const ERASER: &str = "\u{e21e}";
pub const FLAG: &str = "\u{e244}";
pub const PLUS: &str = "\u{e3d4}";
pub const MINUS: &str = "\u{e32a}";
pub const GEAR: &str = "\u{e270}";
pub const CARET_DOWN: &str = "\u{e136}";
pub const CARET_RIGHT: &str = "\u{e13a}";
pub const UNDO: &str = "\u{e038}";
pub const REDO: &str = "\u{e036}";
pub const PLAY: &str = "\u{e3d0}";
pub const CROSSHAIR: &str = "\u{e1d6}";
pub const EXPORT: &str = "\u{e4c0}";
pub const WIFI: &str = "\u{e4ea}";
pub const NEW: &str = "\u{e236}";
pub const OPEN: &str = "\u{e256}";
pub const SAVE: &str = "\u{e248}";
pub const TRASH: &str = "\u{e4a6}";
pub const CHECK: &str = "\u{e182}";
pub const WARNING: &str = "\u{e4e2}";
pub const IMAGE: &str = "\u{e2ca}";
pub const CROSS: &str = "\u{e4f6}";
pub const DOTS: &str = "\u{e1fe}";
pub const ARROW_UP: &str = "\u{e08e}";
pub const ARROW_DOWN: &str = "\u{e03e}";
pub const ARROW_LEFT: &str = "\u{e058}";
pub const ARROW_RIGHT: &str = "\u{e06c}";
pub const DOOR: &str = "\u{e61c}";
pub const MAP_PIN: &str = "\u{e316}";
pub const CURSOR: &str = "\u{e1dc}";
pub const STAMP: &str = "\u{ea48}";
pub const INFO: &str = "\u{e2ce}";

fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

fn pixel() -> FontFamily {
    FontFamily::Name("pixel".into())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let font = |bytes: &'static [u8]| Arc::new(FontData::from_static(bytes));
    fonts.font_data.insert(
        "nunito".into(),
        font(include_bytes!("../assets/fonts/Nunito-Medium.ttf")),
    );
    fonts.font_data.insert(
        "nunito-bold".into(),
        font(include_bytes!("../assets/fonts/Nunito-ExtraBold.ttf")),
    );
    fonts.font_data.insert(
        "pixel".into(),
        font(include_bytes!("../assets/fonts/Silkscreen-Regular.ttf")),
    );
    fonts.font_data.insert(
        "phosphor".into(),
        font(include_bytes!("../assets/fonts/Phosphor.ttf")),
    );
    let defaults = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let family = |first: &str| {
        let mut list = vec![first.to_string(), "phosphor".to_string()];
        list.extend(defaults.iter().cloned());
        list
    };
    fonts
        .families
        .insert(FontFamily::Proportional, family("nunito"));
    fonts.families.insert(bold(), family("nunito-bold"));
    fonts.families.insert(pixel(), family("pixel"));
    ctx.set_fonts(fonts);

    // The same look whether the system is set to light or dark
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(15.0, FontFamily::Proportional),
            ),
            (TextStyle::Heading, FontId::new(14.0, pixel())),
            (
                TextStyle::Monospace,
                FontId::new(14.0, FontFamily::Monospace),
            ),
        ]
        .into();
        // Labels aren't text to select; the hand shows what can be clicked
        style.interaction.selectable_labels = false;
        style.interaction.multi_widget_text_select = false;
        let spacing = &mut style.spacing;
        spacing.item_spacing = vec2(8.0, 8.0);
        spacing.button_padding = vec2(12.0, 6.0);
        spacing.interact_size = vec2(40.0, 30.0);
        spacing.menu_margin = Margin::same(6);
        spacing.window_margin = Margin::same(12);

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = BG;
        v.window_fill = PANEL2;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(12);
        v.window_shadow = Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };
        v.popup_shadow = v.window_shadow;
        v.extreme_bg_color = INPUT;
        v.faint_bg_color = PANEL2;
        v.selection.bg_fill = SELECT;
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.override_text_color = None;
        v.interact_cursor = Some(CursorIcon::PointingHand);
        v.text_cursor.stroke = Stroke::new(2.0, TEXT);
        let radius = CornerRadius::same(PILL);
        for (widget, fill, text) in [
            (&mut v.widgets.noninteractive, PANEL, MUTED),
            (&mut v.widgets.inactive, BUTTON, TEXT),
            (&mut v.widgets.hovered, BUTTON_HOVER, TEXT),
            (&mut v.widgets.active, SELECT, TEXT),
            (&mut v.widgets.open, BUTTON_HOVER, TEXT),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.corner_radius = radius;
            widget.fg_stroke = Stroke::new(1.5, text);
            widget.bg_stroke = Stroke::NONE;
            widget.expansion = 0.0;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        v.widgets.inactive.bg_fill = Color32::from_rgb(0x30, 0x30, 0x30);
    });
}

/// A floating panel on the black background.
pub fn panel_frame() -> Frame {
    Frame::new()
        .fill(PANEL)
        .corner_radius(PANEL_RADIUS)
        .inner_margin(Margin::same(12))
}

/// A panel heading in the pixel font.
pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .text_style(TextStyle::Heading)
            .color(TEXT),
    );
}

pub fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text).family(bold())
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}

/// The logo and name in the top bar, in the pixel font.
pub fn brand(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .family(pixel())
        .size(14.0)
        .color(TEXT)
}

/// The main action: orange, rounded; show it with [`accent`].
pub fn primary(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).family(bold()).color(ACCENT_INK))
}

/// Adds a [`primary`] button in orange, lighter when hovered and darker while pressed.
pub fn accent(ui: &mut Ui, enabled: bool, button: egui::Button) -> Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (state, fill) in [
            (&mut widgets.inactive, ACCENT),
            (&mut widgets.hovered, ACCENT_HOVER),
            (&mut widgets.active, ACCENT_DOWN),
        ] {
            state.weak_bg_fill = fill;
            state.bg_fill = fill;
        }
        ui.add_enabled(enabled, button)
    })
    .inner
}

/// A one-line text field with a border, so it's clearly a place to type: lighter when hovered,
/// blue while typing.
pub fn text_input(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    let inner = Frame::new()
        .fill(INPUT)
        .corner_radius(10)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add(
                TextEdit::singleline(text)
                    .frame(Frame::NONE)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY),
            )
        });
    let field = inner.inner;
    let stroke = if field.has_focus() {
        Stroke::new(2.0, SELECT)
    } else if ui.rect_contains_pointer(inner.response.rect) {
        Stroke::new(1.0, INPUT_HOVER)
    } else {
        Stroke::new(1.0, INPUT_LINE)
    };
    ui.painter()
        .rect_stroke(inner.response.rect, 10, stroke, StrokeKind::Inside);
    field
}

/// Something drawn in a frame that's clicked as a whole, e.g. a tab: the fill changes to
/// `hover` while the mouse is over it.
pub fn surface(
    ui: &mut Ui,
    frame: Frame,
    hover: Option<Color32>,
    add: impl FnOnce(&mut Ui),
) -> Response {
    let mut prepared = frame.begin(ui);
    add(&mut prepared.content_ui);
    let inner = prepared.content_ui.min_rect();
    let m = frame.inner_margin;
    let rect = Rect::from_min_max(
        inner.min - vec2(m.left as f32, m.top as f32),
        inner.max + vec2(m.right as f32, m.bottom as f32),
    );
    if let Some(fill) = hover.filter(|_| ui.rect_contains_pointer(rect)) {
        prepared.frame.fill = fill;
    }
    prepared
        .end(ui)
        .interact(Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// A button showing only text, without a background until hovered.
pub fn quiet<'a>(text: impl egui::IntoAtoms<'a>) -> egui::Button<'a> {
    egui::Button::new(text).frame_when_inactive(false)
}

pub fn danger(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(WARN)).frame_when_inactive(false)
}

/// An on/off switch.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let size = vec2(34.0, 19.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    response = response.on_hover_cursor(CursorIcon::PointingHand);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let painter = ui.painter();
    let track = match (*on, response.hovered()) {
        (true, false) => SELECT,
        (true, true) => Color32::from_rgb(0x5a, 0x98, 0xca),
        (false, false) => Color32::from_rgb(0x3a, 0x3a, 0x3a),
        (false, true) => Color32::from_rgb(0x50, 0x50, 0x50),
    };
    painter.rect_filled(rect, CornerRadius::same(10), track);
    let x = egui::lerp((rect.left() + 9.5)..=(rect.right() - 9.5), t);
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, TEXT);
    response
}

/// A tool in the tool strip: its icon with a short label below.
pub fn tool(ui: &mut Ui, icon: &str, label: &str, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(56.0, 52.0), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter();
    let fill = if on {
        SELECT
    } else if response.hovered() {
        BUTTON_HOVER
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, CornerRadius::same(14), fill);
    let colour = if on || response.hovered() {
        TEXT
    } else {
        MUTED
    };
    painter.text(
        rect.center_top() + vec2(0.0, 17.0),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::new(21.0, FontFamily::Proportional),
        colour,
    );
    painter.text(
        rect.center_bottom() - vec2(0.0, 12.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::new(11.5, bold()),
        colour,
    );
    response
}

/// A ring around a selected picture.
pub fn ring(ui: &Ui, rect: egui::Rect, radius: u8) {
    ui.painter().rect_stroke(
        rect.expand(2.0),
        CornerRadius::same(radius),
        Stroke::new(2.0, SELECT),
        StrokeKind::Outside,
    );
}
