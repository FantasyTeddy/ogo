//! Smash: bounce the ball off the paddle and clear every brick. Three balls; the ball gets faster
//! every level.

use embedded_graphics::{
    mono_font::ascii::{FONT_10X20, FONT_6X10, FONT_8X13},
    primitives::{Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::Alignment,
};
use ogo_sdk::{pitch::*, prelude::*, ui::text};

const WIDTH: i32 = SCREEN_WIDTH as i32;
const HEIGHT: i32 = SCREEN_HEIGHT as i32;
const HUD_HEIGHT: i32 = 16;

const BRICK_COLS: usize = 10;
const BRICK_ROWS: usize = 6;
const BRICK_WIDTH: i32 = 30;
const BRICK_HEIGHT: i32 = 10;
const BRICK_GAP: i32 = 2;
const BRICKS_TOP: i32 = 36;

const PADDLE_WIDTH: i32 = 50;
const PADDLE_HEIGHT: i32 = 6;
const PADDLE_Y: i32 = 222;
/// Pixels per frame.
const PADDLE_SPEED: i32 = 8;

const BALL_SIZE: i32 = 6;
/// Pixels per frame at level 1, more per level, and the fastest it gets.
const BALL_SPEED: f32 = 5.0;
const BALL_SPEEDUP: f32 = 0.4;
const MAX_BALL_SPEED: f32 = 9.0;
/// The ball moves at most this far between collision checks, so it can't pass through a brick.
const BALL_STEP: f32 = 2.0;
/// The steepest bounce, off the end of the paddle: 60° from straight up.
const MAX_BOUNCE_ANGLE: f32 = core::f32::consts::FRAC_PI_3;
/// The most a launch leans to either side: 20°.
const MAX_LAUNCH_ANGLE: f32 = core::f32::consts::PI / 9.0;

const LIVES: u32 = 3;
/// Points per brick hit, and per level cleared times the level.
const BRICK_POINTS: u32 = 10;
const LEVEL_BONUS: u32 = 100;
const LEVEL_CLEAR_FRAMES: u32 = 60;
/// Frames after game over before A starts a new game, so launching doesn't skip the score.
const GAME_OVER_WAIT: u32 = 30;

/// Rows top to bottom: `1` a brick, `2` one that takes two hits, `.` none. After the last level
/// they start over, faster.
const LEVELS: [[&str; BRICK_ROWS]; 5] = [
    [
        "1111111111",
        "1111111111",
        "1111111111",
        "1111111111",
        "1111111111",
        "..........",
    ],
    [
        "....11....",
        "...1111...",
        "..111111..",
        ".11111111.",
        "1111111111",
        "2222222222",
    ],
    [
        "1.1.1.1.1.",
        ".1.1.1.1.1",
        "2.2.2.2.2.",
        ".2.2.2.2.2",
        "1.1.1.1.1.",
        ".1.1.1.1.1",
    ],
    [
        "2222222222",
        "2........2",
        "2.111111.2",
        "2.111111.2",
        "2........2",
        "2222222222",
    ],
    [
        "2222222222",
        "1111111111",
        "..........",
        "2222222222",
        "1111111111",
        "1111111111",
    ],
];

const BACKGROUND: Rgb565 = rgb(14, 16, 28);
const HUD: Rgb565 = rgb(30, 34, 52);
const PADDLE: Rgb565 = rgb(200, 210, 230);
const BALL: Rgb565 = Rgb565::WHITE;
const TITLE: Rgb565 = rgb(230, 150, 60);
const TEXT: Rgb565 = Rgb565::WHITE;
const DIM: Rgb565 = Rgb565::CSS_GRAY;
/// Brick colours by row, top to bottom.
const ROW_COLORS: [Rgb565; BRICK_ROWS] = [
    rgb(220, 80, 80),
    rgb(230, 150, 60),
    rgb(220, 200, 80),
    rgb(110, 190, 90),
    rgb(70, 170, 190),
    rgb(100, 120, 220),
];

const SOUND_START: &[Note] = &[Note::new(587, 60), Note::new(740, 60), Note::new(880, 90)];
const SOUND_LAUNCH: &[Note] = &[Note::new(587, 30), Note::new(880, 40)];
const SOUND_PADDLE: &[Note] = &[Note::new(392, 25)];
const SOUND_WALL: &[Note] = &[Note::new(262, 15)];
/// Brick hits by row, top to bottom: higher up, higher pitch.
const BRICK_PITCHES: [u32; BRICK_ROWS] = [1047, 988, 880, 784, 698, 659];
const SOUND_LOSE: &[Note] = &[
    Note::new(392, 100),
    Note::new(311, 100),
    Note::new(247, 200),
];
const SOUND_LEVEL: &[Note] = &[
    Note::new(523, 70),
    Note::new(659, 70),
    Note::new(784, 70),
    Note::new(1047, 160),
];
const SOUND_DIE: &[Note] = &[
    Note::new(330, 120),
    Note::new(262, 120),
    Note::new(196, 120),
    Note::new(147, 300),
];
const SOUND_BEST: &[Note] = &[Note::new(784, 80), Note::new(988, 80), Note::new(1319, 160)];

/// Music: eight bars in D major, 136 beats per minute at level 1.
const EIGHTH_MS: u32 = 220;
/// Music tempo in percent per level, and the fastest it gets.
const MUSIC_SPEEDUP: u32 = 5;
const MAX_MUSIC_SPEED: u32 = 140;
#[rustfmt::skip]
const MELODY: &[Note] = &[
    n(D5, 1), n(FS5, 1), n(A5, 1), n(FS5, 1), n(D5, 1), n(A4, 1), n(D5, 2),
    n(E5, 1), n(G5, 1), n(B5, 1), n(G5, 1), n(E5, 1), n(B4, 1), n(E5, 2),
    n(FS5, 1), n(A5, 1), n(D6, 2), n(CS6, 1), n(A5, 1), n(FS5, 2),
    n(E5, 2), n(FS5, 1), n(E5, 1), n(D5, 2), n(REST, 2),
    n(B4, 1), n(D5, 1), n(G5, 1), n(D5, 1), n(B4, 1), n(G4, 1), n(B4, 2),
    n(A4, 1), n(CS5, 1), n(E5, 1), n(A5, 1), n(G5, 1), n(E5, 1), n(CS5, 2),
    n(D5, 1), n(E5, 1), n(FS5, 1), n(G5, 1), n(A5, 1), n(B5, 1), n(A5, 1), n(G5, 1),
    n(FS5, 2), n(E5, 1), n(CS5, 1), n(D5, 2), n(REST, 2),
];
#[rustfmt::skip]
const BASS: &[Note] = &[
    n(D3, 2), n(A3, 2), n(D3, 2), n(A3, 2),
    n(E3, 2), n(B3, 2), n(E3, 2), n(B3, 2),
    n(D3, 2), n(A3, 2), n(D3, 2), n(A3, 2),
    n(A3, 2), n(E3, 2), n(A3, 2), n(E3, 2),
    n(G3, 2), n(D3, 2), n(G3, 2), n(D3, 2),
    n(A3, 2), n(E3, 2), n(A3, 2), n(E3, 2),
    n(D3, 2), n(A3, 2), n(G3, 2), n(D3, 2),
    n(A3, 2), n(E3, 2), n(D3, 2), n(A3, 2),
];
const MUSIC: Track = Track::new(&[
    Part {
        wave: Wave::Square,
        notes: MELODY,
    },
    Part {
        wave: Wave::Triangle,
        notes: BASS,
    },
]);

/// A note of the music, `eighths` long.
const fn n(pitch: u32, eighths: u32) -> Note {
    Note::new(pitch, eighths * EIGHTH_MS)
}

/// An RGB888 colour as RGB565.
const fn rgb(r: u8, g: u8, b: u8) -> Rgb565 {
    Rgb565::new(r >> 3, g >> 2, b >> 3)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Title,
    /// The ball sits on the paddle until A launches it.
    Serving,
    Playing,
    LevelClear {
        frames_left: u32,
    },
    GameOver {
        new_best: bool,
        wait: u32,
    },
}

struct Smash {
    state: State,
    /// Hits each brick still takes; 0 is no brick.
    bricks: [[u8; BRICK_COLS]; BRICK_ROWS],
    /// Left edge.
    paddle_x: i32,
    /// Top left corner of the ball.
    ball: (f32, f32),
    /// Pixels per frame.
    velocity: (f32, f32),
    score: u32,
    best: u32,
    lives: u32,
    level: u32,
}

impl Smash {
    fn ball_speed(&self) -> f32 {
        (BALL_SPEED + BALL_SPEEDUP * (self.level - 1) as f32).min(MAX_BALL_SPEED)
    }

    fn music_speed(&self) -> u32 {
        (100 + (self.level - 1) * MUSIC_SPEEDUP).min(MAX_MUSIC_SPEED)
    }

    fn start(&mut self, ctx: &mut Ctx) {
        self.score = 0;
        self.lives = LIVES;
        self.level = 1;
        self.load_level();
        self.paddle_x = (WIDTH - PADDLE_WIDTH) / 2;
        ctx.sound().play(SOUND_START);
        ctx.sound().play_music(MUSIC);
        ctx.sound().set_music_speed(self.music_speed());
        self.state = State::Serving;
    }

    fn load_level(&mut self) {
        let layout = LEVELS[(self.level as usize - 1) % LEVELS.len()];
        for (row, line) in self.bricks.iter_mut().zip(layout) {
            for (brick, c) in row.iter_mut().zip(line.bytes()) {
                *brick = match c {
                    b'1' => 1,
                    b'2' => 2,
                    _ => 0,
                };
            }
        }
    }

    fn move_paddle(&mut self, ctx: &Ctx) {
        if ctx.input().held(Button::Left) {
            self.paddle_x -= PADDLE_SPEED;
        }
        if ctx.input().held(Button::Right) {
            self.paddle_x += PADDLE_SPEED;
        }
        self.paddle_x = self.paddle_x.clamp(0, WIDTH - PADDLE_WIDTH);
    }

    /// On the middle of the paddle.
    fn place_ball(&mut self) {
        self.ball = (
            (self.paddle_x + (PADDLE_WIDTH - BALL_SIZE) / 2) as f32,
            (PADDLE_Y - BALL_SIZE) as f32,
        );
    }

    fn launch(&mut self, ctx: &mut Ctx) {
        // -1.0 to 1.0
        let lean = ctx.random_range(0..21) as f32 / 10.0 - 1.0;
        self.set_direction(lean * MAX_LAUNCH_ANGLE);
        ctx.sound().play(SOUND_LAUNCH);
        self.state = State::Playing;
    }

    /// Sends the ball upwards at `angle` from straight up (positive is to the right).
    fn set_direction(&mut self, angle: f32) {
        let speed = self.ball_speed();
        self.velocity = (speed * angle.sin(), -speed * angle.cos());
    }

    fn ball_rect(&self) -> Rectangle {
        Rectangle::new(
            Point::new(self.ball.0 as i32, self.ball.1 as i32),
            Size::new(BALL_SIZE as u32, BALL_SIZE as u32),
        )
    }

    /// Moves the ball by one frame, in steps small enough to see every brick.
    fn move_ball(&mut self, ctx: &mut Ctx) {
        let steps = (self.ball_speed() / BALL_STEP).ceil().max(1.0);
        let max_x = (WIDTH - BALL_SIZE) as f32;
        for _ in 0..steps as u32 {
            let (dx, dy) = (self.velocity.0 / steps, self.velocity.1 / steps);

            self.ball.0 += dx;
            if self.ball.0 < 0.0 || self.ball.0 > max_x {
                self.ball.0 = self.ball.0.clamp(0.0, max_x);
                self.velocity.0 = -self.velocity.0;
                ctx.sound().play(SOUND_WALL);
            } else if self.hit_brick(ctx) {
                self.ball.0 -= dx;
                self.velocity.0 = -self.velocity.0;
            }

            self.ball.1 += dy;
            if self.ball.1 < HUD_HEIGHT as f32 {
                self.ball.1 = HUD_HEIGHT as f32;
                self.velocity.1 = self.velocity.1.abs();
                ctx.sound().play(SOUND_WALL);
            } else if self.hit_brick(ctx) {
                self.ball.1 -= dy;
                self.velocity.1 = -self.velocity.1;
            } else if self.velocity.1 > 0.0
                && self.ball_rect().intersection(&paddle(self.paddle_x)).size != Size::zero()
            {
                self.bounce_off_paddle(ctx);
            }

            if self.ball.1 > HEIGHT as f32 {
                self.lose_ball(ctx);
                return;
            }
            if self.bricks.iter().flatten().all(|&hits| hits == 0) {
                self.score += LEVEL_BONUS * self.level;
                ctx.sound().play(SOUND_LEVEL);
                self.state = State::LevelClear {
                    frames_left: LEVEL_CLEAR_FRAMES,
                };
                return;
            }
        }
    }

    /// Takes a hit off the first brick the ball touches, if any.
    fn hit_brick(&mut self, ctx: &mut Ctx) -> bool {
        let ball = self.ball_rect();
        for (row, bricks) in self.bricks.iter_mut().enumerate() {
            for (col, hits) in bricks.iter_mut().enumerate() {
                if *hits > 0 && ball.intersection(&brick_rect(row, col)).size != Size::zero() {
                    *hits -= 1;
                    self.score += BRICK_POINTS;
                    ctx.sound().play(&[Note::new(BRICK_PITCHES[row], 30)]);
                    return true;
                }
            }
        }
        false
    }

    /// The further from the middle the ball lands, the steeper it goes off to that side.
    fn bounce_off_paddle(&mut self, ctx: &mut Ctx) {
        let half = PADDLE_WIDTH as f32 / 2.0;
        let paddle_center = self.paddle_x as f32 + half;
        let ball_center = self.ball.0 + BALL_SIZE as f32 / 2.0;
        let offset = ((ball_center - paddle_center) / half).clamp(-1.0, 1.0);
        self.set_direction(offset * MAX_BOUNCE_ANGLE);
        self.ball.1 = (PADDLE_Y - BALL_SIZE) as f32;
        ctx.sound().play(SOUND_PADDLE);
    }

    fn lose_ball(&mut self, ctx: &mut Ctx) {
        self.lives -= 1;
        if self.lives == 0 {
            self.game_over(ctx);
        } else {
            ctx.sound().play(SOUND_LOSE);
            self.state = State::Serving;
        }
    }

    fn next_level(&mut self, ctx: &mut Ctx) {
        self.level += 1;
        self.load_level();
        ctx.sound().set_music_speed(self.music_speed());
        self.state = State::Serving;
    }

    fn game_over(&mut self, ctx: &mut Ctx) {
        ctx.sound().stop_music();
        let new_best = self.score > self.best;
        if new_best {
            self.best = self.score;
            if let Err(e) = ctx.saves().write("best", &self.best) {
                log::warn!("Saving the best score failed: {e:?}");
            }
            ctx.sound().play(SOUND_BEST);
        } else {
            ctx.sound().play(SOUND_DIE);
        }
        self.state = State::GameOver {
            new_best,
            wait: GAME_OVER_WAIT,
        };
    }
}

impl Game for Smash {
    fn new(ctx: &mut Ctx) -> Self {
        Smash {
            state: State::Title,
            bricks: [[0; BRICK_COLS]; BRICK_ROWS],
            paddle_x: (WIDTH - PADDLE_WIDTH) / 2,
            ball: (0.0, 0.0),
            velocity: (0.0, 0.0),
            score: 0,
            best: ctx.saves().read("best").unwrap_or(0),
            lives: LIVES,
            level: 1,
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let start = ctx.input().pressed(Button::A) || ctx.input().pressed(Button::Start);
        match self.state {
            State::Title => {
                if start {
                    self.start(ctx);
                }
            }
            State::Serving => {
                self.move_paddle(ctx);
                self.place_ball();
                if start || ctx.input().pressed(Button::Up) {
                    self.launch(ctx);
                }
            }
            State::Playing => {
                self.move_paddle(ctx);
                self.move_ball(ctx);
            }
            State::LevelClear { frames_left } => {
                if frames_left > 1 {
                    self.state = State::LevelClear {
                        frames_left: frames_left - 1,
                    };
                } else {
                    self.next_level(ctx);
                }
            }
            State::GameOver { new_best, wait } => {
                if wait > 0 {
                    self.state = State::GameOver {
                        new_best,
                        wait: wait - 1,
                    };
                } else if start {
                    self.start(ctx);
                }
            }
        }
    }

    fn draw(&self, fb: &mut Framebuffer) {
        fb.clear(BACKGROUND).ok();
        if self.state == State::Title {
            self.draw_title(fb);
            return;
        }

        self.draw_hud(fb);
        for (row, bricks) in self.bricks.iter().enumerate() {
            for (col, &hits) in bricks.iter().enumerate() {
                if hits > 0 {
                    let color = ROW_COLORS[row];
                    // Two-hit bricks are paler until hit once.
                    let color = if hits > 1 { lighter(color) } else { color };
                    brick(fb, brick_rect(row, col), color);
                }
            }
        }

        let paddle = paddle(self.paddle_x);
        paddle
            .into_styled(PrimitiveStyle::with_fill(PADDLE))
            .draw(fb)
            .ok();
        Rectangle::new(paddle.top_left, Size::new(paddle.size.width, 1))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::WHITE))
            .draw(fb)
            .ok();
        if !matches!(self.state, State::LevelClear { .. }) {
            Circle::new(self.ball_rect().top_left, BALL_SIZE as u32)
                .into_styled(PrimitiveStyle::with_fill(BALL))
                .draw(fb)
                .ok();
        }

        let center_x = WIDTH / 2;
        match self.state {
            State::Serving => text(
                fb,
                "A: launch",
                Point::new(center_x, 170),
                &FONT_8X13,
                DIM,
                Alignment::Center,
            ),
            State::LevelClear { .. } => self.draw_panel(
                fb,
                &format!("Level {} cleared!", self.level),
                &format!("Bonus {}", LEVEL_BONUS * self.level),
                TITLE,
                None,
            ),
            State::GameOver { new_best, wait } => {
                let score = if new_best {
                    format!("New best: {}!", self.score)
                } else {
                    format!("Score {}", self.score)
                };
                let hint = (wait == 0).then_some("A: play again");
                let color = if new_best { TITLE } else { TEXT };
                self.draw_panel(fb, "Game over", &score, color, hint);
            }
            State::Title | State::Playing => {}
        }
    }
}

impl Smash {
    fn draw_title(&self, fb: &mut Framebuffer) {
        let center_x = WIDTH / 2;
        text(
            fb,
            "S M A S H",
            Point::new(center_x, 52),
            &FONT_10X20,
            TITLE,
            Alignment::Center,
        );
        for (row, color) in ROW_COLORS.into_iter().enumerate().take(3) {
            for col in 2..BRICK_COLS - 2 {
                let rect = brick_rect(row, col);
                brick(
                    fb,
                    Rectangle::new(rect.top_left + Point::new(0, 40), rect.size),
                    color,
                );
            }
        }
        text(
            fb,
            "Press A to start",
            Point::new(center_x, 134),
            &FONT_8X13,
            TEXT,
            Alignment::Center,
        );
        for (i, line) in [
            "LEFT/RIGHT: move    A: launch",
            "MENU: pause         VOL: sound",
        ]
        .into_iter()
        .enumerate()
        {
            text(
                fb,
                line,
                Point::new(center_x, 170 + i as i32 * 14),
                &FONT_6X10,
                DIM,
                Alignment::Center,
            );
        }
        text(
            fb,
            &format!("Best {}", self.best),
            Point::new(center_x, 218),
            &FONT_8X13,
            DIM,
            Alignment::Center,
        );
    }

    fn draw_hud(&self, fb: &mut Framebuffer) {
        Rectangle::new(Point::zero(), Size::new(SCREEN_WIDTH, HUD_HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(HUD))
            .draw(fb)
            .ok();
        let middle = HUD_HEIGHT / 2;
        text(
            fb,
            &format!("Score {}", self.score),
            Point::new(6, middle),
            &FONT_8X13,
            TEXT,
            Alignment::Left,
        );
        text(
            fb,
            &format!("Level {}", self.level),
            Point::new(WIDTH / 2, middle),
            &FONT_8X13,
            TEXT,
            Alignment::Center,
        );
        // Balls left, including the one in play.
        for i in 0..self.lives as i32 {
            let left = WIDTH - 6 - BALL_SIZE - i * (BALL_SIZE + 4);
            Circle::new(Point::new(left, middle - BALL_SIZE / 2), BALL_SIZE as u32)
                .into_styled(PrimitiveStyle::with_fill(BALL))
                .draw(fb)
                .ok();
        }
    }

    /// A box in the middle: a heading, a line in `color` and an optional hint.
    fn draw_panel(
        &self,
        fb: &mut Framebuffer,
        heading: &str,
        line: &str,
        color: Rgb565,
        hint: Option<&str>,
    ) {
        let panel = Rectangle::with_center(Point::new(WIDTH / 2, 150), Size::new(220, 90));
        let style = PrimitiveStyleBuilder::new()
            .fill_color(HUD)
            .stroke_color(TEXT)
            .stroke_width(1)
            .build();
        panel.into_styled(style).draw(fb).ok();
        let center_x = panel.center().x;
        let top = panel.top_left.y;
        text(
            fb,
            heading,
            Point::new(center_x, top + 20),
            &FONT_10X20,
            TEXT,
            Alignment::Center,
        );
        text(
            fb,
            line,
            Point::new(center_x, top + 48),
            &FONT_8X13,
            color,
            Alignment::Center,
        );
        if let Some(hint) = hint {
            text(
                fb,
                hint,
                Point::new(center_x, top + 72),
                &FONT_8X13,
                DIM,
                Alignment::Center,
            );
        }
    }
}

fn brick_rect(row: usize, col: usize) -> Rectangle {
    Rectangle::new(
        Point::new(
            BRICK_GAP / 2 + col as i32 * (BRICK_WIDTH + BRICK_GAP),
            BRICKS_TOP + row as i32 * (BRICK_HEIGHT + BRICK_GAP),
        ),
        Size::new(BRICK_WIDTH as u32, BRICK_HEIGHT as u32),
    )
}

fn paddle(x: i32) -> Rectangle {
    Rectangle::new(
        Point::new(x, PADDLE_Y),
        Size::new(PADDLE_WIDTH as u32, PADDLE_HEIGHT as u32),
    )
}

/// A brick with a light top and left edge and a dark bottom and right edge.
fn brick(fb: &mut Framebuffer, rect: Rectangle, color: Rgb565) {
    rect.into_styled(PrimitiveStyle::with_fill(color))
        .draw(fb)
        .ok();
    let (width, height) = (rect.size.width, rect.size.height);
    let light = PrimitiveStyle::with_fill(lighter(color));
    let dark = PrimitiveStyle::with_fill(darker(color));
    for (offset, size, style) in [
        (Point::zero(), Size::new(width, 1), light),
        (Point::zero(), Size::new(1, height), light),
        (Point::new(0, height as i32 - 1), Size::new(width, 1), dark),
        (Point::new(width as i32 - 1, 0), Size::new(1, height), dark),
    ] {
        Rectangle::new(rect.top_left + offset, size)
            .into_styled(style)
            .draw(fb)
            .ok();
    }
}

/// Halfway to white.
fn lighter(color: Rgb565) -> Rgb565 {
    Rgb565::new(
        (color.r() + Rgb565::MAX_R) / 2,
        (color.g() + Rgb565::MAX_G) / 2,
        (color.b() + Rgb565::MAX_B) / 2,
    )
}

/// Halfway to black.
fn darker(color: Rgb565) -> Rgb565 {
    Rgb565::new(color.r() / 2, color.g() / 2, color.b() / 2)
}

ogo_sdk::main!(Smash);
