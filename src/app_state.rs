// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{err_util::LogErr, project_file::ProjectFileJson};

#[derive(Serialize, Deserialize)]
pub struct AppState {
    pub playlist_index: Option<usize>,
    pub volume: f32,
}

impl Default for AppState {
    fn default() -> Self {
        return Self {
            playlist_index: None,
            volume: 1.0,
        };
    }
}

impl AppState {
    pub fn load_or_default() -> Self {
        return match Self::file().load() {
            Ok(state) => state,
            Err(e) => {
                e.log();
                Self::default()
            }
        };
    }

    pub fn save(&self) -> Result<()> {
        return Self::file().save(&self);
    }

    fn file() -> ProjectFileJson {
        return ProjectFileJson::for_data("state.json", "state file");
    }
}
