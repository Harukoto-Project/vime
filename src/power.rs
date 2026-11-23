//! Keeps the process responsive while its desktop window is covered or minimized.
//!
//! The overlay is painted and SteamVR is polled from the desktop window's event loop. When another
//! window such as Virtual Desktop covers it, Windows may throttle the process and coalesce its
//! timers, which stalls that loop for hundreds of milliseconds and starves the VR overlay.

use std::{ffi::c_void, mem::size_of};
use windows_sys::Win32::{
    Media::timeBeginPeriod,
    System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetProcessInformation,
    },
};

/// Timer resolution requested for the lifetime of the process, in milliseconds.
const TIMER_RESOLUTION_MS: u32 = 1;

/// Opts out of execution-speed throttling and timer coalescing. Failure is not fatal: older
/// Windows versions reject the timer flag, and the worst case is the previous behavior.
pub fn disable_throttling() {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        // A cleared state bit with its control bit set turns that throttling off.
        StateMask: 0,
    };
    // The pseudo handle is valid for the current process, and `state` matches the layout that the
    // `ProcessPowerThrottling` class expects.
    unsafe {
        let process = GetCurrentProcess();
        let applied = SetProcessInformation(
            process,
            ProcessPowerThrottling,
            (&raw const state).cast::<c_void>(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        if applied == 0 {
            // Pre-Windows 11 builds do not know the timer flag; retry with the speed flag alone.
            let speed_only = PROCESS_POWER_THROTTLING_STATE {
                ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                ..state
            };
            SetProcessInformation(
                process,
                ProcessPowerThrottling,
                (&raw const speed_only).cast::<c_void>(),
                size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            );
        }
        // The period stays in effect until the process exits.
        timeBeginPeriod(TIMER_RESOLUTION_MS);
    }
}
