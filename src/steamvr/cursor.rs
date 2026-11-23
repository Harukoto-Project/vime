use super::{Pointer, abi::Overlay, checked, geometry};
use std::ffi::CStr;

const IMAGE_SIZE: u32 = 32;

#[derive(Default)]
pub(super) struct Cursor {
    pub handle: u64,
    shown: Option<(Pointer, f32)>,
}

impl Cursor {
    pub(super) fn initialize(
        &mut self,
        overlay: &Overlay,
        key: &CStr,
        color: [u8; 4],
    ) -> Result<(), String> {
        checked(
            overlay.create(key.as_ptr(), c"ViME pointer".as_ptr(), &mut self.handle),
            "VRポインターを作成",
        )?;
        let icon: Vec<_> = (0..IMAGE_SIZE)
            .flat_map(|y| {
                (0..IMAGE_SIZE).flat_map(move |x| {
                    let distance = (x as f32 - 15.5).hypot(y as f32 - 15.5);
                    if distance < 9.5 {
                        color
                    } else if distance < 14. {
                        [14, 39, 61, 255]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
            })
            .collect();
        checked(
            overlay.set_raw(self.handle, icon.as_ptr(), IMAGE_SIZE, IMAGE_SIZE, 4),
            "VRポインターの画像を設定",
        )?;
        checked(
            overlay.sort_order(self.handle, 1),
            "VRポインターの表示順を設定",
        )
    }

    pub(super) fn update(
        &mut self,
        overlay: &Overlay,
        parent: u64,
        pointer: Option<Pointer>,
        width: f32,
        aspect: f32,
    ) {
        let Some(mut pointer) = pointer else {
            if self.shown.is_some() && overlay.hide(self.handle) == 0 {
                self.shown = None;
            }
            return;
        };
        // Sub-pixel movement does not require a compositor update.
        pointer.uv = pointer.uv.map(|axis| (axis * 1024.).round() / 1024.);
        let previous = self.shown;
        if previous == Some((pointer, width)) {
            return;
        }
        let transform = geometry::cursor_transform(pointer.uv, width, aspect);
        if overlay.set_relative_transform(self.handle, parent, &transform) != 0 {
            return;
        }
        if previous.is_none_or(|(last, _)| last.down != pointer.down)
            && overlay.set_width(self.handle, if pointer.down { 0.023 } else { 0.017 }) != 0
        {
            return;
        }
        if previous.is_none() && overlay.show(self.handle) != 0 {
            return;
        }
        self.shown = Some((pointer, width));
    }
}
