//! Double buffering: the game draws into one full-screen buffer while a display thread sends the
//! other one, or the parts of it that changed, to the LCD.

use std::sync::mpsc::{self, Receiver, SyncSender};

use anyhow::{anyhow, Context};
use embedded_graphics::{prelude::*, primitives::Rectangle};
use ogo_board::{display, Canvas, Display};

/// A frame and the areas of it to send to the display.
type Frame = (Box<Canvas>, Vec<Rectangle>);

pub struct Presenter {
    /// The buffer the game draws into, between [`Presenter::frame`] and [`Presenter::present`].
    drawing: Option<Box<Canvas>>,
    to_display: SyncSender<Frame>,
    free: Receiver<Box<Canvas>>,
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
        let (to_game, free) = mpsc::sync_channel::<Box<Canvas>>(2);
        to_game.send(Box::new(Canvas::new(screen)))?;

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
            drawing: Some(Box::new(Canvas::new(screen))),
            to_display,
            free,
        })
    }

    /// The buffer to draw the next frame into. Waits until the display thread has sent it.
    pub fn frame(&mut self) -> anyhow::Result<&mut Canvas> {
        if self.drawing.is_none() {
            self.drawing = Some(
                self.free
                    .recv()
                    .map_err(|_| anyhow!("the display thread has stopped"))?,
            );
        }
        Ok(self.drawing.as_mut().expect("just filled"))
    }

    /// Hands the drawn frame to the display thread, which sends `areas` of it to the display.
    pub fn present(&mut self, areas: Vec<Rectangle>) -> anyhow::Result<()> {
        if let Some(frame) = self.drawing.take() {
            self.to_display
                .send((frame, areas))
                .map_err(|_| anyhow!("the display thread has stopped"))?;
        }
        Ok(())
    }
}
