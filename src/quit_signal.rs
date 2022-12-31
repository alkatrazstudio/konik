// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use signal_hook::{
    consts::{SIGINT, SIGQUIT, SIGTERM},
    iterator::Signals,
};

use crate::{err_util::LogErr, thread_util};

pub fn listen<F>(callback: F)
where
    F: FnOnce() + Send + 'static,
{
    match Signals::new([SIGINT, SIGTERM, SIGQUIT]) {
        Ok(mut signals) => {
            thread_util::thread("quit signal listener", move || {
                let mut callback = Some(callback);
                let mut sigint_sent = false;
                for sig in signals.forever() {
                    if sig == SIGINT {
                        assert!(!sigint_sent, "force quit");
                        sigint_sent = true;
                        print!("\r  \r\n"); // hide ^C
                    }
                    if let Some(callback) = callback.take() {
                        callback();
                    }
                }
            });
        }
        Err(e) => e.log(),
    }
}
