use crate::single_instance;
use eframe::{CreationContext, egui::Context};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, mem::size_of, ptr::null_mut, rc::Rc};
use vime::i18n::tr;
use windows_sys::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{
                DefSubclassProc, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
                NIM_SETFOCUS, NIM_SETVERSION, NIN_SELECT, NINF_KEY, NOTIFYICON_VERSION_4,
                NOTIFYICONDATAW, RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, GWL_STYLE, ICON_SMALL, IDI_APPLICATION,
                IsIconic, LoadIconW, MF_STRING, PostMessageW, RegisterWindowMessageW, STYLESTRUCT,
                SW_HIDE, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SendMessageW,
                SetForegroundWindow, ShowWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
                TrackPopupMenu, WINDOWPOS, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_GETICON,
                WM_NCDESTROY, WM_NULL, WM_STYLECHANGING, WM_WINDOWPOSCHANGING, WS_VISIBLE,
            },
        },
    },
    core::w,
};

const TRAY_MESSAGE: u32 = WM_APP + 1;
const SUBCLASS_ID: usize = 1;
const OPEN: u32 = 1;
const EXIT: u32 = 2;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

pub struct Tray(Rc<State>);

struct State {
    window: Cell<HWND>,
    taskbar_created: u32,
    open_message: u32,
    context: Context,
    has_icon: Cell<bool>,
    hidden: Cell<bool>,
    exit_requested: Cell<bool>,
}

impl Tray {
    pub fn new(creation: &CreationContext<'_>, start_hidden: bool) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = creation.window_handle().ok()?.as_raw() else {
            return None;
        };
        let window = handle.hwnd.get() as HWND;
        let tray = Self::attach(window, creation.egui_ctx.clone(), start_hidden)?;
        tray.0.refresh_icon();
        single_instance::register_window(window);
        Some(tray)
    }

    fn attach(window: HWND, context: Context, start_hidden: bool) -> Option<Self> {
        let state = Rc::new(State {
            window: Cell::new(window),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            open_message: single_instance::open_message(),
            context,
            has_icon: Cell::new(false),
            hidden: Cell::new(start_hidden),
            exit_requested: Cell::new(false),
        });
        // The subclass holds a reference until it is removed or the window is destroyed.
        let reference = Rc::into_raw(Rc::clone(&state));
        if unsafe { SetWindowSubclass(window, Some(window_proc), SUBCLASS_ID, reference as usize) }
            == 0
        {
            unsafe { drop(Rc::from_raw(reference)) };
            return None;
        }
        Some(Self(state))
    }

    pub fn hidden(&self) -> bool {
        self.0.hidden.get()
    }

    pub fn take_exit_request(&self) -> bool {
        self.0.exit_requested.replace(false)
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.0.detach(Rc::as_ptr(&self.0));
    }
}

impl State {
    fn refresh_icon(&self) {
        let added = self.add_icon();
        self.has_icon.set(added);
        if !added && self.hidden.get() {
            self.open();
        }
    }

    fn icon_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.window.get(),
            uID: 1,
            ..Default::default()
        }
    }

    fn add_icon(&self) -> bool {
        let mut data = self.icon_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = TRAY_MESSAGE;
        for (target, unit) in data.szTip.iter_mut().zip("ViME".encode_utf16()) {
            *target = unit;
        }
        unsafe {
            data.hIcon = SendMessageW(data.hWnd, WM_GETICON, ICON_SMALL as usize, 0) as _;
            if data.hIcon.is_null() {
                data.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
            }
            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                return false;
            }
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            if Shell_NotifyIconW(NIM_SETVERSION, &data) != 0 {
                return true;
            }
            Shell_NotifyIconW(NIM_DELETE, &data);
            false
        }
    }

    fn detach(&self, reference: *const State) {
        let data = self.icon_data();
        if self.window.replace(null_mut()).is_null() {
            return;
        }
        single_instance::unregister_window(data.hWnd);
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
            if RemoveWindowSubclass(data.hWnd, Some(window_proc), SUBCLASS_ID) != 0 {
                drop(Rc::from_raw(reference));
            }
        }
    }

    fn open(&self) {
        self.hidden.set(false);
        let window = self.window.get();
        unsafe {
            ShowWindow(
                window,
                if IsIconic(window) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(window);
        }
        self.context.request_repaint();
    }

    fn menu(&self, position: WPARAM) {
        let window = self.window.get();
        let x = position as i16 as i32;
        let y = (position >> 16) as i16 as i32;
        let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<_>>();
        let open = wide(tr("ウィンドウを開く"));
        let exit = wide(tr("ViME を終了"));
        let selected = unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(menu, MF_STRING, OPEN as usize, open.as_ptr());
            AppendMenuW(menu, MF_STRING, EXIT as usize, exit.as_ptr());
            SetForegroundWindow(window);
            let selected = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                x,
                y,
                0,
                window,
                std::ptr::null(),
            );
            PostMessageW(window, WM_NULL, 0, 0);
            DestroyMenu(menu);
            selected as u32
        };
        match selected {
            OPEN => self.open(),
            EXIT => {
                self.exit_requested.set(true);
                self.context.request_repaint();
            }
            _ => unsafe {
                Shell_NotifyIconW(NIM_SETFOCUS, &self.icon_data());
            },
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    reference: usize,
) -> LRESULT {
    let reference = reference as *const State;
    // Keep the state alive while processing messages that may dispatch nested messages.
    let state = unsafe {
        Rc::increment_strong_count(reference);
        Rc::from_raw(reference)
    };
    match message {
        WM_CLOSE if state.has_icon.get() => {
            state.hidden.set(true);
            unsafe { ShowWindow(window, SW_HIDE) };
            return 0;
        }
        WM_WINDOWPOSCHANGING if state.hidden.get() => {
            // eframe shows the window after its first paint, including a paint for VR.
            let position = unsafe { &mut *(lparam as *mut WINDOWPOS) };
            if position.flags & SWP_SHOWWINDOW != 0 {
                position.flags = (position.flags & !SWP_SHOWWINDOW) | SWP_NOACTIVATE;
            }
        }
        WM_STYLECHANGING if state.hidden.get() && wparam as i32 == GWL_STYLE => {
            // winit also applies the visibility flag when updating the native window style.
            let style = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
            style.styleNew &= !WS_VISIBLE;
        }
        TRAY_MESSAGE => {
            match lparam as u16 as u32 {
                NIN_SELECT | NIN_KEYSELECT => state.open(),
                WM_CONTEXTMENU => state.menu(wparam),
                _ => {}
            }
            return 0;
        }
        WM_NCDESTROY => state.detach(reference),
        message if message != 0 && message == state.taskbar_created => state.refresh_icon(),
        message if message != 0 && message == state.open_message => {
            state.open();
            return 0;
        }
        _ => {}
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetWindowLongW, SetWindowLongW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    #[test]
    fn keeps_the_window_hidden_until_reopened_from_the_tray() {
        unsafe {
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                -32000,
                -32000,
                32,
                32,
                null_mut(),
                null_mut(),
                null_mut(),
                std::ptr::null(),
            );
            assert!(!window.is_null());
            let tray = Tray::attach(window, Context::default(), true).unwrap();
            tray.0.has_icon.set(true);
            let style = || GetWindowLongW(window, GWL_STYLE) as u32;

            // eframe's first paint path does both of these through winit.
            ShowWindow(window, SW_SHOW);
            assert_eq!(style() & WS_VISIBLE, 0);
            SetWindowLongW(window, GWL_STYLE, (style() | WS_VISIBLE) as i32);
            assert_eq!(style() & WS_VISIBLE, 0);

            SendMessageW(window, TRAY_MESSAGE, 0, NIN_SELECT as LPARAM);
            assert!(!tray.hidden());
            assert_ne!(style() & WS_VISIBLE, 0);

            SendMessageW(window, WM_CLOSE, 0, 0);
            assert!(tray.hidden());
            assert_eq!(style() & WS_VISIBLE, 0);

            let open_message = single_instance::open_message();
            assert_ne!(open_message, 0);
            SendMessageW(window, open_message, 0, 0);
            assert!(!tray.hidden());
            assert_ne!(style() & WS_VISIBLE, 0);
            DestroyWindow(window);
        }
    }
}
