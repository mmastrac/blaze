#![deny(clippy::all)]
#![forbid(unsafe_code)]

const WIDTH: u32 = 800;
const HEIGHT: u32 = 417;
const FPS: u32 = 60;
const TIME_STEP: Duration = Duration::from_micros(1_000_000 / FPS as u64);

use game_loop::winit;

use game_loop::{Time, TimeTrait as _, game_loop};
use pixels::{Error, Pixels, PixelsBuilder, SurfaceTexture};
use std::sync::Arc;
use std::time::Duration;
use winit::{dpi::LogicalSize, event_loop::EventLoop, window::WindowBuilder};
use winit_input_helper::WinitInputHelper;

use crate::host::lk201::winit::update_keyboard;
use crate::machine::generic::lk201::LK201Sender;

use tracing::{error, info};

/// Uber-struct representing the entire game.
struct Terminal {
    /// Software renderer.
    pixels: Pixels<'static>,
    /// Event manager.
    input: WinitInputHelper,
    /// Game pause state.
    paused: bool,
    /// LK201 keyboard sender.
    sender: LK201Sender,
}

impl Terminal {
    fn new(pixels: Pixels<'static>, sender: LK201Sender) -> Self {
        Self {
            pixels,
            input: WinitInputHelper::new(),
            paused: false,
            sender,
        }
    }

    fn update_controls(&mut self) {
        update_keyboard(&self.input, &self.sender);
    }
}

#[cfg(feature = "wasm")]
/// Retrieve current width and height dimensions of browser client window
fn get_window_size() -> LogicalSize<f64> {
    let client_window = web_sys::window().unwrap();
    let size = LogicalSize::new(
        client_window.inner_width().unwrap().as_f64().unwrap(),
        client_window.inner_height().unwrap().as_f64().unwrap(),
    );

    info!("Graphics: window resized: {}x{}", size.width, size.height);
    size
}

pub fn main(
    sender: LK201Sender,
    render: impl FnMut(&mut [u8]) + 'static,
    step: impl FnMut() + 'static,
) -> Result<(), Error> {
    let future = main_async(sender, render, step);
    #[cfg(feature = "wasm")]
    {
        wasm_bindgen_futures::spawn_local(async {
            if let Err(e) = future.await {
                error!("Graphics error: {}", e);
            }
        });
        Ok(())
    }
    #[cfg(not(feature = "wasm"))]
    pollster::block_on(future)
}

pub async fn main_async(
    sender: LK201Sender,
    mut render: impl FnMut(&mut [u8]) + 'static,
    mut step: impl FnMut() + 'static,
) -> Result<(), Error> {
    let event_loop = EventLoop::new().unwrap();

    #[cfg(feature = "wasm")]
    js_sys::eval(
        r#"""
        console.log("Patching WebGPU GPUAdapter.requestDevice");
        const _oldRequestDevice = window?.GPUAdapter?.prototype?.requestDevice;
        if (_oldRequestDevice) {
            window.GPUAdapter.prototype.requestDevice = async function(limits) {
                delete limits?.requiredLimits?.maxInterStageShaderComponents;
                try {
                    console.log("Requesting WebGPU device with limits:", limits);
                    const device = await _oldRequestDevice.call(this, limits);
                    console.log("WebGPU device acquired:", device);
                    return device;
                } catch (e) {
                    console.error("Error requesting WebGPU device:", e);
                    throw e;
                }
            };
        }
    """#,
    )
    .expect("Failed to evaluate WebGPU patch");

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

    info!("Graphics: window created");

    // Attach winit canvas to body element
    #[cfg(feature = "wasm")]
    {
        use js_sys::Promise;
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen_futures::JsFuture;
        use winit::platform::web::WindowExtWebSys;

        // Attach canvas to DOM first, before initializing pixels
        let canvas = window.canvas().unwrap();
        let canvas_element = web_sys::Element::from(canvas);

        // Ensure canvas is visible and has explicit styling
        let html_canvas: &web_sys::HtmlCanvasElement = canvas_element.dyn_ref().unwrap();
        html_canvas
            .style()
            .set_property("display", "block")
            .unwrap();

        web_sys::window()
            .and_then(|win| win.document())
            .and_then(|doc| doc.body())
            .and_then(|body| body.append_child(&canvas_element).ok())
            .expect("couldn't append canvas to document body");

        info!("Graphics: canvas attached to document body");

        // Listen for resize event on browser client. Adjust winit window dimensions
        // on event trigger
        let closure = Closure::wrap(Box::new({
            let window = Arc::clone(&window);
            move |_e: web_sys::Event| {
                let _ = window.request_inner_size(get_window_size());
            }
        }) as Box<dyn FnMut(_)>);
        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();

        // Trigger initial resize event
        let _ = window.request_inner_size(get_window_size());

        let promise = Promise::new(&mut |resolve, _reject| {
            let closure = Closure::wrap(Box::new(move |_timestamp: f64| {
                resolve.call0(&wasm_bindgen::JsValue::UNDEFINED).unwrap();
            }) as Box<dyn FnMut(f64)>);
            web_sys::window()
                .unwrap()
                .request_animation_frame(closure.as_ref().unchecked_ref())
                .unwrap();
            closure.forget();
        });

        // Yield to the event loop until the animation frame callback has been called
        // This ensures Chrome has rendered the canvas before WebGPU initialization
        JsFuture::from(promise).await.ok();

        info!("Graphics: waited for canvas to be rendered");
    }

    let mut pixels = {
        #[cfg(not(feature = "wasm"))]
        let window_size = window.inner_size();
        #[cfg(feature = "wasm")]
        let window_size = get_window_size().to_physical::<u32>(window.scale_factor());

        info!(
            "Graphics: window size: {}x{}",
            window_size.width, window_size.height
        );

        let surface_texture = SurfaceTexture::new(WIDTH, HEIGHT, Arc::clone(&window));

        let pixel_builder = PixelsBuilder::new(WIDTH as u32, HEIGHT as u32, surface_texture);

        #[cfg(feature = "wasm")]
        let pixel_builder = {
            // Web targets do not support the default texture format
            let texture_format = pixels::wgpu::TextureFormat::Rgba8Unorm;
            pixel_builder
                .texture_format(texture_format)
                .surface_texture_format(texture_format)
        };

        pixel_builder.build_async().await?
    };

    // Use the fill scaling mode which supports non-integer scaling.
    pixels.set_scaling_mode(pixels::ScalingMode::Fill);

    let terminal = Terminal::new(pixels, sender);

    let res = game_loop(
        event_loop,
        window,
        terminal,
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
                error!("pixels.render: {err}");
                g.exit();
            }

            // Sleep the main thread to limit drawing to the fixed time step.
            // See: https://github.com/parasyte/pixels/issues/174
            #[cfg(not(feature = "wasm"))]
            {
                let dt = TIME_STEP.as_secs_f64() - Time::now().sub(&g.current_instant());
                if dt > 0.0 {
                    std::thread::sleep(Duration::from_secs_f64(dt));
                }
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
                    // window_resized() returns physical size, but clamp to reasonable maximum
                    // texture size (most GPUs support up to 16384, but we'll use 8192 to be safe)
                    const MAX_TEXTURE_SIZE: u32 = 8192;
                    let width = size.width.min(MAX_TEXTURE_SIZE);
                    let height = size.height.min(MAX_TEXTURE_SIZE);
                    if let Err(err) = g.game.pixels.resize_surface(width, height) {
                        error!("pixels.resize_surface: {err}");
                        g.exit();
                    }
                }
            }
        },
    );
    res.map_err(|e| Error::UserDefined(Box::new(e)))
}
