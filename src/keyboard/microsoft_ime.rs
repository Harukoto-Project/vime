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
        UI::Input::Ime::{FELANG_CMODE_BESTFIRST, FELANG_REQ_CONV, IFELanguage, MORRSLT},
    },
    core::{PCWSTR, w},
};

const MAX_READING_CHARS: usize = 40;

pub struct MicrosoftIme(Option<IFELanguage>);

/// One clause (文節) of the sentence conversion by the system IME.
pub struct Clause {
    pub reading: String,
    pub text: String,
}

pub struct Lookup {
    requests: Sender<String>,
    results: Receiver<(String, Vec<Clause>)>,
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
                    let clauses = ime.clauses(&reading);
                    if sender.send((reading, clauses)).is_err() {
                        break;
                    }
                    context.request_repaint();
                }
            });
        Self { requests, results }
    }

    pub fn request(&self, reading: &str) {
        if is_convertible(reading) {
            let _ = self.requests.send(reading.to_owned());
        }
    }

    pub fn latest(&self) -> Option<(String, Vec<Clause>)> {
        self.results.try_iter().last()
    }
}

fn is_convertible(reading: &str) -> bool {
    !reading.is_empty()
        && reading.chars().count() <= MAX_READING_CHARS
        && reading.chars().all(|c| matches!(c, 'ぁ'..='ゖ' | 'ー'))
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

    /// Converts the reading and splits the result into clauses. An empty list
    /// means that the IME could not convert it.
    pub fn clauses(&self, reading: &str) -> Vec<Clause> {
        if !is_convertible(reading) {
            return Vec::new();
        }
        let mut input: Vec<u16> = reading.encode_utf16().collect();
        let length = input.len();
        input.push(0);
        let mut result = null_mut();
        // The IME allocates MORRSLT and its backing data in one task-memory block.
        unsafe {
            let status = self.0.as_ref().unwrap().GetJMorphResult(
                FELANG_REQ_CONV,
                FELANG_CMODE_BESTFIRST,
                length as i32,
                PCWSTR(input.as_ptr()),
                null_mut(),
                &mut result,
            );
            let clauses = if status.is_ok() {
                result
                    .as_ref()
                    .and_then(|data| split_clauses(data, &input[..length]))
            } else {
                None
            };
            CoTaskMemFree(Some(result.cast()));
            clauses.unwrap_or_default()
        }
    }
}

/// The IME reports words. A clause is a content word (part of speech other than 0)
/// with the particles and endings (part of speech 0) that follow it.
///
/// # Safety
/// `data` must be a valid result returned by `IFELanguage::GetJMorphResult`.
unsafe fn split_clauses(data: &MORRSLT, reading: &[u16]) -> Option<Vec<Clause>> {
    unsafe {
        let output = std::ptr::addr_of!(data.pwchOutput).read_unaligned();
        let output_length = std::ptr::addr_of!(data.cchOutput).read_unaligned() as usize;
        let words = std::ptr::addr_of!(data.pWDD).read_unaligned();
        let count = std::ptr::addr_of!(data.cWDD).read_unaligned();
        if output.is_null() || words.is_null() || count <= 0 {
            return None;
        }
        let output = slice_from_raw_parts(output.0, output_length).as_ref()?;
        let words = slice_from_raw_parts(words.cast_const(), count as usize).as_ref()?;

        let mut starts: Vec<(usize, usize)> = Vec::new();
        for (index, word) in words.iter().enumerate() {
            let read = word.Anonymous1.wReadPos as usize;
            let shown = word.wDispPos as usize;
            let kind = word.nPos;
            if index == 0 && (read != 0 || shown != 0) {
                return None;
            }
            if index == 0 || kind != 0 {
                starts.push((read, shown));
            }
        }
        starts.push((reading.len(), output.len()));
        if starts.windows(2).any(|pair| {
            pair[0].0 >= pair[1].0
                || pair[0].1 >= pair[1].1
                || pair[1].0 > reading.len()
                || pair[1].1 > output.len()
        }) {
            return None;
        }
        starts
            .windows(2)
            .map(|pair| {
                Some(Clause {
                    reading: String::from_utf16(&reading[pair[0].0..pair[1].0]).ok()?,
                    text: String::from_utf16(&output[pair[0].1..pair[1].1]).ok()?,
                })
            })
            .collect()
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
