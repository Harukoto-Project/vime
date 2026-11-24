use super::{
    DashboardEvent, Frame, Result, SteamVr, abi,
    abi::{ActionPose, ActionSet, Digital, Event, Input, Matrix, Overlay, System},
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
const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const MAX_PENDING_INPUTS: usize = 128;
const CURSOR_SIZE: u32 = 24;
const CURSOR_SCALE_PRESSED: f32 = 8. / 6.5;

pub type KeyPresses = [Option<[f32; 2]>; 2];

pub struct Actions {
    pub toggle_set: u64,
    pub pointer_set: u64,
    pub toggle: u64,
    pub aim: u64,
    pub click: u64,
    pub hands: [u64; 2],
}

pub struct VrWorker {
    state: Arc<Mutex<ControllerState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl VrWorker {
    pub fn start(
        session: &SteamVr,
        input: Input,
        actions: Actions,
        size: [u32; 2],
        wake_ui: impl Fn() + Send + 'static,
    ) -> Result<Self> {
        let system = System(session.system.0);
        let sync = Overlay(session.overlay.0);
        let dashboard = session.dashboard;
        let state = Arc::new(Mutex::new(ControllerState {
            input,
            overlay: Overlay(session.overlay.0),
            actions,
            keyboard: session.handle,
            cursors: session.cursors.map(|handle| Cursor {
                handle,
                position: None,
                width: None,
            }),
            size: size.map(|axis| axis as f32),
            width: 1.08,
            panel: None,
            filters: Default::default(),
            armed: [false; 2],
            frame: Frame::default(),
            pending: false,
            dashboard_position: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("vime-vr".into())
            .spawn(move || {
                let mut frame_interval = DEFAULT_FRAME_INTERVAL;
                let mut check_frequency = Instant::now();
                while !worker_stop.load(Ordering::Relaxed) {
                    let (notify, quit, active) = {
                        let mut state = worker_state.lock().unwrap();
                        let changed = state.poll(&system, dashboard);
                        // A single wake covers all input queued before the UI drains it.
                        let notify = changed && !std::mem::replace(&mut state.pending, true);
                        let active = state.frame.dashboard_active
                            || (state.panel.is_some() && !state.frame.dashboard_visible);
                        (notify, state.frame.quit, active)
                    };
                    if notify {
                        wake_ui();
                    }
                    if quit {
                        break;
                    }
                    if active {
                        let start = Instant::now();
                        if start >= check_frequency {
                            frame_interval = display_interval(&system);
                            check_frequency = start + Duration::from_secs(1);
                        }
                        let timeout_ms = frame_interval.as_micros().div_ceil(1_000) as u32;
                        if sync.wait_frame_sync(timeout_ms) != 0 {
                            // A stopped compositor can report an error immediately.
                            thread::park_timeout(frame_interval.saturating_sub(start.elapsed()));
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

impl Drop for VrWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn display_interval(system: &System) -> Duration {
    let mut error = 0;
    let hz = system.float_property(abi::HMD, abi::DISPLAY_FREQUENCY, &mut error);
    if error == 0 && (30.0..=1_000.0).contains(&hz) {
        Duration::from_secs_f64(1.0 / f64::from(hz))
    } else {
        DEFAULT_FRAME_INTERVAL
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
    frame: Frame,
    pending: bool,
    dashboard_position: Option<[f32; 2]>,
}

impl ControllerState {
    pub fn take_frame(&mut self) -> Frame {
        self.pending = false;
        Frame {
            toggle: std::mem::take(&mut self.frame.toggle) && !self.frame.dashboard_visible,
            keyboard_presses: std::mem::take(&mut self.frame.keyboard_presses),
            keyboard_down: self.frame.keyboard_down,
            dashboard_visible: self.frame.dashboard_visible,
            dashboard_active: self.frame.dashboard_active,
            dashboard_events: std::mem::take(&mut self.frame.dashboard_events),
            quit: self.frame.quit,
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
        self.frame.toggle = false;
        self.frame.keyboard_presses.clear();
        self.clear_pointers();
    }

    fn clear_pointers(&mut self) {
        self.armed = [false; 2];
        self.frame.keyboard_down = [false; 2];
        for (filter, cursor) in self.filters.iter_mut().zip(&mut self.cursors) {
            filter.reset();
            cursor.hide(&self.overlay);
        }
    }

    fn poll(&mut self, system: &System, dashboard: u64) -> bool {
        let mut event = Event::default();
        for _ in 0..64 {
            if !system.poll_event(&mut event, size_of::<Event>() as u32) {
                break;
            }
            if event.kind == abi::QUIT {
                self.frame.quit = true;
                return true;
            }
        }

        let visible = self.overlay.dashboard_visible();
        let active = visible && self.overlay.dashboard_active(dashboard);
        let mut changed =
            visible != self.frame.dashboard_visible || active != self.frame.dashboard_active;
        if !active && self.frame.dashboard_active {
            self.frame.dashboard_events.clear();
            self.queue_dashboard_event(DashboardEvent::Leave);
        }
        self.frame.dashboard_visible = visible;
        self.frame.dashboard_active = active;

        for _ in 0..64 {
            if !self
                .overlay
                .poll_event(dashboard, &mut event, size_of::<Event>() as u32)
            {
                break;
            }
            if !active {
                continue;
            }
            match event.kind {
                abi::MOUSE_MOVE if self.dashboard_position != Some(event.mouse().0) => {
                    self.queue_dashboard_event(DashboardEvent::Move(event.mouse().0));
                }
                abi::MOUSE_DOWN | abi::MOUSE_UP if event.mouse().1 == abi::MOUSE_LEFT => {
                    self.queue_dashboard_event(DashboardEvent::Button {
                        position: event.mouse().0,
                        down: event.kind == abi::MOUSE_DOWN,
                    });
                }
                abi::FOCUS_LEAVE | abi::OVERLAY_HIDDEN => {
                    self.queue_dashboard_event(DashboardEvent::Leave);
                }
                _ => continue,
            }
            changed = true;
        }
        self.poll_controllers() || changed
    }

    fn queue_dashboard_event(&mut self, event: DashboardEvent) {
        let events = &mut self.frame.dashboard_events;
        if let DashboardEvent::Move(position) = event {
            self.dashboard_position = Some(position);
            if let Some(last @ DashboardEvent::Move(_)) = events.last_mut() {
                *last = event;
                return;
            }
        } else if matches!(event, DashboardEvent::Leave) {
            self.dashboard_position = None;
        }
        if events.len() == MAX_PENDING_INPUTS {
            events.clear();
            events.push(DashboardEvent::Leave);
        }
        events.push(event);
    }

    fn poll_controllers(&mut self) -> bool {
        let panel = self.panel.filter(|_| !self.frame.dashboard_visible);
        let before_down = self.frame.keyboard_down;
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
            return before_down != self.frame.keyboard_down;
        }

        let toggle = !self.frame.dashboard_visible
            && self
                .digital(self.actions.toggle, 0)
                .is_some_and(|data| data.state && data.changed);
        self.frame.toggle |= toggle;

        let Some(panel) = panel else {
            self.clear_pointers();
            self.frame.keyboard_presses.clear();
            return toggle || before_down != self.frame.keyboard_down;
        };
        let now = Instant::now();
        let mut presses = [None; 2];
        for (hand, press) in presses.iter_mut().enumerate() {
            let source = self.actions.hands[hand];
            let click = self.digital(self.actions.click, source);
            let down = click.as_ref().is_some_and(|data| data.state);
            let pressed = self.armed[hand] && down;
            self.armed[hand] = click.is_some_and(|data| !data.state);
            self.frame.keyboard_down[hand] = down;
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
        let pressed = !toggle && presses.iter().any(Option::is_some);
        if pressed && self.frame.keyboard_presses.len() < MAX_PENDING_INPUTS {
            self.frame.keyboard_presses.push(presses);
        }
        toggle || pressed || before_down != self.frame.keyboard_down
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
