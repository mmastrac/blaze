#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use std::time::Duration;
use winit::event_loop::ControlFlow;

const UPDATE_FPS: u32 = 60;
const RENDER_FPS: u32 = 30;
const IDLE_RENDER_FPS: u32 = 2;
const UPDATE_STEP: Duration = Duration::from_micros(1_000_000 / UPDATE_FPS as u64);
const RENDER_STEP: Duration = Duration::from_micros(1_000_000 / RENDER_FPS as u64);
const IDLE_RENDER_STEP: Duration = Duration::from_micros(1_000_000 / IDLE_RENDER_FPS as u64);

/// What to do at end-of-cycle (`about_to_wait`)
#[derive(Debug, Clone)]
pub struct IdlePlan {
    /// Set this via `event_loop.set_control_flow(...)`
    pub control_flow: ControlFlow,
    /// If true, call `window.request_redraw()`
    pub request_redraw: bool,
}

pub struct FramePolicy {
    // Fixed update rate (30Hz)
    update_step: Duration,
    next_update: Instant,

    // Render throttling
    last_present: Instant,
    max_render_interval: Duration, // 30fps cap when active

    dirty: bool,

    pub will_redraw: bool,
    pub updates_to_run: u32,

    idle_render_enabled: bool,
}

impl FramePolicy {
    pub fn new() -> Self {
        let now = Instant::now();

        let update_step = UPDATE_STEP;
        let max_render_interval = RENDER_STEP;

        Self {
            update_step,
            next_update: now + update_step,

            last_present: now - max_render_interval,
            max_render_interval,

            dirty: true,
            will_redraw: false,
            updates_to_run: 0,

            idle_render_enabled: false,
        }
    }

    /// Call on resume/start to avoid huge catch-up steps.
    pub fn reset(&mut self) {
        let now = Instant::now();
        self.next_update = now + self.update_step;
        self.last_present = now - self.max_render_interval;
    }

    /// Turn idle rendering on/off (hook up later to inactivity detection).
    pub fn set_idle_render_enabled(&mut self, enabled: bool) {
        self.idle_render_enabled = enabled;
        if enabled {
            self.dirty = true; // ensure we eventually show something
        }
    }

    pub fn plan_tick(&mut self, cause: winit::event::StartCause) {
        let now = Instant::now();
        let mut updates = 0;

        while now >= self.next_update {
            updates += 1;
            self.next_update += self.update_step;

            // prevent spiral-of-death; clamp catch-up
            if updates >= 5 {
                self.next_update = now + self.update_step;
                break;
            }
        }

        if updates > 0 {
            self.dirty = true;
        }
        self.updates_to_run = updates;
    }

    pub fn plan_idle(&mut self) -> IdlePlan {
        let now = Instant::now();

        let render_due = now.duration_since(self.last_present) >= self.max_render_interval;

        // Active render: only if dirty and due.
        let request_redraw = self.dirty && render_due;

        // Sleep until the next "interesting" deadline
        let next_deadline = self.next_update;

        IdlePlan {
            control_flow: ControlFlow::WaitUntil(next_deadline),
            request_redraw,
        }
    }

    pub fn on_presented(&mut self) {
        self.last_present = Instant::now();
        self.dirty = false;
        self.will_redraw = false;
    }

    pub fn on_present_failed_retry(&mut self) {
        self.dirty = true;
    }

    pub fn on_request_redraw(&mut self) {
        self.will_redraw = true;
    }
}
