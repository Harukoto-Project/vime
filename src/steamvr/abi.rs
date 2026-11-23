//! The Win64 OpenVR C ABI. Slots match Valve's versioned openvr_capi.h.
//! https://github.com/ValveSoftware/openvr/blob/master/headers/openvr_capi.h

use std::ffi::{CStr, c_char, c_void};

pub type Init = unsafe extern "C" fn(*mut i32, i32) -> u32;
pub type Shutdown = unsafe extern "C" fn();
pub type GenericInterface = unsafe extern "C" fn(*const c_char, *mut i32) -> *const *const c_void;

pub const SYSTEM: &CStr = c"FnTable:IVRSystem_026";
pub const APPLICATIONS: &CStr = c"FnTable:IVRApplications_008";
pub const OVERLAY: &CStr = c"FnTable:IVROverlay_028";
pub const INPUT: &CStr = c"FnTable:IVRInput_011";
pub const STANDING: i32 = 1;
pub const APPLICATION_OVERLAY: i32 = 2;
pub const QUIT: u32 = 700;

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

#[repr(C, align(8))]
#[derive(Default)]
pub struct Event {
    pub kind: u32,
    _rest: [u32; 15],
}

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
    tracking_pose: 12(origin: i32, prediction: f32, output: *mut Pose, count: u32) -> ();
    poll_event: 30(output: *mut Event, size: u32) -> bool;
    acknowledge_quit: 47() -> ();
});

table!(Applications {
    add_manifest: 0(path: *const c_char, temporary: bool) -> i32;
    identify: 11(pid: u32, key: *const c_char) -> i32;
    scene_pid: 30() -> u32;
});

table!(Overlay {
    create: 1(key: *const c_char, name: *const c_char, output: *mut u64) -> i32;
    destroy: 3(handle: u64) -> i32;
    set_width: 22(handle: u64, width: f32) -> i32;
    set_transform: 33(handle: u64, origin: i32, matrix: *const Matrix) -> i32;
    show: 43(handle: u64) -> i32;
    hide: 44(handle: u64) -> i32;
    set_raw: 62(handle: u64, pixels: *const u8, width: u32, height: u32, depth: u32) -> i32;
    dashboard_visible: 68() -> bool;
});

table!(Input {
    set_manifest: 0(path: *const c_char) -> i32;
    set_handle: 1(path: *const c_char, handle: *mut u64) -> i32;
    action_handle: 2(path: *const c_char, handle: *mut u64) -> i32;
    source_handle: 3(path: *const c_char, handle: *mut u64) -> i32;
    update: 4(sets: *const ActionSet, size: u32, count: u32) -> i32;
    digital: 5(action: u64, data: *mut Digital, size: u32, device: u64) -> i32;
    aim: 7(action: u64, origin: i32, time: f32, data: *mut ActionPose, size: u32, device: u64) -> i32;
    open_bindings: 32(key: *const c_char, set: u64, device: u64, desktop: bool) -> i32;
});
