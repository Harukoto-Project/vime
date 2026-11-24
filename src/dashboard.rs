use crate::{
    keyboard::{self, Action, Keyboard, palette::Palette},
    overlay_surface::{cancel_pointer, pointer_button},
    widgets,
};
use eframe::egui::{
    self, Button, Context, Event, Frame, Margin, Pos2, Response, RichText, Stroke, Ui, WidgetText,
};
use vime::{
    brand,
    i18n::{Lang, tr, tr_args},
    settings::{Destination, KeyboardTheme, Settings},
    steamvr::DashboardEvent,
    update::{self, Update},
};

pub const SIZE: [u32; 2] = [1040, 640];
const MAX_PENDING_INPUTS: usize = 128;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Keyboard,
    Settings,
}

impl Page {
    pub fn tabs(page: &mut Self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            choice(ui, page, Self::Keyboard, tr("キーボード"));
            choice(ui, page, Self::Settings, tr("設定"));
        });
    }
}

pub enum Command {
    Keyboard(Action),
    OpenBindings,
    SaveSettings,
    Update(update::Action),
}

#[derive(Default)]
pub struct Dashboard {
    page: Page,
    mouse: Option<Pos2>,
    down: bool,
    events: Vec<Event>,
    /// The repeating key the pointer is holding down.
    held: Option<Action>,
    pub active: bool,
    pub redraw: bool,
}

impl Dashboard {
    pub fn reset(&mut self) {
        self.feed(false, []);
        self.redraw = true;
    }

    pub fn feed(&mut self, active: bool, events: impl IntoIterator<Item = DashboardEvent>) {
        if self.active != active {
            self.active = active;
            self.redraw = true;
            if !active {
                self.leave();
            }
        }
        for event in events {
            if self.events.len() >= MAX_PENDING_INPUTS {
                self.events.clear();
                self.leave();
            }
            match event {
                DashboardEvent::Move([x, y]) if x.is_finite() && y.is_finite() => {
                    let position = Pos2::new(x, y);
                    self.mouse = Some(position);
                    if let Some(last @ Event::PointerMoved(_)) = self.events.last_mut() {
                        *last = Event::PointerMoved(position);
                    } else {
                        self.events.push(Event::PointerMoved(position));
                    }
                }
                DashboardEvent::Button {
                    position: [x, y],
                    down,
                } if down != self.down => {
                    if self.mouse.is_none() && x.is_finite() && y.is_finite() {
                        let position = Pos2::new(x, y);
                        self.mouse = Some(position);
                        self.events.push(Event::PointerMoved(position));
                    }
                    if let Some(position) = self.mouse {
                        self.down = down;
                        self.events.push(pointer_button(position, down));
                    }
                }
                DashboardEvent::Leave => self.leave(),
                _ => {}
            }
        }
        self.redraw |= !self.events.is_empty();
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn held_key(&self) -> Option<&Action> {
        self.held.as_ref().filter(|_| self.active && self.down)
    }

    fn leave(&mut self) {
        cancel_pointer(&mut self.events, self.down);
        self.held = None;
        self.mouse = None;
        self.down = false;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        keyboard: &Keyboard,
        applied: &Settings,
        draft: &mut Settings,
        (message, update, osc_port): (&str, &Update, Option<u16>),
        caret_visible: bool,
    ) -> Option<Command> {
        let mut command = None;
        let mut held = None;
        let old_page = self.page;
        Frame::new()
            .fill(Palette::of(ui.ctx()).panel)
            .inner_margin(Margin::symmetric(32, 21))
            .show(ui, |ui| {
                ui.set_min_size(egui::vec2(SIZE[0] as f32 - 64., SIZE[1] as f32 - 42.));
                brand::wordmark(ui, 25.);
                ui.add_space(4.);
                let large = ui.style_mut();
                large.spacing.interact_size.y = 38.;
                for font in large.text_styles.values_mut() {
                    font.size = font.size.max(19.);
                }
                Page::tabs(&mut self.page, ui);
                ui.add_space(12.);
                match self.page {
                    Page::Keyboard => {
                        let width = ui.available_width().min(SIZE[0] as f32 - 64.);
                        let interaction = keyboard::show(
                            ui,
                            keyboard,
                            width,
                            applied.destination,
                            applied.keyboard_theme,
                            caret_visible,
                        );
                        held = interaction.held;
                        command = interaction.action.map(Command::Keyboard);
                    }
                    Page::Settings => {
                        command = settings(ui, applied, draft, message, update, osc_port);
                    }
                }
            });
        self.held = held;
        self.redraw |= old_page != self.page;
        command
    }
}

fn settings(
    ui: &mut Ui,
    applied: &Settings,
    draft: &mut Settings,
    message: &str,
    update: &Update,
    osc_port: Option<u16>,
) -> Option<Command> {
    let mut command = None;
    ui.columns(2, |columns| {
        let [output, controls] = columns else {
            unreachable!("two dashboard settings columns")
        };
        output.heading(tr("送信先"));
        output.add_space(7.);
        destination_options(output, &mut draft.destination);
        output.add_space(10.);
        if draft.destination == Destination::Osc {
            output.label(tr("VRChat の OSC を有効にしてください"));
            output.add_space(8.);
            osc_query_option(output, draft, osc_port);
            output.horizontal(|ui| {
                ui.label(osc_port_label(draft));
                ui.label(RichText::new(draft.osc_port.to_string()).strong());
            });
            output.horizontal(|ui| {
                for step in [-100, -1, 1, 100] {
                    if ui.button(format!("{step:+}")).clicked() {
                        draft.osc_port = (i32::from(draft.osc_port) + step).clamp(1, 65535) as u16;
                    }
                }
            });
            widgets::checkbox(output, &mut draft.osc_notify, tr("チャット通知音"));
        } else {
            output.label(tr("VRアプリの入力欄を開き、Windows で前面にします"));
        }

        controls.heading(tr("VR キーボード"));
        controls.add_space(7.);
        controls.label(tr("右スティック押し込みで開閉"));
        controls.label(tr("トリガーでキーを選択"));
        controls.add_space(7.);
        if controls
            .button(tr("コントローラーの割り当てを編集"))
            .clicked()
        {
            command = Some(Command::OpenBindings);
        }
        controls.add_space(10.);
        controls.label(tr("キーボード幅"));
        controls.add(widgets::Slider::new(&mut draft.overlay_width, 0.65..=1.65).suffix(" m"));
        controls.label(tr_args("Windows の開閉キー  {0}", &[&applied.hotkey]));
        keyboard_theme_options(controls, &mut draft.keyboard_theme);
        controls.add_space(10.);
        controls.label(tr("言語"));
        language_options(controls, &mut draft.language);
    });
    ui.add_space(24.);
    ui.separator();
    ui.add_space(10.);
    ui.horizontal(|ui| {
        let save = primary(ui.ctx(), tr("設定を保存"));
        if ui.add_enabled(draft != applied, save).clicked() {
            command = Some(Command::SaveSettings);
        }
        if !message.is_empty() {
            ui.label(message);
        }
    });
    ui.add_space(12.);
    update.show(ui).map(Command::Update).or(command)
}

pub fn destination_options(ui: &mut Ui, destination: &mut Destination) {
    ui.horizontal_wrapped(|ui| {
        choice(ui, destination, Destination::Osc, Destination::Osc.label());
        choice(
            ui,
            destination,
            Destination::SteamVrInput,
            Destination::SteamVrInput.label(),
        );
    });
}

pub fn keyboard_theme_options(ui: &mut Ui, theme: &mut KeyboardTheme) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("キーボードのテーマ"));
        for option in KeyboardTheme::ALL {
            choice(ui, theme, option, option.label());
        }
    });
}

pub fn osc_query_option(ui: &mut Ui, draft: &mut Settings, port: Option<u16>) {
    ui.horizontal_wrapped(|ui| {
        widgets::checkbox(ui, &mut draft.osc_query, tr("OSCQuery で自動検出"));
        if draft.osc_query
            && let Some(port) = port
        {
            ui.label(tr_args("検出済み · UDP {0}", &[&port.to_string()]));
        }
    });
}

pub fn osc_port_label(draft: &Settings) -> &'static str {
    if draft.osc_query {
        tr("未検出時の UDP ポート")
    } else {
        tr("UDP ポート")
    }
}

pub fn language_options(ui: &mut Ui, language: &mut Lang) {
    ui.horizontal_wrapped(|ui| {
        for option in Lang::ALL {
            choice(ui, language, option, option.name());
        }
    });
}

/// A toggle button for picking one of several values; the chosen one is tinted
/// and outlined like the selected keys of the keyboard.
pub fn choice<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    text: impl Into<WidgetText>,
) -> Response {
    let selected = *current == value;
    let mut button = Button::selectable(selected, text.into());
    if selected {
        button = button.stroke(Stroke::new(1., Palette::of(ui.ctx()).selected_border));
    }
    let mut response = ui.add(button);
    if response.clicked() && !selected {
        *current = value;
        response.mark_changed();
    }
    response
}

/// The accent-colored button for the main action of a page.
pub fn primary<'a>(context: &Context, label: &str) -> Button<'a> {
    let palette = Palette::of(context);
    Button::new(
        RichText::new(label.to_owned())
            .color(palette.on_accent)
            .strong(),
    )
    .fill(palette.accent)
    .stroke(Stroke::NONE)
}
