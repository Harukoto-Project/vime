use std::{
    io,
    os::windows::io::{FromRawHandle, OwnedHandle},
    ptr::{null, null_mut},
    thread,
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{
            AllowSetForegroundWindow, EnumWindows, GetPropW, GetWindowThreadProcessId,
            PostMessageW, RegisterWindowMessageW, RemovePropW, SetPropW,
        },
    },
    core::w,
};

pub fn start(launch_from_steamvr: bool) -> io::Result<Option<OwnedHandle>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let handle = unsafe { CreateMutexW(null(), 0, w!("Local\\org.harukoto.vime.Instance")) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        if !already_running {
            return Ok(Some(handle));
        }
        // Only the desktop instance keeps the mutex handle between attempts.
        drop(handle);
        if launch_from_steamvr || reopen() || Instant::now() >= deadline {
            return Ok(None);
        }
        // The first process may still be creating its window, or may be exiting.
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn open_message() -> u32 {
    unsafe { RegisterWindowMessageW(w!("org.harukoto.vime.OpenWindow")) }
}

pub fn register_window(window: HWND) {
    unsafe { SetPropW(window, w!("org.harukoto.vime.InstanceWindow"), window) };
}

pub fn unregister_window(window: HWND) {
    unsafe { RemovePropW(window, w!("org.harukoto.vime.InstanceWindow")) };
}

fn reopen() -> bool {
    let message = open_message();
    if message == 0 {
        return false;
    }

    let mut window: HWND = null_mut();
    unsafe {
        EnumWindows(Some(find_window), (&mut window as *mut HWND) as LPARAM);
        if window.is_null() {
            return false;
        }
        let mut process = 0;
        if GetWindowThreadProcessId(window, &mut process) == 0 {
            return false;
        }
        AllowSetForegroundWindow(process);
        PostMessageW(window, message, 0, 0) != 0
    }
}

unsafe extern "system" fn find_window(window: HWND, result: LPARAM) -> i32 {
    if unsafe { GetPropW(window, w!("org.harukoto.vime.InstanceWindow")) } != window {
        return 1;
    }
    unsafe { *(result as *mut HWND) = window };
    0
}
