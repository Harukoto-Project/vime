use crate::{
    keyboard::{self, Action, Keyboard},
    overlay_surface::{cancel_pointer, pointer_button},
};
use eframe::egui::{self, Color32, Event, Frame, Margin, Pos2, RichText, Stroke, Ui};
use vime::{
    settings::{Destination, Settings},
    steamvr::DashboardEvent,
};

pub const SIZE: [u32; 2] = [1040, 640];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Keyboard,
    Settings,
}

impl Page {
    pub fn tabs(page: &mut Self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(page, Self::Keyboard, "キーボード");
            ui.selectable_value(page, Self::Settings, "設定");
        });
    }
}

pub enum Command {
    Keyboard(Action),
    OpenBindings,
    SaveSettings,
}

#[derive(Default)]
pub struct Dashboard {
    page: Page,
    mouse: Option<Pos2>,
    down: bool,
    events: Vec<Event>,
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

    fn leave(&mut self) {
        cancel_pointer(&mut self.events, self.down);
        self.mouse = None;
        self.down = false;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        keyboard: &Keyboard,
        applied: &Settings,
        draft: &mut Settings,
        message: &str,
    ) -> Option<Command> {
        let mut command = None;
        let old_page = self.page;
        Frame::new()
            .fill(Color32::from_rgb(27, 37, 51))
            .inner_margin(Margin::symmetric(32, 21))
            .show(ui, |ui| {
                ui.set_min_size(egui::vec2(SIZE[0] as f32 - 64., SIZE[1] as f32 - 42.));
                ui.label(RichText::new("ViME").size(33.).strong());
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
                        command = keyboard::show(ui, keyboard, width, applied.destination)
                            .map(Command::Keyboard);
                    }
                    Page::Settings => {
                        command = settings(ui, applied, draft, message);
                    }
                }
            });
        self.redraw |= old_page != self.page;
        command
    }
}

fn settings(
    ui: &mut Ui,
    applied: &Settings,
    draft: &mut Settings,
    message: &str,
) -> Option<Command> {
    let mut command = None;
    ui.columns(2, |columns| {
        let [output, controls] = columns else {
            unreachable!("two dashboard settings columns")
        };
        output.heading("送信先");
        output.add_space(7.);
        destination_options(output, &mut draft.destination);
        output.add_space(10.);
        if draft.destination == Destination::Osc {
            output.label("VRChat の OSC を有効にしてください");
            output.add_space(8.);
            output.horizontal(|ui| {
                ui.label("UDP ポート");
                ui.label(RichText::new(draft.osc_port.to_string()).strong());
            });
            output.horizontal(|ui| {
                for step in [-100, -1, 1, 100] {
                    if ui.button(format!("{step:+}")).clicked() {
                        draft.osc_port = (i32::from(draft.osc_port) + step).clamp(1, 65535) as u16;
                    }
                }
            });
            output.checkbox(&mut draft.osc_notify, "チャット通知音");
        } else {
            output.label("VRアプリの入力欄を開き、Windows で前面にします");
        }

        controls.heading("VR キーボード");
        controls.add_space(7.);
        controls.label("右スティック押し込みで開閉");
        controls.label("トリガーでキーを選択");
        controls.add_space(7.);
        if controls.button("コントローラーの割り当てを編集").clicked() {
            command = Some(Command::OpenBindings);
        }
        controls.add_space(10.);
        controls.label("キーボード幅");
        controls.add(egui::Slider::new(&mut draft.overlay_width, 0.65..=1.65).suffix(" m"));
        controls.label(format!("Windows の開閉キー  {}", applied.hotkey));
    });
    ui.add_space(24.);
    ui.separator();
    ui.add_space(10.);
    ui.horizontal(|ui| {
        let save = egui::Button::new(
            RichText::new("設定を保存")
                .color(Color32::from_rgb(17, 36, 53))
                .strong(),
        )
        .fill(Color32::from_rgb(165, 216, 255))
        .stroke(Stroke::NONE);
        if ui.add_enabled(draft != applied, save).clicked() {
            command = Some(Command::SaveSettings);
        }
        if !message.is_empty() {
            ui.label(message);
        }
    });
    command
}

pub fn destination_options(ui: &mut Ui, destination: &mut Destination) {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(destination, Destination::Osc, Destination::Osc.label());
        ui.selectable_value(
            destination,
            Destination::SteamVrInput,
            Destination::SteamVrInput.label(),
        );
    });
}
