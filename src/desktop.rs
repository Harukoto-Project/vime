use crate::{
    dashboard::{self, Command, Dashboard, Page},
    hotkeys::Hotkeys,
    keyboard::{self, Action, Keyboard, VrKeys},
    overlay_surface::OverlaySurface,
};
use eframe::egui::{
    self, Align, Button, Color32, FontData, FontDefinitions, FontFamily, Frame, Layout, Margin,
    RichText, ScrollArea, Stroke, Ui, ViewportBuilder,
};
use std::{
    env, fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use vime::{
    output,
    settings::{Destination, Settings},
    steamvr::{Pointer, SteamVr, Surface},
};

const BACKGROUND: Color32 = Color32::from_rgb(20, 26, 35);
const PANEL: Color32 = Color32::from_rgb(29, 37, 49);
const EDGE: Color32 = Color32::from_rgb(53, 67, 83);
const INK: Color32 = Color32::from_rgb(241, 245, 251);
const SECONDARY: Color32 = Color32::from_rgb(176, 192, 209);
const ACCENT: Color32 = Color32::from_rgb(159, 211, 255);
const ERROR: Color32 = Color32::from_rgb(255, 177, 168);

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([980., 826.])
            .with_min_inner_size([670., 560.])
            .with_app_id("org.harukoto.vime"),
        renderer: eframe::Renderer::Glow,
        glow_options: egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "ViME",
        options,
        Box::new(|creation| Ok(Box::new(App::new(creation)?))),
    )
}

struct App {
    directory: PathBuf,
    settings: Settings,
    draft: Settings,
    hotkeys: Option<Hotkeys>,
    keyboard: Keyboard,
    surfaces: Result<Surfaces, String>,
    vr: Option<SteamVr>,
    auto_connect: bool,
    page: Page,
    dashboard: Dashboard,
    world_keys: Option<VrKeys>,
    pending_world_keys: Option<VrKeys>,
    last_world: Option<(u64, Destination, bool)>,
    pointers: [Option<Pointer>; 2],
    last_pointers: [Option<Pointer>; 2],
    paint_requested: bool,
    dashboard_revision: Option<(u64, bool)>,
    draw_errors: [Option<String>; 2],
    steamvr_message: String,
    settings_message: String,
    settings_error: bool,
}

struct Surfaces {
    keyboard: OverlaySurface,
    dashboard: OverlaySurface,
}

impl Surfaces {
    fn new(gl: Arc<eframe::glow::Context>, fonts: FontDefinitions) -> Result<Self, String> {
        let keyboard =
            OverlaySurface::new(gl.clone(), fonts.clone(), visuals(), keyboard::SIZE, true)?;
        match OverlaySurface::new(gl, fonts, visuals(), dashboard::SIZE, false) {
            Ok(dashboard) => Ok(Self {
                keyboard,
                dashboard,
            }),
            Err(error) => {
                keyboard.destroy();
                Err(error)
            }
        }
    }
}

impl App {
    fn new(creation: &eframe::CreationContext<'_>) -> std::io::Result<Self> {
        let fonts = windows_fonts();
        creation.egui_ctx.set_fonts(fonts.clone());
        creation.egui_ctx.set_visuals(visuals());
        let surfaces = creation
            .gl
            .clone()
            .ok_or_else(|| "OpenGL を初期化できません".to_owned())
            .and_then(|gl| Surfaces::new(gl, fonts));

        let directory = Settings::directory()?;
        let (settings, mut settings_message) = match Settings::load(&directory) {
            Ok(settings) => (settings, String::new()),
            Err(error) => (
                Settings::default(),
                format!("設定を読み込めませんでした ({error})"),
            ),
        };
        let hotkeys = match Hotkeys::new(creation.egui_ctx.clone()) {
            Ok(mut hotkeys) => {
                match hotkeys.prepare(&settings.hotkey) {
                    Ok(key) => hotkeys.activate(key),
                    Err(error) => settings_message = error,
                }
                Some(hotkeys)
            }
            Err(error) => {
                settings_message = format!("Windows ショートカットを初期化できません ({error})");
                None
            }
        };
        Ok(Self {
            directory,
            draft: settings.clone(),
            settings,
            hotkeys,
            keyboard: Keyboard::new(creation.egui_ctx.clone()),
            surfaces,
            vr: None,
            auto_connect: true,
            page: Page::default(),
            dashboard: Dashboard::default(),
            world_keys: None,
            pending_world_keys: None,
            last_world: None,
            pointers: [None; 2],
            last_pointers: [None; 2],
            paint_requested: false,
            dashboard_revision: None,
            draw_errors: [None, None],
            steamvr_message: String::new(),
            settings_error: !settings_message.is_empty(),
            settings_message,
        })
    }

    fn apply_settings(&mut self) -> Result<(), String> {
        self.draft.hotkey = self.draft.hotkey.trim().to_owned();
        let prepared = match self.hotkeys.as_ref() {
            Some(hotkeys) => Some(hotkeys.prepare(&self.draft.hotkey)?),
            None if self.settings.hotkey != self.draft.hotkey => {
                return Err("Windows ショートカットを設定できません".into());
            }
            None => None,
        };
        if let Err(error) = self.draft.save(&self.directory) {
            if let (Some(hotkeys), Some(key)) = (&self.hotkeys, prepared) {
                hotkeys.cancel(key);
            }
            return Err(format!("設定を保存できません ({error})"));
        }
        if let (Some(hotkeys), Some(key)) = (&mut self.hotkeys, prepared) {
            hotkeys.activate(key);
        }
        self.settings = self.draft.clone();
        if let Some(vr) = &mut self.vr
            && let Err(error) = vr.set_width(self.settings.overlay_width)
        {
            self.steamvr_message = error;
        }
        Ok(())
    }

    fn connect(&mut self) {
        if let Err(error) = &self.surfaces {
            self.steamvr_message = error.clone();
            return;
        }
        match SteamVr::connect(&self.directory, dashboard::SIZE) {
            Ok(vr) => {
                self.steamvr_message.clear();
                self.dashboard.reset();
                self.vr = Some(vr);
            }
            Err(error) => self.steamvr_message = error,
        }
    }

    fn toggle(&mut self) {
        let Some(vr) = &mut self.vr else {
            self.steamvr_message = "先に SteamVR に接続してください".into();
            return;
        };
        let new_visibility = !vr.visible();
        let result = if new_visibility {
            vr.position(self.settings.overlay_width)
        } else {
            vr.hide()
        };
        self.steamvr_message = result.err().unwrap_or_default();
        if let Ok(surfaces) = &mut self.surfaces {
            surfaces.keyboard.reset();
        }
        self.world_keys = None;
        self.pending_world_keys = None;
        self.last_world = None;
        self.pointers = [None; 2];
        self.last_pointers = [None; 2];
        self.draw_errors[0] = None;
    }

    fn disconnect(&mut self) {
        self.vr = None;
        self.world_keys = None;
        self.pending_world_keys = None;
        self.last_world = None;
        self.pointers = [None; 2];
        self.last_pointers = [None; 2];
        self.dashboard.reset();
        self.draw_errors = [None, None];
        if let Ok(surfaces) = &mut self.surfaces {
            surfaces.keyboard.reset();
            surfaces.dashboard.reset();
        }
    }

    fn save_draft(&mut self) {
        match self.apply_settings() {
            Ok(()) => {
                self.settings_error = false;
                self.settings_message = "設定を保存しました".into();
            }
            Err(error) => {
                self.settings_error = true;
                self.settings_message = error;
            }
        }
        self.dashboard.redraw = true;
    }

    fn keyboard_action(&mut self, action: Action) {
        match action {
            Action::Close => {
                if self.vr.as_ref().is_some_and(SteamVr::visible) {
                    self.toggle();
                }
            }
            Action::Enter if self.keyboard.composer.has_preedit() => {
                self.keyboard.apply(Action::Commit)
            }
            Action::Enter => {
                let text = self.keyboard.composer.text();
                let sent = match self.settings.destination {
                    Destination::Osc => {
                        output::send_osc(&text, self.settings.osc_port, self.settings.osc_notify)
                            .map(|_| "OSCパケットを送信しました")
                    }
                    Destination::SteamVrInput => output::send_scene_input(
                        self.vr.as_ref().map_or(0, SteamVr::scene_pid),
                        &text,
                    )
                    .map(|_| "入力イベントを送信しました"),
                };
                match sent {
                    Ok(message) => {
                        self.keyboard.clear();
                        self.keyboard.status(message);
                    }
                    Err(error) => self.keyboard.status(error.to_string()),
                }
            }
            action => self.keyboard.apply(action),
        }
    }

    fn paint_overlay(&mut self) {
        let (Some(vr), Ok(surfaces)) = (&mut self.vr, &mut self.surfaces) else {
            return;
        };
        let now = Instant::now();
        let (caret_visible, _) = self.keyboard.caret_blink(now);
        if vr.visible() && surfaces.keyboard.can_present(now) {
            let frame = (
                self.keyboard.revision,
                self.settings.destination,
                caret_visible,
            );
            let keyboard_changed = self.last_world != Some(frame);
            if keyboard_changed {
                self.last_world = Some(frame);
                self.pending_world_keys = Some(surfaces.keyboard.render(Vec::new(), |ui| {
                    keyboard::show_vr(ui, &self.keyboard, self.settings.destination, caret_visible)
                }));
            }
            if keyboard_changed
                || pointer_changed(&surfaces.keyboard, now, self.last_pointers, self.pointers)
            {
                let keys = self
                    .pending_world_keys
                    .as_ref()
                    .or(self.world_keys.as_ref());
                let pointers = self.pointers.map(|pointer| {
                    let pointer = pointer?;
                    Some((keys?.position(pointer.uv), pointer.down))
                });
                surfaces.keyboard.compose_pointers(pointers);
                self.last_pointers = self.pointers;
            }
            if let Some(result) = surfaces
                .keyboard
                .present(|texture| vr.present(Surface::Keyboard, texture))
            {
                if result.is_ok()
                    && let Some(keys) = self.pending_world_keys.take()
                {
                    self.world_keys = Some(keys);
                }
                self.draw_errors[0] = result.err();
            }
        }

        let mut command = None;
        let revision = Some((self.keyboard.revision, caret_visible));
        if surfaces.dashboard.can_present(now)
            && (self.dashboard.redraw
                || (self.dashboard.active && self.dashboard_revision != revision))
        {
            self.dashboard.redraw = false;
            self.dashboard_revision = revision;
            let events = self.dashboard.take_events();
            command = surfaces.dashboard.render(events, |ui| {
                self.dashboard.show(
                    ui,
                    &self.keyboard,
                    &self.settings,
                    &mut self.draft,
                    &self.settings_message,
                    caret_visible,
                )
            });
        }
        if let Some(result) = surfaces
            .dashboard
            .present(|texture| vr.present(Surface::Dashboard, texture))
        {
            self.draw_errors[1] = result.err();
        }

        match command {
            Some(Command::Keyboard(action)) => self.keyboard_action(action),
            Some(Command::OpenBindings) => {
                self.settings_message = vr.open_bindings(false).err().unwrap_or_default();
                self.dashboard.redraw = true;
            }
            Some(Command::SaveSettings) => self.save_draft(),
            None => {}
        }
    }

    fn world_key_presses(&mut self, presses: [Option<[f32; 2]>; 2]) {
        let Some(keys) = &self.world_keys else {
            return;
        };
        let actions = presses.map(|point| point.and_then(|point| keys.action_at(point)));
        let mut entered = false;
        for action in actions.into_iter().flatten() {
            if matches!(action, Action::Enter) && std::mem::replace(&mut entered, true) {
                continue;
            }
            let close = matches!(action, Action::Close);
            self.keyboard_action(action);
            if close {
                break;
            }
        }
    }

    fn page(&mut self, ui: &mut Ui) {
        ui.set_max_width(ui.available_width().min(1080.));
        ui.style_mut().spacing.interact_size.y = 30.;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("ViME").size(35.).strong().color(INK));
                ui.label(
                    RichText::new("SteamVR向け IMEキーボード")
                        .size(15.)
                        .color(SECONDARY),
                );
            });
        });
        ui.add_space(17.);
        self.steamvr_panel(ui);
        ui.add_space(14.);
        Page::tabs(&mut self.page, ui);
        ui.add_space(14.);
        if self.page == Page::Settings {
            let before = self.draft.clone();
            self.settings_panel(ui);
            self.dashboard.redraw |= self.draft != before;
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("キーボードプレビュー").size(18.).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(self.settings.destination.label())
                        .size(13.)
                        .color(SECONDARY),
                );
            });
        });
        ui.add_space(5.);
        let width = ui.available_width().min(900.);
        if let Some(action) = keyboard::show(
            ui,
            &self.keyboard,
            width,
            self.settings.destination,
            self.keyboard.caret_blink(Instant::now()).0,
        ) {
            self.keyboard_action(action);
            ui.ctx().request_repaint();
        }
        ui.label(
            RichText::new("VR では右スティック押し込みで開閉し、左右のトリガーでキーを選びます。")
                .color(SECONDARY),
        );
    }

    fn steamvr_panel(&mut self, ui: &mut Ui) {
        panel().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("SteamVR").size(17.).strong());
                    let status = match &self.vr {
                        Some(vr) if vr.visible() => "接続済み · キーボード表示中",
                        Some(_) => "接続済み",
                        None => "未接続",
                    };
                    ui.label(RichText::new(status).color(if self.vr.is_some() {
                        ACCENT
                    } else {
                        SECONDARY
                    }));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(vr) = &self.vr {
                        let visible = vr.visible();
                        if ui.button("切断").clicked() {
                            self.disconnect();
                            self.steamvr_message.clear();
                            return;
                        }
                        if ui.button("VR の割り当てを編集").clicked() {
                            self.steamvr_message = vr.open_bindings(true).err().unwrap_or_default();
                        }
                        if ui
                            .add(Button::new(if visible {
                                "キーボードを閉じる"
                            } else {
                                "キーボードを開く"
                            }))
                            .clicked()
                        {
                            self.toggle();
                        }
                    } else if ui.add(primary("SteamVR に接続")).clicked() {
                        self.connect();
                    }
                });
            });
            if !self.steamvr_message.is_empty() {
                ui.label(RichText::new(&self.steamvr_message).color(ERROR));
            }
            for error in self.draw_errors.iter().flatten() {
                ui.label(RichText::new(error).color(ERROR));
            }
        });
    }

    fn settings_panel(&mut self, ui: &mut Ui) {
        panel().show(ui, |ui| {
            ui.label(RichText::new("設定").size(18.).strong());
            ui.add_space(5.);
            ui.columns(2, |columns| {
                let [destination, binding] = columns else {
                    unreachable!("two settings columns")
                };
                destination.label(RichText::new("送信先").color(SECONDARY));
                dashboard::destination_options(destination, &mut self.draft.destination);
                if self.draft.destination == Destination::Osc {
                    destination.horizontal_wrapped(|ui| {
                        ui.label("UDP ポート");
                        ui.add(egui::DragValue::new(&mut self.draft.osc_port).range(1..=65535));
                        ui.checkbox(&mut self.draft.osc_notify, "チャット通知音");
                    });
                    destination.label(
                        RichText::new("VRChat側でOSCを有効にしてください")
                            .size(12.)
                            .color(SECONDARY),
                    );
                } else {
                    destination.label(
                        RichText::new("VRアプリの入力欄を開き、Windowsで前面にします")
                            .size(12.)
                            .color(SECONDARY),
                    );
                }

                binding.label(RichText::new("Windows 開閉ショートカット").color(SECONDARY));
                binding.add(
                    egui::TextEdit::singleline(&mut self.draft.hotkey)
                        .hint_text("Ctrl+Shift+F8")
                        .desired_width(200.),
                );
                binding.horizontal_wrapped(|ui| {
                    ui.label("キーボード幅");
                    ui.add(
                        egui::Slider::new(&mut self.draft.overlay_width, 0.65..=1.65).suffix(" m"),
                    );
                });
            });
            ui.add_space(10.);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.settings_message).small().color(
                    if self.settings_error {
                        ERROR
                    } else {
                        SECONDARY
                    },
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(self.draft != self.settings, primary("設定を保存"))
                        .clicked()
                    {
                        self.save_draft();
                    }
                });
            });
        });
    }
}

impl eframe::App for App {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.keyboard.poll_candidates();
        if std::mem::take(&mut self.auto_connect) {
            self.connect();
        }
        if self.hotkeys.as_ref().is_some_and(Hotkeys::pressed) {
            self.toggle();
        }
        if let Some(vr) = &mut self.vr {
            let frame = vr.poll(keyboard::ASPECT);
            if frame.quit {
                self.disconnect();
                self.steamvr_message = "SteamVR との接続が終了しました".into();
            } else {
                self.dashboard
                    .feed(frame.dashboard_active, frame.dashboard_events);
                if frame.toggle {
                    self.toggle();
                } else {
                    self.pointers = frame.pointers.map(|pointer| {
                        pointer.map(|mut pointer| {
                            for (axis, pixels) in pointer.uv.iter_mut().zip(keyboard::SIZE) {
                                *axis = (*axis * pixels as f32).round() / pixels as f32;
                            }
                            pointer
                        })
                    });
                    self.world_key_presses(frame.keyboard_presses);
                }
            }
        }
        let now = Instant::now();
        let (caret_visible, blink_after) = self.keyboard.caret_blink(now);
        let desktop_minimized = context.input(|input| input.viewport().minimized == Some(true));
        if !desktop_minimized {
            context.request_repaint_after(blink_after);
        }
        if let (Some(vr), Ok(surfaces)) = (&self.vr, &self.surfaces) {
            let visible = vr.visible() || self.dashboard.active;
            let keyboard_changed = vr.visible()
                && surfaces.keyboard.can_present(now)
                && (self.last_world
                    != Some((
                        self.keyboard.revision,
                        self.settings.destination,
                        caret_visible,
                    ))
                    || pointer_changed(&surfaces.keyboard, now, self.last_pointers, self.pointers)
                    || self.draw_errors[0].is_some());
            let dashboard_changed = surfaces.dashboard.can_present(now)
                && (self.dashboard.redraw
                    || self.draw_errors[1].is_some()
                    || (self.dashboard.active
                        && self.dashboard_revision
                            != Some((self.keyboard.revision, caret_visible))));
            if (keyboard_changed || dashboard_changed) && !self.paint_requested {
                self.paint_requested = true;
                context.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            }
            // Poll VR independently of the desktop monitor's refresh rate.
            context.request_repaint_after(Duration::from_micros(if visible {
                11_111
            } else {
                33_333
            }));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.paint_requested = false;
        self.paint_overlay();
        if ui.input(|input| input.viewport().minimized == Some(true)) {
            return;
        }
        // The root ui is rebuilt by eframe, including after a hidden-window VR frame.
        Frame::new()
            .fill(BACKGROUND)
            .inner_margin(Margin::symmetric(27, 18))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| self.page(ui));
            });
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.disconnect();
        if gl.is_some()
            && let Ok(surfaces) = std::mem::replace(&mut self.surfaces, Err(String::new()))
        {
            surfaces.keyboard.destroy();
            surfaces.dashboard.destroy();
        }
    }
}

fn pointer_changed(
    surface: &OverlaySurface,
    now: Instant,
    previous: [Option<Pointer>; 2],
    current: [Option<Pointer>; 2],
) -> bool {
    current != previous
        && (surface.pointer_frame_due(now)
            || current.map(|pointer| pointer.map(|pointer| pointer.down))
                != previous.map(|pointer| pointer.map(|pointer| pointer.down)))
}

fn primary(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(BACKGROUND).strong()).fill(ACCENT)
}

fn panel() -> Frame {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1., EDGE))
        .corner_radius(12)
        .inner_margin(Margin::symmetric(16, 12))
}

fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BACKGROUND;
    visuals.extreme_bg_color = BACKGROUND;
    visuals.selection.bg_fill = Color32::from_rgb(53, 88, 119);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(41, 52, 68);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(57, 73, 93);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(70, 90, 113);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals
}

fn windows_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    if let Some(windows) = env::var_os("WINDIR") {
        let folder = PathBuf::from(windows).join("Fonts");
        for (name, file) in [
            ("meiryo", "meiryo.ttc"),
            ("malgun", "malgun.ttf"),
            ("emoji", "seguiemj.ttf"),
        ] {
            if let Ok(data) = fs::read(folder.join(file)) {
                fonts
                    .font_data
                    .insert(name.to_owned(), Arc::new(FontData::from_owned(data)));
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }
    fonts
}
