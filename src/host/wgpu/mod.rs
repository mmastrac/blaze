#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub const REAL_WIDTH: u32 = 800;
pub const REAL_HEIGHT: u32 = 416;
// TODO: Waiting on pixels to support non-square aspect ratios
pub const ASPECT_RATIO: f64 = 4.0 / 3.0;
pub const WINDOW_WIDTH: u32 = REAL_WIDTH as u32;
pub const WINDOW_HEIGHT: u32 = (REAL_WIDTH as f64 / ASPECT_RATIO as f64) as u32;

use pixels::{Error, Pixels, PixelsBuilder, SurfaceTexture};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi;
use winit::event::WindowEvent;
use winit::event_loop::EventLoopProxy;
use winit::{dpi::LogicalSize, event_loop::EventLoop, window::Window};
use winit_input_helper::WinitInputHelper;

use crate::host::lk201::winit::update_keyboard;
use crate::host::wgpu::policy::FramePolicy;
use lk201::LK201Sender;

use tracing::{debug, error, info};

mod policy;

enum PixelsState {
    None(EventLoopProxy<Pixels<'static>>),
    Initializing {
        window: Arc<winit::window::Window>,
        size: Option<dpi::PhysicalSize<u32>>,
    },
    Running {
        window: Arc<winit::window::Window>,
        pixels: Pixels<'static>,
    },
}

/// Uber-struct representing the entire game.
struct Terminal {
    /// Software renderer.
    pixels: PixelsState,
    /// Event manager.
    input: WinitInputHelper,
    /// Game pause state.
    paused: bool,
    /// LK201 keyboard sender.
    sender: LK201Sender,
    /// Frame policy.
    frame_policy: FramePolicy,
    /// Render function.
    render: Box<dyn FnMut(&mut [u8])>,
    /// Step function.
    step: Box<dyn FnMut()>,
}

impl Terminal {
    fn new(
        sender: LK201Sender,
        proxy: EventLoopProxy<Pixels<'static>>,
        render: Box<dyn FnMut(&mut [u8])>,
        step: Box<dyn FnMut()>,
    ) -> Self {
        Self {
            pixels: PixelsState::None(proxy),
            input: WinitInputHelper::new(),
            paused: false,
            frame_policy: FramePolicy::new(),
            sender,
            render,
            step,
        }
    }

    fn window(&self) -> &winit::window::Window {
        match &self.pixels {
            PixelsState::Initializing { window, .. } => window,
            PixelsState::Running { window, .. } => window,
            PixelsState::None(..) => unreachable!(),
        }
    }

    fn update_controls(&mut self) {
        update_keyboard(&self.input, &self.sender);
    }

    fn init_window(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("VT420")
                .with_inner_size(LogicalSize::new(REAL_WIDTH as f64, REAL_HEIGHT as f64))
                .with_min_inner_size(LogicalSize::new(REAL_WIDTH as f64, REAL_HEIGHT as f64)),
        ) {
            Ok(window) => window,
            Err(e) => {
                error!("Failed to create window: {}", e);
                event_loop.exit();
                return;
            }
        };
        info!("Graphics: window created");

        let PixelsState::None(proxy) = &mut self.pixels else {
            unreachable!();
        };
        let proxy = proxy.clone();
        let window = Arc::new(window);
        self.pixels = PixelsState::Initializing {
            window: window.clone(),
            size: None,
        };
        let window = window.clone();
        let future = async move {
            match create_pixels(window).await {
                Ok(pixels) => {
                    info!("Graphics: sending pixels event");
                    if let Err(e) = proxy.send_event(pixels) {
                        error!("Graphics: Event loop closed during initialization: {e}");
                        return;
                    }
                    info!("Graphics: pixels event sent");
                }
                Err(e) => {
                    log_pixels_error(e);
                }
            }
        };

        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(future);
        }

        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(future);

        info!("Graphics: window initialized");
    }
}

impl ApplicationHandler<Pixels<'static>> for Terminal {
    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.update_controls();
        for _ in 0..self.frame_policy.updates_to_run {
            (self.step)();
        }
        if self.frame_policy.will_redraw {
            match &mut self.pixels {
                PixelsState::Running { pixels, .. } => {
                    (self.render)(pixels.frame_mut());
                    if let Err(err) = pixels.render() {
                        error!("Graphics: pixels.render failed: {err}");
                        self.frame_policy.on_present_failed_retry();
                    } else {
                        self.frame_policy.on_presented();
                    }
                }
                // The surface may not exist yet. Render when it does.
                _ => {}
            }
        }
        let idle = self.frame_policy.plan_idle();
        event_loop.set_control_flow(idle.control_flow);
        if idle.request_redraw {
            self.window().request_redraw();
        }
        self.input.end_step();
    }

    fn device_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        debug!("Graphics: got device event: {event:?}");
        self.input.process_device_event(&event);
    }

    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}
    fn memory_warning(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}
    fn new_events(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        match cause {
            winit::event::StartCause::Init => {
                info!("Graphics: starting");
                self.init_window(event_loop);
            }
            winit::event::StartCause::ResumeTimeReached { .. }
            | winit::event::StartCause::WaitCancelled { .. }
            | winit::event::StartCause::Poll => {
                self.frame_policy.plan_tick(cause);
                self.input.step();
            }
        }
    }
    fn resumed(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}
    fn suspended(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}
    fn user_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        event: Pixels<'static>,
    ) {
        info!("Graphics: got pixels event");
        let PixelsState::Initializing { window, size } = &mut self.pixels else {
            unreachable!();
        };
        let mut pixels = event;
        let window = window.clone();
        let size = size.clone();
        if let Some(size) = size {
            info!(
                "Graphics: resizing surface to {}x{}",
                size.width, size.height
            );
            if let Err(err) = pixels.resize_surface(size.width, size.height) {
                error!("Graphics: pixels.resize_surface: {err}");
                event_loop.exit();
            }
        }
        self.pixels = PixelsState::Running { window, pixels };
        self.window().request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        self.input.process_window_event(&event);
        if let Some(resize) = self.input.window_resized() {
            match &mut self.pixels {
                PixelsState::None(..) => {
                    unreachable!();
                }
                PixelsState::Initializing { size, .. } => {
                    *size = Some(resize);
                }
                PixelsState::Running { pixels, .. } => {
                    // window_resized() returns physical size, but clamp to reasonable maximum
                    // texture size (most GPUs support up to 16384, but we'll use 4096 to be safe)
                    const MAX_TEXTURE_SIZE: u32 = 4096;
                    let width = resize.width.min(MAX_TEXTURE_SIZE);
                    let height = resize.height.min(MAX_TEXTURE_SIZE);
                    if let Err(err) = pixels.resize_surface(width, height) {
                        error!("Graphics: pixels.resize_surface: {err}");
                        event_loop.exit();
                    }
                }
            }
        }

        match event {
            WindowEvent::RedrawRequested => {
                self.frame_policy.on_request_redraw();
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            _ => {}
        }
    }
}

#[cfg(target_arch = "wasm32")]
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

fn log_pixels_error(e: Error) {
    match e {
        Error::AdapterNotFound => {
            error!("Graphics error: Adapter not found");
        }
        Error::CreateSurface(e) => {
            error!("Graphics error: Create surface: {}", e);
        }
        Error::DeviceNotFound(e) => {
            error!("Graphics error: Device not found: {}", e);
        }
        Error::InvalidTexture(e) => {
            error!("Graphics error: Invalid texture: {}", e);
        }
        Error::UserDefined(e) => {
            error!("Graphics error: {}", e);
        }
        _ => {
            error!("Graphics error: Unexpected error: {}", e);
        }
    }
}

pub fn main(
    sender: LK201Sender,
    render: impl FnMut(&mut [u8]) + 'static,
    step: impl FnMut() + 'static,
) -> Result<(), Error> {
    let event_loop = EventLoop::<Pixels<'static>>::with_user_event()
        .build()
        .map_err(|e| Error::UserDefined(Box::new(e)))?;
    let proxy = event_loop.create_proxy();
    let mut terminal = Terminal::new(sender, proxy, Box::new(render), Box::new(step));

    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(terminal);
    }

    #[cfg(not(target_arch = "wasm32"))]
    event_loop
        .run_app(&mut terminal)
        .map_err(|e| Error::UserDefined(Box::new(e)))?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn get_canvas(#[allow(unused)] window: &winit::window::Window) -> web_sys::HtmlCanvasElement {
    use winit::platform::web::WindowExtWebSys;
    window.canvas().unwrap()
}

#[cfg(target_arch = "wasm32")]
pub async fn attach_canvas(window: &Arc<winit::window::Window>) {
    use js_sys::Promise;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::JsFuture;

    let canvas = get_canvas(&window);

    web_sys::window()
        .and_then(|win| win.document())
        .and_then(|doc| doc.body())
        .and_then(|body| body.append_child(&canvas).ok())
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
}

async fn create_pixels(window: Arc<winit::window::Window>) -> Result<Pixels<'static>, Error> {
    #[cfg(target_arch = "wasm32")]
    attach_canvas(&window).await;

    #[cfg(not(target_arch = "wasm32"))]
    let window_size = window.inner_size();
    #[cfg(target_arch = "wasm32")]
    let window_size = get_window_size().to_physical::<u32>(window.scale_factor());

    info!(
        "Graphics: window size: {}x{}",
        window_size.width, window_size.height
    );

    let surface_texture = SurfaceTexture::new(REAL_WIDTH, REAL_HEIGHT, Arc::clone(&window));

    let pixel_builder = PixelsBuilder::new(REAL_WIDTH, REAL_HEIGHT, surface_texture);
    #[cfg(target_arch = "wasm32")]
    let pixel_builder = {
        // Web targets do not support the default texture format
        let texture_format = pixels::wgpu::TextureFormat::Rgba8Unorm;
        pixel_builder
            .texture_format(texture_format)
            .surface_texture_format(texture_format)
            .wgpu_backend(pixels::wgpu::Backends::GL)
    };

    let mut pixels = pixel_builder.build_async().await?;
    info!("Graphics: pixels created");

    // Use the fill scaling mode which supports non-integer scaling.
    pixels.set_scaling_mode(pixels::ScalingMode::Fill);

    Ok(pixels)
}
