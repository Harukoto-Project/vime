use eframe::{
    egui::{
        self, Color32, Event, FontDefinitions, Pos2, RawInput, Rect, Shape, Ui, Vec2, Visuals,
        epaint::ClippedShape,
    },
    glow::{self, HasContext},
};
use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};
use vime::i18n::tr;

const RETRY_DELAY: Duration = Duration::from_millis(250);
const CURSOR_INTERVAL: Duration = Duration::from_micros(16_667);
const SLOW_CURSOR_INTERVAL: Duration = Duration::from_millis(50);
const CURSOR_DAMAGE_MARGIN: f32 = 10.;

pub struct OverlaySurface {
    context: egui::Context,
    painter: egui_glow::Painter,
    gl: Arc<glow::Context>,
    target: Target,
    background: Option<Target>,
    background_changed: bool,
    drawn_pointers: [Option<(Pos2, bool)>; 2],
    size: [u32; 2],
    pending: bool,
    retry_after: Option<Instant>,
    next_pointer_frame: Option<Instant>,
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
        visuals: Visuals,
        size: [u32; 2],
        cache_background: bool,
    ) -> Result<Self, String> {
        let mut painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|error| error.to_string())?;
        let target = match make_target(&gl, size) {
            Ok(target) => target,
            Err(error) => {
                painter.destroy();
                return Err(error);
            }
        };
        let background = if cache_background {
            match make_target(&gl, size) {
                Ok(background) => Some(background),
                Err(error) => {
                    target.destroy(&gl);
                    painter.destroy();
                    return Err(error);
                }
            }
        } else {
            None
        };
        let context = egui::Context::default();
        context.set_fonts(fonts);
        context.set_visuals(visuals);
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
            background,
            background_changed: false,
            drawn_pointers: [None; 2],
            size,
            pending: false,
            retry_after: None,
            next_pointer_frame: None,
        })
    }

    pub fn render<T>(&mut self, events: Vec<Event>, mut draw: impl FnMut(&mut Ui) -> T) -> T {
        let screen_rect = Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(self.size[0] as f32, self.size[1] as f32),
        );
        let input = RawInput {
            screen_rect: Some(screen_rect),
            events,
            ..RawInput::default()
        };
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
            let target = self.background.as_ref().unwrap_or(&self.target);
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            self.painter.clear(self.size, [0.0; 4]);
            self.painter.paint_and_update_textures(
                self.size,
                output.pixels_per_point,
                &shapes,
                &mut output.textures_delta,
            );
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        }
        self.pending = self.background.is_none();
        self.background_changed = self.background.is_some();
        result.expect("egui invokes the overlay draw callback")
    }

    pub fn compose_pointers(&mut self, pointers: [Option<(Pos2, bool)>; 2]) {
        let Some(background) = &self.background else {
            return;
        };
        let [width, height] = self.size;
        let clip_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width as f32, height as f32));
        let colors = [
            Color32::from_rgb(215, 194, 255),
            Color32::from_rgb(159, 211, 255),
        ];
        let shapes: Vec<_> = pointers
            .into_iter()
            .zip(colors)
            .filter_map(|(pointer, color)| {
                let (position, down) = pointer?;
                let radius = if down { 6. } else { 4.5 };
                Some([
                    Shape::circle_filled(position, radius + 2., Color32::from_rgb(14, 39, 61)),
                    Shape::circle_filled(position, radius, color),
                ])
            })
            .flatten()
            .map(|shape| ClippedShape::new(clip_rect, shape))
            .collect();

        // Restore the whole keyboard only when its content changes. Otherwise copy
        // just the previous pointer footprints before stamping the new pointers.
        unsafe {
            let draw = self
                .gl
                .get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            let read = self
                .gl
                .get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, Some(background.framebuffer));
            self.gl
                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.target.framebuffer));
            if self.background_changed {
                blit_rect(&self.gl, self.size, clip_rect);
            } else {
                for (position, _) in self.drawn_pointers.into_iter().flatten() {
                    let damage =
                        Rect::from_center_size(position, Vec2::splat(CURSOR_DAMAGE_MARGIN * 2.));
                    blit_rect(&self.gl, self.size, damage.intersect(clip_rect));
                }
            }
            if !shapes.is_empty() {
                let primitives = self.context.tessellate(shapes, 1.);
                self.painter.paint_primitives(self.size, 1., &primitives);
            }
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        }
        self.background_changed = false;
        self.drawn_pointers = pointers;
        self.pending = true;
    }

    pub fn can_present(&self, now: Instant) -> bool {
        self.retry_after.is_none_or(|retry| retry <= now)
    }

    pub fn pointer_frame_due(&self, now: Instant) -> bool {
        self.next_pointer_frame.is_none_or(|next| next <= now)
    }

    pub fn reset(&mut self) {
        self.pending = false;
        self.retry_after = None;
        self.next_pointer_frame = None;
        self.background_changed = false;
        self.drawn_pointers = [None; 2];
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
                if self.background.is_some() {
                    let interval = (now.elapsed() * 2).clamp(CURSOR_INTERVAL, SLOW_CURSOR_INTERVAL);
                    let previous_deadline = self.next_pointer_frame.unwrap_or(now).min(now);
                    self.next_pointer_frame =
                        Some((previous_deadline + interval).max(now + interval / 2));
                }
            }
            Err(_) => self.retry_after = Some(Instant::now() + RETRY_DELAY),
        }
        Some(result)
    }

    pub fn destroy(mut self) {
        self.painter.destroy();
        self.target.destroy(&self.gl);
        if let Some(background) = self.background {
            background.destroy(&self.gl);
        }
    }
}

fn blit_rect(gl: &glow::Context, size: [u32; 2], rect: Rect) {
    let x0 = rect.min.x.floor() as i32;
    let x1 = rect.max.x.ceil() as i32;
    // UI positions start at the top, OpenGL framebuffer coordinates at the bottom.
    let y0 = size[1] as i32 - rect.max.y.ceil() as i32;
    let y1 = size[1] as i32 - rect.min.y.floor() as i32;
    unsafe {
        gl.blit_framebuffer(
            x0,
            y0,
            x1,
            y1,
            x0,
            y0,
            x1,
            y1,
            glow::COLOR_BUFFER_BIT,
            glow::NEAREST,
        );
    }
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
