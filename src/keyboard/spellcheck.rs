use windows::{
    Win32::{
        Globalization::{ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

pub struct Spellcheck(Option<ISpellChecker>);

impl Spellcheck {
    pub fn open() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let result = (|| {
                let factory: ISpellCheckerFactory =
                    CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER)?;
                factory
                    .CreateSpellChecker(w!("en-US"))
                    .map(|checker| Self(Some(checker)))
            })();
            if result.is_err() {
                CoUninitialize();
            }
            result
        }
    }

    pub fn suggestions(&self, word: &str) -> Vec<String> {
        if !word
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '\'' | '’'))
        {
            return Vec::new();
        }
        let input: Vec<u16> = word.encode_utf16().chain([0]).collect();
        let word = PCWSTR(input.as_ptr());
        let checker = self.0.as_ref().unwrap();
        unsafe {
            let Ok(errors) = checker.Check(word) else {
                return Vec::new();
            };
            let mut error = None;
            if errors.Next(&mut error).is_err() || error.is_none() {
                return Vec::new();
            }
            let Ok(results) = checker.Suggest(word) else {
                return Vec::new();
            };
            let mut suggestions = Vec::new();
            for _ in 0..3 {
                let mut value = [PWSTR::null()];
                let mut fetched = 0;
                if results.Next(&mut value, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                if let Ok(text) = value[0].to_string() {
                    suggestions.push(text);
                }
                CoTaskMemFree(Some(value[0].as_ptr().cast()));
            }
            suggestions
        }
    }
}

impl Drop for Spellcheck {
    fn drop(&mut self) {
        drop(self.0.take());
        unsafe { CoUninitialize() };
    }
}
