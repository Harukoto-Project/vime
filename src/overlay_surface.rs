use eframe::{
    egui::{self, Event, FontDefinitions, Pos2, RawInput, Rect, Ui, Vec2},
    glow::{self, HasContext},
};
use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};
use crate::keyboard::palette::Palette;
use vime::i18n::tr;

const RETRY_DELAY: Duration = Duration::from_millis(250);

pub struct OverlaySurface {
    context: egui::Context,
    painter: egui_glow::Painter,
    gl: Arc<glow::Context>,
    target: Target,
    texture_size: [u32; 2],
    pixels_per_point: f32,
    pending: bool,
    retry_after: Option<Instant>,
}

struct Target {
    framebuffer: glow::Framebuffer,
    texture: glow::Texture,
}

impl Target {
    fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.texture);
        }
    }
}

impl OverlaySurface {
    pub fn new(
        gl: Arc<glow::Context>,
        fonts: FontDefinitions,
        palette: &'static Palette,
        logical_size: [u32; 2],
        pixels_per_point: f32,
    ) -> Result<Self, String> {
        let mut painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|error| error.to_string())?;
        let texture_size = logical_size.map(|axis| (axis as f32 * pixels_per_point).round() as u32);
        let target = match make_target(&gl, texture_size) {
            Ok(target) => target,
            Err(error) => {
                painter.destroy();
                return Err(error);
            }
        };
        let context = egui::Context::default();
        context.set_fonts(fonts);
        palette.install(&context);
        context.options_mut(|options| {
            options.max_passes = std::num::NonZeroUsize::MIN;
            // Hand-held dashboard rays move slightly while a trigger is squeezed.
            options.input_options.max_click_dist = 14.;
        });

        Ok(Self {
            context,
            painter,
            gl,
            target,
            texture_size,
            pixels_per_point,
            pending: false,
            retry_after: None,
        })
    }

    pub fn context(&self) -> &egui::Context {
        &self.context
    }

    pub fn render<T>(&mut self, events: Vec<Event>, mut draw: impl FnMut(&mut Ui) -> T) -> T {
        let input = overlay_input(self.texture_size, self.pixels_per_point, events);
        let mut result = None;
        let mut output = self.context.run_ui(input, |ui| result = Some(draw(ui)));
        let shapes = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);

        // The texture stays on the GPU and belongs to the current eframe GL context.
        unsafe {
            let draw = self
                .gl
                .get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            let read = self
                .gl
                .get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(self.target.framebuffer));
            self.painter.clear(self.texture_size, [0.0; 4]);
            self.painter.paint_and_update_textures(
                self.texture_size,
                output.pixels_per_point,
                &shapes,
                &mut output.textures_delta,
            );
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        }
        self.pending = true;
        result.expect("egui invokes the overlay draw callback")
    }

    pub fn can_present(&self, now: Instant) -> bool {
        self.present_delay(now).is_zero()
    }

    pub fn present_delay(&self, now: Instant) -> Duration {
        self.retry_after
            .map_or(Duration::ZERO, |retry| retry.saturating_duration_since(now))
    }

    pub fn reset(&mut self) {
        self.pending = false;
        self.retry_after = None;
    }

    pub fn present(
        &mut self,
        submit: impl FnOnce(NonZeroU32) -> Result<(), String>,
    ) -> Option<Result<(), String>> {
        let now = Instant::now();
        if !self.pending || !self.can_present(now) {
            return None;
        }
        let result = unsafe {
            let draw = self
                .gl
                .get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            let read = self
                .gl
                .get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
            self.gl.flush();
            let result = submit(self.target.texture.0);
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
            result
        };
        match &result {
            Ok(()) => {
                self.pending = false;
                self.retry_after = None;
            }
            Err(_) => self.retry_after = Some(Instant::now() + RETRY_DELAY),
        }
        Some(result)
    }

    pub fn destroy(mut self) {
        self.painter.destroy();
        self.target.destroy(&self.gl);
    }
}

pub(crate) fn overlay_input(
    texture_size: [u32; 2],
    pixels_per_point: f32,
    events: Vec<Event>,
) -> RawInput {
    let screen_rect = Rect::from_min_size(
        Pos2::ZERO,
        Vec2::new(texture_size[0] as f32, texture_size[1] as f32) / pixels_per_point,
    );
    let mut input = RawInput {
        screen_rect: Some(screen_rect),
        events,
        ..RawInput::default()
    };
    input
        .viewports
        .entry(input.viewport_id)
        .or_default()
        .native_pixels_per_point = Some(pixels_per_point);
    input
}

fn make_target(gl: &glow::Context, size: [u32; 2]) -> Result<Target, String> {
    unsafe {
        let framebuffer = gl.create_framebuffer()?;
        let texture = match gl.create_texture() {
            Ok(texture) => texture,
            Err(error) => {
                gl.delete_framebuffer(framebuffer);
                return Err(error);
            }
        };
        let draw = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        let read = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            size[0] as i32,
            size[1] as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        let complete = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        gl.bind_texture(glow::TEXTURE_2D, None);
        if !complete {
            gl.delete_framebuffer(framebuffer);
            gl.delete_texture(texture);
            return Err(tr("VR の描画先を作成できません").into());
        }
        Ok(Target {
            framebuffer,
            texture,
        })
    }
}

pub fn pointer_button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

pub fn cancel_pointer(events: &mut Vec<Event>, down: bool) {
    if down {
        let outside = Pos2::new(-32., -32.);
        events.push(Event::PointerMoved(outside));
        events.push(pointer_button(outside, false));
    }
    events.push(Event::PointerGone);
}
