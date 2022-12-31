// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::io::{self, Write};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

#[derive(Parser, Serialize, Deserialize, Clone)]
#[clap(author, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Print version information
    #[clap(long, short = 'v')]
    pub version: bool,

    #[clap(value_parser)]
    pub paths: Vec<String>,
}

#[derive(Subcommand, Serialize, Deserialize, Clone)]
pub enum Command {
    /// Authenticate with Last.fm
    #[clap(name = "lastfm-auth")]
    LastFMAuth,

    /// Authenticate with ListenBrainz
    #[clap(name = "listenbrainz-auth")]
    ListenBrainzAuth,

    /// Open the data folder
    #[clap(name = "data-folder")]
    DataFolder,

    /// Print a short manual
    Readme,

    /// Print detailed version information
    Version,
}

pub fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().context("cannot flush stdout")?;
    let mut s = String::default();
    io::stdin().read_line(&mut s).context("cannot read line")?;
    return Ok(s.trim().to_string());
}
