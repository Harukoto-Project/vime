use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use std::sync::mpsc::{Receiver, channel};
use vime::i18n::{tr, tr_args};

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    current: Option<HotKey>,
    events: Receiver<GlobalHotKeyEvent>,
}

impl Hotkeys {
    pub fn new(context: eframe::egui::Context) -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|error| error.to_string())?;
        let (sender, events) = channel();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed && sender.send(event).is_ok() {
                context.request_repaint();
            }
        }));
        Ok(Self {
            manager,
            current: None,
            events,
        })
    }

    pub fn prepare(&self, shortcut: &str) -> Result<HotKey, String> {
        let key: HotKey = shortcut.trim().parse().map_err(|_| {
            tr("ショートカットは Ctrl+Shift+F8 のように入力してください").to_owned()
        })?;
        if self.current != Some(key) {
            self.manager.register(key).map_err(|error| {
                tr_args(
                    "ショートカットを登録できません ({0})",
                    &[&error.to_string()],
                )
            })?;
        }
        Ok(key)
    }

    pub fn activate(&mut self, key: HotKey) {
        if let Some(old) = self.current.replace(key).filter(|old| *old != key) {
            let _ = self.manager.unregister(old);
        }
    }

    pub fn cancel(&self, key: HotKey) {
        if self.current != Some(key) {
            let _ = self.manager.unregister(key);
        }
    }

    pub fn pressed(&self) -> bool {
        let mut pressed = false;
        for event in self.events.try_iter() {
            if self.current.is_some_and(|current| current.id() == event.id) {
                pressed = !pressed;
            }
        }
        pressed
    }
}
