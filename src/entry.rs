// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{env::current_dir, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};

use crate::{
    app,
    cli::{self, Args},
    err_util::println_with_date,
    lastfm::LastFM,
    listenbrainz::ListenBrainz,
    project_file::ProjectFileString,
    project_info, quit_signal, show_file,
    singleton::Singleton,
};

const SINGLETON_ID: &str = "bfde662d-2ed2-4672-b3bb-ca27b6b97002";

#[derive(Serialize, Deserialize)]
struct SingletonPayload {
    cli_args: Args,
    current_dir: String,
}

pub fn main() -> Result<()> {
    let cli_args = Args::parse();
    if cli_args.version {
        println!("{}", project_info::version());
        return Ok(());
    }
    if let Some(cmd) = &cli_args.command {
        match cmd {
            cli::Command::LastFMAuth => LastFM::cli_auth()?,
            cli::Command::ListenBrainzAuth => ListenBrainz::cli_auth()?,
            cli::Command::DataFolder => {
                let dir =
                    ProjectFileString::dir_for_data().context("cannot get the config directory")?;
                let dir_str = dir
                    .to_str()
                    .context("cannot convert data directory path to string")?;
                show_file::open_folder(dir_str)?;
            }
            cli::Command::Readme => project_info::print_readme(),
            cli::Command::Version => project_info::print_version_info(),
        }
        return Ok(());
    }

    let singleton_payload = SingletonPayload {
        cli_args: cli_args.clone(),
        current_dir: current_dir()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            .to_string(),
    };
    let singleton_name = format!("{}-{SINGLETON_ID}", project_info::name());
    let single = Singleton::new(&singleton_name, move || Some(singleton_payload))?;
    if let Some(single) = single {
        println_with_date("starting up...");
        let cur_dir = current_dir().unwrap_or_default();
        let app_handle = app::start(&cli_args, &cur_dir)?;

        let app = app_handle.app.clone();
        single.listen(move |payload| {
            app.lock()
                .unwrap()
                .new_args(&payload.cli_args, &PathBuf::from(&payload.current_dir));
        })?;

        let app = app_handle.app.clone();
        quit_signal::listen(move || {
            app.lock().unwrap().quit();
        });

        println_with_date("started");
        app_handle.wait();
        println_with_date("shutdown complete");
    }
    return Ok(());
}
