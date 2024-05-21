// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    // all
    clippy::needless_return,

    // pedantic
    clippy::doc_markdown,
    clippy::module_name_repetitions,
    clippy::needless_raw_string_hashes,
    clippy::redundant_closure_for_method_calls,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::cast_possible_wrap,

    // nursery
    clippy::option_if_let_else,
    clippy::missing_const_for_fn,
    clippy::use_self, // bugged for macros expansions
)]

mod app;
mod app_state;
mod cli;
mod cue;
mod decoder;
mod entry;
mod err_util;
mod hotkeys;
mod lastfm;
mod listenbrainz;
mod media_controls;
mod player;
mod playlist_man;
mod popup;
mod project_file;
mod project_info;
mod quit_signal;
mod show_file;
mod singleton;
mod stream_base;
mod stream_man;
mod symphonia_stream;
mod sys_vol;
mod thread_util;
mod tray_icon;

fn main() -> anyhow::Result<()> {
    return entry::main();
}
