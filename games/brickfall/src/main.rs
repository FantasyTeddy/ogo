//! Brickfall: steer the falling bricks and fill whole rows to clear them. Gets faster every ten
//! rows.

use embedded_graphics::{
    mono_font::ascii::{FONT_10X20, FONT_6X10, FONT_8X13},
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::Alignment,
};
use ogo_sdk::{prelude::*, ui::text};

const COLS: usize = 10;
const ROWS: usize = 20;
/// Rows above the visible ones, where new pieces start.
const HIDDEN_ROWS: usize = 2;
const ALL_ROWS: usize = HIDDEN_ROWS + ROWS;

const CELL: i32 = 11;
const BOARD_WIDTH: i32 = COLS as i32 * CELL;
const BOARD_HEIGHT: i32 = ROWS as i32 * CELL;
const BOARD_X: i32 = (SCREEN_WIDTH as i32 - BOARD_WIDTH) / 2;
const BOARD_Y: i32 = (SCREEN_HEIGHT as i32 - BOARD_HEIGHT) / 2;
/// Centres of the panels left and right of the board.
const LEFT_X: i32 = BOARD_X / 2;
const RIGHT_X: i32 = (BOARD_X + BOARD_WIDTH + SCREEN_WIDTH as i32) / 2;

/// Frames per row of falling, by level; the last one applies to all higher levels.
const GRAVITY: [u32; 15] = [30, 25, 21, 17, 14, 11, 9, 7, 6, 5, 4, 3, 3, 2, 1];
const LINES_PER_LEVEL: u32 = 10;
/// Frames per row while DOWN is held.
const SOFT_DROP_INTERVAL: u32 = 1;
/// Frames a piece lies on something before it locks. Moving or turning it starts over, up to
/// `MAX_LOCK_RESETS` times per piece.
const LOCK_DELAY: u32 = 15;
const MAX_LOCK_RESETS: u32 = 15;
/// Holding LEFT or RIGHT: frames until it repeats, then frames per step.
const REPEAT_DELAY: u32 = 7;
const REPEAT_INTERVAL: u32 = 2;
/// Frames the full rows flash before they disappear.
const CLEAR_FRAMES: u32 = 12;
/// Frames after game over before A starts a new game, so turning a piece doesn't skip the score.
const GAME_OVER_WAIT: u32 = 30;
/// Points for 1 to 4 rows at once, times the level.
const LINE_POINTS: [u32; 4] = [100, 300, 500, 800];
/// Points per row for a soft drop (DOWN) and a hard drop (UP).
const SOFT_DROP_POINTS: u32 = 1;
const HARD_DROP_POINTS: u32 = 2;
/// Positions tried, in order, when a turned piece doesn't fit where it is.
const KICKS: [(i32, i32); 6] = [(0, 0), (-1, 0), (1, 0), (-2, 0), (2, 0), (0, -1)];

const BACKGROUND: Rgb565 = rgb(20, 22, 32);
const WELL: Rgb565 = rgb(6, 7, 12);
const FRAME: Rgb565 = rgb(90, 96, 120);
const PANEL: Rgb565 = rgb(32, 35, 50);
const TITLE: Rgb565 = rgb(222, 184, 110);
const TEXT: Rgb565 = Rgb565::WHITE;
const DIM: Rgb565 = Rgb565::CSS_GRAY;
const FLASH: Rgb565 = Rgb565::WHITE;

const SOUND_START: &[Note] = &[Note::new(392, 60), Note::new(523, 60), Note::new(659, 90)];
const SOUND_TURN: &[Note] = &[Note::new(740, 15)];
const SOUND_LOCK: &[Note] = &[Note::new(147, 35)];
const SOUND_CLEAR: [&[Note]; 4] = [
    &[Note::new(523, 50), Note::new(659, 70)],
    &[Note::new(523, 50), Note::new(659, 50), Note::new(784, 70)],
    &[
        Note::new(523, 45),
        Note::new(659, 45),
        Note::new(784, 45),
        Note::new(1047, 80),
    ],
    &[
        Note::new(523, 40),
        Note::new(659, 40),
        Note::new(784, 40),
        Note::new(1047, 40),
        Note::new(1319, 40),
        Note::new(1568, 120),
    ],
];
const SOUND_LEVEL: &[Note] = &[Note::new(0, 60), Note::new(1047, 60), Note::new(1397, 120)];
const SOUND_DIE: &[Note] = &[
    Note::new(330, 120),
    Note::new(262, 120),
    Note::new(196, 120),
    Note::new(147, 300),
];
const SOUND_BEST: &[Note] = &[Note::new(784, 80), Note::new(988, 80), Note::new(1319, 160)];

/// An RGB888 colour as RGB565.
const fn rgb(r: u8, g: u8, b: u8) -> Rgb565 {
    Rgb565::new(r >> 3, g >> 2, b >> 3)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    I,
    O,
    T,
    S,
    Z,
    J,
    L,
}

const KINDS: [Kind; 7] = [
    Kind::I,
    Kind::O,
    Kind::T,
    Kind::S,
    Kind::Z,
    Kind::J,
    Kind::L,
];

impl Kind {
    /// The cells when the piece appears, in a `size` × `size` box.
    fn cells(self) -> [(i32, i32); 4] {
        match self {
            Kind::I => [(0, 1), (1, 1), (2, 1), (3, 1)],
            Kind::O => [(0, 0), (1, 0), (0, 1), (1, 1)],
            Kind::T => [(1, 0), (0, 1), (1, 1), (2, 1)],
            Kind::S => [(1, 0), (2, 0), (0, 1), (1, 1)],
            Kind::Z => [(0, 0), (1, 0), (1, 1), (2, 1)],
            Kind::J => [(0, 0), (0, 1), (1, 1), (2, 1)],
            Kind::L => [(2, 0), (0, 1), (1, 1), (2, 1)],
        }
    }

    /// The piece turns within a box this wide and high.
    fn size(self) -> i32 {
        match self {
            Kind::I => 4,
            Kind::O => 2,
            _ => 3,
        }
    }

    fn color(self) -> Rgb565 {
        match self {
            Kind::I => rgb(204, 96, 64),
            Kind::O => rgb(222, 184, 110),
            Kind::T => rgb(156, 96, 168),
            Kind::S => rgb(116, 166, 84),
            Kind::Z => rgb(176, 56, 60),
            Kind::J => rgb(84, 116, 180),
            Kind::L => rgb(64, 156, 156),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    kind: Kind,
    /// Quarter turns clockwise.
    rotation: u8,
    x: i32,
    y: i32,
}

impl Piece {
    fn new(kind: Kind) -> Self {
        Piece {
            kind,
            rotation: 0,
            x: (COLS as i32 - kind.size()) / 2,
            // The lower row of the piece is the top visible row.
            y: HIDDEN_ROWS as i32 - 1,
        }
    }

    fn moved(self, dx: i32, dy: i32) -> Self {
        Piece {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }

    fn turned(self, quarter_turns: u8) -> Self {
        Piece {
            rotation: (self.rotation + quarter_turns) % 4,
            ..self
        }
    }

    /// The board cells it covers.
    fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        let n = self.kind.size();
        self.kind.cells().into_iter().map(move |(mut x, mut y)| {
            for _ in 0..self.rotation {
                (x, y) = (n - 1 - y, x);
            }
            (self.x + x, self.y + y)
        })
    }
}

type Board = [[Option<Kind>; COLS]; ALL_ROWS];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Title,
    Playing,
    /// Full rows flash before they disappear.
    Clearing {
        frames_left: u32,
    },
    GameOver {
        new_best: bool,
        wait: u32,
    },
}

struct Brickfall {
    state: State,
    board: Board,
    piece: Piece,
    next: Kind,
    /// The rest of the current shuffled set of all seven kinds.
    bag: Vec<Kind>,
    full_rows: Vec<usize>,
    score: u32,
    lines: u32,
    best: u32,
    /// Frames until the piece falls a row.
    fall_countdown: u32,
    /// Frames the piece has been lying on something.
    lock_frames: u32,
    lock_resets: u32,
    /// LEFT/RIGHT held: -1, 1, or 0 for neither; and for how many frames.
    shift_direction: i32,
    shift_frames: u32,
}

impl Brickfall {
    fn level(&self) -> u32 {
        1 + self.lines / LINES_PER_LEVEL
    }

    fn gravity(&self) -> u32 {
        let index = (self.level() as usize - 1).min(GRAVITY.len() - 1);
        GRAVITY[index]
    }

    fn start(&mut self, ctx: &mut Ctx) {
        self.board = [[None; COLS]; ALL_ROWS];
        self.bag.clear();
        self.score = 0;
        self.lines = 0;
        self.next = self.next_kind(ctx);
        self.state = State::Playing;
        ctx.sound().play(SOUND_START);
        self.spawn(ctx);
    }

    fn next_kind(&mut self, ctx: &Ctx) -> Kind {
        if self.bag.is_empty() {
            self.bag.extend_from_slice(&KINDS);
            for i in (1..self.bag.len()).rev() {
                let j = ctx.random_range(0..i as u32 + 1) as usize;
                self.bag.swap(i, j);
            }
        }
        self.bag.pop().unwrap_or(Kind::T)
    }

    fn spawn(&mut self, ctx: &mut Ctx) {
        self.piece = Piece::new(self.next);
        self.next = self.next_kind(ctx);
        self.fall_countdown = self.gravity();
        self.lock_frames = 0;
        self.lock_resets = 0;
        if !self.fits(self.piece) {
            self.game_over(ctx);
        }
    }

    fn fits(&self, piece: Piece) -> bool {
        piece.cells().all(|(x, y)| {
            (0..COLS as i32).contains(&x)
                && (0..ALL_ROWS as i32).contains(&y)
                && self.board[y as usize][x as usize].is_none()
        })
    }

    fn grounded(&self) -> bool {
        !self.fits(self.piece.moved(0, 1))
    }

    /// Moves the piece if it fits there.
    fn try_move(&mut self, dx: i32, dy: i32) -> bool {
        let moved = self.piece.moved(dx, dy);
        if !self.fits(moved) {
            return false;
        }
        self.piece = moved;
        if dx != 0 {
            self.delay_lock();
        }
        true
    }

    fn turn(&mut self, quarter_turns: u8, ctx: &mut Ctx) {
        let turned = self.piece.turned(quarter_turns);
        let Some(kicked) = KICKS
            .iter()
            .map(|&(dx, dy)| turned.moved(dx, dy))
            .find(|&piece| self.fits(piece))
        else {
            return;
        };
        self.piece = kicked;
        self.delay_lock();
        ctx.sound().play(SOUND_TURN);
    }

    /// The piece moved while lying on something: give it time again, but not forever.
    fn delay_lock(&mut self) {
        if self.lock_frames > 0 && self.lock_resets < MAX_LOCK_RESETS {
            self.lock_frames = 0;
            self.lock_resets += 1;
        }
    }

    /// Where the piece would land.
    fn landing(&self) -> Piece {
        let mut piece = self.piece;
        while self.fits(piece.moved(0, 1)) {
            piece = piece.moved(0, 1);
        }
        piece
    }

    fn play(&mut self, ctx: &mut Ctx) {
        let input = *ctx.input();

        if input.pressed(Button::A) {
            self.turn(1, ctx);
        } else if input.pressed(Button::B) {
            self.turn(3, ctx);
        }

        let direction = match (input.held(Button::Left), input.held(Button::Right)) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        if direction != self.shift_direction {
            self.shift_direction = direction;
            self.shift_frames = 0;
        }
        if direction != 0 {
            let repeat = self.shift_frames >= REPEAT_DELAY
                && (self.shift_frames - REPEAT_DELAY).is_multiple_of(REPEAT_INTERVAL);
            if self.shift_frames == 0 || repeat {
                self.try_move(direction, 0);
            }
            self.shift_frames += 1;
        }

        if input.pressed(Button::Up) {
            let landing = self.landing();
            self.score += (landing.y - self.piece.y) as u32 * HARD_DROP_POINTS;
            self.piece = landing;
            self.lock(ctx);
            return;
        }

        let soft_drop = input.held(Button::Down);
        let interval = if soft_drop {
            SOFT_DROP_INTERVAL.min(self.gravity())
        } else {
            self.gravity()
        };
        self.fall_countdown = self.fall_countdown.min(interval).saturating_sub(1);
        if self.fall_countdown == 0 {
            self.fall_countdown = interval;
            if self.try_move(0, 1) && soft_drop {
                self.score += SOFT_DROP_POINTS;
            }
        }

        if self.grounded() {
            self.lock_frames += 1;
            if self.lock_frames >= LOCK_DELAY {
                self.lock(ctx);
            }
        } else {
            self.lock_frames = 0;
        }
    }

    /// The piece becomes part of the board.
    fn lock(&mut self, ctx: &mut Ctx) {
        for (x, y) in self.piece.cells() {
            self.board[y as usize][x as usize] = Some(self.piece.kind);
        }
        self.full_rows = (0..ALL_ROWS)
            .filter(|&row| self.board[row].iter().all(Option::is_some))
            .collect();
        if self.full_rows.is_empty() {
            ctx.sound().play(SOUND_LOCK);
            self.spawn(ctx);
        } else {
            let count = self.full_rows.len().min(LINE_POINTS.len());
            self.score += LINE_POINTS[count - 1] * self.level();
            ctx.sound().play(SOUND_CLEAR[count - 1]);
            self.state = State::Clearing {
                frames_left: CLEAR_FRAMES,
            };
        }
    }

    fn clear_rows(&mut self, ctx: &mut Ctx) {
        let mut board = [[None; COLS]; ALL_ROWS];
        let mut target = ALL_ROWS;
        for row in (0..ALL_ROWS).rev() {
            if !self.full_rows.contains(&row) {
                target -= 1;
                board[target] = self.board[row];
            }
        }
        self.board = board;

        let level = self.level();
        self.lines += self.full_rows.len() as u32;
        self.full_rows.clear();
        if self.level() > level {
            ctx.sound().play(SOUND_LEVEL);
        }
        self.state = State::Playing;
        self.spawn(ctx);
    }

    fn game_over(&mut self, ctx: &mut Ctx) {
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

impl Game for Brickfall {
    fn new(ctx: &mut Ctx) -> Self {
        Brickfall {
            state: State::Title,
            board: [[None; COLS]; ALL_ROWS],
            piece: Piece::new(Kind::T),
            next: Kind::T,
            bag: Vec::with_capacity(KINDS.len()),
            full_rows: Vec::with_capacity(4),
            score: 0,
            lines: 0,
            best: ctx.saves().read("best").unwrap_or(0),
            fall_countdown: 0,
            lock_frames: 0,
            lock_resets: 0,
            shift_direction: 0,
            shift_frames: 0,
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
            State::Playing => self.play(ctx),
            State::Clearing { frames_left } => {
                if frames_left > 1 {
                    self.state = State::Clearing {
                        frames_left: frames_left - 1,
                    };
                } else {
                    self.clear_rows(ctx);
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
        match self.state {
            State::Title => self.draw_title(fb),
            State::Playing | State::Clearing { .. } | State::GameOver { .. } => {
                self.draw_board(fb);
                self.draw_panels(fb);
                if let State::GameOver { new_best, wait } = self.state {
                    self.draw_game_over(fb, new_best, wait == 0);
                }
            }
        }
    }
}

impl Brickfall {
    fn draw_title(&self, fb: &mut Framebuffer) {
        let center_x = SCREEN_WIDTH as i32 / 2;
        text(
            fb,
            "B R I C K F A L L",
            Point::new(center_x, 56),
            &FONT_10X20,
            TITLE,
            Alignment::Center,
        );
        let gap = 6;
        let left = center_x - (KINDS.len() as i32 * (CELL + gap) - gap) / 2;
        for (i, kind) in KINDS.into_iter().enumerate() {
            brick(
                fb,
                Point::new(left + i as i32 * (CELL + gap), 80),
                kind.color(),
            );
        }
        text(
            fb,
            "Press A to start",
            Point::new(center_x, 122),
            &FONT_8X13,
            TEXT,
            Alignment::Center,
        );
        for (i, line) in [
            "LEFT/RIGHT: move    A/B: turn",
            "DOWN: drop faster   UP: drop",
            "MENU: pause         VOL: volume",
        ]
        .into_iter()
        .enumerate()
        {
            text(
                fb,
                line,
                Point::new(center_x, 160 + i as i32 * 14),
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

    fn draw_board(&self, fb: &mut Framebuffer) {
        let style = PrimitiveStyleBuilder::new()
            .fill_color(WELL)
            .stroke_color(FRAME)
            .stroke_width(1)
            .build();
        Rectangle::new(
            Point::new(BOARD_X - 1, BOARD_Y - 1),
            Size::new(BOARD_WIDTH as u32 + 2, BOARD_HEIGHT as u32 + 2),
        )
        .into_styled(style)
        .draw(fb)
        .ok();

        let flash =
            matches!(self.state, State::Clearing { frames_left } if frames_left / 3 % 2 == 0);
        for (y, row) in self.board.iter().enumerate().skip(HIDDEN_ROWS) {
            let color = (flash && self.full_rows.contains(&y)).then_some(FLASH);
            for (x, cell) in row.iter().enumerate() {
                if let Some(kind) = cell {
                    brick(
                        fb,
                        cell_origin(x as i32, y as i32),
                        color.unwrap_or(kind.color()),
                    );
                }
            }
        }

        if self.state == State::Playing {
            let ghost = PrimitiveStyle::with_stroke(darker(self.piece.kind.color()), 1);
            for (x, y) in self
                .landing()
                .cells()
                .filter(|&(_, y)| y >= HIDDEN_ROWS as i32)
            {
                Rectangle::new(
                    cell_origin(x, y),
                    Size::new(CELL as u32 - 1, CELL as u32 - 1),
                )
                .into_styled(ghost)
                .draw(fb)
                .ok();
            }
            for (x, y) in self.piece.cells().filter(|&(_, y)| y >= HIDDEN_ROWS as i32) {
                brick(fb, cell_origin(x, y), self.piece.kind.color());
            }
        }
    }

    fn draw_panels(&self, fb: &mut Framebuffer) {
        for (i, (label, value)) in [
            ("SCORE", self.score),
            ("LINES", self.lines),
            ("LEVEL", self.level()),
        ]
        .into_iter()
        .enumerate()
        {
            stat(fb, LEFT_X, 30 + i as i32 * 56, label, value);
        }

        text(
            fb,
            "NEXT",
            Point::new(RIGHT_X, 30),
            &FONT_8X13,
            DIM,
            Alignment::Center,
        );
        let preview = Rectangle::with_center(Point::new(RIGHT_X, 66), Size::new(64, 44));
        preview
            .into_styled(PrimitiveStyle::with_fill(PANEL))
            .draw(fb)
            .ok();
        let cells = self.next.cells();
        let (min_x, max_x) = min_max(cells.iter().map(|&(x, _)| x));
        let (min_y, max_y) = min_max(cells.iter().map(|&(_, y)| y));
        let width = (max_x - min_x + 1) * CELL;
        let height = (max_y - min_y + 1) * CELL;
        let origin = preview.center() - Point::new(width / 2, height / 2);
        for (x, y) in cells {
            let offset = Point::new((x - min_x) * CELL, (y - min_y) * CELL);
            brick(fb, origin + offset, self.next.color());
        }

        stat(fb, RIGHT_X, 142, "BEST", self.best);
    }

    fn draw_game_over(&self, fb: &mut Framebuffer, new_best: bool, ready: bool) {
        let panel = Rectangle::with_center(
            Point::new(SCREEN_WIDTH as i32 / 2, SCREEN_HEIGHT as i32 / 2),
            Size::new(200, 100),
        );
        let style = PrimitiveStyleBuilder::new()
            .fill_color(PANEL)
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
            format!("New best: {}!", self.score)
        } else {
            format!("Score {}", self.score)
        };
        text(
            fb,
            &score,
            Point::new(center_x, top + 50),
            &FONT_8X13,
            if new_best { TITLE } else { TEXT },
            Alignment::Center,
        );
        if ready {
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
}

fn cell_origin(x: i32, y: i32) -> Point {
    Point::new(
        BOARD_X + x * CELL,
        BOARD_Y + (y - HIDDEN_ROWS as i32) * CELL,
    )
}

/// One cell: a block with a light top and left edge and a dark bottom and right edge, and a gap of
/// one pixel to the next cell.
fn brick(fb: &mut Framebuffer, origin: Point, color: Rgb565) {
    let size = CELL as u32 - 1;
    let last = size as i32 - 1;
    Rectangle::new(origin, Size::new(size, size))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(fb)
        .ok();
    let light = PrimitiveStyle::with_fill(lighter(color));
    let dark = PrimitiveStyle::with_fill(darker(color));
    for (offset, size, style) in [
        (Point::zero(), Size::new(size, 1), light),
        (Point::zero(), Size::new(1, size), light),
        (Point::new(0, last), Size::new(size, 1), dark),
        (Point::new(last, 0), Size::new(1, size), dark),
    ] {
        Rectangle::new(origin + offset, size)
            .into_styled(style)
            .draw(fb)
            .ok();
    }
}

/// A label with a number below it.
fn stat(fb: &mut Framebuffer, center_x: i32, y: i32, label: &str, value: u32) {
    text(
        fb,
        label,
        Point::new(center_x, y),
        &FONT_8X13,
        DIM,
        Alignment::Center,
    );
    text(
        fb,
        &value.to_string(),
        Point::new(center_x, y + 20),
        &FONT_10X20,
        TEXT,
        Alignment::Center,
    );
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

fn min_max(values: impl Iterator<Item = i32>) -> (i32, i32) {
    values.fold((i32::MAX, i32::MIN), |(min, max), v| {
        (min.min(v), max.max(v))
    })
}

ogo_sdk::main!(Brickfall);
