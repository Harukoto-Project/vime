mod abi;
mod actions;
mod geometry;

use abi::{
    ActionPose, ActionSet, Applications, Digital, Event, Input, Matrix, Overlay, Pose, System,
    Texture, TextureBounds,
};
use libloading::Library;
use std::{
    ffi::{CStr, CString, c_void},
    path::Path,
    sync::{Mutex, MutexGuard},
};

type Result<T> = std::result::Result<T, String>;
static RUNTIME_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Pointer {
    pub hand: usize,
    pub uv: [f32; 2],
    pub down: bool,
}

#[derive(Default)]
pub struct Frame {
    pub toggle: bool,
    pub pointer: Option<Pointer>,
    pub dashboard_active: bool,
    pub dashboard_events: Vec<DashboardEvent>,
    pub quit: bool,
}

#[derive(Clone, Copy)]
pub enum DashboardEvent {
    Move([f32; 2]),
    Button(bool),
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
            .map_err(|_| "このプロセスは既に SteamVR に接続しています")?;
        let path = actions::runtime_dll().ok_or("SteamVR が見つかりません")?;
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
        checked(error, "SteamVR に接続")?;
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
            Err(format!("{} を取得できません", version.to_string_lossy()))
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
    toggle_set: u64,
    pointer_set: u64,
    toggle_action: u64,
    aim_action: u64,
    click_action: u64,
    hands: [u64; 2],
    panel: Option<Matrix>,
    world_shown: bool,
    width: f32,
    preferred_hand: usize,
    holding_trigger: bool,
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
                    .ok_or("SteamVR のパスをUTF-8に変換できません")?,
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
            "ViME を SteamVR に登録",
        )?;
        checked(
            applications.identify(std::process::id(), actions::APP_KEY.as_ptr()),
            "SteamVR アプリを関連付け",
        )?;
        checked(
            input.set_manifest(actions_path.as_ptr()),
            "コントローラー設定を読み込み",
        )?;
        let set = |path: &CStr| {
            let mut handle = 0;
            checked(
                input.set_handle(path.as_ptr(), &mut handle),
                "SteamVR アクションセットを取得",
            )?;
            Ok::<_, String>(handle)
        };
        let action = |path: &CStr| {
            let mut handle = 0;
            checked(
                input.action_handle(path.as_ptr(), &mut handle),
                "SteamVR アクションを取得",
            )?;
            Ok::<_, String>(handle)
        };
        let hand = |path: &CStr| {
            let mut handle = 0;
            checked(
                input.source_handle(path.as_ptr(), &mut handle),
                "SteamVR コントローラーを取得",
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
            "VRキーボードを作成",
        )?;
        let mut session = Self {
            _runtime: runtime,
            system,
            overlay,
            input,
            applications,
            handle,
            dashboard: 0,
            toggle_set,
            pointer_set,
            toggle_action,
            aim_action,
            click_action,
            hands,
            panel: None,
            world_shown: false,
            width: 1.08,
            preferred_hand: 1,
            holding_trigger: false,
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
            "SteamVR ダッシュボードに ViME を追加",
        )?;
        self.configure_texture(self.dashboard)?;
        checked(
            self.overlay.set_width(self.dashboard, 1.9),
            "ダッシュボードのパネル幅を設定",
        )?;
        checked(
            self.overlay.input_method(self.dashboard, abi::INPUT_MOUSE),
            "ダッシュボードのポインターを設定",
        )?;
        checked(
            self.overlay
                .mouse_scale(self.dashboard, &size.map(|axis| axis as f32)),
            "ダッシュボードの座標を設定",
        )?;

        // The static icon is submitted once. Interactive panels use persistent GL textures.
        let icon: Vec<_> = (0..64)
            .flat_map(|y| {
                (0..64).flat_map(move |x| {
                    let case = (8..56).contains(&x) && (17..47).contains(&y);
                    let keys = [22, 29].iter().any(|row| (*row..row + 4).contains(&y))
                        && (14..50).contains(&x)
                        && (x - 14) % 7 < 4;
                    let space = (37..41).contains(&y) && (21..43).contains(&x);
                    if keys || space {
                        [191, 226, 255, 255]
                    } else if case {
                        [44, 65, 91, 255]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
            })
            .collect();
        if thumbnail != 0 {
            self.overlay.set_raw(thumbnail, icon.as_ptr(), 64, 64, 4);
        }
        Ok(())
    }

    fn configure_texture(&self, handle: u64) -> Result<()> {
        let bounds = TextureBounds {
            u_min: 0.,
            v_min: 1.,
            u_max: 1.,
            v_max: 0.,
        };
        checked(
            self.overlay.texture_bounds(handle, &bounds),
            "VR テクスチャの向きを設定",
        )?;
        checked(
            self.overlay.flag(handle, abi::PREMULTIPLIED_ALPHA, true),
            "VR テクスチャの透過を設定",
        )
    }

    pub fn open_bindings(&self, desktop: bool) -> Result<()> {
        checked(
            self.input
                .open_bindings(actions::APP_KEY.as_ptr(), self.toggle_set, 0, desktop),
            "SteamVR バインド画面を開く",
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
            return Err("ヘッドセットのトラッキングを取得できません".into());
        }
        let panel = geometry::anchor(&head.transform);
        self.set_width(width)?;
        checked(
            self.overlay
                .set_transform(self.handle, abi::STANDING, &panel),
            "VRキーボードを配置",
        )?;
        self.panel = Some(panel);
        self.armed = [false; 2];
        self.holding_trigger = false;
        Ok(())
    }

    pub fn set_width(&mut self, width: f32) -> Result<()> {
        checked(
            self.overlay.set_width(self.handle, width),
            "VRキーボードの幅を変更",
        )?;
        self.width = width;
        Ok(())
    }

    pub fn hide(&mut self) -> Result<()> {
        if self.world_shown {
            checked(self.overlay.hide(self.handle), "VRキーボードを閉じる")?;
            self.world_shown = false;
        }
        self.panel = None;
        self.holding_trigger = false;
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
                Surface::Keyboard => "VRキーボードを描画",
                Surface::Dashboard => "ダッシュボードを描画",
            },
        )?;
        if matches!(surface, Surface::Keyboard) && self.visible() && !self.world_shown {
            checked(self.overlay.show(handle), "VRキーボードを表示")?;
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
                        Some(DashboardEvent::Button(event.kind == abi::MOUSE_DOWN))
                    }
                    abi::FOCUS_LEAVE | abi::OVERLAY_HIDDEN => Some(DashboardEvent::Leave),
                    _ => None,
                };
                dashboard_events.extend(input);
            }
        }
        let pointing = self.world_shown && !dashboard;
        let mut frame = Frame {
            dashboard_active,
            dashboard_events,
            ..Frame::default()
        };
        let mut sets = [ActionSet::default(), ActionSet::default()];
        sets[0].set = self.toggle_set;
        sets[0].priority = 1;
        sets[1].set = self.pointer_set;
        sets[1].priority = 20;
        if self.input.update(
            sets.as_ptr(),
            size_of::<ActionSet>() as u32,
            if pointing { 2 } else { 1 },
        ) != 0
        {
            self.armed = [false; 2];
            self.holding_trigger = false;
            return frame;
        }

        let toggle = !dashboard
            && self
                .digital(self.toggle_action, 0)
                .is_some_and(|data| data.state && data.changed);
        let pointer = if let Some(panel) = self.panel.filter(|_| pointing) {
            let previous = self.preferred_hand;
            let other = 1 - previous;
            let first = self.pointer(previous, &panel, aspect);
            let second = self.pointer(other, &panel, aspect);
            let selected = if self.holding_trigger {
                first
            } else if second.is_some_and(|p| p.down) && !first.is_some_and(|p| p.down) {
                second
            } else {
                first.or(second)
            };
            if let Some(pointer) = selected {
                self.preferred_hand = pointer.hand;
            }
            self.holding_trigger = selected.is_some_and(|pointer| pointer.down);
            selected
        } else {
            self.armed = [false; 2];
            self.holding_trigger = false;
            None
        };
        frame.toggle = toggle;
        frame.pointer = pointer;
        frame
    }

    fn pointer(&mut self, hand: usize, panel: &Matrix, aspect: f32) -> Option<Pointer> {
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
        let Some(click) = self.digital(self.click_action, source) else {
            self.armed[hand] = false;
            return None;
        };
        if code != 0 || !pose.active || !pose.pose.valid {
            self.armed[hand] = false;
            return None;
        }
        let uv = geometry::hit(panel, &pose.pose.transform, self.width, aspect);
        if !click.state {
            self.armed[hand] = true;
        } else if uv.is_none() {
            self.armed[hand] = false;
        }
        uv.map(|uv| Pointer {
            hand,
            uv,
            down: self.armed[hand] && click.state,
        })
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
        Err(format!("{operation}に失敗しました (OpenVR {code})"))
    }
}
