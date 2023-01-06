// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use anyhow::Result;
use tauri_hotkey::{Hotkey, HotkeyManager, Key};

use crate::err_util::IgnoreErr;

#[derive(Copy, Clone)]
pub enum HotKeyAction {
    StopPlay,
    Next,
    Prev,
    NextDir,
    PrevDir,
    PauseToggle,
    VolUp,
    VolDown,
    SysVolUp,
    SysVolDown,
}

const ACTIONS: [(Key, HotKeyAction); 10] = [
    (Key::NUM5, HotKeyAction::StopPlay),
    (Key::NUM6, HotKeyAction::Next),
    (Key::NUM4, HotKeyAction::Prev),
    (Key::NUM9, HotKeyAction::NextDir),
    (Key::NUM7, HotKeyAction::PrevDir),
    (Key::NUM0, HotKeyAction::PauseToggle),
    (Key::NUM2, HotKeyAction::VolDown),
    (Key::NUM8, HotKeyAction::VolUp),
    (Key::NUM1, HotKeyAction::SysVolDown),
    (Key::NUM3, HotKeyAction::SysVolUp),
];

pub struct HotKeys {
    manager: HotkeyManager,
}

impl HotKeys {
    pub fn new() -> Self {
        return Self {
            manager: HotkeyManager::new(),
        };
    }

    pub fn register<F>(&mut self, f: F) -> Result<()>
    where
        F: Fn(HotKeyAction) + Clone + Sync + Send + 'static,
    {
        for (key, action) in &ACTIONS {
            let f = f.clone();
            self.manager.register(
                Hotkey {
                    keys: vec![*key],
                    modifiers: vec![],
                },
                move || f(*action),
            )?;
        }
        return Ok(());
    }

    #[allow(dead_code)]
    pub fn unregister(&mut self) {
        self.manager.unregister_all().ignore_err();
    }
}
