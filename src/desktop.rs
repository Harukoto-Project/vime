mod learned_typos;

use crate::{
    dashboard::{self, Command, Dashboard},
    hotkeys::Hotkeys,
    keyboard::{self, Action, CalibrationJob, Keyboard, Repeater, VrKeys},
    overlay_surface::OverlaySurface,
    power,
};
use eframe::egui::{
    self, Align, Button, Color32, FontData, FontDefinitions, FontFamily, Frame, Layout, Margin,
    RichText, ScrollArea, Stroke, Ui, ViewportBuilder,
};
use learned_typos::LearnedTypos;
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use vime::{
    brand,
    calibration::{self, Profile},
    i18n::{self, Lang, tr, tr_args},
    local_d1::{self, Download},
    misconversions, output,
    settings::{D1Device, Destination, Settings, TypoProvider, TypoSettings},
    steamvr::{Pointer, SteamVr, Surface},
};

const BACKGROUND: Color32 = Color32::from_rgb(20, 26, 35);
const PANEL: Color32 = Color32::from_rgb(29, 37, 49);
const EDGE: Color32 = Color32::from_rgb(53, 67, 83);
const INK: Color32 = Color32::from_rgb(241, 245, 251);
const SECONDARY: Color32 = Color32::from_rgb(176, 192, 209);
const ACCENT: Color32 = Color32::from_rgb(159, 211, 255);
const ERROR: Color32 = Color32::from_rgb(255, 177, 168);
const WIDE_LAYOUT: f32 = 640.;
const MAX_PAGE_WIDTH: f32 = 1080.;
const HIDDEN_PAINT_RETRY: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Eq, PartialEq)]
enum Welcome {
    Recommended,
    Chosen,
    Later,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Page {
    #[default]
    Keyboard,
    Settings,
    LearnedTypos,
}

pub fn run() -> eframe::Result {
    power::disable_throttling();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([980., 826.])
            .with_min_inner_size([480., 360.])
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
    /// Shown until the first settings are saved.
    welcome: bool,
    learned_typos: LearnedTypos,
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
    api_key_edits: HashMap<String, Option<String>>,
    api_key_before_url: Option<(TypoProvider, String)>,
    model_download: Option<Download>,
    model_error: Option<std::io::Error>,
    misconversion_download: Option<misconversions::Download>,
    misconversion_error: Option<std::io::Error>,
    calibration_job: Option<(String, CalibrationJob)>,
    calibration_error: Option<(String, std::io::Error)>,
    calibration_attempts: HashMap<String, Instant>,
    calibration_check: Instant,
    /// Held-key repeat for the desktop window and the VR dashboard.
    repeaters: [Repeater; 2],
    world_repeaters: [Repeater; 2],
}

const DESKTOP: usize = 0;
const DASHBOARD: usize = 1;

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
        let welcome = !Settings::saved(&directory);
        let mut draft = settings.clone();
        if welcome {
            draft.typo = TypoSettings::recommended();
        }

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
        let mut keyboard = Keyboard::new(creation.egui_ctx.clone());
        keyboard.configure_typo(&settings.typo, &directory);
        Ok(Self {
            directory,
            draft,
            settings,
            hotkeys,
            keyboard,
            surfaces,
            vr: None,
            auto_connect: true,
            page: Page::default(),
            welcome,
            learned_typos: LearnedTypos::default(),
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
            api_key_edits: HashMap::new(),
            api_key_before_url: None,
            model_download: None,
            misconversion_download: None,
            misconversion_error: None,
            model_error: None,
            calibration_job: None,
            calibration_error: None,
            calibration_attempts: HashMap::new(),
            calibration_check: Instant::now(),
            repeaters: Default::default(),
            world_repeaters: Default::default(),
        })
    }

    fn apply_settings(&mut self) -> Result<(), String> {
        self.draft.hotkey = self.draft.hotkey.trim().to_owned();
        let mut proposed = self.draft.clone();
        if self.key_before_url(&proposed.typo).is_some() {
            let scope = editable_api_key_scope(&proposed.typo).ok_or_else(|| {
                tr("System One の HTTP または HTTPS URL を入力してください").to_owned()
            })?;
            if !self.api_key_edits.contains_key(&scope) {
                self.apply_api_key_edit(&mut proposed.typo, &scope)
                    .map_err(|error| {
                        tr_args(
                            "API キーを保護できませんでした ({0})",
                            &[&error.to_string()],
                        )
                    })?;
            }
        }
        for scope in self.api_key_edits.keys() {
            self.apply_api_key_edit(&mut proposed.typo, scope)
                .map_err(|error| {
                    tr_args(
                        "API キーを保護できませんでした ({0})",
                        &[&error.to_string()],
                    )
                })?;
        }
        let prepared = match self.hotkeys.as_ref() {
            Some(hotkeys) => Some(hotkeys.prepare(&self.draft.hotkey)?),
            None if self.settings.hotkey != self.draft.hotkey => {
                return Err(tr("Windows ショートカットを設定できません").into());
            }
            None => None,
        };
        if let Err(error) = proposed.save(&self.directory) {
            if let (Some(hotkeys), Some(key)) = (&self.hotkeys, prepared) {
                hotkeys.cancel(key);
            }
            return Err(tr_args("設定を保存できません ({0})", &[&error.to_string()]));
        }
        if let (Some(hotkeys), Some(key)) = (&mut self.hotkeys, prepared) {
            hotkeys.activate(key);
        }
        self.draft = proposed;
        self.api_key_edits.clear();
        self.api_key_before_url = None;
        self.settings = self.draft.clone();
        self.calibration_check = Instant::now();
        self.keyboard
            .configure_typo(&self.settings.typo, &self.directory);
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
        self.world_repeaters = Default::default();
    }

    fn disconnect(&mut self) {
        self.vr = None;
        self.world_keys = None;
        self.pending_world_keys = None;
        self.last_world = None;
        self.pointers = [None; 2];
        self.last_pointers = [None; 2];
        self.dashboard.reset();
        self.world_repeaters = Default::default();
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
        if !matches!(action, Action::MoveCaret(_)) {
            keyboard::play_click();
        }
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
                self.keyboard.finish_typo_edit();
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
        let dashboard_rendered = surfaces.dashboard.can_present(now)
            && (self.dashboard.redraw
                || (self.dashboard.active && self.dashboard_revision != revision));
        if dashboard_rendered {
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

        let click = match command {
            Some(Command::Keyboard(action)) => Some(action),
            Some(Command::OpenBindings) => {
                self.settings_message = vr.open_bindings(false).err().unwrap_or_default();
                self.dashboard.redraw = true;
                None
            }
            Some(Command::SaveSettings) => {
                self.save_draft();
                None
            }
            None => None,
        };
        if dashboard_rendered {
            let held = self.dashboard.held_key().cloned();
            self.pointer_repeats(DASHBOARD, click, held);
        }
    }

    fn world_key_presses(&mut self, presses: [Option<[f32; 2]>; 2]) {
        let Some(keys) = &self.world_keys else {
            return;
        };
        let actions = presses.map(|point| point.and_then(|point| keys.action_at(point)));
        let mut entered = false;
        for (hand, action) in actions.into_iter().enumerate() {
            let Some(action) = action else { continue };
            if matches!(action, Action::Enter) && std::mem::replace(&mut entered, true) {
                continue;
            }
            let close = matches!(action, Action::Close);
            if action.repeats() {
                self.world_repeaters[hand].begin(&action, Instant::now());
            }
            self.keyboard_action(action);
            if close {
                break;
            }
        }
    }

    /// Applies the keys that repeat while a VR controller holds them.
    fn world_key_repeats(&mut self, buttons_down: [bool; 2]) {
        let now = Instant::now();
        for (hand, down) in buttons_down.into_iter().enumerate() {
            if let Some(action) = self.world_repeaters[hand].repeat_while(down, now) {
                self.keyboard_action(action);
            }
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
        ui.style_mut().spacing.interact_size.y = 30.;
        let width = ui.available_width();
        ui.horizontal(|ui| {
            brand::wordmark(ui, 26.);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                egui::ComboBox::from_id_salt("desktop_language")
                    .selected_text(self.draft.language.name())
                    .show_ui(ui, |ui| {
                        for option in Lang::ALL {
                            ui.selectable_value(&mut self.draft.language, option, option.name());
                        }
                    })
                    .response
                    .on_hover_text(tr("言語"));
            });
        });
        ui.label(
            RichText::new(tr("SteamVR向け IMEキーボード"))
                .size(15.)
                .color(SECONDARY),
        );
        if self.welcome {
            self.welcome_panel(ui);
            return;
        }
        ui.add_space(17.);
        self.steamvr_panel(ui);
        ui.add_space(14.);
        let previous = self.page;
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.page, Page::Keyboard, tr("キーボード"));
            ui.selectable_value(&mut self.page, Page::Settings, tr("設定"));
            ui.selectable_value(&mut self.page, Page::LearnedTypos, tr("学習済みの誤字"));
        });
        ui.add_space(14.);
        if self.page == Page::LearnedTypos {
            if previous != self.page {
                self.learned_typos.refresh_on_open();
            }
            let removed = self.learned_typos.show(
                ui,
                &self.directory,
                self.keyboard.learned_typos_revision(),
            );
            if !removed.is_empty()
                && let Err(error) = self.keyboard.forget_typos(removed, &self.directory)
            {
                self.learned_typos.delete_failed(error);
            }
            return;
        }
        if self.page == Page::Settings {
            let before = self.draft.clone();
            self.settings_panel(ui);
            self.dashboard.redraw |= self.draft != before;
            return;
        }
        let title = RichText::new(tr("キーボードプレビュー")).size(18.).strong();
        let destination = RichText::new(self.settings.destination.label())
            .size(13.)
            .color(SECONDARY);
        if width >= WIDE_LAYOUT {
            ui.horizontal(|ui| {
                ui.label(title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(destination);
                });
            });
        } else {
            ui.label(title);
            ui.label(destination);
        }
        ui.add_space(5.);
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

    fn start_model_download(&mut self, context: &egui::Context, device: D1Device) {
        let context = context.clone();
        self.model_error = None;
        match Download::start(self.directory.clone(), device, move || {
            context.request_repaint()
        }) {
            Ok(download) => self.model_download = Some(download),
            Err(error) => self.model_error = Some(error),
        }
    }

    fn start_misconversion_download(&mut self, context: &egui::Context) {
        let context = context.clone();
        self.misconversion_error = None;
        match misconversions::Download::start(self.directory.clone(), move || {
            context.request_repaint()
        }) {
            Ok(download) => self.misconversion_download = Some(download),
            Err(error) => self.misconversion_error = Some(error),
        }
    }

    fn welcome_panel(&mut self, ui: &mut Ui) {
        ui.add_space(17.);
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(tr("ViME へようこそ")).size(22.).strong());
            ui.add_space(5.);
            ui.label(tr(
                "はじめに誤字修正を設定します。あとから設定画面でいつでも変えられます。",
            ));
            ui.add_space(10.);
            let recommended = TypoSettings::recommended();
            let bytes = local_d1::required_bytes(&self.directory, recommended.d1_device)
                + if misconversions::installed(&self.directory) {
                    0
                } else {
                    misconversions::DATA_BYTES
                };
            if ui
                .add(primary(tr("おすすめ設定で始める")).min_size(egui::vec2(220., 34.)))
                .clicked()
            {
                self.finish_welcome(Welcome::Recommended, ui.ctx());
            }
            ui.label(
                RichText::new(tr_args(
                    "d1-2B（ローカル・CUDA）と日本語の誤変換データを取得して始めます（約 {0} GB）",
                    &[&format!("{:.1}", bytes as f64 / 1_000_000_000.)],
                ))
                .small()
                .color(SECONDARY),
            );
            ui.add_space(10.);
            ui.separator();
            ui.add_space(10.);
            ui.label(RichText::new(tr("または、自分で選ぶ")).color(SECONDARY));
            ui.add_space(4.);
            self.typo_settings(ui);
            ui.add_space(10.);
            ui.label(
                RichText::new(tr(
                    "ダウンロードは完了を待たずに先へ進めます。進み具合は設定画面に表示されます",
                ))
                .small()
                .color(SECONDARY),
            );
            ui.add_space(5.);
            if self.settings_error {
                ui.label(RichText::new(&self.settings_message).small().color(ERROR));
                // Another app may hold the shortcut, and Settings is out of reach here.
                ui.label(tr("Windows 開閉ショートカット"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.hotkey)
                        .hint_text("Ctrl+Shift+F8")
                        .desired_width(200.),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(tr("この内容で始める")).clicked() {
                    self.finish_welcome(Welcome::Chosen, ui.ctx());
                }
                if ui.button(tr("あとで設定する")).clicked() {
                    self.finish_welcome(Welcome::Later, ui.ctx());
                }
            });
        });
    }

    /// Saves what the welcome page chose. The recommended setup also starts
    /// every download it needs, so nothing is left to do in Settings.
    fn finish_welcome(&mut self, choice: Welcome, context: &egui::Context) {
        match choice {
            Welcome::Recommended => self.draft.typo = TypoSettings::recommended(),
            Welcome::Later => self.draft.typo = TypoSettings::default(),
            Welcome::Chosen => {}
        }
        match self.apply_settings() {
            Ok(()) => {
                self.welcome = false;
                self.settings_error = false;
                self.settings_message.clear();
                if choice == Welcome::Recommended {
                    let device = self.settings.typo.d1_device;
                    if !local_d1::installed(&self.directory, device) {
                        self.start_model_download(context, device);
                    }
                    if !misconversions::installed(&self.directory) {
                        self.start_misconversion_download(context);
                    }
                }
            }
            Err(error) => {
                self.settings_error = true;
                self.settings_message = error;
            }
        }
        self.dashboard.redraw = true;
    }
    fn steamvr_panel(&mut self, ui: &mut Ui) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if ui.available_width() >= WIDE_LAYOUT {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| self.steamvr_status(ui));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.steamvr_actions(ui);
                    });
                });
            } else {
                self.steamvr_status(ui);
                ui.add_space(5.);
                self.steamvr_actions(ui);
            }
            if !self.steamvr_message.is_empty() {
                ui.label(RichText::new(&self.steamvr_message).color(ERROR));
            }
            for error in self.draw_errors.iter().flatten() {
                ui.label(RichText::new(error).color(ERROR));
            }
        });
    }

    fn steamvr_status(&self, ui: &mut Ui) {
        ui.label(RichText::new("SteamVR").size(17.).strong());
        let status = match &self.vr {
            Some(vr) if vr.visible() => tr("接続済み · キーボード表示中"),
            Some(_) => tr("接続済み"),
            None => tr("未接続"),
        };
        ui.label(RichText::new(status).color(if self.vr.is_some() { ACCENT } else { SECONDARY }));
    }

    fn steamvr_actions(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(vr) = &self.vr {
                let toggle = if vr.visible() {
                    tr("キーボードを閉じる")
                } else {
                    tr("キーボードを開く")
                };
                let toggle_clicked = ui.button(toggle).clicked();
                if ui.button(tr("VR の割り当てを編集")).clicked() {
                    self.steamvr_message = vr.open_bindings(true).err().unwrap_or_default();
                }
                if ui.button(tr("切断")).clicked() {
                    self.disconnect();
                    self.steamvr_message.clear();
                } else if toggle_clicked {
                    self.toggle();
                }
            } else if ui.add(primary(tr("SteamVR に接続"))).clicked() {
                self.connect();
            }
        });
    }

    fn settings_panel(&mut self, ui: &mut Ui) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(tr("設定")).size(18.).strong());
            ui.add_space(5.);
            if ui.available_width() >= WIDE_LAYOUT {
                ui.columns_const(|[destination, controls]| {
                    self.destination_settings(destination);
                    self.control_settings(controls);
                });
            } else {
                self.destination_settings(ui);
                ui.add_space(10.);
                ui.separator();
                ui.add_space(10.);
                self.control_settings(ui);
            }

            ui.add_space(10.);
            ui.separator();
            ui.add_space(10.);
            self.typo_settings(ui);
            ui.add_space(10.);
            if !self.settings_message.is_empty() {
                ui.label(RichText::new(&self.settings_message).small().color(
                    if self.settings_error {
                        ERROR
                    } else {
                        SECONDARY
                    },
                ));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.draft != self.settings
                            || !self.api_key_edits.is_empty()
                            || self.api_key_before_url.is_some(),
                        primary(tr("設定を保存")),
                    )
                    .clicked()
                {
                    self.save_draft();
                }
            });
        });
    }

    fn destination_settings(&mut self, ui: &mut Ui) {
        ui.label(RichText::new(tr("送信先")).color(SECONDARY));
        dashboard::destination_options(ui, &mut self.draft.destination);
        let hint = if self.draft.destination == Destination::Osc {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("UDP ポート"));
                ui.add(egui::DragValue::new(&mut self.draft.osc_port).range(1..=65535));
                ui.checkbox(&mut self.draft.osc_notify, tr("チャット通知音"));
            });
            tr("VRChat側でOSCを有効にしてください")
        } else {
            tr("VRアプリの入力欄を開き、Windowsで前面にします")
        };
        ui.label(RichText::new(hint).size(12.).color(SECONDARY));
    }

    fn control_settings(&mut self, ui: &mut Ui) {
        ui.label(RichText::new(tr("Windows 開閉ショートカット")).color(SECONDARY));
        ui.add(
            egui::TextEdit::singleline(&mut self.draft.hotkey)
                .hint_text("Ctrl+Shift+F8")
                .desired_width(200.),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("キーボード幅"));
            ui.add(egui::Slider::new(&mut self.draft.overlay_width, 0.65..=1.65).suffix(" m"));
        });
    }

    fn typo_settings(&mut self, ui: &mut Ui) {
        let typo = &mut self.draft.typo;
        ui.label(RichText::new(tr("誤字修正")).color(SECONDARY));
        egui::ComboBox::from_id_salt("typo_provider")
            .selected_text(typo.provider.label())
            .show_ui(ui, |ui| {
                for provider in TypoProvider::ALL {
                    ui.selectable_value(&mut typo.provider, provider, provider.label());
                }
            });
        if typo.provider == TypoProvider::Off {
            return;
        }
        if typo.provider == TypoProvider::D1Local {
            self.model_panel(ui);
        } else if matches!(typo.provider, TypoProvider::D1 | TypoProvider::Custom) {
            ui.label("System One URL");
            ui.add(system_one_field(&mut typo.url).hint_text("http://127.0.0.1:8080/v1/systemone"));
            if typo.provider == TypoProvider::Custom {
                ui.label(tr("モデル名（任意）"));
                ui.add(system_one_field(&mut typo.model));
            }
        } else {
            ui.label(typo.endpoint());
        }
        if self.draft.typo.provider != TypoProvider::D1Local {
            self.api_key_field(ui);
            ui.label(
                RichText::new(tr("判定時は対象の語と前後の一部が設定先へ送信されます")).small(),
            );
        }
        // The load test needs the model, so it waits until after the welcome page.
        if !self.welcome {
            self.calibration_panel(ui);
        }
        self.misconversion_panel(ui);
        if self.draft.typo == self.settings.typo
            && let Some(error) = self.keyboard.typo_error()
        {
            ui.label(
                RichText::new(tr_args(
                    "誤字判定に失敗しました ({0})",
                    &[&error.to_string()],
                ))
                .color(ERROR),
            );
        }
    }

    fn api_key_field(&mut self, ui: &mut Ui) {
        let scope = editable_api_key_scope(&self.draft.typo);
        let provider = self.draft.typo.provider;
        let saved = scope
            .as_ref()
            .is_some_and(|scope| self.draft.typo.protected_api_keys.contains_key(scope));
        let edit = scope
            .as_ref()
            .and_then(|scope| self.api_key_edits.get(scope));
        let mut text = match edit {
            Some(pending) => pending.clone().unwrap_or_default(),
            None => self
                .key_before_url(&self.draft.typo)
                .unwrap_or_default()
                .to_owned(),
        };
        ui.label(tr("API キー"));
        let removed = matches!(edit, Some(None));
        let placeholder = if saved && !removed {
            tr("保存済み（変更する場合は入力）")
        } else {
            tr("API キーを入力")
        };
        if ui
            .add(
                api_key_input(&mut text, placeholder)
                    .id_salt((self.draft.typo.key_variable(), scope.as_deref())),
            )
            .changed()
        {
            if let Some(scope) = &scope {
                if self.key_before_url(&self.draft.typo).is_some() {
                    self.api_key_before_url = None;
                }
                if text.trim().is_empty() {
                    self.api_key_edits.remove(scope);
                } else {
                    self.api_key_edits.insert(scope.clone(), Some(text));
                }
            } else {
                self.api_key_before_url = (!text.trim().is_empty()).then_some((provider, text));
            }
        }
        if saved
            && !removed
            && let Some(scope) = scope
            && ui.small_button(tr("保存済みのキーを削除")).clicked()
        {
            self.api_key_edits.insert(scope, None);
        }
    }

    fn key_before_url(&self, typo: &TypoSettings) -> Option<&str> {
        self.api_key_before_url
            .as_ref()
            .filter(|(provider, _)| *provider == typo.provider)
            .map(|(_, key)| key.as_str())
    }

    fn apply_api_key_edit(&self, typo: &mut TypoSettings, scope: &str) -> std::io::Result<()> {
        match self.api_key_edits.get(scope) {
            Some(Some(key)) => {
                let protected = vime::credentials::protect(key.trim(), scope)?;
                typo.protected_api_keys.insert(scope.to_owned(), protected);
            }
            Some(None) => {
                typo.protected_api_keys.remove(scope);
            }
            None if editable_api_key_scope(typo).as_deref() == Some(scope) => {
                if let Some(key) = self.key_before_url(typo) {
                    let protected = vime::credentials::protect(key.trim(), scope)?;
                    typo.protected_api_keys.insert(scope.to_owned(), protected);
                }
            }
            None => {}
        }
        Ok(())
    }

    fn model_panel(&mut self, ui: &mut Ui) {
        let selected = &mut self.draft.typo.d1_device;
        let before = *selected;
        ui.add_enabled_ui(self.model_download.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("推論デバイス"));
                ui.radio_value(selected, D1Device::Cpu, "CPU");
                ui.radio_value(selected, D1Device::Vulkan, tr("GPU（Vulkan）"));
                ui.radio_value(selected, D1Device::Cuda, tr("GPU（CUDA）"));
            });
        });
        let device = *selected;
        if device != before {
            self.model_error = None;
        }
        ui.label(RichText::new(tr("入力の判定はこの PC 内で行います")).small());
        if let Some(download) = &self.model_download {
            let received = download.received();
            let total = download.total();
            ui.add(
                egui::ProgressBar::new(received as f32 / total.max(1) as f32).text(format!(
                    "{:.0} / {:.0} MB",
                    received as f64 / 1_000_000.,
                    total as f64 / 1_000_000.
                )),
            );
            if ui
                .add_enabled(
                    !download.cancelling(),
                    Button::new(tr("ダウンロードを中止")),
                )
                .clicked()
            {
                download.cancel();
            }
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        } else if local_d1::installed(&self.directory, device) {
            ui.label(tr("d1-2B Q4_K_M をダウンロード済み"));
        } else {
            let bytes = local_d1::required_bytes(&self.directory, device) as f64;
            let label = if local_d1::model_available(&self.directory) {
                tr_args(
                    "推論エンジンをダウンロード（約 {0} MB）",
                    &[&format!("{:.0}", bytes / 1_000_000.)],
                )
            } else {
                tr_args(
                    "モデルをダウンロード（約 {0} GB）",
                    &[&format!("{:.1}", bytes / 1_000_000_000.)],
                )
            };
            if ui.button(label).clicked() {
                self.start_model_download(ui.ctx(), device);
            }
        }
        ui.hyperlink_to(tr("モデルのライセンス"), local_d1::MODEL_LICENSE);
        if let Some(error) = &self.model_error {
            ui.label(
                RichText::new(tr_args(
                    "モデルを取得できませんでした ({0})",
                    &[&error.to_string()],
                ))
                .color(ERROR),
            );
        }
    }

    /// The optional Japanese misconversion table. The dataset is fetched from
    /// Kyoto University only when the user asks and is counted on this PC.
    fn misconversion_panel(&mut self, ui: &mut Ui) {
        ui.add_space(8.);
        ui.label(RichText::new(tr("日本語の誤変換データ（任意）")).color(SECONDARY));
        if let Some(download) = &self.misconversion_download {
            if download.summarizing() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("データを集計しています"));
                });
            } else {
                let received = download.received();
                let total = misconversions::DATA_BYTES;
                ui.add(
                    egui::ProgressBar::new(received as f32 / total as f32).text(format!(
                        "{:.0} / {:.0} MB",
                        received as f64 / 1_000_000.,
                        total as f64 / 1_000_000.
                    )),
                );
            }
            if ui
                .add_enabled(
                    !download.cancelling(),
                    Button::new(tr("ダウンロードを中止")),
                )
                .clicked()
            {
                download.cancel();
            }
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        } else if misconversions::installed(&self.directory) {
            ui.label(tr("誤変換データを取得済み"));
            if ui.small_button(tr("取得したデータを削除")).clicked() {
                match misconversions::remove(&self.directory) {
                    Ok(()) => {
                        self.misconversion_error = None;
                        self.keyboard.reload_misconversions();
                    }
                    Err(error) => self.misconversion_error = Some(error),
                }
            }
        } else {
            ui.label(
                RichText::new(tr(
                    "直されやすい漢字の語を、京都大学の公開データから集計して使います",
                ))
                .small(),
            );
            let megabytes = misconversions::DATA_BYTES as f64 / 1_000_000.;
            if ui
                .button(tr_args(
                    "ダウンロード（約 {0} MB）",
                    &[&format!("{megabytes:.0}")],
                ))
                .clicked()
            {
                self.start_misconversion_download(ui.ctx());
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(
                tr("出典: 日本語Wikipedia入力誤りデータセット（京都大学）"),
                misconversions::SOURCE_PAGE,
            );
            ui.hyperlink_to(
                tr("データのライセンス（CC BY-SA 3.0）"),
                misconversions::LICENSE,
            );
        });
        if let Some(error) = &self.misconversion_error {
            ui.label(
                RichText::new(tr_args(
                    "誤変換データを取得できませんでした ({0})",
                    &[&error.to_string()],
                ))
                .color(ERROR),
            );
        }
    }

    fn calibration_panel(&mut self, ui: &mut Ui) {
        let Some(key) = self.draft.typo.calibration_key().map(str::to_owned) else {
            return;
        };
        let profile = self
            .draft
            .typo
            .calibration
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let mut limits = profile.limits();
        let mut overrides = profile.overrides;
        let previous = overrides.clone();
        ui.add_space(8.);
        ui.label(RichText::new(tr("キャリブレーション")).color(SECONDARY));
        if let Some(measurement) = profile.measurement {
            ui.label(tr_args(
                "逐次 p95 {0} ms · 選択時 p95 {1} ms · 実測 {2} req/s",
                &[
                    &measurement.sequential_p95_ms.to_string(),
                    &measurement.p95_ms.to_string(),
                    &format!("{:.2}", measurement.throughput),
                ],
            ));
        }
        let stale = self
            .draft
            .typo
            .calibration
            .get(&key)
            .is_none_or(|p| p.stale(calibration::now_seconds()));
        let measuring = self.calibration_job.is_some();
        if ui
            .add_enabled(
                !measuring && stale && self.can_calibrate(&self.draft.typo),
                Button::new(tr("負荷を測定")),
            )
            .clicked()
        {
            let mut typo = self.draft.typo.clone();
            let result = typo
                .api_key_scope()
                .map_or(Ok(()), |scope| self.apply_api_key_edit(&mut typo, &scope));
            if let Err(error) = result {
                self.calibration_error = Some((key.clone(), error));
            } else {
                self.start_calibration(typo);
            }
        }
        if self
            .calibration_job
            .as_ref()
            .is_some_and(|(target, _)| target == &key)
        {
            ui.label(tr("計測中（最大 60 秒）"));
        } else if let Some((target, error)) = &self.calibration_error
            && target == &key
        {
            ui.label(
                RichText::new(tr_args("計測に失敗しました ({0})", &[&error.to_string()]))
                    .color(ERROR),
            );
        }
        let max_rate = limits.requests_per_second.max(100.0);
        if ui
            .add(
                egui::Slider::new(&mut limits.concurrent, 1..=calibration::MAX_CONCURRENCY)
                    .text(tr("最大同時数")),
            )
            .changed()
        {
            overrides.concurrent = Some(limits.concurrent);
        }
        if ui
            .add(
                egui::Slider::new(&mut limits.requests_per_second, 0.01..=max_rate)
                    .logarithmic(true)
                    .max_decimals(2)
                    .suffix(" req/s")
                    .text(tr("最大送信レート")),
            )
            .changed()
        {
            overrides.requests_per_second = Some(limits.requests_per_second);
        }
        if ui
            .add(
                egui::Slider::new(
                    &mut limits.debounce_ms,
                    calibration::MIN_DEBOUNCE_MS..=calibration::MAX_DEBOUNCE_MS,
                )
                .suffix(" ms")
                .text(tr("入力待ち時間")),
            )
            .changed()
        {
            overrides.debounce_ms = Some(limits.debounce_ms);
        }
        if previous != Default::default() && ui.button(tr("自動設定に戻す")).clicked() {
            overrides = Default::default();
        }
        if overrides != previous {
            self.draft
                .typo
                .calibration
                .entry(key)
                .or_default()
                .overrides = overrides;
        }
    }

    fn can_calibrate(&self, typo: &TypoSettings) -> bool {
        typo.calibration_key().is_some()
            && if typo.provider == TypoProvider::D1Local {
                local_d1::installed(&self.directory, typo.d1_device)
            } else {
                vime::system_one::valid_url(typo.endpoint())
            }
    }

    fn start_calibration(&mut self, typo: TypoSettings) {
        let Some(key) = typo.calibration_key().map(str::to_owned) else {
            return;
        };
        self.calibration_attempts
            .insert(key.clone(), Instant::now());
        self.calibration_error = None;
        match self.keyboard.calibrate(typo, &self.directory) {
            Ok(job) => self.calibration_job = Some((key, job)),
            Err(error) => self.calibration_error = Some((key, error)),
        }
    }

    fn poll_calibration(&mut self, context: &egui::Context) {
        if let Some(result) = self
            .calibration_job
            .as_ref()
            .and_then(|(_, job)| job.result())
        {
            let (key, _) = self.calibration_job.take().unwrap();
            let result = result.and_then(|measurement| {
                let mut settings = self.settings.clone();
                settings
                    .typo
                    .calibration
                    .entry(key.clone())
                    .or_default()
                    .measurement = Some(measurement);
                settings.save(&self.directory)?;
                self.settings = settings;
                self.draft
                    .typo
                    .calibration
                    .entry(key.clone())
                    .or_default()
                    .measurement = Some(measurement);
                self.keyboard
                    .configure_typo(&self.settings.typo, &self.directory);
                Ok(())
            });
            if let Err(error) = result {
                self.calibration_error = Some((key, error));
            }
            self.keyboard.warm_local_typo();
        }
        let now = Instant::now();
        if now >= self.calibration_check {
            self.calibration_check = now + Duration::from_secs(60);
            let typo = &self.settings.typo;
            if self.calibration_job.is_none()
                && self.can_calibrate(typo)
                && let Some(key) = typo.calibration_key()
                && typo
                    .calibration
                    .get(key)
                    .is_none_or(|profile: &Profile| profile.stale(calibration::now_seconds()))
                && self
                    .calibration_attempts
                    .get(key)
                    .is_none_or(|attempt| now.duration_since(*attempt) >= Duration::from_secs(3600))
            {
                self.start_calibration(typo.clone());
            }
        }
        context.request_repaint_after(self.calibration_check.saturating_duration_since(now));
    }
}

fn system_one_field(value: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(value)
        .align(egui::Align2::LEFT_CENTER)
        .margin(Margin {
            left: 8,
            right: 8,
            top: 6,
            bottom: 4,
        })
        .min_size(egui::vec2(0., 30.))
        .desired_width(420.)
}

fn api_key_input<'a>(value: &'a mut String, placeholder: &str) -> egui::TextEdit<'a> {
    system_one_field(value)
        .password(true)
        .hint_text(placeholder)
}

fn editable_api_key_scope(typo: &TypoSettings) -> Option<String> {
    typo.api_key_scope()
        .filter(|_| vime::system_one::valid_url(typo.endpoint()))
}

impl eframe::App for App {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(result) = self.model_download.as_ref().and_then(Download::result) {
            self.model_download = None;
            self.model_error = result
                .err()
                .filter(|error| error.kind() != std::io::ErrorKind::Interrupted);
            if local_d1::installed(&self.directory, self.settings.typo.d1_device) {
                self.calibration_check = Instant::now();
                self.keyboard.warm_local_typo();
            }
        }
        if let Some(result) = self
            .misconversion_download
            .as_ref()
            .and_then(misconversions::Download::result)
        {
            self.misconversion_download = None;
            self.misconversion_error = result
                .err()
                .filter(|error| error.kind() != std::io::ErrorKind::Interrupted);
            self.keyboard.reload_misconversions();
        }
        self.poll_calibration(context);
        if i18n::language() != self.draft.language {
            // The language picker takes effect at once; saving only remembers it.
            i18n::set_language(self.draft.language);
            self.keyboard.status("");
            self.dashboard.redraw = true;
        }
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
                    self.world_key_repeats(frame.keyboard_down);
                }
            }
        }
        if self.dashboard.held_key().is_some()
            && let Some(action) = self.repeaters[DASHBOARD].repeat_while(true, Instant::now())
        {
            self.keyboard_action(action);
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
        let margin = if ui.available_width() >= WIDE_LAYOUT {
            Margin::symmetric(27, 18)
        } else {
            Margin::symmetric(14, 12)
        };
        Frame::new()
            .fill(BACKGROUND)
            .inner_margin(margin)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let width = ui.available_width().min(MAX_PAGE_WIDTH);
                        ui.with_layout(Layout::top_down(Align::Center), |ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 0.),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(width);
                                    self.page(ui);
                                },
                            );
                        });
                    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_url_and_masked_key_fields_accept_paste() {
        for (password, pasted) in [
            (false, "http://127.0.0.1:8080/v1/systemone"),
            (true, "synthetic-test-key"),
        ] {
            let context = egui::Context::default();
            let mut value = String::new();
            let mut render = |events, focus| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let field = if password {
                            api_key_input(&mut value, "API key")
                        } else {
                            system_one_field(&mut value)
                        };
                        let response = ui.add(field.id_salt("paste_test"));
                        if focus {
                            response.request_focus();
                        }
                    },
                );
                output.textures_delta.clear();
            };
            render(Vec::new(), true);
            render(vec![egui::Event::Paste(pasted.into())], false);
            assert_eq!(value, pasted);
        }
    }
}
