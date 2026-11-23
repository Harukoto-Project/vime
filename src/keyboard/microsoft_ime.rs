use std::{
    ptr::{null_mut, slice_from_raw_parts},
    sync::mpsc::{Receiver, Sender, channel},
    thread,
};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, CLSIDFromProgID, COINIT_APARTMENTTHREADED, CoCreateInstance,
            CoInitializeEx, CoTaskMemFree, CoUninitialize,
        },
        UI::Input::Ime::{FELANG_CMODE_BESTFIRST, FELANG_REQ_CONV, IFELanguage},
    },
    core::{PCWSTR, w},
};

pub struct MicrosoftIme(Option<IFELanguage>);

pub struct Lookup {
    requests: Sender<String>,
    results: Receiver<(String, Vec<String>)>,
}

impl Lookup {
    pub fn start(context: eframe::egui::Context) -> Self {
        let (requests, receiver) = channel::<String>();
        let (sender, results) = channel();
        let _ = thread::Builder::new()
            .name("vime-ime".into())
            .spawn(move || {
                let Ok(ime) = MicrosoftIme::open() else {
                    return;
                };
                while let Ok(reading) = receiver.recv() {
                    let reading = receiver.try_iter().last().unwrap_or(reading);
                    let candidates = ime.candidates(&reading);
                    if sender.send((reading, candidates)).is_err() {
                        break;
                    }
                    context.request_repaint();
                }
            });
        Self { requests, results }
    }

    pub fn request(&self, reading: &str) {
        if !reading.is_empty()
            && reading.chars().count() <= 40
            && reading.chars().all(|c| matches!(c, 'ぁ'..='ゖ' | 'ー'))
        {
            let _ = self.requests.send(reading.to_owned());
        }
    }

    pub fn latest(&self) -> Option<(String, Vec<String>)> {
        self.results.try_iter().last()
    }
}

impl MicrosoftIme {
    pub fn open() -> windows::core::Result<Self> {
        // Open and close the COM interface on the same worker thread.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let result = (|| {
                let class_id = CLSIDFromProgID(w!("MSIME.Japan"))?;
                let engine: IFELanguage = CoCreateInstance(&class_id, None, CLSCTX_INPROC_SERVER)?;
                engine.Open()?;
                Ok(Self(Some(engine)))
            })();
            if result.is_err() {
                CoUninitialize();
            }
            result
        }
    }

    pub fn candidates(&self, reading: &str) -> Vec<String> {
        if reading.is_empty()
            || reading.chars().count() > 40
            || !reading.chars().all(|c| matches!(c, 'ぁ'..='ゖ' | 'ー'))
        {
            return Vec::new();
        }

        let mut input: Vec<_> = reading.encode_utf16().collect();
        let length = input.len() as i32;
        input.push(0);
        let mut candidates = Vec::new();
        for rank in 0..6 {
            let mut result = null_mut();
            let chars = if rank == 0 {
                PCWSTR(input.as_ptr())
            } else {
                PCWSTR::null()
            };
            let count = if rank == 0 { length } else { 0 };
            // The IME allocates MORRSLT and its backing data in one task-memory block.
            let candidate = unsafe {
                let status = self.0.as_ref().unwrap().GetJMorphResult(
                    FELANG_REQ_CONV,
                    FELANG_CMODE_BESTFIRST,
                    count,
                    chars,
                    null_mut(),
                    &mut result,
                );
                let candidate = result
                    .as_ref()
                    .and_then(|data| {
                        slice_from_raw_parts(data.pwchOutput.0, data.cchOutput as usize).as_ref()
                    })
                    .map(String::from_utf16_lossy);
                CoTaskMemFree(Some(result.cast()));
                if status.is_err() {
                    None
                } else {
                    candidate.map(|value| value.trim_end_matches('\0').to_owned())
                }
            };
            let Some(candidate) = candidate.filter(|candidate| !candidate.is_empty()) else {
                break;
            };
            if !candidates.contains(&candidate) && candidate != reading {
                candidates.push(candidate);
            }
        }
        candidates
    }
}

impl Drop for MicrosoftIme {
    fn drop(&mut self) {
        // Close and release the interface before leaving its COM apartment.
        unsafe {
            if let Some(engine) = self.0.take() {
                let _ = engine.Close();
                drop(engine);
            }
            CoUninitialize();
        }
    }
}
