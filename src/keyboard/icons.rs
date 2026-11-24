use eframe::{
    egui::{Color32, ColorImage, Id, Pos2, Rect, TextureOptions, Ui, Vec2},
    icon_data::from_png_bytes,
};
use std::sync::{Arc, LazyLock};
use vime::i18n::tr;

#[derive(Clone, Copy, Debug, Hash)]
pub(super) enum Icon {
    Send,
    Copy,
    Paste,
    Backspace,
    Enter,
}

static IMAGES: LazyLock<[Arc<ColorImage>; 5]> = LazyLock::new(|| {
    [
        include_bytes!("../../assets/keyboard/send.png").as_slice(),
        include_bytes!("../../assets/keyboard/content-copy.png").as_slice(),
        include_bytes!("../../assets/keyboard/content-paste.png").as_slice(),
        include_bytes!("../../assets/keyboard/backspace.png").as_slice(),
        include_bytes!("../../assets/keyboard/keyboard-return.png").as_slice(),
    ]
    .map(|bytes| {
        let png = from_png_bytes(bytes).expect("embedded Material icon is a PNG");
        Arc::new(ColorImage::from_rgba_unmultiplied(
            [png.width as usize, png.height as usize],
            &png.rgba,
        ))
    })
});

impl Icon {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Send => tr("送信"),
            Self::Copy => tr("コピー"),
            Self::Paste => tr("貼り付け"),
            Self::Backspace => "Backspace",
            Self::Enter => "Enter",
        }
    }

    pub(super) fn paint(self, ui: &Ui, center: Pos2, size: f32, tint: Color32) {
        let texture = ui.ctx().load_texture_cached(
            "Material keyboard icon",
            Id::unique(("keyboard-icon", self)),
            Arc::clone(&IMAGES[self as usize]),
            TextureOptions::LINEAR,
        );
        ui.painter().image(
            texture.id(),
            Rect::from_center_size(center, Vec2::splat(size)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
            tint,
        );
    }
}
