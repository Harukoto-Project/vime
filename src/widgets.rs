//! Flat controls drawn in the palette of the selected theme, replacing the
//! look of egui's stock checkbox, radio button, slider and progress bar.

use crate::keyboard::palette::Palette;
use eframe::egui::{
    self, Color32, CursorIcon, DragValue, Key, Margin, ProgressBar, Rect, Response, Sense, Stroke,
    StrokeKind, TextEdit, Ui, Widget, WidgetText, emath::Numeric, pos2, vec2,
};
use std::ops::RangeInclusive;

const BOX: f32 = 18.;
const RAIL_HEIGHT: f32 = 4.;
const HANDLE_RADIUS: f32 = 7.;
const RAIL_WIDTH: f32 = 190.;

/// The colors of a control in its current state.
struct Tone {
    fill: Color32,
    border: Color32,
    mark: Color32,
}

fn tone(ui: &Ui, response: &Response, on: bool) -> Tone {
    let palette = Palette::of(ui.ctx());
    if !ui.is_enabled() {
        return Tone {
            fill: palette.disabled,
            border: palette.disabled_border,
            mark: palette.disabled_ink,
        };
    }
    if on {
        return Tone {
            fill: if response.is_pointer_button_down_on() {
                palette.selected_ink
            } else {
                palette.accent
            },
            border: palette.accent,
            mark: palette.on_accent,
        };
    }
    Tone {
        fill: if response.is_pointer_button_down_on() {
            palette.pressed
        } else if response.hovered() {
            palette.hover
        } else {
            palette.key
        },
        border: if response.hovered() {
            palette.hover_border
        } else {
            palette.input_border
        },
        mark: palette.accent,
    }
}

fn focus_ring(ui: &Ui, response: &Response, rect: Rect, rounding: f32) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.),
            rounding + 2.,
            Stroke::new(2., Palette::of(ui.ctx()).selected_border),
            StrokeKind::Outside,
        );
    }
}

/// A square box that fills with the accent color and shows a tick when checked.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, text: impl Into<WidgetText>) -> Response {
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            let (rect, mut response) = ui.allocate_exact_size(vec2(BOX, BOX), Sense::click());
            let label = ui.add(egui::Label::new(text).sense(Sense::click()));
            response |= label;
            if response.clicked() {
                *checked = !*checked;
                response.mark_changed();
            }
            let tone = tone(ui, &response, *checked);
            ui.painter().rect(
                rect,
                5.,
                tone.fill,
                Stroke::new(1., tone.border),
                StrokeKind::Inside,
            );
            if *checked {
                let tick = Stroke::new(2., tone.mark);
                ui.painter().add(egui::Shape::line(
                    vec![
                        rect.min + vec2(4.5, 9.5),
                        rect.min + vec2(7.8, 12.8),
                        rect.min + vec2(13.5, 5.8),
                    ],
                    tick,
                ));
            }
            focus_ring(ui, &response, rect, 5.);
            if response.hovered() && ui.is_enabled() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            response
        })
        .inner;
    let enabled = ui.is_enabled();
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, *checked, ""));
    response
}

/// A round option that shows a ring and a dot when it is the chosen value.
pub fn radio<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    text: impl Into<WidgetText>,
) -> Response {
    let selected = *current == value;
    let mut response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            let (rect, mut response) = ui.allocate_exact_size(vec2(BOX, BOX), Sense::click());
            let label = ui.add(egui::Label::new(text).sense(Sense::click()));
            response |= label;
            let tone = tone(ui, &response, false);
            let palette = Palette::of(ui.ctx());
            let (ring, dot) = if !ui.is_enabled() {
                (tone.border, tone.mark)
            } else if selected {
                (palette.accent, palette.accent)
            } else {
                (tone.border, tone.mark)
            };
            ui.painter().circle(
                rect.center(),
                BOX / 2. - 0.5,
                tone.fill,
                Stroke::new(if selected { 2. } else { 1. }, ring),
            );
            if selected {
                ui.painter().circle_filled(rect.center(), 4.5, dot);
            }
            focus_ring(ui, &response, rect, BOX / 2.);
            if response.hovered() && ui.is_enabled() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            response
        })
        .inner;
    if response.clicked() && !selected {
        *current = value;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, ""));
    response
}

/// A rail with an accent-colored fill up to a round handle, a number box next
/// to it for exact values, and an optional label after both.
pub struct Slider<'a, N: Numeric> {
    value: &'a mut N,
    range: RangeInclusive<N>,
    suffix: &'a str,
    logarithmic: bool,
    max_decimals: Option<usize>,
    text: Option<WidgetText>,
}

impl<'a, N: Numeric> Slider<'a, N> {
    pub fn new(value: &'a mut N, range: RangeInclusive<N>) -> Self {
        Self {
            value,
            range,
            suffix: "",
            logarithmic: false,
            max_decimals: None,
            text: None,
        }
    }

    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = suffix;
        self
    }

    /// Spreads the values evenly in ratio instead of evenly in distance. The
    /// range must stay above zero.
    pub fn logarithmic(mut self, logarithmic: bool) -> Self {
        self.logarithmic = logarithmic;
        self
    }

    pub fn max_decimals(mut self, decimals: usize) -> Self {
        self.max_decimals = Some(decimals);
        self
    }

    pub fn text(mut self, text: impl Into<WidgetText>) -> Self {
        self.text = Some(text.into());
        self
    }
}

impl<N: Numeric> Widget for Slider<'_, N> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            value,
            range,
            suffix,
            logarithmic,
            max_decimals,
            text,
        } = self;
        let (min, max) = (range.start().to_f64(), range.end().to_f64());
        let logarithmic = logarithmic && min > 0. && max > min;
        let to_ratio = |value: f64| {
            let ratio = if logarithmic {
                (value / min).ln() / (max / min).ln()
            } else {
                (value - min) / (max - min)
            };
            if ratio.is_finite() {
                ratio.clamp(0., 1.)
            } else {
                0.
            }
        };
        let from_ratio = |ratio: f64| {
            let ratio = ratio.clamp(0., 1.);
            let value = if logarithmic {
                min * (max / min).powf(ratio)
            } else {
                min + (max - min) * ratio
            };
            if N::INTEGRAL { value.round() } else { value }
        };

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.;
            let palette = Palette::of(ui.ctx());
            let width = (ui.available_width() - 130.).clamp(90., RAIL_WIDTH);
            let (area, mut response) =
                ui.allocate_exact_size(vec2(width, 24.), Sense::click_and_drag());
            let travel = (area.width() - 2. * HANDLE_RADIUS).max(1.);
            let current = value.to_f64();
            let mut next = current;
            if let Some(pointer) = response.interact_pointer_pos()
                && (response.dragged() || response.is_pointer_button_down_on())
            {
                next = from_ratio(f64::from(
                    (pointer.x - area.left() - HANDLE_RADIUS) / travel,
                ));
            }
            if response.has_focus() {
                let step = if N::INTEGRAL {
                    1. / (max - min).max(1.)
                } else {
                    0.02
                };
                let (left, right) = ui.input(|input| {
                    (
                        input.key_pressed(Key::ArrowLeft) || input.key_pressed(Key::ArrowDown),
                        input.key_pressed(Key::ArrowRight) || input.key_pressed(Key::ArrowUp),
                    )
                });
                if left {
                    next = from_ratio(to_ratio(next) - step);
                } else if right {
                    next = from_ratio(to_ratio(next) + step);
                }
            }
            if next != current {
                *value = N::from_f64(next);
                response.mark_changed();
            }

            let enabled = ui.is_enabled();
            let ratio = to_ratio(value.to_f64());
            let center = pos2(
                area.left() + HANDLE_RADIUS + travel * ratio as f32,
                area.center().y,
            );
            let rail = Rect::from_center_size(area.center(), vec2(area.width(), RAIL_HEIGHT));
            let accent = if enabled {
                palette.accent
            } else {
                palette.disabled_ink
            };
            let painter = ui.painter();
            painter.rect_filled(rail, RAIL_HEIGHT / 2., palette.track);
            painter.rect_stroke(
                rail,
                RAIL_HEIGHT / 2.,
                Stroke::new(1., palette.border),
                StrokeKind::Outside,
            );
            painter.rect_filled(
                Rect::from_min_max(rail.min, pos2(center.x, rail.max.y)),
                RAIL_HEIGHT / 2.,
                accent,
            );
            if enabled && (response.hovered() || response.dragged()) {
                painter.circle_filled(center, HANDLE_RADIUS + 5., accent.gamma_multiply(0.22));
            }
            painter.circle(center, HANDLE_RADIUS, accent, Stroke::new(2., palette.key));
            focus_ring(
                ui,
                &response,
                Rect::from_center_size(center, vec2(14., 14.)),
                7.,
            );
            if enabled && response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }

            let speed = if logarithmic {
                (value.to_f64() * 0.01).max(min / 100.)
            } else {
                (max - min) / 200.
            };
            let mut field = DragValue::new(&mut *value)
                .range(range.clone())
                .suffix(suffix)
                .speed(speed);
            if let Some(decimals) = max_decimals {
                field = field.max_decimals(decimals);
            }
            if number_field(ui, field).changed() {
                response.mark_changed();
            }
            if let Some(text) = text {
                ui.label(text);
            }
            response
        })
        .inner
    }
}

/// A drag value that looks like a text field: input color, centered number.
pub fn number_field(ui: &mut Ui, field: DragValue<'_>) -> Response {
    let palette = Palette::of(ui.ctx());
    ui.scope(|ui| {
        let visuals = &mut ui.visuals_mut().widgets;
        for state in [
            &mut visuals.inactive,
            &mut visuals.hovered,
            &mut visuals.active,
        ] {
            state.weak_bg_fill = palette.input;
            state.bg_fill = palette.input;
        }
        visuals.inactive.bg_stroke = Stroke::new(1., palette.input_border);
        ui.add_sized(vec2(78., 28.), field)
    })
    .inner
}

/// A single-line text field with room to breathe.
pub fn text_input(value: &mut String) -> TextEdit<'_> {
    TextEdit::singleline(value)
        .align(egui::Align2::LEFT_CENTER)
        .margin(Margin {
            left: 10,
            right: 10,
            top: 7,
            bottom: 5,
        })
        .min_size(vec2(0., 32.))
}

/// A progress bar in the accent color, with `text` inside.
pub fn progress(ui: &mut Ui, fraction: f32, text: impl Into<WidgetText>) -> Response {
    let palette = Palette::of(ui.ctx());
    ui.add(
        ProgressBar::new(fraction)
            .text(text)
            .fill(palette.accent)
            .desired_height(18.)
            .corner_radius(9),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput};
    use vime::settings::KeyboardTheme;

    struct State {
        checked: bool,
        device: i32,
        width: f32,
        rate: f32,
        rects: [Rect; 4],
    }

    fn show(ui: &mut Ui, state: &mut State) {
        state.rects[0] = checkbox(ui, &mut state.checked, "check").rect;
        state.rects[1] = radio(ui, &mut state.device, 2, "radio").rect;
        state.rects[2] = ui
            .add(Slider::new(&mut state.width, 0.65..=1.65).suffix(" m"))
            .rect;
        state.rects[3] = ui
            .add(Slider::new(&mut state.rate, 0.01..=100.).logarithmic(true))
            .rect;
    }

    fn frame(context: &egui::Context, state: &mut State, events: Vec<Event>) {
        let mut output = context.run_ui(
            RawInput {
                events,
                ..Default::default()
            },
            |ui| show(ui, state),
        );
        output.textures_delta.clear();
    }

    fn click(context: &egui::Context, state: &mut State, position: egui::Pos2) {
        frame(context, state, vec![Event::PointerMoved(position)]);
        for pressed in [true, false] {
            let event = Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(context, state, vec![event]);
        }
        frame(context, state, Vec::new());
    }

    #[test]
    fn controls_follow_clicks_in_both_themes() {
        for theme in KeyboardTheme::ALL {
            let context = egui::Context::default();
            Palette::for_theme(theme).install(&context);
            let mut state = State {
                checked: false,
                device: 0,
                width: 0.65,
                rate: 1.,
                rects: [Rect::NOTHING; 4],
            };
            frame(&context, &mut state, Vec::new());
            frame(&context, &mut state, Vec::new());
            let [check, radio_rect, slider, log] = state.rects;

            click(&context, &mut state, check.left_center() + vec2(9., 0.));
            assert!(state.checked, "{theme:?} checkbox");
            click(
                &context,
                &mut state,
                radio_rect.left_center() + vec2(9., 0.),
            );
            assert_eq!(state.device, 2, "{theme:?} radio");
            click(
                &context,
                &mut state,
                pos2(slider.left() + 48., slider.center().y),
            );
            assert!(
                state.width > 0.65 && state.width < 1.65,
                "{theme:?} slider {}",
                state.width
            );
            let before = state.rate;
            click(
                &context,
                &mut state,
                pos2(log.left() + 108., log.center().y),
            );
            assert!(state.rate > before, "{theme:?} log slider {}", state.rate);
        }
    }
}
