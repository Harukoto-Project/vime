use eframe::{
    egui::{ColorImage, IconData, Id, Image, TextureHandle, TextureOptions, Ui},
    icon_data::from_png_bytes,
};
use std::sync::{Arc, LazyLock};

static ICON: LazyLock<Arc<IconData>> = LazyLock::new(|| {
    Arc::new(
        from_png_bytes(include_bytes!("../assets/icon.png")).expect("embedded ViME icon is a PNG"),
    )
});
static WORDMARK: LazyLock<Arc<ColorImage>> = LazyLock::new(|| {
    let png = from_png_bytes(include_bytes!("../assets/wordmark.png"))
        .expect("embedded ViME wordmark is a PNG");
    Arc::new(ColorImage::from_rgba_unmultiplied(
        [png.width as usize, png.height as usize],
        &png.rgba,
    ))
});

pub fn icon() -> Arc<IconData> {
    Arc::clone(&ICON)
}

pub fn wordmark_texture(ui: &Ui) -> TextureHandle {
    ui.ctx().load_texture_cached(
        "ViME wordmark",
        Id::unique("vime-wordmark"),
        Arc::clone(&WORDMARK),
        TextureOptions::LINEAR,
    )
}

pub fn wordmark(ui: &mut Ui, height: f32) {
    let texture = wordmark_texture(ui);
    let size = texture.size_vec2();
    ui.add(Image::new((texture.id(), size * (height / size.y))).alt_text("ViME"));
}
