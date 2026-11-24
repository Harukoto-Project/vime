use eframe::{
    egui::{
        Align, ColorImage, IconData, Id, Image, Layout, RichText, TextureHandle, TextureOptions, Ui,
    },
    icon_data::from_png_bytes,
};
use std::sync::{Arc, LazyLock};

use crate::update::VERSION_LABEL;

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
    ui.allocate_ui_with_layout(
        eframe::egui::vec2(0., height),
        Layout::left_to_right(Align::Max),
        |ui| {
            ui.spacing_mut().item_spacing.x = 5.;
            // The bitmap is light; tint it with the theme's text color so that it
            // also reads on a light background.
            let tint = ui.visuals().widgets.noninteractive.fg_stroke.color;
            ui.add(
                Image::new((texture.id(), size * (height / size.y)))
                    .tint(tint)
                    .alt_text("ViME"),
            );
            ui.add(
                eframe::egui::Label::new(RichText::new(VERSION_LABEL).size(10.).weak()).extend(),
            );
        },
    );
}
