//! Walks around a world on the PC: `cargo run --example walk [-- world.adv]`, without a file in the
//! sample world. Arrow keys walk, Space or Enter is A, Shift is B (run), Esc quits. Without sound.

use std::error::Error;

use adventure_engine::{sample, Buttons, Game, FPS, HEIGHT, WIDTH};
use adventure_format::World;
use minifb::{Key, Scale, Window, WindowOptions};

fn main() -> Result<(), Box<dyn Error>> {
    let world = match std::env::args().nth(1) {
        Some(path) => World::from_bytes(&std::fs::read(path)?)?,
        None => sample::world(),
    };
    let mut game = Game::new(world, 0)?;
    let options = WindowOptions {
        scale: Scale::X4,
        ..WindowOptions::default()
    };
    let mut window = Window::new("Adventure", WIDTH, HEIGHT, options)?;
    window.set_target_fps(FPS as usize);

    let mut picture = vec![0u16; WIDTH * HEIGHT];
    let mut rgb = vec![0u32; WIDTH * HEIGHT];
    while window.is_open() && !window.is_key_down(Key::Escape) {
        let down = |keys: &[Key]| keys.iter().any(|&key| window.is_key_down(key));
        let buttons = Buttons {
            up: down(&[Key::Up]),
            down: down(&[Key::Down]),
            left: down(&[Key::Left]),
            right: down(&[Key::Right]),
            a: down(&[Key::Space, Key::Enter]),
            b: down(&[Key::LeftShift, Key::RightShift]),
        };
        game.step(buttons);
        game.draw(&mut picture);
        for (out, &pixel) in rgb.iter_mut().zip(&picture) {
            let (r, g, b) = (
                (pixel >> 11) as u32,
                ((pixel >> 5) & 0x3F) as u32,
                (pixel & 0x1F) as u32,
            );
            *out = (r << 3 | r >> 2) << 16 | (g << 2 | g >> 4) << 8 | (b << 3 | b >> 2);
        }
        window.update_with_buffer(&rgb, WIDTH, HEIGHT)?;
    }
    Ok(())
}
