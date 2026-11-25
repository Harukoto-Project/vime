use super::{
    DashboardEvent, Frame, KeyFeedback, KeyboardTargets, Result, SteamVr, abi,
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
    pub grab: u64,
    pub close: u64,
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
            keyboard: session.handle,
            actions,
            cursors: session.cursors.map(|handle| Cursor {
                handle,
                position: None,
                width: None,
            }),
            size: size.map(|axis| axis as f32),
            width: 1.08,
            panel: None,
            grab: None,
            grab_armed: [false; 2],
            filters: Default::default(),
            toggle_armed: false,
            armed: [false; 2],
            frame: Frame::default(),
            pending: false,
            dashboard_position: None,
            keyboard_targets: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("vime-vr".into())
            .spawn(move || {
                let mut display = DisplayTiming::read(&system);
                let mut refresh_timing = Instant::now() + Duration::from_secs(1);
                while !worker_stop.load(Ordering::Relaxed) {
                    let (notify, quit, active) = {
                        let mut state = worker_state.lock().unwrap();
                        let changed = state.poll(&system, dashboard, &display);
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
                        if start >= refresh_timing {
                            display = DisplayTiming::read(&system);
                            refresh_timing = start + Duration::from_secs(1);
                        }
                        let frame_interval = display.frame_interval;
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

struct DisplayTiming {
    frame_interval: Duration,
    vsync_to_photons: f32,
}

impl DisplayTiming {
    fn read(system: &System) -> Self {
        let mut error = 0;
        let hz = system.float_property(abi::HMD, abi::DISPLAY_FREQUENCY, &mut error);
        let frame_interval = if error == 0 && (30.0..=1_000.0).contains(&hz) {
            Duration::from_secs_f64(1.0 / f64::from(hz))
        } else {
            DEFAULT_FRAME_INTERVAL
        };
        error = 0;
        let delay = system.float_property(abi::HMD, abi::SECONDS_FROM_VSYNC_TO_PHOTONS, &mut error);
        let vsync_to_photons = if error == 0 && delay.is_finite() && delay >= 0.0 {
            delay
        } else {
            0.0
        };
        Self {
            frame_interval,
            vsync_to_photons,
        }
    }

    fn prediction(&self, system: &System) -> f32 {
        let mut elapsed = 0.0;
        let remaining = if system.time_since_last_vsync(&mut elapsed, std::ptr::null_mut())
            && elapsed.is_finite()
            && elapsed >= 0.0
        {
            (self.frame_interval.as_secs_f32() - elapsed).max(0.0)
        } else {
            0.0
        };
        remaining + self.vsync_to_photons
    }
}

pub struct ControllerState {
    input: Input,
    overlay: Overlay,
    keyboard: u64,
    actions: Actions,
    cursors: [Cursor; 2],
    size: [f32; 2],
    width: f32,
    panel: Option<Matrix>,
    grab: Option<PanelGrab>,
    grab_armed: [bool; 2],
    filters: [geometry::PointerFilter; 2],
    toggle_armed: bool,
    armed: [bool; 2],
    frame: Frame,
    pending: bool,
    dashboard_position: Option<[f32; 2]>,
    keyboard_targets: Option<KeyboardTargets>,
}

struct PanelGrab {
    hand: usize,
    offset: Matrix,
}

impl ControllerState {
    pub fn take_frame(&mut self) -> Frame {
        self.pending = false;
        Frame {
            toggle: std::mem::take(&mut self.frame.toggle) && !self.frame.dashboard_visible,
            keyboard_presses: std::mem::take(&mut self.frame.keyboard_presses),
            keyboard_down: self.frame.keyboard_down,
            keyboard_feedback: self.frame.keyboard_feedback,
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

    pub fn set_keyboard_targets(&mut self, targets: KeyboardTargets) {
        self.keyboard_targets = Some(targets);
    }

    pub fn set_panel(&mut self, panel: Option<Matrix>) {
        self.panel = panel;
        self.cancel_grab();
        if panel.is_none() {
            self.keyboard_targets = None;
        }
        // The Vive shortcut moves between action sets on visibility changes.
        // Wait for release so a held press can't immediately undo the toggle.
        self.toggle_armed = false;
        self.frame.toggle = false;
        self.frame.keyboard_presses.clear();
        self.clear_pointers();
    }

    fn cancel_grab(&mut self) {
        self.grab = None;
        self.grab_armed = [false; 2];
    }

    fn clear_pointers(&mut self) {
        self.armed = [false; 2];
        self.frame.keyboard_down = [false; 2];
        self.frame.keyboard_feedback = [KeyFeedback::default(); 2];
        for (filter, cursor) in self.filters.iter_mut().zip(&mut self.cursors) {
            filter.reset();
            cursor.hide(&self.overlay);
        }
    }

    fn poll(&mut self, system: &System, dashboard: u64, display: &DisplayTiming) -> bool {
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
        self.poll_controllers(system, display) || changed
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

    fn poll_controllers(&mut self, system: &System, display: &DisplayTiming) -> bool {
        let panel = self.panel.filter(|_| !self.frame.dashboard_visible);
        let before_down = self.frame.keyboard_down;
        let before_feedback = self.frame.keyboard_feedback;
        let mut sets = [ActionSet::default(), ActionSet::default()];
        sets[0].set = self.actions.toggle_set;
        sets[0].priority = 1;
        sets[1].set = self.actions.pointer_set;
        // Only sources bound to the keyboard set override the scene application's inputs.
        sets[1].priority = abi::OVERLAY_GLOBAL_ACTION_PRIORITY;
        if self.input.update(
            sets.as_ptr(),
            size_of::<ActionSet>() as u32,
            if panel.is_some() { 2 } else { 1 },
        ) != 0
        {
            self.toggle_armed = false;
            self.cancel_grab();
            self.clear_pointers();
            return before_down != self.frame.keyboard_down
                || before_feedback != self.frame.keyboard_feedback;
        }

        let toggle = self.poll_toggle(panel.is_some());
        self.frame.toggle |= toggle;

        let Some(panel) = panel else {
            self.cancel_grab();
            self.clear_pointers();
            self.frame.keyboard_presses.clear();
            return toggle
                || before_down != self.frame.keyboard_down
                || before_feedback != self.frame.keyboard_feedback;
        };
        let prediction = display.prediction(system);
        let now = Instant::now();
        let poses = std::array::from_fn(|hand| self.aim_pose(hand, prediction));
        if toggle || self.poll_grab(&panel, &poses) {
            self.clear_pointers();
            return toggle
                || before_down != self.frame.keyboard_down
                || before_feedback != self.frame.keyboard_feedback;
        }

        let aspect = self.size[0] / self.size[1];
        let mut presses = [None; 2];
        for (hand, press) in presses.iter_mut().enumerate() {
            let source = self.actions.hands[hand];
            let click = self.digital(self.actions.click, source);
            let down = click.as_ref().is_some_and(|data| data.state);
            let pressed = self.armed[hand] && down;
            self.armed[hand] = click.is_some_and(|data| !data.state);
            self.frame.keyboard_down[hand] = down;
            let hit = poses[hand]
                .as_ref()
                .and_then(|aim| geometry::hit(&panel, aim, self.width, aspect));
            let uv = if let Some(uv) = hit {
                Some(self.filters[hand].update(uv, now, aspect))
            } else {
                self.filters[hand].reset();
                None
            };
            let feedback = &mut self.frame.keyboard_feedback[hand];
            feedback.hovered = uv.and_then(|uv| self.keyboard_targets.as_ref()?.target(uv));
            if !down || uv.is_none() {
                feedback.pressed = None;
            } else if pressed {
                feedback.pressed = feedback.hovered;
            }
            if let Some(uv) = uv {
                self.cursors[hand].update(&self.overlay, &panel, uv, down, self.width, self.size);
                *press = pressed.then_some(uv);
            } else {
                self.cursors[hand].hide(&self.overlay);
            }
        }
        let pressed = !toggle && presses.iter().any(Option::is_some);
        if pressed && self.frame.keyboard_presses.len() < MAX_PENDING_INPUTS {
            self.frame.keyboard_presses.push(presses);
        }
        toggle
            || pressed
            || before_down != self.frame.keyboard_down
            || before_feedback != self.frame.keyboard_feedback
    }

    fn poll_toggle(&mut self, keyboard_visible: bool) -> bool {
        let mut shortcuts = [None, None];
        if !self.frame.dashboard_visible {
            shortcuts[0] = self.digital(self.actions.toggle, 0);
            if keyboard_visible {
                shortcuts[1] = self.digital(self.actions.close, 0);
            }
        }
        let down = shortcuts.iter().flatten().any(|data| data.state);
        let pressed = self.toggle_armed && down;
        self.toggle_armed = !down && shortcuts.iter().any(Option::is_some);
        pressed
    }

    fn poll_grab(&mut self, panel: &Matrix, poses: &[Option<Matrix>; 2]) -> bool {
        let grips = self
            .actions
            .hands
            .map(|source| self.digital(self.actions.grab, source));
        let armed = self.grab_armed;
        self.grab_armed =
            std::array::from_fn(|hand| grips[hand].as_ref().is_some_and(|grip| !grip.state));
        let down = |hand: usize| grips[hand].as_ref().is_some_and(|grip| grip.state);

        if let Some(grab) = &self.grab {
            if down(grab.hand)
                && let Some(pose) = &poses[grab.hand]
            {
                let moved = geometry::compose(pose, &grab.offset);
                if self
                    .overlay
                    .set_transform(self.keyboard, abi::STANDING, &moved)
                    == 0
                {
                    self.panel = Some(moved);
                }
                return true;
            }
            self.grab = None;
            return false;
        }

        let aspect = self.size[0] / self.size[1];
        for (hand, pose) in poses.iter().enumerate() {
            if armed[hand]
                && down(hand)
                && let Some(pose) = pose
                && geometry::hit(panel, pose, self.width, aspect).is_some()
            {
                // Keep the controller-to-panel offset from the moment of the grab.
                self.grab = Some(PanelGrab {
                    hand,
                    offset: geometry::relative_transform(pose, panel),
                });
                return true;
            }
        }
        false
    }

    fn aim_pose(&self, hand: usize, prediction: f32) -> Option<Matrix> {
        let mut pose = ActionPose::default();
        let code = self.input.aim(
            self.actions.aim,
            abi::STANDING,
            prediction,
            &mut pose,
            size_of::<ActionPose>() as u32,
            self.actions.hands[hand],
        );
        (code == 0 && pose.active && pose.pose.valid && pose.pose.connected)
            .then_some(pose.pose.transform)
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
        panel: &Matrix,
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
        let transform = geometry::cursor_transform(panel, uv, width, size[0] / size[1]);
        if overlay.set_transform(self.handle, abi::STANDING, &transform) == 0
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
    let (rgba_pixels, _) = pixels.as_chunks_mut::<4>();
    for (index, pixel) in rgba_pixels.iter_mut().enumerate() {
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
