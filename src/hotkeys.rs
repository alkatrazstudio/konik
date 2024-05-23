// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

use crate::{err_util::IgnoreErr, thread_util};

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

const ACTIONS: [(Code, HotKeyAction); 10] = [
    (Code::Numpad5, HotKeyAction::StopPlay),
    (Code::Numpad6, HotKeyAction::Next),
    (Code::Numpad4, HotKeyAction::Prev),
    (Code::Numpad9, HotKeyAction::NextDir),
    (Code::Numpad7, HotKeyAction::PrevDir),
    (Code::Numpad0, HotKeyAction::PauseToggle),
    (Code::Numpad2, HotKeyAction::VolDown),
    (Code::Numpad8, HotKeyAction::VolUp),
    (Code::Numpad1, HotKeyAction::SysVolDown),
    (Code::Numpad3, HotKeyAction::SysVolUp),
];

const THREAD_SLEEP: Duration = Duration::from_millis(100);

pub struct HotKeys {
    thread: Option<JoinHandle<()>>,
    stop_flag: Arc<Mutex<bool>>,
}

impl HotKeys {
    pub fn new() -> Self {
        return Self {
            thread: None,
            stop_flag: Arc::new(Mutex::new(false)),
        };
    }

    pub fn start<F>(&mut self, action_func: F) -> Result<()>
    where
        F: Fn(HotKeyAction) + Clone + Sync + Send + 'static,
    {
        let mut id_action_map = HashMap::new();
        let mut hotkeys = Vec::new();
        for (code, action) in ACTIONS {
            let hotkey = HotKey::new(None, code);
            let id = hotkey.id();
            hotkeys.push(hotkey);
            id_action_map.insert(id, action);
        }

        let manager = GlobalHotKeyManager::new()?;
        manager.register_all(&hotkeys)?;

        let stop_flag = self.stop_flag.clone();
        let thread = thread_util::thread("hotkeys manager", move || {
            while !*stop_flag.lock().unwrap() {
                if let Ok(event) = GlobalHotKeyEvent::receiver().recv_timeout(THREAD_SLEEP) {
                    if event.state == HotKeyState::Pressed {
                        if let Some(action) = id_action_map.get(&event.id) {
                            action_func(*action);
                        }
                    }
                }
            }
            manager.unregister_all(&hotkeys).ignore_err();
            drop(manager); // this will move the manager into the closure and will keep it alive
        });
        self.thread = Some(thread);

        return Ok(());
    }

    pub fn stop(&mut self) {
        *self.stop_flag.lock().unwrap() = true;
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
