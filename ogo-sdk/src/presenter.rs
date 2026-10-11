//! Double buffering: the game draws into one full-screen buffer while a display thread sends the
//! other one to the LCD.

use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

use anyhow::{anyhow, Context};
use embedded_graphics::{prelude::*, primitives::Rectangle};
use ogo_board::{display, Canvas, Display};

pub struct Presenter {
    /// The buffer the game draws into, between [`Presenter::frame`] and [`Presenter::present`].
    drawing: Option<Box<Canvas>>,
    to_display: SyncSender<Box<Canvas>>,
    free: Receiver<Box<Canvas>>,
}

/// The display handle only ever lives in the display thread once it's been moved there.
struct SendDisplay(Display);
unsafe impl Send for SendDisplay {}

impl Presenter {
    pub fn new(display: Display) -> anyhow::Result<Self> {
        let screen = Rectangle::new(Point::zero(), Size::new(display::WIDTH.into(), display::HEIGHT.into()));
        let (to_display, frames) = mpsc::sync_channel::<Box<Canvas>>(1);
        let (to_game, free) = mpsc::sync_channel::<Box<Canvas>>(2);
        to_game.send(Box::new(Canvas::new(screen)))?;

        let display = SendDisplay(display);
        thread::Builder::new()
            .name("display".into())
            .stack_size(8192)
            .spawn(move || {
                let SendDisplay(mut display) = display;
                for frame in frames {
                    if let Err(e) = frame.present(&mut display) {
                        log::error!("Sending a frame to the display failed: {e:?}");
                    }
                    if to_game.send(frame).is_err() {
                        return;
                    }
                }
            })
            .context("can't start the display thread")?;

        Ok(Self { drawing: Some(Box::new(Canvas::new(screen))), to_display, free })
    }

    /// The buffer to draw the next frame into. Waits until the display thread has sent it.
    pub fn frame(&mut self) -> anyhow::Result<&mut Canvas> {
        if self.drawing.is_none() {
            self.drawing = Some(self.free.recv().map_err(|_| anyhow!("the display thread has stopped"))?);
        }
        Ok(self.drawing.as_mut().expect("just filled"))
    }

    /// Hands the drawn frame to the display thread.
    pub fn present(&mut self) -> anyhow::Result<()> {
        if let Some(frame) = self.drawing.take() {
            self.to_display.send(frame).map_err(|_| anyhow!("the display thread has stopped"))?;
        }
        Ok(())
    }
}
