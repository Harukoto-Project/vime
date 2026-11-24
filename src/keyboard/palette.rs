use eframe::egui::{
    self, Color32, CornerRadius, Shadow, Stroke, Visuals,
    style::{ScrollStyle, Selection, WidgetVisuals},
    vec2,
};
use vime::settings::KeyboardTheme;

/// The flat colors shared by the keyboard, the SteamVR dashboard and the
/// desktop window. The theme chosen in Settings picks one of the two palettes.
pub(crate) struct Palette {
    pub dark: bool,
    pub panel: Color32,
    /// Raised surfaces such as the cards of the desktop window.
    pub card: Color32,
    pub input: Color32,
    pub track: Color32,
    pub key: Color32,
    pub border: Color32,
    pub input_border: Color32,
    pub ink: Color32,
    pub secondary: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub selected: Color32,
    pub selected_border: Color32,
    pub selected_ink: Color32,
    pub clause: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub hover_border: Color32,
    pub disabled: Color32,
    pub disabled_border: Color32,
    pub disabled_ink: Color32,
    pub correction: Color32,
    pub correction_border: Color32,
    pub correction_ink: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub warning_ink: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const LIGHT: Palette = Palette {
    dark: false,
    panel: rgb(0xf6f8fa),
    card: Color32::WHITE,
    input: Color32::WHITE,
    track: rgb(0xe6eaf0),
    key: Color32::WHITE,
    border: rgb(0xd3dae3),
    input_border: rgb(0xc5cfdb),
    ink: rgb(0x17202b),
    secondary: rgb(0x5d6a79),
    accent: rgb(0x1f6fe5),
    on_accent: Color32::WHITE,
    selected: rgb(0xe2edfd),
    selected_border: rgb(0x8cb8ff),
    selected_ink: rgb(0x104eae),
    clause: rgb(0xd1e1fa),
    hover: rgb(0xedf3f9),
    pressed: rgb(0xdce4ee),
    hover_border: rgb(0x9eb3ca),
    disabled: rgb(0xf0f2f5),
    disabled_border: rgb(0xe0e5eb),
    disabled_ink: rgb(0x8793a2),
    correction: rgb(0xe2f4e8),
    correction_border: rgb(0x74c995),
    correction_ink: rgb(0x146939),
    error: rgb(0xae242c),
    warning: rgb(0xfff3d6),
    warning_ink: rgb(0x7a4b00),
};

const DARK: Palette = Palette {
    dark: true,
    panel: rgb(0x161b22),
    card: rgb(0x161b22),
    input: rgb(0x11151a),
    track: rgb(0x11151a),
    key: rgb(0x222934),
    border: rgb(0x2f3744),
    input_border: rgb(0x2f3744),
    ink: rgb(0xeef2f7),
    secondary: rgb(0x98a5b4),
    accent: rgb(0x8cc8ff),
    on_accent: rgb(0x10283c),
    selected: rgb(0x1b3450),
    selected_border: rgb(0x396d9f),
    selected_ink: rgb(0xe4f0ff),
    clause: rgb(0x203e60),
    hover: rgb(0x2b3340),
    pressed: rgb(0x37414e),
    hover_border: rgb(0x566c84),
    disabled: rgb(0x1a1f26),
    disabled_border: rgb(0x282f39),
    disabled_ink: rgb(0x697686),
    correction: rgb(0x14341f),
    correction_border: rgb(0x2d9258),
    correction_ink: rgb(0xa4eac1),
    error: rgb(0xffb1a8),
    warning: rgb(0x3d2d19),
    warning_ink: rgb(0xffdea0),
};

impl Palette {
    pub fn for_theme(theme: KeyboardTheme) -> &'static Self {
        match theme {
            KeyboardTheme::Light => &LIGHT,
            KeyboardTheme::Dark => &DARK,
        }
    }

    /// Makes this palette the look of every widget drawn in `context`, and what
    /// [`Palette::of`] returns for it.
    pub fn install(&'static self, context: &egui::Context) {
        context.set_visuals(self.visuals());
        context.global_style_mut(|style| {
            style.spacing.item_spacing = vec2(10., 8.);
            style.spacing.button_padding = vec2(14., 5.);
            style.spacing.interact_size = vec2(40., 30.);
            style.spacing.combo_width = 0.;
            style.spacing.scroll = ScrollStyle::floating();
        });
        context.data_mut(|data| data.insert_temp(palette_id(), self));
    }

    /// The palette installed in `context`; dark until one is installed.
    pub fn of(context: &egui::Context) -> &'static Self {
        context
            .data(|data| data.get_temp::<&'static Self>(palette_id()))
            .unwrap_or(&DARK)
    }

    fn visuals(&self) -> Visuals {
        let mut visuals = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        visuals.override_text_color = None;
        visuals.weak_text_color = Some(self.secondary);
        visuals.hyperlink_color = self.accent;
        visuals.error_fg_color = self.error;
        visuals.warn_fg_color = self.warning_ink;
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.card;
        visuals.window_stroke = Stroke::new(1., self.border);
        visuals.extreme_bg_color = self.input;
        visuals.faint_bg_color = self.hover;
        visuals.code_bg_color = self.track;
        // Selected buttons keep the frame of an unselected one; their text takes
        // `selection.stroke`, which also outlines a focused text field.
        visuals.selection = Selection {
            bg_fill: self.selected,
            stroke: Stroke::new(1., self.selected_ink),
        };
        visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
        visuals.striped = false;
        visuals.window_corner_radius = CornerRadius::same(12);
        visuals.menu_corner_radius = CornerRadius::same(10);
        let shadow = Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(if self.dark { 120 } else { 36 }),
        };
        visuals.popup_shadow = shadow;
        visuals.window_shadow = shadow;

        let ink = Stroke::new(1., self.ink);
        let radius = CornerRadius::same(8);
        let widget = |fill: Color32, rail: Color32, border: Color32| WidgetVisuals {
            weak_bg_fill: fill,
            bg_fill: rail,
            bg_stroke: Stroke::new(1., border),
            corner_radius: radius,
            fg_stroke: ink,
            expansion: 0.,
        };
        let widgets = &mut visuals.widgets;
        widgets.noninteractive = widget(self.card, self.card, self.border);
        widgets.inactive = widget(self.key, self.track, self.input_border);
        widgets.hovered = widget(self.hover, self.hover, self.hover_border);
        widgets.active = widget(self.pressed, self.pressed, self.hover_border);
        widgets.open = widget(self.hover, self.hover, self.hover_border);
        visuals
    }
}

fn palette_id() -> egui::Id {
    egui::Id::unique("vime-palette")
}
