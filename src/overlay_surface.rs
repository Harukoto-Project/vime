use crate::keyboard::{self, Action, Keyboard, SIZE};
use eframe::{
    egui::{self, Event, FontDefinitions, Pos2, RawInput, Rect, Vec2},
    glow::{self, HasContext},
};
use std::sync::Arc;
use vime::{settings::Destination, steamvr::Pointer};

type Result<T> = std::result::Result<T, String>;

pub struct OverlaySurface {
    context: egui::Context,
    painter: egui_glow::Painter,
    gl: Arc<glow::Context>,
    framebuffer: glow::Framebuffer,
    texture: glow::Texture,
    rgba: Vec<u8>,
    last_pointer: Option<Pointer>,
    last_frame: Option<(u64, Destination)>,
}

impl OverlaySurface {
    pub fn new(gl: Arc<glow::Context>, fonts: FontDefinitions) -> Result<Self> {
        let mut painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|error| error.to_string())?;
        let (framebuffer, texture) = match make_target(&gl) {
            Ok(target) => target,
            Err(error) => {
                painter.destroy();
                return Err(error);
            }
        };
        let context = egui::Context::default();
        context.set_fonts(fonts);
        context.options_mut(|options| options.max_passes = std::num::NonZeroUsize::MIN);

        Ok(Self {
            context,
            painter,
            gl,
            framebuffer,
            texture,
            rgba: vec![0; (SIZE[0] * SIZE[1] * 4) as usize],
            last_pointer: None,
            last_frame: None,
        })
    }

    pub fn reset(&mut self) {
        self.last_frame = None;
    }

    pub fn render(
        &mut self,
        keyboard: &Keyboard,
        destination: Destination,
        pointer: Option<Pointer>,
    ) -> Option<(&[u8], Option<Action>)> {
        let pointer = pointer.map(|mut pointer| {
            for (coordinate, size) in pointer.uv.iter_mut().zip(SIZE) {
                *coordinate = (*coordinate * size as f32).round() / size as f32;
            }
            pointer
        });
        let frame = (keyboard.revision, destination);
        if self.last_frame == Some(frame) && pointer == self.last_pointer {
            return None;
        }

        let events = pointer_events(self.last_pointer, pointer);
        self.last_pointer = pointer;
        self.last_frame = Some(frame);
        let screen_rect =
            Rect::from_min_size(Pos2::ZERO, Vec2::new(SIZE[0] as f32, SIZE[1] as f32));
        let input = RawInput {
            screen_rect: Some(screen_rect),
            events,
            ..RawInput::default()
        };
        let mut action = None;
        let mut output = self.context.run_ui(input, |ui| {
            action = keyboard::show(
                ui,
                keyboard,
                SIZE[0] as f32,
                destination,
                pointer.map(position),
                true,
            );
        });

        let shapes = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        // Read the private target while eframe's GL context is current and restore
        // the default drawing target before eframe paints the Windows UI.
        unsafe {
            let previous = self.gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING);
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            self.painter.clear(SIZE, [0.0; 4]);
            self.painter.paint_and_update_textures(
                SIZE,
                output.pixels_per_point,
                &shapes,
                &mut output.textures_delta,
            );
            self.gl.read_pixels(
                0,
                0,
                SIZE[0] as i32,
                SIZE[1] as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut self.rgba)),
            );
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
        }
        flip_and_unpremultiply(&mut self.rgba);
        Some((&self.rgba, action))
    }

    pub fn destroy(mut self) {
        self.painter.destroy();
        unsafe {
            self.gl.delete_framebuffer(self.framebuffer);
            self.gl.delete_texture(self.texture);
        }
    }
}

fn make_target(gl: &glow::Context) -> Result<(glow::Framebuffer, glow::Texture)> {
    unsafe {
        let framebuffer = gl.create_framebuffer()?;
        let texture = match gl.create_texture() {
            Ok(texture) => texture,
            Err(error) => {
                gl.delete_framebuffer(framebuffer);
                return Err(error);
            }
        };

        let previous = gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING);
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
            SIZE[0] as i32,
            SIZE[1] as i32,
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
        gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
        gl.bind_texture(glow::TEXTURE_2D, None);
        if !complete {
            gl.delete_framebuffer(framebuffer);
            gl.delete_texture(texture);
            return Err("VRキーボードの描画先を作成できません".into());
        }
        Ok((framebuffer, texture))
    }
}

fn position(pointer: Pointer) -> Pos2 {
    Pos2::new(
        pointer.uv[0] * SIZE[0] as f32,
        pointer.uv[1] * SIZE[1] as f32,
    )
}

fn pointer_events(previous: Option<Pointer>, current: Option<Pointer>) -> Vec<Event> {
    let mut events = Vec::new();
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let same_hand = matches!((previous, current), (Some(previous), Some(current)) if previous.hand == current.hand);
    if previous.is_some_and(|previous| previous.down) && !same_hand {
        let outside = Pos2::new(-32., -32.);
        events.push(Event::PointerMoved(outside));
        events.push(button(outside, false));
    }
    if let Some(current) = current {
        let pos = position(current);
        events.push(Event::PointerMoved(pos));
        let was_down = same_hand && previous.is_some_and(|previous| previous.down);
        if current.down != was_down {
            events.push(button(pos, current.down));
        }
    } else {
        events.push(Event::PointerGone);
    }
    events
}

fn flip_and_unpremultiply(pixels: &mut [u8]) {
    let stride = SIZE[0] as usize * 4;
    let (top, bottom) = pixels.split_at_mut(pixels.len() / 2);
    for (bottom, top) in bottom
        .chunks_exact_mut(stride)
        .rev()
        .zip(top.chunks_exact_mut(stride))
    {
        top.swap_with_slice(bottom);
    }
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}
