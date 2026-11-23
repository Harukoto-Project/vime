use std::{
    ptr::null_mut,
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::Media::Audio::{PlaySoundA, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

static CLICKS: [&[u8]; 4] = [
    include_bytes!("../../assets/keyboard/keypress-004.wav"),
    include_bytes!("../../assets/keyboard/keypress-013.wav"),
    include_bytes!("../../assets/keyboard/keypress-006.wav"),
    include_bytes!("../../assets/keyboard/keypress-017.wav"),
];
static NEXT_CLICK: AtomicUsize = AtomicUsize::new(0);

pub fn play_click() {
    let sample = CLICKS[NEXT_CLICK.fetch_add(1, Ordering::Relaxed) % CLICKS.len()];
    // SAFETY: Each embedded WAV remains valid for the entire asynchronous playback.
    unsafe {
        PlaySoundA(
            sample.as_ptr(),
            null_mut(),
            SND_ASYNC | SND_MEMORY | SND_NODEFAULT,
        );
    }
}
