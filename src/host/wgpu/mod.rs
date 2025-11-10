#![deny(clippy::all)]
#![forbid(unsafe_code)]

const WIDTH: u32 = 800;
const HEIGHT: u32 = 417;
const FPS: u32 = 60;
const TIME_STEP: Duration = Duration::from_micros(1_000_000 / FPS as u64);

use game_loop::winit;

use game_loop::winit::keyboard::Key;
use game_loop::{Time, TimeTrait as _, game_loop};
use pixels::{Error, Pixels, SurfaceTexture};
use std::sync::Arc;
use std::time::Duration;
use winit::{dpi::LogicalSize, event_loop::EventLoop, keyboard::KeyCode, window::WindowBuilder};
use winit_input_helper::WinitInputHelper;

use crate::machine::generic::lk201::{LK201Sender, SpecialKey};

/// Uber-struct representing the entire game.
struct Game {
    /// Software renderer.
    pixels: Pixels<'static>,
    /// Event manager.
    input: WinitInputHelper,
    /// Game pause state.
    paused: bool,
    /// LK201 keyboard sender.
    sender: LK201Sender,
}

impl Game {
    fn new(pixels: Pixels<'static>, sender: LK201Sender, debug: bool) -> Self {
        Self {
            pixels,
            input: WinitInputHelper::new(),
            paused: false,
            sender,
        }
    }

    fn update_controls(&mut self) {
        for (key, mapping) in [
            (KeyCode::F1, SpecialKey::F1),
            (KeyCode::F2, SpecialKey::F2),
            (KeyCode::F3, SpecialKey::F3),
            (KeyCode::F4, SpecialKey::F4),
            (KeyCode::F5, SpecialKey::F5),
            (KeyCode::ArrowUp, SpecialKey::Up),
            (KeyCode::ArrowDown, SpecialKey::Down),
            (KeyCode::ArrowLeft, SpecialKey::Left),
            (KeyCode::ArrowRight, SpecialKey::Right),
            (KeyCode::Enter, SpecialKey::Return),
        ] {
            if self.input.key_pressed(key) {
                if self.input.held_control() {
                    if self.input.held_shift() {
                        self.sender.send_shift_ctrl_special_key(mapping);
                    } else {
                        self.sender.send_ctrl_special_key(mapping);
                    }
                    self.sender.send_ctrl_special_key(mapping);
                } else if self.input.held_shift() {
                    self.sender.send_shift_special_key(mapping);
                } else {
                    self.sender.send_special_key(mapping);
                }
            }
        }

        b"abcdefghijklmnopqrstuvwxyz 0123456789"
            .iter()
            .for_each(|&c| {
                let s = &[c];
                let s = str::from_utf8(s).unwrap();
                if self.input.key_pressed_logical(Key::Character(s)) {
                    self.sender.send_char(c as char);
                }
            });

        if self.input.key_pressed(KeyCode::Escape) {
            self.sender.send_escape();
        }
    }

    fn reset_game(&mut self) {}
}

pub fn main(
    sender: LK201Sender,
    mut render: impl FnMut(&mut [u8]) + 'static,
    mut step: impl FnMut() + 'static,
) -> Result<(), Error> {
    let event_loop = EventLoop::new().unwrap();

    let window = {
        let size = LogicalSize::new(WIDTH as f64, HEIGHT as f64);
        let scaled_size = LogicalSize::new(WIDTH as f64 * 2.0, HEIGHT as f64 * 2.0);
        let window = WindowBuilder::new()
            .with_title("VT420")
            .with_inner_size(scaled_size)
            .with_min_inner_size(size)
            .build(&event_loop)
            .unwrap();
        Arc::new(window)
    };

    let mut pixels = {
        let window_size = window.inner_size();
        let surface_texture =
            SurfaceTexture::new(window_size.width, window_size.height, Arc::clone(&window));
        Pixels::new(WIDTH as u32, HEIGHT as u32, surface_texture)?
    };

    // Use the fill scaling mode which supports non-integer scaling.
    pixels.set_scaling_mode(pixels::ScalingMode::Fill);

    let game = Game::new(pixels, sender, true);

    let res = game_loop(
        event_loop,
        window,
        game,
        FPS as u32,
        0.1,
        move |g| {
            // Update the world
            if !g.game.paused {
                step();
            }
        },
        move |g| {
            // Drawing
            // g.game.world.draw(g.game.pixels.frame_mut());
            render(g.game.pixels.frame_mut());
            if let Err(err) = g.game.pixels.render() {
                // log_error("pixels.render", err);
                g.exit();
            }

            // Sleep the main thread to limit drawing to the fixed time step.
            // See: https://github.com/parasyte/pixels/issues/174
            let dt = TIME_STEP.as_secs_f64() - Time::now().sub(&g.current_instant());
            if dt > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(dt));
            }
        },
        |g, event| {
            // Let winit_input_helper collect events to build its state.
            if g.game.input.update(event) {
                // Update controls
                g.game.update_controls();

                // Close events
                if g.game.input.close_requested() {
                    g.exit();
                    return;
                }

                // Resize the window
                if let Some(size) = g.game.input.window_resized() {
                    if let Err(err) = g.game.pixels.resize_surface(size.width, size.height) {
                        // log_error("pixels.resize_surface", err);
                        g.exit();
                    }
                }
            }
        },
    );
    res.map_err(|e| Error::UserDefined(Box::new(e)))
}
