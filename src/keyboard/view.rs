use super::{Action, Keyboard, icons::Icon};
use eframe::egui::text::{CCursor, Galley};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, TextFormat, Ui, Vec2, text::LayoutJob,
};
use std::sync::Arc;
use vime::{
    brand,
    i18n::tr,
    ime::{Language, hangul_key_label, japanese_key_label, search_emoji},
    output::CHAT_LIMIT,
    settings::Destination,
};

pub const SIZE: [u32; 2] = [860, 424];
pub const ASPECT: f32 = SIZE[0] as f32 / SIZE[1] as f32;
pub const EMOJI_PAGE_SIZE: usize = 40;
pub const CANDIDATE_PAGE_SIZE: usize = 7;
/// Fewer candidates fit next to the clause arrows while converting.
pub const CONVERSION_PAGE_SIZE: usize = 5;

const INK: Color32 = Color32::from_rgb(241, 245, 251);
const SECONDARY: Color32 = Color32::from_rgb(183, 199, 217);
const ACCENT: Color32 = Color32::from_rgb(159, 211, 255);
const DARK_INK: Color32 = Color32::from_rgb(14, 39, 61);

pub fn show(
    ui: &mut Ui,
    keyboard: &Keyboard,
    width: f32,
    destination: Destination,
    caret_visible: bool,
) -> Interaction {
    draw(ui, keyboard, width, destination, false, caret_visible).0
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
    caret_visible: bool,
) -> VrKeys {
    draw(
        ui,
        keyboard,
        SIZE[0] as f32,
        destination,
        true,
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
    pub fn position(&self, [u, v]: [f32; 2]) -> Pos2 {
        self.rect.min + Vec2::new(u * self.rect.width(), v * self.rect.height())
    }

    pub fn action_at(&self, uv: [f32; 2]) -> Option<Action> {
        let position = self.position(uv);
        if let Some(editor) = &self.editor
            && editor.area.contains(position)
        {
            return Some(editor.action(position));
        }
        self.keys
            .iter()
            .filter(|(rect, _)| rect.expand(2.).contains(position))
            .min_by(|(a, _), (b, _)| {
                a.distance_sq_to_pos(position)
                    .total_cmp(&b.distance_sq_to_pos(position))
            })
            .map(|(_, action)| action.clone())
    }
}

fn draw(
    ui: &mut Ui,
    keyboard: &Keyboard,
    width: f32,
    destination: Destination,
    controller_input: bool,
    caret_visible: bool,
) -> (Interaction, VrKeys) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, width / ASPECT), Sense::hover());
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
        controller_input,
        keys: Vec::new(),
        editor: None,
    };
    view.background();
    view.header(keyboard, controller_input);
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
    controller_input: bool,
    keys: Vec<(Rect, Action)>,
    editor: Option<EditorHit>,
}

#[derive(Clone, Copy)]
enum Face {
    Key,
    Subtle,
    Selected,
    Primary,
    Disabled,
}

impl Board<'_> {
    fn box_at(&self, x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(
            self.rect.min + Vec2::new(x, y) * self.scale,
            Vec2::new(width, height) * self.scale,
        )
    }

    fn background(&self) {
        let painter = self.ui.painter();
        let panel = self.box_at(4., 4., 852., 416.);
        let radius = 24. * self.scale;
        painter.rect_filled(
            panel,
            radius,
            Color32::from_rgba_unmultiplied(25, 33, 46, 238),
        );
        gradient(
            painter,
            panel.shrink(2. * self.scale),
            radius - 2. * self.scale,
        );
        painter.rect_stroke(
            panel,
            radius,
            Stroke::new(1.1, Color32::from_white_alpha(69)),
            StrokeKind::Inside,
        );
        painter.line_segment(
            [
                self.box_at(37., 5., 1., 1.).left_top(),
                self.box_at(385., 5., 1., 1.).left_top(),
            ],
            Stroke::new(1., Color32::from_white_alpha(33)),
        );
    }

    fn header(&mut self, keyboard: &Keyboard, can_close: bool) {
        let wordmark = brand::wordmark_texture(self.ui);
        let size = wordmark.size_vec2();
        self.ui.painter().image(
            wordmark.id(),
            self.box_at(24., 26., 18. * size.x / size.y, 18.),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
            Color32::WHITE,
        );
        let track = self.box_at(256., 15., 348., 40.);
        self.ui
            .painter()
            .rect_filled(track, 10. * self.scale, Color32::from_black_alpha(59));
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
    }

    fn editor(&mut self, keyboard: &Keyboard, destination: Destination, caret_visible: bool) {
        const TOP: f32 = 74.;
        const HEIGHT: f32 = 47.;
        let rect = self.box_at(22., TOP, 616., HEIGHT);
        let painter = self.ui.painter();
        painter.rect(
            rect,
            12. * self.scale,
            Color32::from_black_alpha(73),
            Stroke::new(1., Color32::from_white_alpha(37)),
            StrokeKind::Inside,
        );
        let (before, after) = keyboard.composer.committed_parts();
        let preedit = keyboard.composer.preedit();
        let mut job = LayoutJob::default();
        let font = FontId::proportional(24. * self.scale);
        job.append(before, 0., TextFormat::simple(font.clone(), INK));
        for (text, focused) in keyboard.composer.preedit_segments() {
            // The clause being converted is highlighted like in a desktop IME.
            let format = if focused {
                TextFormat {
                    background: Color32::from_rgba_unmultiplied(111, 181, 237, 96),
                    underline: Stroke::new(2.5 * self.scale, ACCENT),
                    ..TextFormat::simple(font.clone(), INK)
                }
            } else {
                TextFormat {
                    underline: Stroke::new(1.5 * self.scale, ACCENT),
                    ..TextFormat::simple(font.clone(), ACCENT)
                }
            };
            job.append(&text, 0., format);
        }
        job.append(after, 0., TextFormat::simple(font.clone(), INK));
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
        text_painter.galley(origin, galley.clone(), INK);
        if empty {
            self.label(
                text_area.translate(Vec2::new(7. * self.scale, 0.)),
                Align2::LEFT_CENTER,
                tr("メッセージを入力"),
                24.,
                SECONDARY,
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
                Stroke::new(1.8 * self.scale, ACCENT),
            );
        }

        let hit = EditorHit {
            area: text_area,
            origin,
            galley,
        };
        if self.controller_input {
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
                    Color32::LIGHT_RED
                } else {
                    SECONDARY
                },
            );
        }
        if !keyboard.status.is_empty() {
            self.label(
                self.box_at(36., TOP + HEIGHT + 1., 588., 19.),
                Align2::LEFT_CENTER,
                &keyboard.status,
                14.,
                ACCENT,
            );
        }
        let divider = self.box_at(718., TOP + 5., 0., HEIGHT - 10.);
        painter.line_segment(
            [divider.min, divider.max],
            Stroke::new(self.scale, Color32::from_white_alpha(37)),
        );
        let clear = self.box_at(601., TOP + 7.5, 28., 32.);
        let (hovered, pressed) =
            self.button_interaction(clear, tr("全消去"), !empty, Action::Clear);
        let face = if empty { Face::Disabled } else { Face::Subtle };
        let (_, _, color) = face.colors(hovered, pressed);
        self.label(
            clear,
            Align2::CENTER_CENTER,
            "×",
            if pressed { 17. } else { 20. },
            color,
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
                SECONDARY,
            );
            return;
        }
        let mut x = 22.;
        if let Some(word) = correction {
            self.button(
                self.box_at(x, 141., 103., 36.),
                &format!("✓ {word}"),
                17.,
                Face::Selected,
                Action::CorrectTypo(word.to_owned()),
            );
            x += 110.;
        }
        if let Some(conversion) = keyboard.composer.conversion() {
            // Clause navigation, like the arrow keys of a desktop IME.
            let last = conversion.segments().len() - 1;
            for (label, step, enabled) in [
                ("←", -1, conversion.focus() > 0),
                ("→", 1, conversion.focus() < last),
            ] {
                self.button(
                    self.box_at(x, 141., 40., 36.),
                    label,
                    19.,
                    if enabled {
                        Face::Subtle
                    } else {
                        Face::Disabled
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
                    .layout_no_wrap(value.clone(), FontId::proportional(17. * self.scale), INK)
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
            self.button(
                self.box_at(799., 141., 38., 36.),
                "›",
                24.,
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
        let mut top = self.jis_rect(1, 13.5, 1.5);
        let bottom = self.jis_rect(2, 13.75, 1.25);
        top.max.y = bottom.min.y;
        let enabled = !matches!(face, Face::Disabled);
        let label = Icon::Enter.label();
        let (top_hover, top_press) = self.button_interaction(top, label, enabled, Action::Enter);
        let (bottom_hover, bottom_press) =
            self.button_interaction(bottom, label, enabled, Action::Enter);
        let (fill, border, text) =
            face.colors(top_hover || bottom_hover, top_press || bottom_press);
        // One L-shaped outline with every corner rounded like the other keys.
        // The shape is star-shaped around a point in the overlap of both parts,
        // so a triangle fan fills it without seams between semi-transparent parts.
        let outline = rounded_outline(
            &[
                top.left_top(),
                top.right_top(),
                bottom.right_bottom(),
                bottom.left_bottom(),
                bottom.left_top(),
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
        painter.add(Shape::closed_line(outline, Stroke::new(1., border)));
        Icon::Enter.paint(
            self.ui,
            Pos2::new(bottom.center().x, (top.top() + bottom.bottom()) / 2.),
            24. * self.scale,
            text,
        );
    }

    fn emoji_toggle(&mut self, open: bool) {
        self.button(
            self.box_at(22., 364., 60., 40.),
            "😃",
            22.,
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
            SECONDARY,
        );
        self.button(
            self.box_at(331., 364., 49., 40.),
            "‹",
            24.,
            Face::Key,
            Action::PageEmoji(-1),
        );
        self.label(
            self.box_at(387., 364., 86., 40.),
            Align2::CENTER_CENTER,
            &format!("{} / {total_pages}", keyboard.emoji_page + 1),
            16.,
            INK,
        );
        self.button(
            self.box_at(480., 364., 49., 40.),
            "›",
            24.,
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
        if self.controller_input {
            if enabled {
                self.keys.push((rect, action));
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
        let (painted, text) = self.button_face(rect, label, face, action);
        self.label(
            painted.shrink(4. * self.scale),
            Align2::CENTER_CENTER,
            label,
            size,
            text,
        );
    }

    fn icon_button(&mut self, rect: Rect, icon: Icon, face: Face, action: Action) {
        let (painted, text) = self.button_face(rect, icon.label(), face, action);
        icon.paint(self.ui, painted.center(), 24. * self.scale, text);
    }

    fn button_face(
        &mut self,
        rect: Rect,
        label: &str,
        face: Face,
        action: Action,
    ) -> (Rect, Color32) {
        let (hovered, pressed) =
            self.button_interaction(rect, label, !matches!(face, Face::Disabled), action);
        let (fill, border, text) = face.colors(hovered, pressed);
        let painted = if pressed {
            rect.shrink(1. * self.scale)
        } else {
            rect
        };
        self.ui.painter().rect(
            painted,
            8. * self.scale,
            fill,
            Stroke::new(1., border),
            StrokeKind::Inside,
        );
        (painted, text)
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
    fn colors(self, hovered: bool, pressed: bool) -> (Color32, Color32, Color32) {
        match self {
            Face::Disabled => (
                Color32::from_white_alpha(8),
                Color32::from_white_alpha(19),
                Color32::from_rgb(137, 151, 167),
            ),
            Face::Primary => (
                if pressed {
                    Color32::from_rgb(116, 183, 235)
                } else if hovered {
                    Color32::from_rgb(187, 226, 255)
                } else {
                    ACCENT
                },
                Color32::from_white_alpha(150),
                DARK_INK,
            ),
            Face::Selected => (
                Color32::from_rgba_unmultiplied(
                    111,
                    181,
                    237,
                    if pressed {
                        112
                    } else if hovered {
                        90
                    } else {
                        67
                    },
                ),
                Color32::from_rgba_unmultiplied(143, 205, 252, 102),
                INK,
            ),
            Face::Subtle | Face::Key => (
                Color32::from_white_alpha(if pressed {
                    67
                } else if hovered {
                    45
                } else if matches!(self, Face::Key) {
                    23
                } else {
                    0
                }),
                Color32::from_white_alpha(if hovered {
                    86
                } else if matches!(self, Face::Key) {
                    37
                } else {
                    0
                }),
                if matches!(self, Face::Key) || hovered {
                    INK
                } else {
                    SECONDARY
                },
            ),
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

fn gradient(painter: &Painter, rect: Rect, radius: f32) {
    // A softly lit interior. The edge stays clear over both dark and light VR scenes.
    let tint = |point: Pos2| {
        let x = (point.x - rect.left()) / rect.width();
        let y = (point.y - rect.top()) / rect.height();
        let alpha = (43. - 24. * y - 11. * x).round() as u8;
        Color32::from_rgba_unmultiplied(156, 192, 229, alpha)
    };
    let corners = [
        rect.left_top() + Vec2::splat(radius),
        rect.right_top() + Vec2::new(-radius, radius),
        rect.right_bottom() - Vec2::splat(radius),
        rect.left_bottom() + Vec2::new(radius, -radius),
    ];
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.center(), tint(rect.center()));
    for (index, center) in corners.into_iter().enumerate() {
        for step in 0..=6 {
            let angle = (2. + index as f32 + step as f32 / 6.) * std::f32::consts::FRAC_PI_2;
            let point = center + radius * Vec2::angled(angle);
            mesh.colored_vertex(point, tint(point));
        }
    }
    let perimeter = mesh.vertices.len() as u32 - 1;
    for index in 0..perimeter {
        mesh.add_triangle(0, index + 1, (index + 1) % perimeter + 1);
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay_surface::pointer_button;

    #[test]
    fn long_candidates_share_the_row_and_leave_the_page_button_reachable() {
        let context = egui::Context::default();
        let mut keyboard = Keyboard::new(context.clone());
        keyboard.candidates = (0..12)
            .map(|index| format!("candidate {index} with a long description"))
            .collect();
        keyboard.candidates[2] = "a".into();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let keys = show_vr(ui, &keyboard, Destination::Osc, true);
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
    fn vr_editor_actions_are_separate_from_the_text_hit_area() {
        let context = egui::Context::default();
        let mut keyboard = Keyboard::new(context.clone());
        let hit = |keys: &VrKeys, x, y| keys.action_at([x / SIZE[0] as f32, y / SIZE[1] as f32]);
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let keys = show_vr(ui, &keyboard, Destination::Osc, true);
            assert!(hit(&keys, 615., 97.).is_none());
            assert!(hit(&keys, 679., 97.).is_none());
            assert!(hit(&keys, 754., 97.).is_none());
            assert!(matches!(hit(&keys, 813., 97.), Some(Action::Paste)));
        });
        output.textures_delta.clear();
        keyboard.apply(Action::Key('a'));
        let mut clear = None;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let keys = show_vr(ui, &keyboard, Destination::Osc, true);
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
                let keys = show_vr(ui, &keyboard, Destination::Osc, true);
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
                    result = show(ui, &keyboard, width, Destination::Osc, true);
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
