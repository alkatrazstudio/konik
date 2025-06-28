// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use notify_rust::Notification;

use crate::{err_util::IgnoreErr, project_info, thread_util};

pub struct Popup {
    handle_id: Arc<Mutex<Option<u32>>>,
}

impl Popup {
    pub fn new() -> Self {
        return Self {
            handle_id: Arc::new(Mutex::new(None)),
        };
    }

    pub fn show(&self, body: &str) {
        let handle_id = self.handle_id.clone();

        let body = body.to_string();
        thread_util::thread("popup", move || {
            Self::show_raw(&body, &handle_id).ignore_err();
        });
    }

    fn show_raw(body: &str, handle_id_arc: &Arc<Mutex<Option<u32>>>) -> Result<()> {
        let mut popup = Notification::new();
        let html_body = html_escape::encode_text(body);
        let popup = popup.body(&html_body).appname(project_info::title());
        let mut handle_id_guarded = handle_id_arc.lock().unwrap();
        let handle;
        let cur_handle_id;
        if let Some(handle_id) = *handle_id_guarded {
            cur_handle_id = Some(handle_id);
            handle = match popup.id(handle_id).show() {
                Ok(handle) => handle,
                Err(e) => {
                    if e.to_string() == "Created too many similar notifications in quick succession"
                    {
                        // This warning is useless, so we ignore it without logging it.
                        // But the only way to this is to compare the text of the error.
                        return Ok(());
                    }
                    bail!(anyhow!(e).context("cannot update popup"));
                }
            }
        } else {
            handle = popup.show().context("cannot create popup")?;
            *handle_id_guarded = Some(handle.id());
            cur_handle_id = Some(handle.id());
        }

        drop(handle_id_guarded);

        handle.on_close(|| {
            let mut handle_id_guarded = handle_id_arc.lock().unwrap();
            if let Some(handle_id) = *handle_id_guarded {
                if Some(handle_id) == cur_handle_id {
                    *handle_id_guarded = None;
                }
            }
        });
        return Ok(());
    }
}
