//! The handheld the editor's Play window shows the game on: like the device in spirit (screen on
//! top, a cross pad and A/B below, four small buttons in between), not a copy of any real one.

use adventure_engine::Buttons;
use eframe::egui::{
    self, load::SizedTexture, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Pos2,
    Rect, Stroke, StrokeKind, TextureHandle, Ui,
};

/// The handheld's size in its own units; it's scaled to fit the window.
const SIZE: egui::Vec2 = vec2(400.0, 700.0);

const BODY: Color32 = Color32::from_rgb(0xcf, 0xcf, 0xd3);
const BODY_EDGE: Color32 = Color32::from_rgb(0xa6, 0xa6, 0xac);
const BEZEL: Color32 = Color32::from_rgb(0x1d, 0x1d, 0x22);
const PAD: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x31);
const PRESSED: Color32 = Color32::from_rgb(0x55, 0x55, 0x5c);
const RED: Color32 = Color32::from_rgb(0x8e, 0x2f, 0x3b);
const RED_PRESSED: Color32 = Color32::from_rgb(0xc2, 0x47, 0x55);
const PRINT: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x72);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandheldButton {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
}

pub struct Handheld {
    /// Where the handheld's (0, 0) is on the screen, and how big a unit is
    origin: Pos2,
    scale: f32,
}

// In the handheld's units
const SCREEN: Rect = Rect {
    min: pos2(40.0, 50.0),
    max: pos2(360.0, 290.0),
};
const PAD_CENTRE: Pos2 = pos2(108.0, 470.0);
const ARM: f32 = 46.0;
const ARM_WIDTH: f32 = 40.0;
const A_CENTRE: Pos2 = pos2(318.0, 446.0);
const B_CENTRE: Pos2 = pos2(246.0, 494.0);
const BUTTON_RADIUS: f32 = 29.0;

impl Handheld {
    /// As large as fits in `area`, leaving room for a line of help below.
    pub fn fit(area: Rect) -> Handheld {
        let room = area.size() - vec2(24.0, 52.0);
        let fits = (room.x / SIZE.x).min(room.y / SIZE.y).max(0.3);
        // In steps of a half, so the 160 x 120 picture shows at a whole multiple: even pixels
        let scale = if fits >= 1.0 {
            (fits * 2.0).floor() / 2.0
        } else {
            fits
        };
        let origin = pos2(area.center().x - SIZE.x * scale / 2.0, area.top() + 12.0);
        Handheld { origin, scale }
    }

    fn at(&self, p: Pos2) -> Pos2 {
        self.origin + p.to_vec2() * self.scale
    }

    fn rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.at(r.min), self.at(r.max))
    }

    fn arm(&self, button: HandheldButton) -> Option<Rect> {
        let (dx, dy) = match button {
            HandheldButton::Up => (0.0, -1.0),
            HandheldButton::Down => (0.0, 1.0),
            HandheldButton::Left => (-1.0, 0.0),
            HandheldButton::Right => (1.0, 0.0),
            _ => return None,
        };
        let centre = PAD_CENTRE + vec2(dx, dy) * ARM * 0.75;
        let size = if dx == 0.0 {
            vec2(ARM_WIDTH, ARM)
        } else {
            vec2(ARM, ARM_WIDTH)
        };
        Some(self.rect(Rect::from_center_size(centre, size)))
    }

    /// Whether the point is on that button.
    pub fn hit(&self, button: HandheldButton, pos: Pos2) -> bool {
        match button {
            HandheldButton::A => self.at(A_CENTRE).distance(pos) <= BUTTON_RADIUS * self.scale,
            HandheldButton::B => self.at(B_CENTRE).distance(pos) <= BUTTON_RADIUS * self.scale,
            _ => self.arm(button).is_some_and(|r| r.contains(pos)),
        }
    }

    pub fn paint(&self, ui: &Ui, screen: &TextureHandle, held: Buttons) {
        let painter = ui.painter();
        let s = self.scale;
        let radius = |r: f32| CornerRadius::same((r * s).round().clamp(0.0, 255.0) as u8);

        // Body, with a slight shadow
        let body = self.rect(Rect::from_min_size(pos2(0.0, 0.0), SIZE));
        painter.rect_filled(
            body.translate(vec2(0.0, 6.0 * s)),
            radius(36.0),
            Color32::from_black_alpha(90),
        );
        painter.rect_filled(body, radius(36.0), BODY);
        painter.rect_stroke(
            body,
            radius(36.0),
            Stroke::new(2.0 * s, BODY_EDGE),
            StrokeKind::Inside,
        );

        // The screen in its frame, a light, and the name below it
        let bezel = self.rect(Rect::from_min_max(pos2(20.0, 24.0), pos2(380.0, 318.0)));
        painter.rect_filled(bezel, radius(18.0), BEZEL);
        painter.circle_filled(
            self.at(pos2(34.0, 37.0)),
            3.5 * s,
            Color32::from_rgb(0x6c, 0xd0, 0x6c),
        );
        let image = self.rect(SCREEN);
        egui::Image::from_texture(SizedTexture::new(screen.id(), image.size())).paint_at(ui, image);
        painter.text(
            self.at(pos2(200.0, 305.0)),
            Align2::CENTER_CENTER,
            "ADVENTURE",
            FontId::new(10.0 * s, FontFamily::Name("pixel".into())),
            PRINT,
        );

        // Four small buttons under the screen
        for (i, label) in ["MENU", "VOL", "SELECT", "START"].into_iter().enumerate() {
            let centre = pos2(70.0 + i as f32 * 86.7, 350.0);
            painter.rect_filled(
                self.rect(Rect::from_center_size(centre, vec2(56.0, 14.0))),
                radius(7.0),
                PAD,
            );
            painter.text(
                self.at(centre + vec2(0.0, 18.0)),
                Align2::CENTER_CENTER,
                label,
                FontId::new(9.5 * s, FontFamily::Proportional),
                PRINT,
            );
        }

        // The cross pad, its pressed arm lighter
        let pressed = [
            (HandheldButton::Up, held.up),
            (HandheldButton::Down, held.down),
            (HandheldButton::Left, held.left),
            (HandheldButton::Right, held.right),
        ];
        let vertical = self.rect(Rect::from_center_size(
            PAD_CENTRE,
            vec2(ARM_WIDTH, ARM * 2.5),
        ));
        let horizontal = self.rect(Rect::from_center_size(
            PAD_CENTRE,
            vec2(ARM * 2.5, ARM_WIDTH),
        ));
        painter.rect_filled(vertical, radius(8.0), PAD);
        painter.rect_filled(horizontal, radius(8.0), PAD);
        for (button, on) in pressed {
            if on {
                if let Some(arm) = self.arm(button) {
                    painter.rect_filled(arm.shrink(3.0 * s), radius(6.0), PRESSED);
                }
            }
        }
        painter.circle_filled(
            self.at(PAD_CENTRE),
            9.0 * s,
            Color32::from_rgb(0x24, 0x24, 0x28),
        );

        // A and B
        for (centre, label, on) in [(A_CENTRE, "A", held.a), (B_CENTRE, "B", held.b)] {
            let c = self.at(centre);
            painter.circle_filled(
                c + vec2(0.0, 3.0 * s),
                BUTTON_RADIUS * s,
                Color32::from_black_alpha(60),
            );
            painter.circle_filled(c, BUTTON_RADIUS * s, if on { RED_PRESSED } else { RED });
            painter.text(
                self.at(centre + vec2(BUTTON_RADIUS + 8.0, BUTTON_RADIUS - 2.0)),
                Align2::CENTER_CENTER,
                label,
                FontId::new(15.0 * s, FontFamily::Name("bold".into())),
                PRINT,
            );
        }

        // Speaker holes
        for row in 0..5 {
            for col in 0..6 {
                let p = pos2(268.0 + col as f32 * 13.0, 600.0 + row as f32 * 13.0);
                painter.circle_filled(self.at(p), 2.6 * s, Color32::from_rgb(0x9c, 0x9c, 0xa3));
            }
        }
    }
}
