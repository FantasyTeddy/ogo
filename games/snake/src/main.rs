//! Snake: eat the food, don't hit the walls or yourself. Gets faster as it grows.

use std::collections::VecDeque;

use embedded_graphics::{
    mono_font::ascii::{FONT_10X20, FONT_8X13},
    primitives::{Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::Alignment,
};
use ogo_sdk::{pitch::*, prelude::*, ui::text};

const CELL: i32 = 16;
const HUD_HEIGHT: i32 = 16;
const COLS: i32 = SCREEN_WIDTH as i32 / CELL;
const ROWS: i32 = (SCREEN_HEIGHT as i32 - HUD_HEIGHT) / CELL;
const START_LENGTH: usize = 3;
/// Frames per step at the start, and the fastest it gets.
const START_INTERVAL: u32 = 6;
const MIN_INTERVAL: u32 = 2;
/// Food eaten per speed-up.
const FOOD_PER_SPEEDUP: u32 = 5;
/// Turns pressed faster than the snake moves are remembered, up to this many.
const MAX_QUEUED_TURNS: usize = 2;

const BACKGROUND: Rgb565 = Rgb565::new(2, 5, 3);
const HUD: Rgb565 = Rgb565::new(4, 8, 4);
const BODY: Rgb565 = Rgb565::CSS_LIME_GREEN;
const HEAD: Rgb565 = Rgb565::CSS_GREEN_YELLOW;
const FOOD: Rgb565 = Rgb565::CSS_RED;
const TEXT: Rgb565 = Rgb565::WHITE;
const DIM: Rgb565 = Rgb565::CSS_GRAY;

const SOUND_START: &[Note] = &[Note::new(523, 60), Note::new(784, 90)];
const SOUND_EAT: &[Note] = &[Note::new(880, 40), Note::new(1175, 60)];
const SOUND_DIE: &[Note] = &[
    Note::new(392, 120),
    Note::new(330, 120),
    Note::new(262, 240),
];
const SOUND_BEST: &[Note] = &[Note::new(784, 80), Note::new(988, 80), Note::new(1319, 160)];

/// Music: eight bars in C major, 140 beats per minute at the start.
const EIGHTH_MS: u32 = 214;
/// Music tempo in percent per step of speed-up.
const MUSIC_SPEEDUP: u32 = 10;
#[rustfmt::skip]
const MELODY: &[Note] = &[
    n(C5, 1), n(E5, 1), n(G5, 1), n(E5, 1), n(C5, 2), n(G4, 2),
    n(A4, 1), n(C5, 1), n(E5, 1), n(C5, 1), n(A4, 2), n(REST, 2),
    n(F4, 1), n(A4, 1), n(C5, 1), n(A4, 1), n(G4, 1), n(B4, 1), n(D5, 1), n(B4, 1),
    n(C5, 2), n(G4, 1), n(E4, 1), n(C4, 2), n(REST, 2),
    n(E5, 1), n(E5, 1), n(D5, 1), n(C5, 1), n(D5, 2), n(G4, 2),
    n(C5, 1), n(C5, 1), n(B4, 1), n(A4, 1), n(B4, 2), n(E4, 2),
    n(A4, 1), n(B4, 1), n(C5, 1), n(D5, 1), n(E5, 1), n(F5, 1), n(G5, 2),
    n(G5, 1), n(F5, 1), n(E5, 1), n(D5, 1), n(C5, 2), n(REST, 2),
];
#[rustfmt::skip]
const BASS: &[Note] = &[
    n(C3, 2), n(G3, 2), n(C3, 2), n(G3, 2),
    n(A3, 2), n(E3, 2), n(A3, 2), n(E3, 2),
    n(F3, 2), n(C3, 2), n(G3, 2), n(D3, 2),
    n(C3, 2), n(G3, 2), n(C3, 2), n(G3, 2),
    n(C3, 2), n(G3, 2), n(G3, 2), n(D3, 2),
    n(A3, 2), n(E3, 2), n(E3, 2), n(B3, 2),
    n(F3, 2), n(C3, 2), n(G3, 2), n(D3, 2),
    n(C3, 2), n(G3, 2), n(C3, 2), n(G3, 2),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn offset(self) -> (i32, i32) {
        match self {
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Title,
    Playing,
    GameOver { new_best: bool },
}

struct Snake {
    state: State,
    /// Head first.
    body: VecDeque<(i32, i32)>,
    direction: Direction,
    turns: VecDeque<Direction>,
    food: (i32, i32),
    eaten: u32,
    best: u32,
    /// Frames until the next step.
    countdown: u32,
}

impl Snake {
    fn score(&self) -> u32 {
        self.eaten
    }

    fn interval(&self) -> u32 {
        START_INTERVAL
            .saturating_sub(self.eaten / FOOD_PER_SPEEDUP)
            .max(MIN_INTERVAL)
    }

    fn start(&mut self, ctx: &mut Ctx) {
        let (x, y) = (COLS / 2, ROWS / 2);
        self.body = (0..START_LENGTH as i32).map(|i| (x - i, y)).collect();
        self.direction = Direction::Right;
        self.turns.clear();
        self.eaten = 0;
        self.countdown = self.interval();
        self.place_food(ctx);
        self.state = State::Playing;
        ctx.sound().play(SOUND_START);
        ctx.sound().play_music(MUSIC);
        ctx.sound().set_music_speed(self.music_speed());
    }

    /// Faster with the snake.
    fn music_speed(&self) -> u32 {
        100 + (START_INTERVAL - self.interval()) * MUSIC_SPEEDUP
    }

    fn place_food(&mut self, ctx: &Ctx) {
        let free = (COLS * ROWS) as usize - self.body.len();
        if free == 0 {
            return;
        }
        // The n-th free cell, counted row by row.
        let mut n = ctx.random_range(0..free as u32);
        for y in 0..ROWS {
            for x in 0..COLS {
                if self.body.contains(&(x, y)) {
                    continue;
                }
                if n == 0 {
                    self.food = (x, y);
                    return;
                }
                n -= 1;
            }
        }
    }

    fn queue_turns(&mut self, ctx: &Ctx) {
        for (button, direction) in [
            (Button::Up, Direction::Up),
            (Button::Down, Direction::Down),
            (Button::Left, Direction::Left),
            (Button::Right, Direction::Right),
        ] {
            if !ctx.input().pressed(button) {
                continue;
            }
            let last = self.turns.back().copied().unwrap_or(self.direction);
            if direction != last
                && direction != last.opposite()
                && self.turns.len() < MAX_QUEUED_TURNS
            {
                self.turns.push_back(direction);
            }
        }
    }

    fn step(&mut self, ctx: &mut Ctx) {
        if let Some(turn) = self.turns.pop_front() {
            self.direction = turn;
        }
        let (dx, dy) = self.direction.offset();
        let (hx, hy) = self.body[0];
        let head = (hx + dx, hy + dy);

        let eats = head == self.food;
        if !eats {
            self.body.pop_back();
        }
        let hits_wall = head.0 < 0 || head.0 >= COLS || head.1 < 0 || head.1 >= ROWS;
        if hits_wall || self.body.contains(&head) {
            self.game_over(ctx);
            return;
        }
        self.body.push_front(head);

        if eats {
            self.eaten += 1;
            ctx.sound().play(SOUND_EAT);
            ctx.sound().set_music_speed(self.music_speed());
            self.place_food(ctx);
        }
    }

    fn game_over(&mut self, ctx: &mut Ctx) {
        ctx.sound().stop_music();
        let new_best = self.score() > self.best;
        if new_best {
            self.best = self.score();
            if let Err(e) = ctx.saves().write("best", &self.best) {
                log::warn!("Saving the best score failed: {e:?}");
            }
            ctx.sound().play(SOUND_BEST);
        } else {
            ctx.sound().play(SOUND_DIE);
        }
        self.state = State::GameOver { new_best };
    }
}

impl Game for Snake {
    fn new(ctx: &mut Ctx) -> Self {
        Snake {
            state: State::Title,
            body: VecDeque::new(),
            direction: Direction::Right,
            turns: VecDeque::new(),
            food: (0, 0),
            eaten: 0,
            best: ctx.saves().read("best").unwrap_or(0),
            countdown: 0,
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        match self.state {
            State::Title | State::GameOver { .. } => {
                if ctx.input().pressed(Button::A) || ctx.input().pressed(Button::Start) {
                    self.start(ctx);
                }
            }
            State::Playing => {
                self.queue_turns(ctx);
                self.countdown = self.countdown.saturating_sub(1);
                if self.countdown == 0 {
                    self.countdown = self.interval();
                    self.step(ctx);
                }
            }
        }
    }

    fn draw(&self, fb: &mut Framebuffer) {
        fb.clear(BACKGROUND).ok();

        // Score bar
        Rectangle::new(Point::zero(), Size::new(SCREEN_WIDTH, HUD_HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(HUD))
            .draw(fb)
            .ok();
        let middle = HUD_HEIGHT / 2;
        text(
            fb,
            &format!("Score {}", self.score()),
            Point::new(6, middle),
            &FONT_8X13,
            TEXT,
            Alignment::Left,
        );
        text(
            fb,
            &format!("Best {}", self.best),
            Point::new(SCREEN_WIDTH as i32 - 6, middle),
            &FONT_8X13,
            TEXT,
            Alignment::Right,
        );

        match self.state {
            State::Title => {
                let center_x = SCREEN_WIDTH as i32 / 2;
                text(
                    fb,
                    "S N A K E",
                    Point::new(center_x, 90),
                    &FONT_10X20,
                    HEAD,
                    Alignment::Center,
                );
                text(
                    fb,
                    "Press A to start",
                    Point::new(center_x, 140),
                    &FONT_8X13,
                    TEXT,
                    Alignment::Center,
                );
                text(
                    fb,
                    "MENU: pause   VOL: volume",
                    Point::new(center_x, 200),
                    &FONT_8X13,
                    DIM,
                    Alignment::Center,
                );
            }
            State::Playing | State::GameOver { .. } => {
                self.draw_field(fb);
                if let State::GameOver { new_best } = self.state {
                    self.draw_game_over(fb, new_best);
                }
            }
        }
    }
}

impl Snake {
    fn draw_field(&self, fb: &mut Framebuffer) {
        let (fx, fy) = cell_origin(self.food);
        Circle::new(Point::new(fx + 2, fy + 2), (CELL - 4) as u32)
            .into_styled(PrimitiveStyle::with_fill(FOOD))
            .draw(fb)
            .ok();

        for (i, &cell) in self.body.iter().enumerate() {
            let (x, y) = cell_origin(cell);
            let color = if i == 0 { HEAD } else { BODY };
            Rectangle::new(
                Point::new(x + 1, y + 1),
                Size::new(CELL as u32 - 2, CELL as u32 - 2),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(fb)
            .ok();
        }
    }

    fn draw_game_over(&self, fb: &mut Framebuffer, new_best: bool) {
        let panel = Rectangle::with_center(
            Point::new(SCREEN_WIDTH as i32 / 2, 128),
            Size::new(220, 100),
        );
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
            "Game over",
            Point::new(center_x, top + 20),
            &FONT_10X20,
            TEXT,
            Alignment::Center,
        );
        let score = if new_best {
            format!("New best: {}!", self.score())
        } else {
            format!("Score {}", self.score())
        };
        text(
            fb,
            &score,
            Point::new(center_x, top + 50),
            &FONT_8X13,
            if new_best { HEAD } else { TEXT },
            Alignment::Center,
        );
        text(
            fb,
            "A: play again",
            Point::new(center_x, top + 78),
            &FONT_8X13,
            DIM,
            Alignment::Center,
        );
    }
}

fn cell_origin((x, y): (i32, i32)) -> (i32, i32) {
    (x * CELL, HUD_HEIGHT + y * CELL)
}

ogo_sdk::main!(Snake);
