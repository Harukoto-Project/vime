mod abi;
mod actions;
mod geometry;

use crate::{
    brand,
    i18n::{tr, tr_args},
};
use abi::{
    ActionPose, ActionSet, Applications, Digital, Event, Input, Matrix, Overlay, Pose, System,
    Texture,
};
use libloading::Library;
use std::{
    ffi::{CStr, CString, c_void},
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Instant,
};

type Result<T> = std::result::Result<T, String>;
static RUNTIME_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Pointer {
    pub uv: [f32; 2],
    pub down: bool,
}

#[derive(Default)]
pub struct Frame {
    pub toggle: bool,
    pub keyboard_presses: [Option<[f32; 2]>; 2],
    pub keyboard_down: [bool; 2],
    pub pointers: [Option<Pointer>; 2],
    pub dashboard_visible: bool,
    pub dashboard_active: bool,
    pub input_error: Option<i32>,
    pub dashboard_events: Vec<DashboardEvent>,
    pub quit: bool,
}

#[derive(Clone, Copy)]
pub enum DashboardEvent {
    Move([f32; 2]),
    Button { position: [f32; 2], down: bool },
    Leave,
}

#[derive(Clone, Copy)]
pub enum Surface {
    Keyboard,
    Dashboard,
}

struct Runtime {
    _library: Library,
    shutdown: abi::Shutdown,
    interface: abi::GenericInterface,
    _guard: MutexGuard<'static, ()>,
}

impl Runtime {
    fn connect() -> Result<Self> {
        let guard = RUNTIME_LOCK
            .try_lock()
            .map_err(|_| tr("このプロセスは既に SteamVR に接続しています"))?;
        let path = actions::runtime_dll().ok_or(tr("SteamVR が見つかりません"))?;
        // Only use the SteamVR installation DLL, never the process search path.
        let library = unsafe { Library::new(path) }.map_err(|error| error.to_string())?;
        let mut error = 0;
        let (shutdown, interface) = unsafe {
            let init = *library
                .get::<abi::Init>(b"VR_InitInternal")
                .map_err(|e| e.to_string())?;
            let shutdown = *library
                .get::<abi::Shutdown>(b"VR_ShutdownInternal")
                .map_err(|e| e.to_string())?;
            let interface = *library
                .get::<abi::GenericInterface>(b"VR_GetGenericInterface")
                .map_err(|e| e.to_string())?;
            init(&mut error, abi::APPLICATION_OVERLAY);
            (shutdown, interface)
        };
        checked(error, tr("SteamVR への接続"))?;
        Ok(Self {
            _library: library,
            shutdown,
            interface,
            _guard: guard,
        })
    }

    fn table(&self, version: &CStr) -> Result<*const *const c_void> {
        let mut error = 0;
        let table = unsafe { (self.interface)(version.as_ptr(), &mut error) };
        checked(error, &version.to_string_lossy())?;
        if table.is_null() {
            Err(tr_args(
                "{0} を取得できません",
                &[&version.to_string_lossy()],
            ))
        } else {
            Ok(table)
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { (self.shutdown)() };
    }
}

pub struct SteamVr {
    _runtime: Runtime,
    system: System,
    overlay: Overlay,
    input: Input,
    applications: Applications,
    handle: u64,
    dashboard: u64,
    pointer_filters: [geometry::PointerFilter; 2],
    toggle_set: u64,
    pointer_set: u64,
    toggle_action: u64,
    aim_action: u64,
    click_action: u64,
    hands: [u64; 2],
    panel: Option<Matrix>,
    world_shown: bool,
    width: f32,
    armed: [bool; 2],
    quitting: bool,
}

impl SteamVr {
    pub fn connect(directory: &Path, dashboard_size: [u32; 2]) -> Result<Self> {
        let runtime = Runtime::connect()?;
        let (app_path, actions_path) =
            actions::write_manifests(directory).map_err(|error| error.to_string())?;
        let path_string = |path: &Path| {
            CString::new(
                path.to_str()
                    .ok_or(tr("SteamVR のパスをUTF-8に変換できません"))?,
            )
            .map_err(|error| error.to_string())
        };
        let app_path = path_string(&app_path)?;
        let actions_path = path_string(&actions_path)?;
        let system = System(runtime.table(abi::SYSTEM)?);
        let overlay = Overlay(runtime.table(abi::OVERLAY)?);
        let input = Input(runtime.table(abi::INPUT)?);
        let applications = Applications(runtime.table(abi::APPLICATIONS)?);

        checked(
            applications.add_manifest(app_path.as_ptr(), true),
            tr("ViME を SteamVR に登録"),
        )?;
        checked(
            applications.identify(std::process::id(), actions::APP_KEY.as_ptr()),
            tr("SteamVR アプリを関連付け"),
        )?;
        checked(
            input.set_manifest(actions_path.as_ptr()),
            tr("コントローラー設定を読み込み"),
        )?;
        let set = |path: &CStr| {
            let mut handle = 0;
            checked(
                input.set_handle(path.as_ptr(), &mut handle),
                tr("SteamVR アクションセットを取得"),
            )?;
            Ok::<_, String>(handle)
        };
        let action = |path: &CStr| {
            let mut handle = 0;
            checked(
                input.action_handle(path.as_ptr(), &mut handle),
                tr("SteamVR アクションを取得"),
            )?;
            Ok::<_, String>(handle)
        };
        let hand = |path: &CStr| {
            let mut handle = 0;
            checked(
                input.source_handle(path.as_ptr(), &mut handle),
                tr("SteamVR コントローラーを取得"),
            )?;
            Ok::<_, String>(handle)
        };

        let toggle_set = set(actions::TOGGLE_SET)?;
        let pointer_set = set(actions::POINTER_SET)?;
        let toggle_action = action(actions::TOGGLE)?;
        let aim_action = action(actions::AIM)?;
        let click_action = action(actions::CLICK)?;
        let hands = [hand(c"/user/hand/left")?, hand(c"/user/hand/right")?];

        let mut handle = 0;
        checked(
            overlay.create(actions::APP_KEY.as_ptr(), c"ViME".as_ptr(), &mut handle),
            tr("VRキーボードを作成"),
        )?;
        let mut session = Self {
            _runtime: runtime,
            system,
            overlay,
            input,
            applications,
            handle,
            dashboard: 0,
            pointer_filters: Default::default(),
            toggle_set,
            pointer_set,
            toggle_action,
            aim_action,
            click_action,
            hands,
            panel: None,
            world_shown: false,
            width: 1.08,
            armed: [false; 2],
            quitting: false,
        };
        session.configure_texture(handle)?;
        session.create_dashboard(dashboard_size)?;
        Ok(session)
    }

    fn create_dashboard(&mut self, size: [u32; 2]) -> Result<()> {
        let mut thumbnail = 0;
        checked(
            self.overlay.create_dashboard(
                actions::DASHBOARD_KEY.as_ptr(),
                c"ViME".as_ptr(),
                &mut self.dashboard,
                &mut thumbnail,
            ),
            tr("SteamVR ダッシュボードに ViME を追加"),
        )?;
        self.configure_texture(self.dashboard)?;
        checked(
            self.overlay.set_width(self.dashboard, 1.9),
            tr("ダッシュボードのパネル幅を設定"),
        )?;
        checked(
            self.overlay.input_method(self.dashboard, abi::INPUT_MOUSE),
            tr("ダッシュボードのポインターを設定"),
        )?;
        checked(
            self.overlay
                .mouse_scale(self.dashboard, &size.map(|axis| axis as f32)),
            tr("ダッシュボードの座標を設定"),
        )?;

        // The static icon is submitted once. Interactive panels use persistent GL textures.
        if thumbnail != 0 {
            let icon = brand::icon();
            self.overlay
                .set_raw(thumbnail, icon.rgba.as_ptr(), icon.width, icon.height, 4);
        }
        Ok(())
    }

    fn configure_texture(&self, handle: u64) -> Result<()> {
        checked(
            self.overlay.flag(handle, abi::PREMULTIPLIED_ALPHA, true),
            tr("VR テクスチャの透過を設定"),
        )
    }

    pub fn open_bindings(&self, desktop: bool) -> Result<()> {
        checked(
            self.input
                .open_bindings(actions::APP_KEY.as_ptr(), self.toggle_set, 0, desktop),
            tr("SteamVR バインド画面を開く"),
        )
    }

    pub fn scene_pid(&self) -> u32 {
        self.applications.scene_pid()
    }

    pub fn visible(&self) -> bool {
        self.panel.is_some()
    }

    pub fn position(&mut self, width: f32) -> Result<()> {
        let mut head = Pose::default();
        self.system.tracking_pose(abi::STANDING, 0.0, &mut head, 1);
        if !head.valid || !head.connected {
            return Err(tr("ヘッドセットのトラッキングを取得できません").into());
        }
        let panel = geometry::anchor(&head.transform);
        self.set_width(width)?;
        checked(
            self.overlay
                .set_transform(self.handle, abi::STANDING, &panel),
            tr("VRキーボードを配置"),
        )?;
        self.panel = Some(panel);
        self.armed = [false; 2];
        Ok(())
    }

    pub fn set_width(&mut self, width: f32) -> Result<()> {
        checked(
            self.overlay.set_width(self.handle, width),
            tr("VRキーボードの幅を変更"),
        )?;
        self.width = width;
        Ok(())
    }

    pub fn hide(&mut self) -> Result<()> {
        if self.world_shown {
            checked(self.overlay.hide(self.handle), tr("VRキーボードを閉じる"))?;
            self.world_shown = false;
        }
        self.panel = None;
        self.clear_pointers();
        Ok(())
    }

    /// Submit while the graphics context owning this OpenGL texture is current.
    pub fn present(&mut self, surface: Surface, name: std::num::NonZeroU32) -> Result<()> {
        let handle = match surface {
            Surface::Keyboard => self.handle,
            Surface::Dashboard => self.dashboard,
        };
        let texture = Texture {
            handle: name.get() as usize as *mut c_void,
            kind: abi::TEXTURE_OPENGL,
            color_space: abi::COLOR_GAMMA,
        };
        checked(
            self.overlay.set_texture(handle, &texture),
            match surface {
                Surface::Keyboard => tr("VRキーボードを描画"),
                Surface::Dashboard => tr("ダッシュボードを描画"),
            },
        )?;
        if matches!(surface, Surface::Keyboard) && self.visible() && !self.world_shown {
            checked(self.overlay.show(handle), tr("VRキーボードを表示"))?;
            self.world_shown = true;
        }
        Ok(())
    }

    pub fn poll(&mut self, aspect: f32) -> Frame {
        let mut event = Event::default();
        for _ in 0..64 {
            if !self
                .system
                .poll_event(&mut event, size_of::<Event>() as u32)
            {
                break;
            }
            if event.kind == abi::QUIT {
                self.system.acknowledge_quit();
                self.quitting = true;
                return Frame {
                    quit: true,
                    ..Frame::default()
                };
            }
        }

        let dashboard = self.overlay.dashboard_visible();
        let dashboard_active = dashboard && self.overlay.dashboard_active(self.dashboard);
        let mut dashboard_events = Vec::new();
        for _ in 0..64 {
            if !self
                .overlay
                .poll_event(self.dashboard, &mut event, size_of::<Event>() as u32)
            {
                break;
            }
            if dashboard_active {
                let input = match event.kind {
                    abi::MOUSE_MOVE => Some(DashboardEvent::Move(event.mouse().0)),
                    abi::MOUSE_DOWN | abi::MOUSE_UP if event.mouse().1 == abi::MOUSE_LEFT => {
                        Some(DashboardEvent::Button {
                            position: event.mouse().0,
                            down: event.kind == abi::MOUSE_DOWN,
                        })
                    }
                    abi::FOCUS_LEAVE | abi::OVERLAY_HIDDEN => Some(DashboardEvent::Leave),
                    _ => None,
                };
                dashboard_events.extend(input);
            }
        }
        let pointing = self.world_shown && !dashboard;
        let mut frame = Frame {
            dashboard_visible: dashboard,
            dashboard_active,
            dashboard_events,
            ..Frame::default()
        };
        let mut sets = [ActionSet::default(), ActionSet::default()];
        sets[0].set = self.toggle_set;
        sets[0].priority = 1;
        sets[1].set = self.pointer_set;
        sets[1].priority = 20;
        let input_error = self.input.update(
            sets.as_ptr(),
            size_of::<ActionSet>() as u32,
            if pointing { 2 } else { 1 },
        );
        if input_error != 0 {
            frame.input_error = Some(input_error);
            self.clear_pointers();
            return frame;
        }

        let toggle = !dashboard
            && self
                .digital(self.toggle_action, 0)
                .is_some_and(|data| data.state && data.changed);
        if let Some(panel) = self.panel.filter(|_| pointing) {
            let now = Instant::now();
            for hand in 0..2 {
                let click = self.digital(self.click_action, self.hands[hand]);
                let down = click.as_ref().is_some_and(|data| data.state);
                let pressed = self.armed[hand] && down;
                self.armed[hand] = click.is_some_and(|data| !data.state);
                frame.keyboard_down[hand] = down;
                if let Some(uv) = self.pointer_position(hand, &panel, aspect, now) {
                    frame.pointers[hand] = Some(Pointer { uv, down });
                    frame.keyboard_presses[hand] = pressed.then_some(uv);
                }
            }
        } else {
            self.clear_pointers();
        }
        frame.toggle = toggle;
        frame
    }

    fn pointer_position(
        &mut self,
        hand: usize,
        panel: &Matrix,
        aspect: f32,
        now: Instant,
    ) -> Option<[f32; 2]> {
        let source = self.hands[hand];

        let mut pose = ActionPose::default();
        let code = self.input.aim(
            self.aim_action,
            abi::STANDING,
            0.0,
            &mut pose,
            size_of::<ActionPose>() as u32,
            source,
        );
        if code != 0 || !pose.active || !pose.pose.valid {
            self.pointer_filters[hand].reset();
            return None;
        }
        let Some(uv) = geometry::hit(panel, &pose.pose.transform, self.width, aspect) else {
            self.pointer_filters[hand].reset();
            return None;
        };
        Some(self.pointer_filters[hand].update(uv, now, aspect))
    }

    fn clear_pointers(&mut self) {
        self.armed = [false; 2];
        for filter in &mut self.pointer_filters {
            filter.reset();
        }
    }

    fn digital(&self, action: u64, device: u64) -> Option<Digital> {
        let mut data = Digital::default();
        let code = self
            .input
            .digital(action, &mut data, size_of::<Digital>() as u32, device);
        (code == 0 && data.active).then_some(data)
    }
}

impl Drop for SteamVr {
    fn drop(&mut self) {
        if !self.quitting {
            if self.dashboard != 0 {
                self.overlay.destroy(self.dashboard);
            }
            self.overlay.destroy(self.handle);
        }
    }
}

fn checked(code: i32, operation: &str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(tr_args(
            "{0}に失敗しました (OpenVR {1})",
            &[tr(operation), &code.to_string()],
        ))
    }
}
