use super::{Action, Keyboard};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, TextFormat, Ui, Vec2, text::LayoutJob,
};
use vime::{
    ime::{Language, hangul_key_label, search_emoji},
    output::CHAT_LIMIT,
    settings::Destination,
};

pub const SIZE: [u32; 2] = [860, 424];
pub const ASPECT: f32 = SIZE[0] as f32 / SIZE[1] as f32;
pub const EMOJI_PAGE_SIZE: usize = 40;
pub const CANDIDATE_PAGE_SIZE: usize = 7;

const INK: Color32 = Color32::from_rgb(241, 245, 251);
const SECONDARY: Color32 = Color32::from_rgb(183, 199, 217);
const ACCENT: Color32 = Color32::from_rgb(159, 211, 255);
const DARK_INK: Color32 = Color32::from_rgb(14, 39, 61);

pub fn show(
    ui: &mut Ui,
    keyboard: &Keyboard,
    width: f32,
    destination: Destination,
) -> Option<Action> {
    draw(ui, keyboard, width, destination, false).0
}

pub fn show_vr(ui: &mut Ui, keyboard: &Keyboard, destination: Destination) -> VrKeys {
    draw(ui, keyboard, SIZE[0] as f32, destination, true).1
}

pub struct VrKeys {
    rect: Rect,
    keys: Vec<(Rect, Action)>,
}

impl VrKeys {
    pub fn position(&self, [u, v]: [f32; 2]) -> Pos2 {
        self.rect.min + Vec2::new(u * self.rect.width(), v * self.rect.height())
    }

    pub fn action_at(&self, uv: [f32; 2]) -> Option<Action> {
        let position = self.position(uv);
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
) -> (Option<Action>, VrKeys) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, width / ASPECT), Sense::hover());
    let scale = rect.width() / SIZE[0] as f32;
    let mut view = Board {
        ui,
        rect,
        scale,
        action: None,
        controller_input,
        keys: Vec::new(),
    };
    view.background();
    view.header(keyboard, controller_input);
    view.editor(keyboard, destination);
    view.candidates(keyboard);
    if keyboard.emoji {
        view.emoji(keyboard);
    } else {
        view.keys(keyboard, destination);
    }
    (
        view.action,
        VrKeys {
            rect,
            keys: view.keys,
        },
    )
}

struct Board<'a> {
    ui: &'a mut Ui,
    rect: Rect,
    scale: f32,
    action: Option<Action>,
    controller_input: bool,
    keys: Vec<(Rect, Action)>,
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
        self.label(
            self.box_at(24., 18., 110., 32.),
            Align2::LEFT_CENTER,
            "ViME",
            24.,
            INK,
        );
        let track = self.box_at(248., 15., 348., 40.);
        self.ui
            .painter()
            .rect_filled(track, 10. * self.scale, Color32::from_black_alpha(59));
        for (index, language) in Language::ALL.into_iter().enumerate() {
            self.button(
                self.box_at(251. + index as f32 * 114., 18., 114., 34.),
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
        self.button(
            self.box_at(if can_close { 664. } else { 746. }, 15., 91., 40.),
            "絵文字",
            16.,
            if keyboard.emoji {
                Face::Selected
            } else {
                Face::Key
            },
            Action::ToggleEmoji,
        );
        if can_close {
            self.button(
                self.box_at(766., 15., 71., 40.),
                "閉じる",
                16.,
                Face::Key,
                Action::Close,
            );
        }
    }

    fn editor(&mut self, keyboard: &Keyboard, destination: Destination) {
        let japanese = keyboard.composer.language() == Language::Japanese;
        let rect = self.box_at(22., 63., 816., 69.);
        let painter = self.ui.painter();
        painter.rect(
            rect,
            12. * self.scale,
            Color32::from_black_alpha(73),
            Stroke::new(1., Color32::from_white_alpha(37)),
            StrokeKind::Inside,
        );
        let text = keyboard.composer.committed();
        let preedit = keyboard.composer.preedit();
        let mut job = LayoutJob::default();
        let font = FontId::proportional(24. * self.scale);
        if text.is_empty() && preedit.is_empty() {
            job.append(
                "メッセージを入力",
                0.,
                TextFormat::simple(font.clone(), SECONDARY),
            );
        } else {
            job.append(text, 0., TextFormat::simple(font.clone(), INK));
            job.append(
                &preedit,
                0.,
                TextFormat {
                    underline: Stroke::new(1.5 * self.scale, ACCENT),
                    ..TextFormat::simple(font, ACCENT)
                },
            );
        }
        let text_area = self.box_at(36., 70., if japanese { 568. } else { 787. }, 38.);
        let text_painter = painter.with_clip_rect(text_area);
        let galley = painter.layout_job(job);
        let x = text_area.left() - (galley.size().x - text_area.width()).max(0.);
        text_painter.galley(
            Pos2::new(x, text_area.center().y - galley.size().y * 0.5),
            galley,
            INK,
        );
        if destination == Destination::Osc {
            let count = text.encode_utf16().count() + preedit.encode_utf16().count();
            self.label(
                self.box_at(if japanese { 525. } else { 744. }, 107., 79., 18.),
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
        let route = match destination {
            Destination::Osc => "送信先  VRChat Chatbox",
            Destination::SteamVrInput => "送信先  VRアプリの入力欄",
        };
        let (detail, color) = if keyboard.status.is_empty() {
            (route, SECONDARY)
        } else {
            (keyboard.status.as_str(), ACCENT)
        };
        self.label(
            self.box_at(36., 106., if japanese { 480. } else { 695. }, 21.),
            Align2::LEFT_CENTER,
            detail,
            14.,
            color,
        );
        if japanese {
            let text = keyboard.composer.text();
            let can_clear = !text.is_empty();
            self.button(
                self.box_at(620., 77., 91., 39.),
                "全消去",
                15.,
                if can_clear { Face::Key } else { Face::Disabled },
                Action::Clear,
            );
            self.button(
                self.box_at(720., 77., 104., 39.),
                "送信 ↗",
                16.,
                send_face(&text, destination),
                Action::Send,
            );
        }
    }

    fn candidates(&mut self, keyboard: &Keyboard) {
        let values = &keyboard.candidates;
        if values.is_empty() {
            let hint = match keyboard.composer.language() {
                Language::Japanese if keyboard.japanese_direct() => {
                    "半角英数字 · 半角/全角で日本語入力に切り替え"
                }
                Language::Japanese => "ローマ字で入力 · 半角/全角で半角英数字に切り替え",
                Language::English => "絵文字は一覧または :cat: などの短縮名から選択",
                Language::Korean => "2ボル式で入力",
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
        let first = keyboard.candidate_page * CANDIDATE_PAGE_SIZE;
        for value in values.iter().skip(first).take(CANDIDATE_PAGE_SIZE) {
            let measured = self
                .ui
                .painter()
                .layout_no_wrap(value.clone(), FontId::proportional(17. * self.scale), INK)
                .size()
                .x;
            let width = (measured / self.scale + 28.).clamp(50., 103.);
            self.button(
                self.box_at(x, 141., width, 36.),
                value,
                17.,
                Face::Key,
                Action::Candidate(value.clone()),
            );
            x += width + 7.;
        }
        if values.len() > CANDIDATE_PAGE_SIZE {
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
        let shift = keyboard.shift;
        let lang = keyboard.composer.language();
        let text = keyboard.composer.text();
        let has_text = !text.is_empty();
        if lang == Language::Japanese {
            self.jis_keys(keyboard, has_text);
            return;
        }
        let numbers = if shift {
            "!@#$%^&*()_+"
        } else {
            "1234567890-="
        };
        self.row(
            188.,
            numbers
                .chars()
                .map(|key| (key.to_string(), 1., Action::Key(key), Face::Key)),
        );
        self.row(232., letters("qwertyuiop[]", "QWERTYUIOP{}", shift, lang));
        let mut middle: Vec<_> = letters("asdfghjkl;'", "ASDFGHJKL:\"", shift, lang).collect();
        middle.push((
            "削除".to_owned(),
            1.4,
            Action::Backspace,
            if has_text { Face::Key } else { Face::Disabled },
        ));
        self.row(276., middle);
        let shift_key = (
            "Shift".to_owned(),
            1.45,
            Action::Shift,
            if shift { Face::Selected } else { Face::Key },
        );
        let lower = std::iter::once(shift_key.clone())
            .chain(letters("zxcvbnm,./", "ZXCVBNM<>?", shift, lang))
            .chain(std::iter::once(shift_key));
        self.row(320., lower);
        self.row(
            364.,
            [
                (
                    "全消去".into(),
                    1.2,
                    Action::Clear,
                    if has_text {
                        Face::Subtle
                    } else {
                        Face::Disabled
                    },
                ),
                (
                    "確定".into(),
                    1.35,
                    Action::Commit,
                    if keyboard.composer.preedit().is_empty() {
                        Face::Disabled
                    } else {
                        Face::Key
                    },
                ),
                ("スペース".into(), 4.3, Action::Space, Face::Key),
                (
                    "送信 ↗".into(),
                    2.,
                    Action::Send,
                    send_face(&text, destination),
                ),
            ],
        );
    }

    // The 109-key JIS main block is 15 key units wide. Empty modifier
    // positions keep their physical pitch, and 1u keys are square.
    fn jis_rect(&self, row: usize, column: f32, units: f32) -> Rect {
        const PITCH: f32 = 44.;
        self.box_at(
            100. + column * PITCH,
            188. + row as f32 * PITCH,
            units * PITCH - 4.,
            PITCH - 4.,
        )
    }

    fn jis_chars(&mut self, row: usize, start: f32, chars: &str) {
        for (offset, key) in chars.chars().enumerate() {
            self.button(
                self.jis_rect(row, start + offset as f32, 1.),
                &key.to_string(),
                20.,
                Face::Key,
                Action::Key(key),
            );
        }
    }

    fn jis_keys(&mut self, keyboard: &Keyboard, has_text: bool) {
        let shift = keyboard.shift;
        let direct = keyboard.japanese_direct();
        let composing = !keyboard.composer.preedit().is_empty();
        let shift_face = if shift { Face::Selected } else { Face::Key };
        let commit_face = if composing { Face::Key } else { Face::Disabled };

        self.button(
            self.jis_rect(0, 0., 1.),
            "半/全",
            12.,
            if direct { Face::Selected } else { Face::Key },
            Action::ToggleJapaneseWidth,
        );
        self.jis_chars(
            0,
            1.,
            if shift {
                "!\"#$%&'()0=~|"
            } else {
                "1234567890-^¥"
            },
        );
        self.button(
            self.jis_rect(0, 14., 1.),
            "BS",
            14.,
            if has_text { Face::Key } else { Face::Disabled },
            Action::Backspace,
        );

        // Tab occupies the empty 1.5u before Q. The two-row Enter shares the
        // final 1.5u of this row and the final 1.25u of the row below.
        self.jis_chars(
            1,
            1.5,
            if shift {
                "QWERTYUIOP`{"
            } else {
                "qwertyuiop@["
            },
        );
        self.button(
            self.jis_rect(2, 0., 1.75),
            "英数",
            15.,
            if direct { Face::Selected } else { Face::Key },
            Action::JapaneseDirect(true),
        );
        self.jis_chars(
            2,
            1.75,
            if shift {
                "ASDFGHJKL+*}"
            } else {
                "asdfghjkl;:]"
            },
        );
        self.jis_enter(commit_face);

        self.button(
            self.jis_rect(3, 0., 2.25),
            "Shift",
            16.,
            shift_face,
            Action::Shift,
        );
        self.jis_chars(3, 2.25, if shift { "ZXCVBNM<>?_" } else { "zxcvbnm,./\\" });
        self.button(
            self.jis_rect(3, 13.25, 1.75),
            "Shift",
            16.,
            shift_face,
            Action::Shift,
        );

        self.button(
            self.jis_rect(4, 3.75, 1.25),
            "無変換",
            13.,
            commit_face,
            Action::Commit,
        );
        self.button(
            self.jis_rect(4, 5., 2.5),
            "スペース",
            16.,
            Face::Key,
            Action::Space,
        );
        self.button(
            self.jis_rect(4, 7.5, 1.25),
            "変換",
            14.,
            if keyboard.candidates.is_empty() {
                Face::Disabled
            } else {
                Face::Key
            },
            Action::ConvertJapanese,
        );
        self.button(
            self.jis_rect(4, 8.75, 1.25),
            "かな",
            14.,
            if direct { Face::Key } else { Face::Selected },
            Action::JapaneseDirect(false),
        );
    }

    fn jis_enter(&mut self, face: Face) {
        let mut top = self.jis_rect(1, 13.5, 1.5);
        let bottom = self.jis_rect(2, 13.75, 1.25);
        top.max.y = bottom.min.y;
        let enabled = !matches!(face, Face::Disabled);
        let (top_hover, top_press) = self.button_interaction(top, "Enter", enabled, Action::Commit);
        let (bottom_hover, bottom_press) =
            self.button_interaction(bottom, "Enter", enabled, Action::Commit);
        let (fill, border, text) =
            face.colors(top_hover || bottom_hover, top_press || bottom_press);
        let painter = self.ui.painter();
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(top, fill);
        mesh.add_colored_rect(bottom, fill);
        painter.add(Shape::mesh(mesh));
        painter.add(Shape::closed_line(
            vec![
                top.left_top(),
                top.right_top(),
                bottom.right_bottom(),
                bottom.left_bottom(),
                bottom.left_top(),
                top.left_bottom(),
            ],
            Stroke::new(self.scale, border),
        ));
        self.label(
            bottom.shrink(3. * self.scale),
            Align2::CENTER_CENTER,
            "Enter",
            14.,
            text,
        );
    }

    fn row(&mut self, y: f32, keys: impl IntoIterator<Item = (String, f32, Action, Face)>) {
        let keys: Vec<_> = keys.into_iter().collect();
        let gap = 6.;
        let unit =
            (816. - gap * (keys.len() - 1) as f32) / keys.iter().map(|key| key.1).sum::<f32>();
        let mut x = 22.;
        for (label, weight, action, face) in keys {
            let width = unit * weight;
            let font_size = if label.chars().count() > 2 { 16. } else { 21. };
            self.button(
                self.box_at(x, y, width, 40.),
                &label,
                font_size,
                face,
                action,
            );
            x += width + gap;
        }
    }

    fn emoji(&mut self, keyboard: &Keyboard) {
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
            self.box_at(24., 364., 220., 38.),
            Align2::LEFT_CENTER,
            "絵文字・記号",
            15.,
            SECONDARY,
        );
        self.button(
            self.box_at(331., 364., 49., 38.),
            "‹",
            24.,
            Face::Key,
            Action::PageEmoji(-1),
        );
        self.label(
            self.box_at(387., 364., 86., 38.),
            Align2::CENTER_CENTER,
            &format!("{} / {total_pages}", keyboard.emoji_page + 1),
            16.,
            INK,
        );
        self.button(
            self.box_at(480., 364., 49., 38.),
            "›",
            24.,
            Face::Key,
            Action::PageEmoji(1),
        );
        self.button(
            self.box_at(716., 364., 122., 38.),
            "キーボード",
            15.,
            Face::Key,
            Action::ToggleEmoji,
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
            let id = self.ui.scope_id().with((
                (rect.left() - self.rect.left()) as i32,
                (rect.top() - self.rect.top()) as i32,
                label,
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
            if response.clicked() {
                self.action = Some(action);
            }
            (response.hovered(), response.is_pointer_button_down_on())
        }
    }

    fn button(&mut self, rect: Rect, label: &str, size: f32, face: Face, action: Action) {
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
        self.label(
            painted.shrink(4. * self.scale),
            Align2::CENTER_CENTER,
            label,
            size,
            text,
        );
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
        self.ui.painter().with_clip_rect(rect).text(
            point,
            align,
            text,
            FontId::proportional(size * self.scale),
            color,
        );
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
                Color32::from_rgba_unmultiplied(111, 181, 237, 67),
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

fn send_face(text: &str, destination: Destination) -> Face {
    if !text.trim().is_empty()
        && (destination != Destination::Osc || text.encode_utf16().count() <= CHAT_LIMIT)
    {
        Face::Primary
    } else {
        Face::Disabled
    }
}

fn letters<'a>(
    normal: &'a str,
    shifted: &'a str,
    shift: bool,
    lang: Language,
) -> impl Iterator<Item = (String, f32, Action, Face)> + 'a {
    let keys = if shift { shifted } else { normal };
    keys.chars().map(move |key| {
        let label = if lang == Language::Korean {
            hangul_key_label(key).unwrap_or(key)
        } else {
            key
        };
        (label.to_string(), 1., Action::Key(key), Face::Key)
    })
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
