mod abi;
mod actions;
mod geometry;

use abi::{
    ActionPose, ActionSet, Applications, Digital, Event, Input, Matrix, Overlay, Pose, System,
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
    pub quit: bool,
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
    toggle_set: u64,
    pointer_set: u64,
    toggle_action: u64,
    aim_action: u64,
    click_action: u64,
    hands: [u64; 2],
    panel: Option<Matrix>,
    width: f32,
    preferred_hand: usize,
    armed: [bool; 2],
    quitting: bool,
}

impl SteamVr {
    pub fn connect(directory: &Path) -> Result<Self> {
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
        let runtime = Runtime::connect()?;
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
        Ok(Self {
            _runtime: runtime,
            system,
            overlay,
            input,
            applications,
            handle,
            toggle_set,
            pointer_set,
            toggle_action,
            aim_action,
            click_action,
            hands,
            panel: None,
            width: 1.08,
            preferred_hand: 1,
            armed: [false; 2],
            quitting: false,
        })
    }

    pub fn open_bindings(&self) -> Result<()> {
        checked(
            self.input
                .open_bindings(actions::APP_KEY.as_ptr(), 0, 0, true),
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
        checked(self.overlay.show(self.handle), "VRキーボードを表示")?;
        self.panel = Some(panel);
        self.armed = [false; 2];
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
        checked(self.overlay.hide(self.handle), "VRキーボードを閉じる")?;
        self.panel = None;
        Ok(())
    }

    pub fn upload(&self, pixels: &[u8], width: u32, height: u32) -> Result<()> {
        if pixels.len() != width as usize * height as usize * 4 {
            return Err("VRキーボードの画像サイズが一致しません".into());
        }
        checked(
            self.overlay
                .set_raw(self.handle, pixels.as_ptr(), width, height, 4),
            "VRキーボードを描画",
        )
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
        let pointing = self.visible() && !dashboard;
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
            return Frame::default();
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
            let selected = if second.is_some_and(|p| p.down) && !first.is_some_and(|p| p.down) {
                second
            } else {
                first.or(second)
            };
            if let Some(pointer) = selected {
                self.preferred_hand = pointer.hand;
            }
            selected
        } else {
            self.armed = [false; 2];
            None
        };
        Frame {
            toggle,
            pointer,
            quit: false,
        }
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
