use eframe::egui::Color32;
use vime::settings::KeyboardTheme;

pub(super) struct Palette {
    pub panel: Color32,
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
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const LIGHT: Palette = Palette {
    panel: rgb(0xf6f8fa),
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
};

const DARK: Palette = Palette {
    panel: rgb(0x161b22),
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
};

impl Palette {
    pub fn for_theme(theme: KeyboardTheme) -> &'static Self {
        match theme {
            KeyboardTheme::Light => &LIGHT,
            KeyboardTheme::Dark => &DARK,
        }
    }
}
