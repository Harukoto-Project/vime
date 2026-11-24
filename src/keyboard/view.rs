use super::{Action, Keyboard, icons::Icon, palette::Palette, translate::Phase};
use eframe::egui::text::{CCursor, Galley};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    TextFormat, Ui, Vec2, text::LayoutJob,
};
use std::sync::Arc;
use vime::{
    i18n::tr,
    ime::{Language, hangul_key_label, japanese_key_label, search_emoji},
    output::CHAT_LIMIT,
    settings::{Destination, KeyboardTheme},
    steamvr::{KeyFeedback, KeyboardTargets},
};

/// The size of the keyboard with the translation strip above the editor. Without
/// the strip everything sits `STRIP` higher and the bottom of the surface is
/// left transparent.
pub const SIZE: [u32; 2] = [860, 472];
const STRIP: f32 = 48.;

fn height(translating: bool) -> f32 {
    SIZE[1] as f32 - if translating { 0. } else { STRIP }
}
pub const EMOJI_PAGE_SIZE: usize = 40;
pub const CANDIDATE_PAGE_SIZE: usize = 7;
/// Fewer candidates fit next to the clause arrows while converting.
pub const CONVERSION_PAGE_SIZE: usize = 5;

pub fn show(
    ui: &mut Ui,
    keyboard: &Keyboard,
    width: f32,
    destination: Destination,
    theme: KeyboardTheme,
    caret_visible: bool,
) -> Interaction {
    draw(ui, keyboard, width, destination, theme, None, caret_visible).0
}

/// What the pointer did to the keyboard during one frame.
#[derive(Default)]
pub struct Interaction {
    /// A key that was clicked.
    pub action: Option<Action>,
    /// The key that is held down, if it repeats while held.
    pub held: Option<Action>,
}

pub fn show_vr(
    ui: &mut Ui,
    keyboard: &Keyboard,
    destination: Destination,
    theme: KeyboardTheme,
    feedback: [KeyFeedback; 2],
    caret_visible: bool,
) -> VrKeys {
    draw(
        ui,
        keyboard,
        SIZE[0] as f32,
        destination,
        theme,
        Some(feedback),
        caret_visible,
    )
    .1
}

pub struct VrKeys {
    rect: Rect,
    keys: Vec<(Rect, Action)>,
    editor: Option<EditorHit>,
}

impl VrKeys {
    pub fn targets(&self) -> KeyboardTargets {
        KeyboardTargets {
            panel: self.rect,
            editor: self.editor.as_ref().map(|editor| editor.area),
            keys: self.keys.iter().map(|(rect, _)| *rect).collect(),
        }
    }

    pub fn action_at(&self, uv: [f32; 2]) -> Option<Action> {
        let position = KeyboardTargets::position(self.rect, uv);
        if let Some(editor) = &self.editor
            && editor.area.contains(position)
        {
            return Some(editor.action(position));
        }
        KeyboardTargets::key_at(self.keys.iter().map(|(rect, _)| *rect), position)
            .map(|index| self.keys[index].1.clone())
    }
}

fn draw(
    ui: &mut Ui,
    keyboard: &Keyboard,
    width: f32,
    destination: Destination,
    theme: KeyboardTheme,
    feedback: Option<[KeyFeedback; 2]>,
    caret_visible: bool,
) -> (Interaction, VrKeys) {
    let controller_input = feedback.is_some();
    let translating = keyboard.translation().is_some();
    // The VR surface is always drawn in full, so that pointer positions map to
    // the same keys whether or not the strip is shown.
    let units = if controller_input {
        SIZE[1] as f32
    } else {
        height(translating)
    };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, width * units / SIZE[0] as f32),
        Sense::hover(),
    );
    let scale = rect.width() / SIZE[0] as f32;
    let hold_id = response.id.with("repeat_hold");
    let (keep_hold, start_hold) = ui.input(|input| {
        let down = !controller_input && input.focused && input.pointer.primary_down();
        (
            down && !input.pointer.primary_released(),
            down && input.pointer.primary_pressed(),
        )
    });
    let held = if keep_hold {
        ui.data_mut(|data| data.get_temp::<Action>(hold_id))
    } else {
        None
    };
    let mut view = Board {
        ui,
        rect,
        scale,
        action: None,
        held,
        start_hold,
        feedback,
        palette: Palette::for_theme(theme),
        keys: Vec::new(),
        editor: None,
        shift: 0.,
    };
    let shift = if translating { STRIP } else { 0. };
    view.background(shift);
    view.header(keyboard, controller_input);
    // Everything below the header makes room for the translation strip.
    view.shift = shift;
    view.translation(keyboard);
    view.editor(keyboard, destination, caret_visible);
    view.candidates(keyboard);
    if keyboard.emoji {
        view.emoji(keyboard, destination);
    } else {
        view.keys(keyboard, destination);
    }
    view.emoji_toggle(keyboard.emoji);
    if !controller_input {
        // egui can stop tracking a click before the button is released.
        view.ui.data_mut(|data| {
            if let Some(action) = &view.held {
                data.insert_temp(hold_id, action.clone());
            } else {
                data.remove::<Action>(hold_id);
            }
        });
    }
    (
        Interaction {
            action: view.action,
            held: view.held,
        },
        VrKeys {
            rect,
            keys: view.keys,
            editor: view.editor,
        },
    )
}

struct Board<'a> {
    ui: &'a mut Ui,
    rect: Rect,
    scale: f32,
    action: Option<Action>,
    held: Option<Action>,
    start_hold: bool,
    feedback: Option<[KeyFeedback; 2]>,
    palette: &'static Palette,
    keys: Vec<(Rect, Action)>,
    editor: Option<EditorHit>,
    /// How far down the content below the header is moved, in logical units.
    shift: f32,
}

#[derive(Clone, Copy)]
enum Face {
    Key,
    Subtle,
    Selected,
    Primary,
    Disabled,
    DisabledSubtle,
    Correction,
}

impl Board<'_> {
    /// A box positioned as in the keyboard without the translation strip.
    fn box_at(&self, x: f32, y: f32, width: f32, height: f32) -> Rect {
        self.box_abs(x, y + self.shift, width, height)
    }

    /// A box positioned from the top of the keyboard.
    fn box_abs(&self, x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(
            self.rect.min + Vec2::new(x, y) * self.scale,
            Vec2::new(width, height) * self.scale,
        )
    }

    fn background(&self, shift: f32) {
        self.ui.painter().rect(
            self.box_abs(4., 4., 852., 416. + shift),
            24. * self.scale,
            self.palette.panel,
            Stroke::new(self.scale, self.palette.border),
            StrokeKind::Inside,
        );
    }

    fn header(&mut self, keyboard: &Keyboard, can_close: bool) {
        let texture = vime::brand::wordmark_texture(self.ui);
        let size = texture.size_vec2();
        let logo = Rect::from_min_size(
            self.box_at(22., 15., 0., 40.).left_center() - Vec2::new(0., 14. * self.scale),
            size * (28. * self.scale / size.y),
        );
        self.ui.painter().image(
            texture.id(),
            logo,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
            self.palette.ink,
        );
        self.label(
            Rect::from_min_max(
                Pos2::new(
                    logo.right() + 6. * self.scale,
                    logo.bottom() - 18. * self.scale,
                ),
                Pos2::new(logo.right() + 60. * self.scale, logo.bottom()),
            ),
            Align2::LEFT_CENTER,
            vime::update::VERSION_LABEL,
            12.,
            self.palette.secondary,
        );
        let track = self.box_at(256., 15., 348., 40.);
        self.ui
            .painter()
            .rect_filled(track, 10. * self.scale, self.palette.track);
        for (index, language) in Language::ALL.into_iter().enumerate() {
            self.button(
                self.box_at(259. + index as f32 * 114., 18., 114., 34.),
                language.label(),
                17.,
                if language == keyboard.composer.language() {
                    Face::Selected
                } else {
                    Face::Subtle
                },
                Action::Language(language),
            );
        }
        if can_close {
            self.button(
                self.box_at(766., 15., 71., 40.),
                tr("閉じる"),
                16.,
                Face::Key,
                Action::Close,
            );
        }
        // The theme switch is the right-most control; the close button, which
        // only exists in VR, sits to its right.
        let x = if can_close { 712. } else { 797. };
        self.theme_button(self.box_at(x, 15., 48., 40.));
    }

    /// Shows what a press switches to: the sun in the dark theme, the moon in
    /// the light one.
    fn theme_button(&mut self, rect: Rect) {
        let (hovered, pressed) =
            self.button_interaction(rect, tr("テーマを切り替え"), true, Action::ToggleTheme);
        let (fill, border, ink) = Face::Key.colors(self.palette, hovered, pressed);
        let painter = self.ui.painter();
        painter.rect(
            rect,
            8. * self.scale,
            fill,
            Stroke::new(self.scale, border),
            StrokeKind::Inside,
        );
        let center = rect.center();
        let unit = self.scale;
        if self.palette.dark {
            painter.circle_filled(center, 5. * unit, ink);
            for index in 0..8 {
                let direction = Vec2::angled(index as f32 * std::f32::consts::FRAC_PI_4);
                painter.line_segment(
                    [
                        center + direction * 8.5 * unit,
                        center + direction * 11. * unit,
                    ],
                    Stroke::new(2. * unit, ink),
                );
            }
        } else {
            // A crescent: the moon with a bite taken out in the key's own color.
            painter.circle_filled(center, 9. * unit, ink);
            painter.circle_filled(center + Vec2::new(5., -4.) * unit, 8. * unit, fill);
        }
    }

    /// The translation above the editor, which is put into the editor when
    /// pressed, and the button that switches the language it is translated to.
    fn translation(&mut self, keyboard: &Keyboard) {
        const TOP: f32 = 63.;
        const HEIGHT: f32 = 48.;
        let Some(translator) = keyboard.translation() else {
            return;
        };
        let text = translator.text();
        let phase = translator.phase();
        let ready = !text.trim().is_empty();

        let rect = self.box_abs(22., TOP, 616., HEIGHT);
        let face = if ready { Face::Selected } else { Face::Disabled };
        let ink = self.button_face(
            rect,
            tr("翻訳を入力欄に反映"),
            face,
            Action::AcceptTranslation,
        );
        let (message, color) = match phase {
            Phase::Failed(error) => (error.as_str(), self.palette.error),
            _ if ready => (text, ink),
            Phase::Waiting | Phase::Working => (tr("翻訳中…"), self.palette.secondary),
            _ => (
                tr("入力すると翻訳が表示されます"),
                self.palette.secondary,
            ),
        };
        let mut job = LayoutJob::simple(
            message.to_owned(),
            FontId::proportional(17. * self.scale),
            color,
            rect.width() - 28. * self.scale,
        );
        job.wrap.max_rows = 2;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let painter = self.ui.painter().with_clip_rect(rect);
        let galley = painter.layout_job(job);
        let origin = Pos2::new(
            rect.left() + 14. * self.scale,
            rect.center().y - galley.size().y / 2.,
        );
        painter.galley(origin, galley, color);

        let button = self.box_abs(650., TOP, 188., HEIGHT);
        let ink = self.button_face(
            button,
            tr("翻訳先を切り替え"),
            Face::Key,
            Action::CycleTranslationTarget,
        );
        self.label(
            self.box_abs(654., TOP + 5., 180., 14.),
            Align2::CENTER_CENTER,
            tr("翻訳先"),
            11.,
            self.palette.secondary,
        );
        self.label(
            self.box_abs(654., TOP + 19., 180., 24.),
            Align2::CENTER_CENTER,
            translator.target().label(),
            17.,
            ink,
        );
    }

    fn editor(&mut self, keyboard: &Keyboard, destination: Destination, caret_visible: bool) {
        const TOP: f32 = 74.;
        const HEIGHT: f32 = 47.;
        let rect = self.box_at(22., TOP, 616., HEIGHT);
        let painter = self.ui.painter();
        painter.rect(
            rect,
            12. * self.scale,
            self.palette.input,
            Stroke::new(self.scale, self.palette.input_border),
            StrokeKind::Inside,
        );
        let (before, after) = keyboard.composer.committed_parts();
        let preedit = keyboard.composer.preedit();
        let mut job = LayoutJob::default();
        let font = FontId::proportional(24. * self.scale);
        job.append(
            before,
            0.,
            TextFormat::simple(font.clone(), self.palette.ink),
        );
        for (text, focused) in keyboard.composer.preedit_segments() {
            // The clause being converted is highlighted like in a desktop IME.
            let format = if focused {
                TextFormat {
                    background: self.palette.clause,
                    underline: Stroke::new(2.5 * self.scale, self.palette.accent),
                    ..TextFormat::simple(font.clone(), self.palette.ink)
                }
            } else {
                TextFormat {
                    underline: Stroke::new(1.5 * self.scale, self.palette.accent),
                    ..TextFormat::simple(font.clone(), self.palette.accent)
                }
            };
            job.append(&text, 0., format);
        }
        job.append(
            after,
            0.,
            TextFormat::simple(font.clone(), self.palette.ink),
        );
        let show_count = destination == Destination::Osc;
        let text_width = if show_count { 490. } else { 557. };
        let text_area = self.box_at(36., TOP + 4., text_width, HEIGHT - 8.);
        let text_painter = painter.with_clip_rect(text_area);
        let galley = painter.layout_job(job);
        let empty = before.is_empty() && preedit.is_empty() && after.is_empty();
        let caret = galley.pos_from_cursor(CCursor::new(
            before.chars().count() + preedit.chars().count(),
        ));
        let scroll = (caret.min.x + 5. * self.scale - text_area.width()).max(0.);
        let origin = Pos2::new(
            text_area.left() + self.scale - scroll,
            text_area.center().y - caret.center().y,
        );
        text_painter.galley(origin, galley.clone(), self.palette.ink);
        if empty {
            self.label(
                text_area.translate(Vec2::new(7. * self.scale, 0.)),
                Align2::LEFT_CENTER,
                tr("メッセージを入力"),
                24.,
                self.palette.secondary,
            );
        }
        if caret_visible {
            let x = origin.x + if empty { 0. } else { caret.min.x };
            let center = if empty {
                text_area.center().y
            } else {
                origin.y + caret.center().y
            };
            let half_height = 12. * self.scale;
            text_painter.line_segment(
                [
                    Pos2::new(x, center - half_height),
                    Pos2::new(x, center + half_height),
                ],
                Stroke::new(1.8 * self.scale, self.palette.accent),
            );
        }

        let hit = EditorHit {
            area: text_area,
            origin,
            galley,
        };
        if self.feedback.is_some() {
            self.editor = Some(hit);
        } else {
            let response = self
                .ui
                .interact(
                    text_area,
                    self.ui.scope_id().with("message_editor"),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::Text);
            if response.clicked()
                && let Some(position) = response.interact_pointer_pos()
            {
                response.request_focus();
                self.action = Some(hit.action(position));
            } else if response.has_focus()
                && enter_enabled(keyboard, destination)
                && self.ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                self.action = Some(Action::Enter);
            }
        }

        if show_count {
            let count = keyboard.composer.text().encode_utf16().count();
            self.label(
                self.box_at(538., TOP, 56., HEIGHT),
                Align2::RIGHT_CENTER,
                &format!("{count} / {CHAT_LIMIT}"),
                12.,
                if count > CHAT_LIMIT {
                    self.palette.error
                } else {
                    self.palette.secondary
                },
            );
        }
        if !keyboard.status.is_empty() {
            self.label(
                self.box_at(36., TOP + HEIGHT + 1., 588., 19.),
                Align2::LEFT_CENTER,
                &keyboard.status,
                14.,
                self.palette.accent,
            );
        }
        let divider = self.box_at(718., TOP + 5., 0., HEIGHT - 10.);
        painter.line_segment(
            [divider.min, divider.max],
            Stroke::new(self.scale, self.palette.border),
        );
        let clear = self.box_at(601., TOP + 7.5, 28., 32.);
        self.icon_button(
            clear,
            Icon::Close,
            if empty {
                Face::DisabledSubtle
            } else {
                Face::Subtle
            },
            Action::Clear,
        );
        self.icon_button(
            self.box_at(650., TOP, 58., HEIGHT),
            Icon::Send,
            if send_enabled(keyboard, destination) {
                Face::Primary
            } else {
                Face::Disabled
            },
            Action::Send,
        );
        self.icon_button(
            self.box_at(729., TOP, 51., HEIGHT),
            Icon::Copy,
            if empty { Face::Disabled } else { Face::Key },
            Action::Copy,
        );
        self.icon_button(
            self.box_at(787., TOP, 51., HEIGHT),
            Icon::Paste,
            Face::Key,
            Action::Paste,
        );
    }

    fn candidates(&mut self, keyboard: &Keyboard) {
        let values = &keyboard.candidates;
        let correction = keyboard.typo_suggestion();
        if values.is_empty() && correction.is_none() {
            let hint = match keyboard.composer.language() {
                Language::Japanese if keyboard.direct_input() => {
                    tr("半角英数字 · 半角/全角で日本語入力に切り替え")
                }
                Language::Japanese => tr("ローマ字で入力 · 半角/全角で半角英数字に切り替え"),
                Language::English => tr("絵文字は一覧または :cat: などの短縮名から選択"),
                Language::Korean if keyboard.direct_input() => {
                    tr("英数字 · 한/영でハングルに切り替え")
                }
                Language::Korean => tr("2ボル式で入力 · 한/영で英数字に切り替え"),
            };
            self.label(
                self.box_at(24., 141., 660., 36.),
                Align2::LEFT_CENTER,
                hint,
                15.,
                self.palette.secondary,
            );
            return;
        }
        let mut x = 22.;
        if let Some(word) = correction {
            let measured = self.ui.painter().layout_no_wrap(
                word.to_owned(),
                FontId::proportional(17. * self.scale),
                self.palette.ink,
            );
            let width = (measured.size().x / self.scale + 35.).clamp(94., 164.);
            let rect = self.box_at(x, 141., width, 36.);
            let color = self.button_face(
                rect,
                &format!("{}: {word}", Icon::Check.label()),
                Face::Correction,
                Action::CorrectTypo(word.to_owned()),
            );
            Icon::Check.paint(
                self.ui,
                rect.left_center() + Vec2::new(13. * self.scale, 0.),
                16. * self.scale,
                color,
            );
            self.label(
                self.box_at(x + 25., 141., width - 33., 36.),
                Align2::LEFT_CENTER,
                word,
                17.,
                color,
            );
            x += width + 7.;
        }
        if let Some(conversion) = keyboard.composer.conversion() {
            // Clause navigation, like the arrow keys of a desktop IME.
            let last = conversion.segments().len() - 1;
            for (icon, step, enabled) in [
                (Icon::ArrowBack, -1, conversion.focus() > 0),
                (Icon::ArrowForward, 1, conversion.focus() < last),
            ] {
                self.icon_button(
                    self.box_at(x, 141., 40., 36.),
                    icon,
                    if enabled {
                        Face::Subtle
                    } else {
                        Face::DisabledSubtle
                    },
                    Action::MoveSegment(step),
                );
                x += 46.;
            }
            x += 6.;
        }
        let page_size = keyboard.page_size();
        let first = keyboard.candidate_page * page_size;
        let page = values.iter().skip(first).take(page_size);
        let widths: Vec<_> = page
            .clone()
            .map(|value| {
                let measured = self
                    .ui
                    .painter()
                    .layout_no_wrap(
                        value.clone(),
                        FontId::proportional(17. * self.scale),
                        self.palette.ink,
                    )
                    .size()
                    .x;
                (measured / self.scale + 28.).max(50.)
            })
            .collect();
        let right = if values.len() > page_size { 792. } else { 838. };
        let budget = right - x - 7. * widths.len().saturating_sub(1) as f32;
        let cap = candidate_width_cap(&widths, budget);
        for (offset, (value, width)) in page.zip(widths).enumerate() {
            let width = width.min(cap);
            let index = first + offset;
            self.button(
                self.box_at(x, 141., width, 36.),
                value,
                17.,
                if keyboard.candidate_selected == Some(index) {
                    Face::Selected
                } else {
                    Face::Key
                },
                Action::Candidate(value.clone()),
            );
            x += width + 7.;
        }
        if values.len() > page_size {
            self.icon_button(
                self.box_at(799., 141., 38., 36.),
                Icon::ChevronRight,
                Face::Subtle,
                Action::PageCandidates,
            );
        }
    }

    fn keys(&mut self, keyboard: &Keyboard, destination: Destination) {
        let has_text = !keyboard.composer.text().is_empty();
        let enter = enter_face(keyboard, destination);
        match keyboard.composer.language() {
            Language::Japanese => self.jis_keys(keyboard, has_text, enter),
            language => self.ansi_keys(keyboard, language, has_text, enter),
        }
    }

    fn ansi_keys(&mut self, keyboard: &Keyboard, language: Language, has_text: bool, enter: Face) {
        let shift = keyboard.shift;
        let hangul = language == Language::Korean && !keyboard.direct_input();
        let label_hangul = |key| {
            if hangul {
                hangul_key_label(key).unwrap_or(key)
            } else {
                key
            }
        };
        let shift_face = if shift { Face::Selected } else { Face::Key };
        self.physical_chars(
            0,
            0.,
            if shift {
                "~!@#$%^&*()_+"
            } else {
                "`1234567890-="
            },
            |key| key,
        );
        self.icon_button(
            self.jis_rect(0, 13., 2.),
            Icon::Backspace,
            if has_text { Face::Key } else { Face::Disabled },
            Action::Backspace,
        );
        self.physical_chars(
            1,
            1.5,
            if shift {
                "QWERTYUIOP{}"
            } else {
                "qwertyuiop[]"
            },
            label_hangul,
        );
        let slash = if shift { '|' } else { '\\' };
        self.button(
            self.jis_rect(1, 13.5, 1.5),
            &slash.to_string(),
            20.,
            Face::Key,
            Action::Key(slash),
        );
        self.physical_chars(
            2,
            1.75,
            if shift { "ASDFGHJKL:\"" } else { "asdfghjkl;'" },
            label_hangul,
        );
        self.icon_button(
            self.jis_rect(2, 12.75, 2.25),
            Icon::Enter,
            enter,
            Action::Enter,
        );
        self.button(
            self.jis_rect(3, 0., 2.25),
            "Shift",
            16.,
            shift_face,
            Action::Shift,
        );
        self.physical_chars(
            3,
            2.25,
            if shift { "ZXCVBNM<>?" } else { "zxcvbnm,./" },
            label_hangul,
        );
        self.button(
            self.jis_rect(3, 12.25, 2.75),
            "Shift",
            16.,
            shift_face,
            Action::Shift,
        );
        let (start, width) = if language == Language::Korean {
            (5., 5.)
        } else {
            (3.75, 6.25)
        };
        self.button(
            self.jis_rect(4, start, width),
            "Space",
            16.,
            Face::Key,
            Action::Space,
        );
        if language == Language::Korean {
            self.button(
                self.jis_rect(4, 10., 1.25),
                "한/영",
                14.,
                if hangul { Face::Selected } else { Face::Key },
                Action::ToggleDirect,
            );
        }
    }

    // The 109-key JIS main block is 15 key units wide. Empty modifier
    // positions keep their physical pitch, and 1u keys are square.
    fn jis_rect(&self, row: usize, column: f32, units: f32) -> Rect {
        const PITCH: f32 = 44.;
        self.box_at(
            (SIZE[0] as f32 - (15. * PITCH - 4.)) / 2. + column * PITCH,
            188. + row as f32 * PITCH,
            units * PITCH - 4.,
            PITCH - 4.,
        )
    }

    fn physical_chars(
        &mut self,
        row: usize,
        start: f32,
        chars: &str,
        label: impl Fn(char) -> char,
    ) {
        for (offset, key) in chars.chars().enumerate() {
            self.button(
                self.jis_rect(row, start + offset as f32, 1.),
                &label(key).to_string(),
                20.,
                Face::Key,
                Action::Key(key),
            );
        }
    }

    fn jis_keys(&mut self, keyboard: &Keyboard, has_text: bool, enter: Face) {
        let shift = keyboard.shift;
        let direct = keyboard.direct_input();
        let shift_face = if shift { Face::Selected } else { Face::Key };
        let label = |key| if direct { key } else { japanese_key_label(key) };

        self.button(
            self.jis_rect(0, 0., 1.),
            "半/全",
            12.,
            if direct { Face::Selected } else { Face::Key },
            Action::ToggleDirect,
        );
        self.physical_chars(
            0,
            1.,
            if shift {
                "!\"#$%&'()0=~|"
            } else {
                "1234567890-^¥"
            },
            label,
        );
        self.icon_button(
            self.jis_rect(0, 14., 1.),
            Icon::Backspace,
            if has_text { Face::Key } else { Face::Disabled },
            Action::Backspace,
        );

        // Tab and Caps Lock occupy the empty space before Q and A. The two-row
        // Enter shares the final 1.5u of this row and the final 1.25u of the row
        // below.
        self.physical_chars(
            1,
            1.5,
            if shift {
                "QWERTYUIOP`{"
            } else {
                "qwertyuiop@["
            },
            label,
        );
        self.physical_chars(
            2,
            1.75,
            if shift {
                "ASDFGHJKL+*}"
            } else {
                "asdfghjkl;:]"
            },
            label,
        );
        self.jis_enter(enter);

        self.button(
            self.jis_rect(3, 0., 2.25),
            "Shift",
            16.,
            shift_face,
            Action::Shift,
        );
        self.physical_chars(
            3,
            2.25,
            if shift { "ZXCVBNM<>?_" } else { "zxcvbnm,./\\" },
            label,
        );
        self.button(
            self.jis_rect(3, 13.25, 1.75),
            "Shift",
            16.,
            shift_face,
            Action::Shift,
        );

        // Space converts the reading and then cycles the candidates.
        let space = if keyboard.composer.converting() {
            tr("次の候補")
        } else if keyboard.can_convert() {
            tr("変換")
        } else {
            tr("スペース")
        };
        self.button(
            self.jis_rect(4, 3.75, 6.25),
            space,
            16.,
            Face::Key,
            Action::Space,
        );
    }

    fn jis_enter(&mut self, face: Face) {
        let top = self.jis_rect(1, 13.5, 1.5);
        // The step of the L sits at the bottom of the upper row, so it keeps the
        // same gap to the keys beside it as every other key. The lower part
        // spans both rows to leave no dead zone between the two.
        let mut bottom = self.jis_rect(2, 13.75, 1.25);
        bottom.min.y = top.min.y;
        let enabled = !matches!(face, Face::Disabled);
        let label = Icon::Enter.label();
        let (top_hover, top_press) = self.button_interaction(top, label, enabled, Action::Enter);
        let (bottom_hover, bottom_press) =
            self.button_interaction(bottom, label, enabled, Action::Enter);
        let (fill, border, text) = face.colors(
            self.palette,
            top_hover || bottom_hover,
            top_press || bottom_press,
        );
        // One L-shaped outline with every corner rounded like the other keys.
        // The shape is star-shaped around a point in the overlap of both parts,
        // so a triangle fan fills it without seams between semi-transparent parts.
        let outline = rounded_outline(
            &[
                top.left_top(),
                top.right_top(),
                bottom.right_bottom(),
                bottom.left_bottom(),
                Pos2::new(bottom.min.x, top.max.y),
                top.left_bottom(),
            ],
            8. * self.scale,
        );
        let center = Pos2::new(bottom.center().x, top.center().y);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(center, fill);
        for point in &outline {
            mesh.colored_vertex(*point, fill);
        }
        let count = outline.len() as u32;
        for index in 0..count {
            mesh.add_triangle(0, index + 1, (index + 1) % count + 1);
        }
        let painter = self.ui.painter();
        painter.add(Shape::mesh(mesh));
        painter.add(Shape::closed_line(outline, Stroke::new(self.scale, border)));
        Icon::Enter.paint(
            self.ui,
            Pos2::new(bottom.center().x, (top.top() + bottom.bottom()) / 2.),
            24. * self.scale,
            text,
        );
    }

    fn emoji_toggle(&mut self, open: bool) {
        self.icon_button(
            self.box_at(22., 364., 60., 40.),
            Icon::Emoji,
            if open { Face::Selected } else { Face::Key },
            Action::ToggleEmoji,
        );
    }

    fn emoji(&mut self, keyboard: &Keyboard, destination: Destination) {
        let total_pages = search_emoji("").count().div_ceil(EMOJI_PAGE_SIZE);
        for (index, emoji) in search_emoji("")
            .skip(keyboard.emoji_page * EMOJI_PAGE_SIZE)
            .take(EMOJI_PAGE_SIZE)
            .enumerate()
        {
            let x = 22. + (index % 10) as f32 * 82.2;
            let y = 188. + (index / 10) as f32 * 44.;
            self.button(
                self.box_at(x, y, 76.2, 38.),
                emoji.symbol,
                if emoji.symbol.chars().count() > 4 {
                    13.
                } else {
                    22.
                },
                Face::Key,
                Action::Insert(emoji.symbol.into()),
            );
        }
        self.label(
            self.box_at(94., 364., 220., 40.),
            Align2::LEFT_CENTER,
            tr("絵文字・記号"),
            15.,
            self.palette.secondary,
        );
        self.icon_button(
            self.box_at(331., 364., 49., 40.),
            Icon::ChevronLeft,
            Face::Key,
            Action::PageEmoji(-1),
        );
        self.label(
            self.box_at(387., 364., 86., 40.),
            Align2::CENTER_CENTER,
            &format!("{} / {total_pages}", keyboard.emoji_page + 1),
            16.,
            self.palette.ink,
        );
        self.icon_button(
            self.box_at(480., 364., 49., 40.),
            Icon::ChevronRight,
            Face::Key,
            Action::PageEmoji(1),
        );
        self.icon_button(
            self.box_at(716., 364., 122., 40.),
            Icon::Enter,
            enter_face(keyboard, destination),
            Action::Enter,
        );
    }

    fn button_interaction(
        &mut self,
        rect: Rect,
        label: &str,
        enabled: bool,
        action: Action,
    ) -> (bool, bool) {
        if let Some(feedback) = self.feedback {
            if enabled {
                self.keys.push((rect, action));
                return (
                    feedback.iter().any(|hand| hand.hovered == Some(rect)),
                    feedback.iter().any(|hand| hand.pressed == Some(rect)),
                );
            }
            (false, false)
        } else {
            // The label is not part of the id: a held key keeps its identity when
            // its label changes, such as Space switching to the candidate cycle.
            let id = self.ui.scope_id().with((
                (rect.left() - self.rect.left()) as i32,
                (rect.top() - self.rect.top()) as i32,
            ));
            let response = self.ui.interact(
                rect,
                id,
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let response = if enabled {
                response.on_hover_cursor(egui::CursorIcon::PointingHand)
            } else {
                response
            };
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
            if matches!(
                action,
                Action::Send
                    | Action::Copy
                    | Action::Paste
                    | Action::Clear
                    | Action::Candidate(_)
                    | Action::CorrectTypo(_)
                    | Action::AcceptTranslation
                    | Action::CycleTranslationTarget
            ) {
                response.clone().on_hover_text(label);
            }
            if self.start_hold
                && self.held.is_none()
                && enabled
                && action.repeats()
                && response.is_pointer_button_down_on()
            {
                self.held = Some(action.clone());
            }
            let pressed = response.is_pointer_button_down_on()
                || self
                    .held
                    .as_ref()
                    .is_some_and(|held| held.same_key(&action));
            if response.clicked() {
                self.action = Some(action);
            }
            (response.hovered() || response.has_focus(), pressed)
        }
    }

    fn button(&mut self, rect: Rect, label: &str, size: f32, face: Face, action: Action) {
        let text = self.button_face(rect, label, face, action);
        self.label(
            rect.shrink(4. * self.scale),
            Align2::CENTER_CENTER,
            label,
            size,
            text,
        );
    }

    fn icon_button(&mut self, rect: Rect, icon: Icon, face: Face, action: Action) {
        let text = self.button_face(rect, icon.label(), face, action);
        icon.paint(self.ui, rect.center(), 24. * self.scale, text);
    }

    fn button_face(&mut self, rect: Rect, label: &str, face: Face, action: Action) -> Color32 {
        let enabled = !matches!(face, Face::Disabled | Face::DisabledSubtle);
        let (hovered, pressed) = self.button_interaction(rect, label, enabled, action);
        let (fill, border, text) = face.colors(self.palette, hovered, pressed);
        self.ui.painter().rect(
            rect,
            8. * self.scale,
            fill,
            Stroke::new(self.scale, border),
            StrokeKind::Inside,
        );
        text
    }

    fn label(&self, rect: Rect, align: Align2, text: &str, size: f32, color: Color32) {
        let point = Pos2::new(
            match align.x() {
                Align::Min => rect.left(),
                Align::Center => rect.center().x,
                Align::Max => rect.right(),
            },
            rect.center().y,
        );
        let mut job = LayoutJob::simple_singleline(
            text.to_owned(),
            FontId::proportional(size * self.scale),
            color,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width());
        let painter = self.ui.painter().with_clip_rect(rect);
        let galley = painter.layout_job(job);
        let origin = align.anchor_size(point, galley.size()).min;
        painter.galley(origin, galley, color);
    }
}

impl Face {
    fn colors(
        self,
        palette: &Palette,
        hovered: bool,
        pressed: bool,
    ) -> (Color32, Color32, Color32) {
        let tint = |color: Color32| {
            if pressed {
                color.lerp_to_gamma(palette.ink, 0.17)
            } else if hovered {
                color.lerp_to_gamma(palette.ink, 0.08)
            } else {
                color
            }
        };
        match self {
            Face::Disabled => (
                palette.disabled,
                palette.disabled_border,
                palette.disabled_ink,
            ),
            Face::DisabledSubtle => (
                Color32::TRANSPARENT,
                Color32::TRANSPARENT,
                palette.disabled_ink,
            ),
            Face::Primary => (
                tint(palette.accent),
                Color32::TRANSPARENT,
                palette.on_accent,
            ),
            Face::Selected => (
                tint(palette.selected),
                palette.selected_border,
                palette.selected_ink,
            ),
            Face::Correction => (
                tint(palette.correction),
                palette.correction_border,
                palette.correction_ink,
            ),
            Face::Subtle if !hovered && !pressed => (
                Color32::TRANSPARENT,
                Color32::TRANSPARENT,
                palette.secondary,
            ),
            Face::Subtle | Face::Key => {
                let (fill, border) = if pressed {
                    (palette.pressed, palette.hover_border)
                } else if hovered {
                    (palette.hover, palette.hover_border)
                } else {
                    (palette.key, palette.border)
                };
                (fill, border, palette.ink)
            }
        }
    }
}

// Short candidates take only the space they need. Share the rest between long ones.
fn candidate_width_cap(widths: &[f32], mut budget: f32) -> f32 {
    let mut sorted = widths.to_vec();
    sorted.sort_by(f32::total_cmp);
    for (index, &width) in sorted.iter().enumerate() {
        let share = budget / (sorted.len() - index) as f32;
        if width >= share {
            return share;
        }
        budget -= width;
    }
    f32::INFINITY
}

fn enter_enabled(keyboard: &Keyboard, destination: Destination) -> bool {
    keyboard.composer.has_preedit() || send_enabled(keyboard, destination)
}

fn enter_face(keyboard: &Keyboard, destination: Destination) -> Face {
    if enter_enabled(keyboard, destination) {
        Face::Key
    } else {
        Face::Disabled
    }
}

fn send_enabled(keyboard: &Keyboard, destination: Destination) -> bool {
    let text = keyboard.composer.text();
    !text.trim().is_empty()
        && (destination != Destination::Osc || text.encode_utf16().count() <= CHAT_LIMIT)
}

struct EditorHit {
    area: Rect,
    origin: Pos2,
    galley: Arc<Galley>,
}

impl EditorHit {
    fn action(&self, position: Pos2) -> Action {
        Action::MoveCaret(self.galley.cursor_from_pos(position - self.origin).index.0)
    }
}

/// Rounds every corner of a polygon whose corners are right angles. A corner
/// next to a short edge gets a smaller radius so neighbouring arcs never overlap.
fn rounded_outline(corners: &[Pos2], radius: f32) -> Vec<Pos2> {
    use std::f32::consts::{PI, TAU};
    const STEPS: usize = 6;
    let mut outline = Vec::with_capacity(corners.len() * (STEPS + 1));
    for (index, &corner) in corners.iter().enumerate() {
        let before = corners[(index + corners.len() - 1) % corners.len()] - corner;
        let after = corners[(index + 1) % corners.len()] - corner;
        let radius = radius.min(before.length() / 2.).min(after.length() / 2.);
        let (before, after) = (before.normalized(), after.normalized());
        // For a right angle the arc's center is one radius away from both edges.
        let center = corner + (before + after) * radius;
        let first = (corner + before * radius - center).angle();
        let last = (corner + after * radius - center).angle();
        let mut sweep = last - first;
        while sweep > PI {
            sweep -= TAU;
        }
        while sweep < -PI {
            sweep += TAU;
        }
        for step in 0..=STEPS {
            let angle = first + sweep * step as f32 / STEPS as f32;
            outline.push(center + radius * Vec2::angled(angle));
        }
    }
    outline
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay_surface::pointer_button;

    #[test]
    fn both_themes_paint_vr_feedback_at_the_keys_that_receive_input() {
        for theme in KeyboardTheme::ALL {
            let context = egui::Context::default();
            let mut keyboard = Keyboard::new(context.clone());
            keyboard.apply(Action::Language(Language::Japanese));
            let mut targets = [Rect::NOTHING; 2];
            let mut initial = context.run_ui(egui::RawInput::default(), |ui| {
                let keys = show_vr(ui, &keyboard, Destination::Osc, theme, Default::default(), true);
                for (target, letter) in targets.iter_mut().zip(['k', 'o']) {
                    *target = keys.keys.iter()
                        .find(|(_, action)| matches!(action, Action::Key(key) if *key == letter))
                        .unwrap().0;
                    let point = (target.center() - keys.rect.min) / keys.rect.size();
                    assert!(matches!(keys.action_at([point.x, point.y]), Some(Action::Key(key)) if key == letter));
                }
            });
            initial.textures_delta.clear();
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                show_vr(
                    ui,
                    &keyboard,
                    Destination::Osc,
                    theme,
                    [
                        KeyFeedback {
                            hovered: Some(targets[0]),
                            pressed: None,
                        },
                        KeyFeedback {
                            hovered: Some(targets[1]),
                            pressed: Some(targets[1]),
                        },
                    ],
                    true,
                );
            });
            output.textures_delta.clear();
            let palette = Palette::for_theme(theme);
            for (target, color) in targets.into_iter().zip([palette.hover, palette.pressed]) {
                assert!(output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, Shape::Rect(rect) if rect.rect == target && rect.fill == color)
                }));
            }
        }
    }

    #[test]
    fn long_candidates_share_the_row_and_leave_the_page_button_reachable() {
        let context = egui::Context::default();
        let mut keyboard = Keyboard::new(context.clone());
        keyboard.candidates = (0..12)
            .map(|index| format!("candidate {index} with a long description"))
            .collect();
        keyboard.candidates[2] = "a".into();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let keys = show_vr(ui, &keyboard, Destination::Osc, KeyboardTheme::Dark, Default::default(), true);
            let mut right = keys.rect.left();
            let mut count = 0;
            for (rect, action) in &keys.keys {
                match action {
                    Action::Candidate(value) => {
                        assert!(rect.left() >= right);
                        let uv = (rect.center() - keys.rect.min) / keys.rect.size();
                        assert!(matches!(keys.action_at([uv.x, uv.y]), Some(Action::Candidate(hit)) if &hit == value));
                        right = rect.right();
                        count += 1;
                    }
                    Action::PageCandidates => assert!(rect.left() >= right + 6.),
                    _ => {}
                }
            }
            assert_eq!(count, CANDIDATE_PAGE_SIZE);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_translation_strip_moves_the_editor_down_and_offers_the_language_switch() {
        let context = egui::Context::default();
        let mut keyboard = Keyboard::new(context.clone());
        let send_top = |keyboard: &Keyboard| {
            let mut found = None;
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                let keys = show_vr(ui, keyboard, Destination::Osc, KeyboardTheme::Dark, Default::default(), true);
                found = Some((
                    keys.keys.iter().find(|(_, action)| matches!(action, Action::Send)).map(|(rect, _)| rect.top() - keys.rect.top()),
                    keys.keys.iter().any(|(_, action)| matches!(action, Action::CycleTranslationTarget)),
                ));
            });
            output.textures_delta.clear();
            found.unwrap()
        };
        keyboard.apply(Action::Key('a'));
        let (plain, switch) = send_top(&keyboard);
        assert!(!switch);
        keyboard.configure_translation(&vime::settings::TranslationSettings {
            enabled: true,
            url: "http://127.0.0.1:1/v1".into(),
            ..Default::default()
        });
        let (shifted, switch) = send_top(&keyboard);
        assert!(switch);
        // The surface is laid out at its logical size, one point per unit.
        assert!((shifted.unwrap() - plain.unwrap() - STRIP).abs() < 1.);
    }

    #[test]
    fn vr_editor_actions_are_separate_from_the_text_hit_area() {
        let context = egui::Context::default();
        let mut keyboard = Keyboard::new(context.clone());
        let input = || crate::overlay_surface::overlay_input([1935, 954], 2.25, vec![]);
        let hit = |keys: &VrKeys, x, y| keys.action_at([x / SIZE[0] as f32, y / SIZE[1] as f32]);
        let mut output = context.run_ui(input(), |ui| {
            let keys = show_vr(
                ui,
                &keyboard,
                Destination::Osc,
                KeyboardTheme::Dark,
                Default::default(),
                true,
            );
            assert!(hit(&keys, 615., 97.).is_none());
            assert!(hit(&keys, 679., 97.).is_none());
            assert!(hit(&keys, 754., 97.).is_none());
            assert!(matches!(hit(&keys, 813., 97.), Some(Action::Paste)));
        });
        assert_eq!(output.pixels_per_point, 2.25);
        output.textures_delta.clear();
        keyboard.apply(Action::Key('a'));
        let mut clear = None;
        let mut output = context.run_ui(input(), |ui| {
            let keys = show_vr(
                ui,
                &keyboard,
                Destination::Osc,
                KeyboardTheme::Dark,
                Default::default(),
                true,
            );
            assert!(matches!(hit(&keys, 500., 97.), Some(Action::MoveCaret(_))));
            clear = hit(&keys, 615., 97.);
            assert!(matches!(clear, Some(Action::Clear)));
            assert!(matches!(hit(&keys, 679., 97.), Some(Action::Send)));
            assert!(hit(&keys, 718., 97.).is_none());
            assert!(matches!(hit(&keys, 754., 97.), Some(Action::Copy)));
            assert!(matches!(hit(&keys, 813., 97.), Some(Action::Paste)));
        });
        output.textures_delta.clear();
        keyboard.apply(clear.unwrap());
        assert!(keyboard.composer.text().is_empty());
        assert!(keyboard.candidates.is_empty());
    }

    #[test]
    fn repeating_keys_stay_held_after_egui_cancels_the_click() {
        for key in [Action::Backspace, Action::Space] {
            let context = egui::Context::default();
            let mut keyboard = Keyboard::new(context.clone());
            keyboard.apply(Action::Key('a'));
            let width = SIZE[0] as f32;
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 500.));
            let mut position = Pos2::ZERO;
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                let keys = show_vr(
                    ui,
                    &keyboard,
                    Destination::Osc,
                    KeyboardTheme::Dark,
                    Default::default(),
                    true,
                );
                position = keys
                    .keys
                    .iter()
                    .find(|(_, action)| action.same_key(&key))
                    .unwrap()
                    .0
                    .center();
            });
            output.textures_delta.clear();

            let draw = |time, events| {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..Default::default()
                };
                let mut result = Interaction::default();
                let mut output = context.run_ui(input, |ui| {
                    result = show(
                        ui,
                        &keyboard,
                        width,
                        Destination::Osc,
                        KeyboardTheme::Dark,
                        true,
                    );
                });
                output.textures_delta.clear();
                result
            };
            draw(0., vec![]);
            draw(0.1, vec![egui::Event::PointerMoved(position)]);
            let press = draw(0.2, vec![pointer_button(position, true)]);
            assert!(press.held.unwrap().same_key(&key));

            let held = draw(1.2, vec![]);
            assert!(!context.input(|input| input.pointer.could_any_button_be_click()));
            assert!(held.held.unwrap().same_key(&key));

            let outside = Pos2::new(12., 450.);
            let moved = draw(1.3, vec![egui::Event::PointerMoved(outside)]);
            assert!(moved.held.unwrap().same_key(&key));

            let release = draw(1.4, vec![pointer_button(outside, false)]);
            assert!(release.held.is_none());
            assert!(release.action.is_none());
        }
    }
}
