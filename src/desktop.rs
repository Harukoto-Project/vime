use crate::{
    dashboard::{self, Command, Dashboard, Page},
    hotkeys::Hotkeys,
    keyboard::{self, Action, Keyboard, Repeater, VrKeys},
    overlay_surface::OverlaySurface,
    power,
};
use eframe::egui::{
    self, Align, Button, Color32, FontData, FontDefinitions, FontFamily, Frame, Layout, Margin,
    RichText, ScrollArea, Stroke, Ui, ViewportBuilder,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use vime::{
    brand,
    i18n::{self, Lang, tr, tr_args},
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
const HIDDEN_PAINT_RETRY: Duration = Duration::from_millis(50);

pub fn run() -> eframe::Result {
    power::disable_throttling();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([980., 826.])
            .with_min_inner_size([670., 560.])
            .with_icon(brand::icon())
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
    paint_requested: Option<Instant>,
    dashboard_visible: bool,
    dashboard_revision: Option<(u64, bool)>,
    draw_errors: [Option<String>; 2],
    steamvr_message: String,
    settings_message: String,
    settings_error: bool,
    /// Held-key repeat for the desktop window, the VR dashboard and the VR keyboard.
    repeaters: [Repeater; 3],
}

const DESKTOP: usize = 0;
const DASHBOARD: usize = 1;
const WORLD: usize = 2;

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
        i18n::set_language(Lang::system());
        let directory = Settings::directory()?;
        let (settings, mut settings_message) = match Settings::load(&directory) {
            Ok(settings) => (settings, String::new()),
            Err(error) => (
                Settings::default(),
                tr_args("設定を読み込めませんでした ({0})", &[&error.to_string()]),
            ),
        };
        i18n::set_language(settings.language);

        let fonts = windows_fonts();
        creation.egui_ctx.set_fonts(fonts.clone());
        creation.egui_ctx.set_visuals(visuals());
        let surfaces = creation
            .gl
            .clone()
            .ok_or_else(|| tr("OpenGL を初期化できません").to_owned())
            .and_then(|gl| Surfaces::new(gl, fonts));
        let hotkeys = match Hotkeys::new(creation.egui_ctx.clone()) {
            Ok(mut hotkeys) => {
                match hotkeys.prepare(&settings.hotkey) {
                    Ok(key) => hotkeys.activate(key),
                    Err(error) => settings_message = error,
                }
                Some(hotkeys)
            }
            Err(error) => {
                settings_message =
                    tr_args("Windows ショートカットを初期化できません ({0})", &[&error]);
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
            paint_requested: None,
            dashboard_visible: false,
            dashboard_revision: None,
            draw_errors: [None, None],
            steamvr_message: String::new(),
            settings_error: !settings_message.is_empty(),
            settings_message,
            repeaters: Default::default(),
        })
    }

    fn apply_settings(&mut self) -> Result<(), String> {
        self.draft.hotkey = self.draft.hotkey.trim().to_owned();
        let prepared = match self.hotkeys.as_ref() {
            Some(hotkeys) => Some(hotkeys.prepare(&self.draft.hotkey)?),
            None if self.settings.hotkey != self.draft.hotkey => {
                return Err(tr("Windows ショートカットを設定できません").into());
            }
            None => None,
        };
        if let Err(error) = self.draft.save(&self.directory) {
            if let (Some(hotkeys), Some(key)) = (&self.hotkeys, prepared) {
                hotkeys.cancel(key);
            }
            return Err(tr_args("設定を保存できません ({0})", &[&error.to_string()]));
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
            self.steamvr_message = tr("先に SteamVR に接続してください").into();
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
        self.repeaters[WORLD].release();
    }

    fn disconnect(&mut self) {
        self.vr = None;
        self.world_keys = None;
        self.pending_world_keys = None;
        self.last_world = None;
        self.pointers = [None; 2];
        self.last_pointers = [None; 2];
        self.dashboard.reset();
        self.repeaters[WORLD].release();
        self.repeaters[DASHBOARD].release();
        self.dashboard_visible = false;
        self.paint_requested = None;
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
                self.settings_message = tr("設定を保存しました").into();
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
                            .map(|_| tr("OSCパケットを送信しました"))
                    }
                    Destination::SteamVrInput => output::send_scene_input(
                        self.vr.as_ref().map_or(0, SteamVr::scene_pid),
                        &text,
                    )
                    .map(|_| tr("入力イベントを送信しました")),
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
            Some(Command::Keyboard(action)) => {
                // A held key already acted when it was pressed.
                if !(action.repeats() && self.repeaters[DASHBOARD].holding(&action)) {
                    self.keyboard_action(action);
                }
            }
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
            if action.repeats() {
                self.repeaters[WORLD].begin(&action, Instant::now());
            }
            self.keyboard_action(action);
            if close {
                break;
            }
        }
    }

    /// Applies the keys that repeat while a VR controller holds them.
    fn world_key_repeats(&mut self) {
        let held = self.world_keys.as_ref().and_then(|keys| {
            self.pointers
                .iter()
                .flatten()
                .filter(|pointer| pointer.down)
                .find_map(|pointer| keys.action_at(pointer.uv).filter(Action::repeats))
        });
        if let Some(action) = self.repeaters[WORLD].follow(held.as_ref(), Instant::now()) {
            self.keyboard_action(action);
        }
    }

    /// Applies a click from `source` unless its press already acted through the
    /// repeater, then applies the key repeats of a held pointer.
    /// Returns whether any key was applied.
    fn pointer_repeats(
        &mut self,
        source: usize,
        click: Option<Action>,
        held: Option<Action>,
    ) -> bool {
        let repeater = &mut self.repeaters[source];
        let click = click.filter(|action| !(action.repeats() && repeater.holding(action)));
        let repeated = repeater.tick(held.as_ref(), Instant::now());
        let mut acted = false;
        for action in click.into_iter().chain(repeated) {
            self.keyboard_action(action);
            acted = true;
        }
        acted
    }

    fn page(&mut self, ui: &mut Ui) {
        ui.set_max_width(ui.available_width().min(1080.));
        ui.style_mut().spacing.interact_size.y = 30.;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                brand::wordmark(ui, 26.);
                ui.label(
                    RichText::new(tr("SteamVR向け IMEキーボード"))
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
            ui.label(RichText::new(tr("キーボードプレビュー")).size(18.).strong());
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
        let interaction = keyboard::show(
            ui,
            &self.keyboard,
            width,
            self.settings.destination,
            self.keyboard.caret_blink(Instant::now()).0,
        );
        if self.pointer_repeats(DESKTOP, interaction.action, interaction.held) {
            ui.ctx().request_repaint();
        }
        if let Some(wait) = self.repeaters[DESKTOP].wait(Instant::now()) {
            ui.ctx().request_repaint_after(wait);
        }
        let hint = tr("VR では右スティック押し込みで開閉し、左右のトリガーでキーを選びます。");
        ui.label(RichText::new(hint).color(SECONDARY));
    }

    fn steamvr_panel(&mut self, ui: &mut Ui) {
        panel().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("SteamVR").size(17.).strong());
                    let status = match &self.vr {
                        Some(vr) if vr.visible() => tr("接続済み · キーボード表示中"),
                        Some(_) => tr("接続済み"),
                        None => tr("未接続"),
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
                        if ui.button(tr("切断")).clicked() {
                            self.disconnect();
                            self.steamvr_message.clear();
                            return;
                        }
                        if ui.button(tr("VR の割り当てを編集")).clicked() {
                            self.steamvr_message = vr.open_bindings(true).err().unwrap_or_default();
                        }
                        if ui
                            .add(Button::new(if visible {
                                tr("キーボードを閉じる")
                            } else {
                                tr("キーボードを開く")
                            }))
                            .clicked()
                        {
                            self.toggle();
                        }
                    } else if ui.add(primary(tr("SteamVR に接続"))).clicked() {
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
            ui.label(RichText::new(tr("設定")).size(18.).strong());
            ui.add_space(5.);
            ui.columns(2, |columns| {
                let [destination, binding] = columns else {
                    unreachable!("two settings columns")
                };
                destination.label(RichText::new(tr("送信先")).color(SECONDARY));
                dashboard::destination_options(destination, &mut self.draft.destination);
                if self.draft.destination == Destination::Osc {
                    destination.horizontal_wrapped(|ui| {
                        ui.label(tr("UDP ポート"));
                        ui.add(egui::DragValue::new(&mut self.draft.osc_port).range(1..=65535));
                        ui.checkbox(&mut self.draft.osc_notify, tr("チャット通知音"));
                    });
                    destination.label(
                        RichText::new(tr("VRChat側でOSCを有効にしてください"))
                            .size(12.)
                            .color(SECONDARY),
                    );
                } else {
                    destination.label(
                        RichText::new(tr("VRアプリの入力欄を開き、Windowsで前面にします"))
                            .size(12.)
                            .color(SECONDARY),
                    );
                }

                binding.label(RichText::new(tr("Windows 開閉ショートカット")).color(SECONDARY));
                binding.add(
                    egui::TextEdit::singleline(&mut self.draft.hotkey)
                        .hint_text("Ctrl+Shift+F8")
                        .desired_width(200.),
                );
                binding.horizontal_wrapped(|ui| {
                    ui.label(tr("キーボード幅"));
                    ui.add(
                        egui::Slider::new(&mut self.draft.overlay_width, 0.65..=1.65).suffix(" m"),
                    );
                });
                binding.add_space(6.);
                binding.label(RichText::new(tr("言語")).color(SECONDARY));
                dashboard::language_options(binding, &mut self.draft.language);
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
                        .add_enabled(self.draft != self.settings, primary(tr("設定を保存")))
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
        if i18n::language() != self.draft.language {
            // The language picker takes effect at once; saving only remembers it.
            i18n::set_language(self.draft.language);
            self.keyboard.status("");
            self.dashboard.redraw = true;
        }
        self.keyboard.poll_candidates();
        let held = self.dashboard.held.clone();
        if let Some(action) = self.repeaters[DASHBOARD].tick(held.as_ref(), Instant::now()) {
            self.keyboard_action(action);
        }
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
                self.steamvr_message = tr("SteamVR との接続が終了しました").into();
            } else {
                let resume_world =
                    self.dashboard_visible && !frame.dashboard_visible && vr.visible();
                let resume_dashboard = !self.dashboard.active && frame.dashboard_active;
                self.dashboard_visible = frame.dashboard_visible;
                if let Ok(surfaces) = &mut self.surfaces {
                    // Retry timing from a hidden overlay must not carry over when it reappears.
                    if resume_world {
                        surfaces.keyboard.reset();
                        self.pending_world_keys = None;
                        self.last_world = None;
                        self.draw_errors[0] = None;
                    }
                    if resume_dashboard {
                        surfaces.dashboard.reset();
                        self.draw_errors[1] = None;
                    }
                }
                if resume_world || resume_dashboard {
                    self.paint_requested = None;
                }
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
                    self.world_key_repeats();
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
            if (keyboard_changed || dashboard_changed)
                && self
                    .paint_requested
                    .is_none_or(|last| now - last >= HIDDEN_PAINT_RETRY)
            {
                self.paint_requested = Some(now);
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
        self.paint_requested = None;
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
        // Segoe UI for Latin text, then the UI variants of the Japanese and
        // Korean system fonts, like most Windows apps. They go in front of
        // egui's bundled fonts, which stay behind as a last fallback.
        let mut front = 0;
        for (name, file, index) in [
            ("segoe-ui", "segoeui.ttf", 0),
            ("yu-gothic-ui", "YuGothR.ttc", 1),
            ("meiryo-ui", "meiryo.ttc", 1),
            ("malgun", "malgun.ttf", 0),
            ("symbols", "seguisym.ttf", 0),
        ] {
            if let Some(data) = read_font(&folder, file, index) {
                fonts.font_data.insert(name.to_owned(), data);
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .insert(front, name.to_owned());
                front += 1;
            }
        }
        if let Some(data) = read_font(&folder, "seguiemj.ttf", 0) {
            fonts.font_data.insert("emoji".to_owned(), data);
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .push("emoji".to_owned());
        }
    }
    fonts
}

fn read_font(folder: &Path, file: &str, index: u32) -> Option<Arc<FontData>> {
    let mut data = FontData::from_owned(fs::read(folder.join(file)).ok()?);
    data.index = index;
    Some(Arc::new(data))
}
