mod abi;
mod actions;
mod controllers;
mod geometry;

use crate::{
    brand,
    i18n::{tr, tr_args},
};
use abi::{Applications, Event, Input, Matrix, Overlay, Pose, System, Texture};
use controllers::{Actions, ControllerWorker, KeyPresses};
use libloading::Library;
use std::{
    ffi::{CStr, CString, c_void},
    path::Path,
    sync::{Mutex, MutexGuard},
};

type Result<T> = std::result::Result<T, String>;
static RUNTIME_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
pub struct Frame {
    pub toggle: bool,
    pub keyboard_presses: Vec<KeyPresses>,
    pub keyboard_down: [bool; 2],
    pub dashboard_visible: bool,
    pub dashboard_active: bool,
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
    applications: Applications,
    handle: u64,
    dashboard: u64,
    cursors: [u64; 2],
    controllers: Option<ControllerWorker>,
    panel: Option<Matrix>,
    world_shown: bool,
    quitting: bool,
}

impl SteamVr {
    pub fn connect(
        directory: &Path,
        keyboard_size: [u32; 2],
        dashboard_size: [u32; 2],
    ) -> Result<Self> {
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

        let controller_actions = Actions {
            toggle_set: set(actions::TOGGLE_SET)?,
            pointer_set: set(actions::POINTER_SET)?,
            toggle: action(actions::TOGGLE)?,
            aim: action(actions::AIM)?,
            click: action(actions::CLICK)?,
            hands: [hand(c"/user/hand/left")?, hand(c"/user/hand/right")?],
        };

        let mut handle = 0;
        checked(
            overlay.create(actions::APP_KEY.as_ptr(), c"ViME".as_ptr(), &mut handle),
            tr("VRキーボードを作成"),
        )?;
        let mut session = Self {
            _runtime: runtime,
            system,
            overlay,
            applications,
            handle,
            dashboard: 0,
            cursors: [0; 2],
            controllers: None,
            panel: None,
            world_shown: false,
            quitting: false,
        };
        session.configure_texture(handle)?;
        session.create_dashboard(dashboard_size)?;
        session.create_cursors()?;
        session.controllers = Some(ControllerWorker::start(
            input,
            Overlay(session.overlay.0),
            controller_actions,
            handle,
            session.cursors,
            keyboard_size,
            1.08,
        )?);
        Ok(session)
    }

    fn create_cursors(&mut self) -> Result<()> {
        for (hand, (handle, key)) in self
            .cursors
            .iter_mut()
            .zip([
                c"org.harukoto.vime.pointer.left",
                c"org.harukoto.vime.pointer.right",
            ])
            .enumerate()
        {
            let operation = tr("VRキーボードを描画");
            checked(
                self.overlay
                    .create(key.as_ptr(), c"ViME Pointer".as_ptr(), handle),
                operation,
            )?;
            controllers::configure_cursor(&self.overlay, *handle, hand, operation)?;
        }
        Ok(())
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
            self.controllers
                .as_ref()
                .unwrap()
                .state()
                .open_bindings(desktop),
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
        Ok(())
    }

    pub fn set_width(&mut self, width: f32) -> Result<()> {
        checked(
            self.overlay.set_width(self.handle, width),
            tr("VRキーボードの幅を変更"),
        )?;
        self.controllers.as_ref().unwrap().state().set_width(width);
        Ok(())
    }

    pub fn hide(&mut self) -> Result<()> {
        if self.world_shown {
            checked(self.overlay.hide(self.handle), tr("VRキーボードを閉じる"))?;
            self.world_shown = false;
        }
        self.panel = None;
        self.controllers.as_ref().unwrap().state().set_panel(None);
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
            let controllers = self.controllers.as_ref().unwrap();
            controllers.state().set_panel(self.panel);
            controllers.wake();
        }
        Ok(())
    }

    pub fn poll(&mut self) -> Frame {
        let mut event = Event::default();
        for _ in 0..64 {
            if !self
                .system
                .poll_event(&mut event, size_of::<Event>() as u32)
            {
                break;
            }
            if event.kind == abi::QUIT {
                drop(self.controllers.take());
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
        let events = self.controllers.as_ref().unwrap().state().take_events();
        Frame {
            toggle: !dashboard && events.toggle,
            keyboard_presses: if dashboard {
                Vec::new()
            } else {
                events.presses
            },
            keyboard_down: if dashboard { [false; 2] } else { events.down },
            dashboard_visible: dashboard,
            dashboard_active,
            dashboard_events,
            quit: false,
        }
    }
}

impl Drop for SteamVr {
    fn drop(&mut self) {
        drop(self.controllers.take());
        if !self.quitting {
            for handle in self.cursors.into_iter().filter(|&handle| handle != 0) {
                self.overlay.destroy(handle);
            }
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
