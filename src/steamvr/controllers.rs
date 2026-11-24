use super::{
    Result, abi,
    abi::{ActionPose, ActionSet, Digital, Input, Matrix, Overlay},
    checked, geometry,
};
use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const IDLE_INTERVAL: Duration = Duration::from_millis(33);
const SYNC_TIMEOUT_MS: u32 = 16;
const CURSOR_SIZE: u32 = 24;
const CURSOR_SCALE_PRESSED: f32 = 8. / 6.5;

pub type KeyPresses = [Option<[f32; 2]>; 2];

#[derive(Default)]
pub struct ControllerEvents {
    pub toggle: bool,
    pub presses: Vec<KeyPresses>,
    pub down: [bool; 2],
}

pub struct Actions {
    pub toggle_set: u64,
    pub pointer_set: u64,
    pub toggle: u64,
    pub aim: u64,
    pub click: u64,
    pub hands: [u64; 2],
}

pub struct ControllerWorker {
    state: Arc<Mutex<ControllerState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ControllerWorker {
    pub fn start(
        input: Input,
        overlay: Overlay,
        actions: Actions,
        keyboard: u64,
        cursors: [u64; 2],
        size: [u32; 2],
        width: f32,
    ) -> Result<Self> {
        let sync = Overlay(overlay.0);
        let state = Arc::new(Mutex::new(ControllerState {
            input,
            overlay,
            actions,
            keyboard,
            cursors: cursors.map(|handle| Cursor {
                handle,
                position: None,
                width: None,
            }),
            size: size.map(|axis| axis as f32),
            width,
            panel: None,
            filters: Default::default(),
            armed: [false; 2],
            events: ControllerEvents::default(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("vime-vr-controllers".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Relaxed) {
                    let pointing = worker_state.lock().unwrap().poll();
                    if pointing {
                        let start = Instant::now();
                        if sync.wait_frame_sync(SYNC_TIMEOUT_MS) != 0 {
                            // A stopped compositor can return immediately.
                            thread::park_timeout(
                                Duration::from_millis(u64::from(SYNC_TIMEOUT_MS))
                                    .saturating_sub(start.elapsed()),
                            );
                        }
                    } else {
                        thread::park_timeout(IDLE_INTERVAL);
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            state,
            stop,
            thread: Some(thread),
        })
    }

    pub fn state(&self) -> MutexGuard<'_, ControllerState> {
        self.state.lock().unwrap()
    }

    pub fn wake(&self) {
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
}

impl Drop for ControllerWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct ControllerState {
    input: Input,
    overlay: Overlay,
    actions: Actions,
    keyboard: u64,
    cursors: [Cursor; 2],
    size: [f32; 2],
    width: f32,
    panel: Option<Matrix>,
    filters: [geometry::PointerFilter; 2],
    armed: [bool; 2],
    events: ControllerEvents,
}

impl ControllerState {
    pub fn take_events(&mut self) -> ControllerEvents {
        ControllerEvents {
            toggle: std::mem::take(&mut self.events.toggle),
            presses: std::mem::take(&mut self.events.presses),
            down: self.events.down,
        }
    }

    pub fn open_bindings(&self, desktop: bool) -> i32 {
        self.input.open_bindings(
            super::actions::APP_KEY.as_ptr(),
            self.actions.toggle_set,
            0,
            desktop,
        )
    }

    pub fn set_width(&mut self, width: f32) {
        self.width = width;
    }

    pub fn set_panel(&mut self, panel: Option<Matrix>) {
        self.panel = panel;
        self.events = ControllerEvents::default();
        self.clear_pointers();
    }

    fn clear_pointers(&mut self) {
        self.armed = [false; 2];
        self.events.down = [false; 2];
        for (filter, cursor) in self.filters.iter_mut().zip(&mut self.cursors) {
            filter.reset();
            cursor.hide(&self.overlay);
        }
    }

    fn poll(&mut self) -> bool {
        let dashboard = self.overlay.dashboard_visible();
        let panel = self.panel.filter(|_| !dashboard);
        let mut sets = [ActionSet::default(), ActionSet::default()];
        sets[0].set = self.actions.toggle_set;
        sets[0].priority = 1;
        sets[1].set = self.actions.pointer_set;
        sets[1].priority = 20;
        if self.input.update(
            sets.as_ptr(),
            size_of::<ActionSet>() as u32,
            if panel.is_some() { 2 } else { 1 },
        ) != 0
        {
            self.clear_pointers();
            return panel.is_some();
        }

        let toggle = !dashboard
            && self
                .digital(self.actions.toggle, 0)
                .is_some_and(|data| data.state && data.changed);
        self.events.toggle |= toggle;

        let Some(panel) = panel else {
            self.clear_pointers();
            self.events.presses.clear();
            return false;
        };
        let now = Instant::now();
        let mut presses = [None; 2];
        for (hand, press) in presses.iter_mut().enumerate() {
            let source = self.actions.hands[hand];
            let click = self.digital(self.actions.click, source);
            let down = click.as_ref().is_some_and(|data| data.state);
            let pressed = self.armed[hand] && down;
            self.armed[hand] = click.is_some_and(|data| !data.state);
            self.events.down[hand] = down;
            if let Some(uv) = self.pointer_position(hand, source, &panel, now) {
                self.cursors[hand].update(
                    &self.overlay,
                    self.keyboard,
                    uv,
                    down,
                    self.width,
                    self.size,
                );
                *press = pressed.then_some(uv);
            } else {
                self.cursors[hand].hide(&self.overlay);
            }
        }
        if !toggle && presses.iter().any(Option::is_some) {
            self.events.presses.push(presses);
        }
        true
    }

    fn pointer_position(
        &mut self,
        hand: usize,
        source: u64,
        panel: &Matrix,
        now: Instant,
    ) -> Option<[f32; 2]> {
        let mut pose = ActionPose::default();
        let code = self.input.aim(
            self.actions.aim,
            abi::STANDING,
            0.0,
            &mut pose,
            size_of::<ActionPose>() as u32,
            source,
        );
        let aspect = self.size[0] / self.size[1];
        if code == 0
            && pose.active
            && pose.pose.valid
            && let Some(uv) = geometry::hit(panel, &pose.pose.transform, self.width, aspect)
        {
            return Some(self.filters[hand].update(uv, now, aspect));
        }
        self.filters[hand].reset();
        None
    }

    fn digital(&self, action: u64, device: u64) -> Option<Digital> {
        let mut data = Digital::default();
        let code = self
            .input
            .digital(action, &mut data, size_of::<Digital>() as u32, device);
        (code == 0 && data.active).then_some(data)
    }
}

struct Cursor {
    handle: u64,
    position: Option<([f32; 2], f32)>,
    width: Option<f32>,
}

impl Cursor {
    fn update(
        &mut self,
        overlay: &Overlay,
        parent: u64,
        uv: [f32; 2],
        down: bool,
        width: f32,
        size: [f32; 2],
    ) {
        let scale = if down { CURSOR_SCALE_PRESSED } else { 1. };
        let cursor_width = width / size[0] * CURSOR_SIZE as f32 * scale;
        if self.width != Some(cursor_width) {
            if overlay.set_width(self.handle, cursor_width) != 0 {
                return;
            }
            self.width = Some(cursor_width);
        }
        if self.position == Some((uv, width)) {
            return;
        }
        let transform = geometry::cursor_transform(uv, width, size[0] / size[1]);
        if overlay.set_relative_transform(self.handle, parent, &transform) == 0
            && (self.position.is_some() || overlay.show(self.handle) == 0)
        {
            self.position = Some((uv, width));
        }
    }

    fn hide(&mut self, overlay: &Overlay) {
        if self.position.is_some() && overlay.hide(self.handle) == 0 {
            self.position = None;
        }
    }
}

pub fn configure_cursor(
    overlay: &Overlay,
    handle: u64,
    hand: usize,
    operation: &str,
) -> Result<()> {
    checked(overlay.sort_order(handle, hand as u32 + 1), operation)?;
    checked(
        overlay.flag(handle, abi::PREMULTIPLIED_ALPHA, true),
        operation,
    )?;
    let color = [[215, 194, 255], [159, 211, 255]][hand];
    let mut pixels = [0u8; (CURSOR_SIZE * CURSOR_SIZE * 4) as usize];
    for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let center = CURSOR_SIZE as f32 / 2. - 0.5;
        let x = (index as u32 % CURSOR_SIZE) as f32 - center;
        let y = (index as u32 / CURSOR_SIZE) as f32 - center;
        let distance = x.hypot(y);
        let fill = (5. - distance).clamp(0., 1.);
        let alpha = (7. - distance).clamp(0., 1.);
        for ((out, inner), edge) in pixel.iter_mut().zip(color).zip([14, 39, 61]) {
            *out = (inner as f32 * fill + edge as f32 * (alpha - fill)).round() as u8;
        }
        pixel[3] = (alpha * 255.).round() as u8;
    }
    checked(
        overlay.set_raw(handle, pixels.as_ptr(), CURSOR_SIZE, CURSOR_SIZE, 4),
        operation,
    )
}
