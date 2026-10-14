//! Double buffering: the game draws into one buffer while a display thread sends another to the
//! LCD.
//!
//! Full-screen buffers are in PSRAM. A game that uses only part of the screen ([`Game::area`])
//! draws into buffers of just that size in internal RAM instead, which are much faster to write
//! and to send.
//!
//! [`Game::area`]: crate::Game::area

use std::sync::mpsc::{self, Receiver, SyncSender};

use anyhow::{anyhow, Context};
use embedded_graphics::{prelude::*, primitives::Rectangle};
use ogo_board::{display, in_internal_ram, Canvas, Display};

/// A buffer and the parts of it (in its own coordinates) to send to the display.
type Frame = (Canvas, Vec<Rectangle>);

/// Buffers of each kind: one being drawn, one being sent.
const BUFFERS: usize = 2;

pub struct Presenter {
    screen: Rectangle,
    to_display: SyncSender<Frame>,
    sent: Receiver<Canvas>,
    /// Free full-screen buffers.
    full: Vec<Canvas>,
    /// The game's area and its free buffers.
    area: Option<Rectangle>,
    area_buffers: Vec<Canvas>,
}

/// The display handle only ever lives in the display thread once it's been moved there.
struct SendDisplay(Display);
unsafe impl Send for SendDisplay {}

impl Presenter {
    pub fn new(display: Display) -> anyhow::Result<Self> {
        let screen = Rectangle::new(
            Point::zero(),
            Size::new(display::WIDTH.into(), display::HEIGHT.into()),
        );
        let (to_display, frames) = mpsc::sync_channel::<Frame>(1);
        // Room for the small extra buffers too (the volume box, clearing where it was)
        let (to_game, sent) = mpsc::sync_channel::<Canvas>(4 * BUFFERS);

        let display = SendDisplay(display);
        ogo_board::thread::spawn_on_second_core("display", 8192, move || {
            let SendDisplay(mut display) = display;
            for (frame, areas) in frames {
                for area in areas {
                    if let Err(e) = frame.present_area(&mut display, area) {
                        log::error!("Sending a frame to the display failed: {e:?}");
                    }
                }
                if to_game.send(frame).is_err() {
                    return;
                }
            }
        })
        .context("can't start the display thread")?;

        Ok(Self {
            screen,
            to_display,
            sent,
            full: (0..BUFFERS).map(|_| Canvas::new(screen)).collect(),
            area: None,
            area_buffers: Vec::new(),
        })
    }

    /// A full-screen buffer to draw into. Waits until the display thread has sent one.
    pub fn take_full(&mut self) -> anyhow::Result<Canvas> {
        loop {
            if let Some(canvas) = self.full.pop() {
                return Ok(canvas);
            }
            self.receive()?;
        }
    }

    /// A buffer covering `area` of the screen, in internal RAM. Waits until the display thread has
    /// sent one. For a new area, both buffers are made at once, while internal RAM still has
    /// blocks large enough.
    pub fn take_area(&mut self, area: Rectangle) -> anyhow::Result<Canvas> {
        if self.area != Some(area) {
            self.area = Some(area);
            self.area_buffers = (0..BUFFERS)
                .map(|_| in_internal_ram(|| Canvas::new(area)))
                .collect();
        }
        loop {
            if let Some(canvas) = self.area_buffers.pop() {
                return Ok(canvas);
            }
            self.receive()?;
        }
    }

    /// Takes back a buffer that wasn't sent.
    pub fn recycle(&mut self, canvas: Canvas) {
        if canvas.area() == self.screen {
            self.full.push(canvas);
        } else if Some(canvas.area()) == self.area {
            self.area_buffers.push(canvas);
        }
        // Otherwise it's for an area the game no longer uses: dropped
    }

    /// Hands a drawn buffer to the display thread, which sends `areas` of it (in the buffer's own
    /// coordinates) to the display.
    pub fn present(&mut self, canvas: Canvas, areas: Vec<Rectangle>) -> anyhow::Result<()> {
        self.to_display
            .send((canvas, areas))
            .map_err(|_| anyhow!("the display thread has stopped"))
    }

    fn receive(&mut self) -> anyhow::Result<()> {
        let canvas = self
            .sent
            .recv()
            .map_err(|_| anyhow!("the display thread has stopped"))?;
        self.recycle(canvas);
        Ok(())
    }
}
