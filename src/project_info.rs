// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{io::Write, process::Stdio};

use anyhow::{Context, Result};

use crate::err_util::IgnoreErr;

mod built {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

include!(concat!(env!("OUT_DIR"), "/readme.rs"));

pub const fn name() -> &'static str {
    return env!("CARGO_PKG_NAME");
}

pub const fn version() -> &'static str {
    return env!("CARGO_PKG_VERSION");
}

pub const fn title() -> &'static str {
    return env!("PROJECT_TITLE");
}

pub const fn organization() -> &'static str {
    return env!("PROJECT_ORGANIZATION");
}

pub const fn qualifier() -> &'static str {
    return env!("PROJECT_QUALIFIER");
}

pub fn print_version_info() {
    println!("version: {}", version());
    println!("git commit: {}", built::GIT_COMMIT_HASH.unwrap_or_default());
    println!("build time: {}", built::BUILT_TIME_UTC);
    println!("rustc version: {}", built::RUSTC_VERSION);
    println!(
        "target system: {}-{}",
        built::CFG_OS,
        built::CFG_TARGET_ARCH
    );
    println!("debug: {}", built::DEBUG);
}

fn print_readme_via_less() -> Result<()> {
    let mut child = std::process::Command::new("less")
        .arg("-R")
        .stdin(Stdio::piped())
        .spawn()
        .context("cannot run \"less\"")?;
    let child_stdin = child
        .stdin
        .as_mut()
        .context("no stdin when spawning \"less\"")?;
    write!(child_stdin, "{README}").context("cannot write README to \"less\" stdin")?;
    child.wait().context("\"less\" exited abnormally")?;
    return Ok(());
}

pub fn print_readme() {
    if !print_readme_via_less().to_bool() {
        print!("{README}");
    }
}
