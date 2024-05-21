// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::time::Duration;

use anyhow::{bail, Context, Result};
use dbus::blocking::Connection;
use url::Url;

pub fn show_file(path: &str) -> Result<()> {
    return run_method(path, "ShowItems");
}

pub fn open_folder(path: &str) -> Result<()> {
    return run_method(path, "ShowFolders");
}

fn run_method(path: &str, method: &str) -> Result<()> {
    let conn = Connection::new_session().context("cannot create D-Bus session")?;
    let proxy = conn.with_proxy(
        "org.freedesktop.FileManager1",
        "/org/freedesktop/FileManager1",
        Duration::from_millis(5000),
    );
    match Url::from_file_path(path) {
        Ok(url) => {
            let url_str = url.as_str();
            proxy
                .method_call("org.freedesktop.FileManager1", method, (vec![url_str], ""))
                .with_context(|| format!("failed to call D-Bus method {method} on {url_str}"))?;
        }
        Err(()) => bail!("can't transform a path into URL: {}", path),
    }
    return Ok(());
}
