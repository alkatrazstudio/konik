// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use crate::{err_util::LogErr, stream_base::Stream, symphonia_stream::SymphoniaStream};
use anyhow::{bail, Result};

fn open_stream<T: Stream + 'static>(path: &str) -> Option<Box<dyn Stream>> {
    if !T::is_path_supported(path) {
        return None;
    }

    return match T::open(path) {
        Ok(source) => Some(Box::new(source)),
        Err(e) => {
            e.context(format!("cannot open {path}")).log();
            None
        }
    };
}

pub fn is_path_supported(path: &str) -> bool {
    if SymphoniaStream::is_path_supported(path) {
        return true;
    }
    return false;
}

pub fn open(path: &str) -> Result<Box<dyn Stream>> {
    if let Some(stream) = open_stream::<SymphoniaStream>(path) {
        return Ok(stream);
    }

    bail!("file not supported: {}", path);
}
