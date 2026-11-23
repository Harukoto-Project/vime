//! The Win64 OpenVR C ABI. Slots match Valve's versioned openvr_capi.h.
//! https://github.com/ValveSoftware/openvr/blob/v1.23.7/headers/openvr_capi.h

use std::ffi::{CStr, c_char, c_void};

pub type Init = unsafe extern "C" fn(*mut i32, i32) -> u32;
pub type Shutdown = unsafe extern "C" fn();
pub type GenericInterface = unsafe extern "C" fn(*const c_char, *mut i32) -> *const *const c_void;

pub const SYSTEM: &CStr = c"FnTable:IVRSystem_022";
pub const APPLICATIONS: &CStr = c"FnTable:IVRApplications_007";
pub const OVERLAY: &CStr = c"FnTable:IVROverlay_026";
pub const INPUT: &CStr = c"FnTable:IVRInput_010";
pub const STANDING: i32 = 1;
pub const APPLICATION_OVERLAY: i32 = 2;
pub const QUIT: u32 = 700;
pub const MOUSE_MOVE: u32 = 300;
pub const MOUSE_DOWN: u32 = 301;
pub const MOUSE_UP: u32 = 302;
pub const FOCUS_LEAVE: u32 = 304;
pub const OVERLAY_HIDDEN: u32 = 501;
pub const MOUSE_LEFT: u32 = 1;
pub const INPUT_MOUSE: i32 = 1;
pub const PREMULTIPLIED_ALPHA: i32 = 2_097_152;
pub const TEXTURE_OPENGL: i32 = 1;
pub const COLOR_GAMMA: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Matrix(pub [[f32; 4]; 3]);

#[repr(C)]
#[derive(Default)]
pub struct Pose {
    pub transform: Matrix,
    velocity: [f32; 3],
    angular_velocity: [f32; 3],
    tracking_result: i32,
    pub valid: bool,
    pub connected: bool,
}

#[repr(C)]
#[derive(Default)]
pub struct Digital {
    pub active: bool,
    origin: u64,
    pub state: bool,
    pub changed: bool,
    update_time: f32,
}

#[repr(C)]
#[derive(Default)]
pub struct ActionPose {
    pub active: bool,
    origin: u64,
    pub pose: Pose,
}

#[repr(C)]
#[derive(Default)]
pub struct ActionSet {
    pub set: u64,
    restricted_device: u64,
    secondary_set: u64,
    padding: u32,
    pub priority: i32,
}

#[repr(C)]
pub struct Texture {
    pub handle: *mut c_void,
    pub kind: i32,
    pub color_space: i32,
}

#[repr(C)]
pub struct TextureBounds {
    pub u_min: f32,
    pub v_min: f32,
    pub u_max: f32,
    pub v_max: f32,
}

#[repr(C)]
#[derive(Default)]
pub struct Event {
    pub kind: u32,
    _device: u32,
    _age: f32,
    data: [u64; 6],
}

impl Event {
    pub fn mouse(&self) -> ([f32; 2], u32) {
        let x = f32::from_bits(self.data[0] as u32);
        let y = f32::from_bits((self.data[0] >> 32) as u32);
        ([x, y], self.data[1] as u32)
    }
}

const _: () = {
    assert!(size_of::<Event>() == 64 && std::mem::offset_of!(Event, data) == 16);
    assert!(size_of::<Pose>() == 80 && size_of::<ActionPose>() == 96);
    assert!(size_of::<Digital>() == 24 && size_of::<ActionSet>() == 32);
    assert!(size_of::<Texture>() == 16);
};

macro_rules! table {
    ($name:ident { $($method:ident : $slot:literal ($($argument:ident : $ty:ty),*) -> $ret:ty;)* }) => {
        pub struct $name(pub *const *const c_void);

        impl $name {
            $(
                pub fn $method(&self, $($argument: $ty),*) -> $ret {
                    // Each wrapper belongs to one pinned interface. The session keeps the DLL
                    // and its tables alive, and supplies correctly sized arguments.
                    unsafe {
                        let call: unsafe extern "system" fn($($ty),*) -> $ret =
                            std::mem::transmute(*self.0.add($slot));
                        call($($argument),*)
                    }
                }
            )*
        }
    };
}

table!(System {
    tracking_pose: 11(origin: i32, prediction: f32, output: *mut Pose, count: u32) -> ();
    poll_event: 29(output: *mut Event, size: u32) -> bool;
    acknowledge_quit: 43() -> ();
});

table!(Applications {
    add_manifest: 0(path: *const c_char, temporary: bool) -> i32;
    identify: 11(pid: u32, key: *const c_char) -> i32;
    scene_pid: 29() -> u32;
});

table!(Overlay {
    create: 1(key: *const c_char, name: *const c_char, output: *mut u64) -> i32;
    destroy: 2(handle: u64) -> i32;
    flag: 10(handle: u64, flag: i32, enabled: bool) -> i32;
    set_width: 21(handle: u64, width: f32) -> i32;
    texture_bounds: 29(handle: u64, bounds: *const TextureBounds) -> i32;
    set_transform: 32(handle: u64, origin: i32, matrix: *const Matrix) -> i32;
    show: 43(handle: u64) -> i32;
    hide: 44(handle: u64) -> i32;
    poll_event: 48(handle: u64, output: *mut Event, size: u32) -> bool;
    input_method: 50(handle: u64, method: i32) -> i32;
    mouse_scale: 52(handle: u64, scale: *const [f32; 2]) -> i32;
    set_texture: 60(handle: u64, texture: *const Texture) -> i32;
    set_raw: 62(handle: u64, pixels: *const u8, width: u32, height: u32, depth: u32) -> i32;
    create_dashboard: 67(key: *const c_char, name: *const c_char, main: *mut u64, thumbnail: *mut u64) -> i32;
    dashboard_visible: 68() -> bool;
    dashboard_active: 69(handle: u64) -> bool;
});

table!(Input {
    set_manifest: 0(path: *const c_char) -> i32;
    set_handle: 1(path: *const c_char, handle: *mut u64) -> i32;
    action_handle: 2(path: *const c_char, handle: *mut u64) -> i32;
    source_handle: 3(path: *const c_char, handle: *mut u64) -> i32;
    update: 4(sets: *const ActionSet, size: u32, count: u32) -> i32;
    digital: 5(action: u64, data: *mut Digital, size: u32, device: u64) -> i32;
    aim: 7(action: u64, origin: i32, time: f32, data: *mut ActionPose, size: u32, device: u64) -> i32;
    open_bindings: 30(key: *const c_char, set: u64, device: u64, desktop: bool) -> i32;
});
