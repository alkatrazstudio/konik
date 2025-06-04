// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::project_info;

pub struct ProjectFileString {
    description: &'static str,
    paths: Option<ProjectFilePaths>,
}

struct ProjectFilePaths {
    dir: PathBuf,
    full_filename: PathBuf,
}

pub struct ProjectFileJson {
    file: ProjectFileString,
}

impl ProjectFileString {
    fn dirs() -> Option<ProjectDirs> {
        let mut proj_title = project_info::title().to_string();
        if cfg!(debug_assertions) {
            proj_title += "_Debug";
        }
        return ProjectDirs::from(
            project_info::qualifier(),
            project_info::organization(),
            &proj_title,
        );
    }

    pub fn dir_for_data() -> Option<PathBuf> {
        return Self::dirs().map(|dirs| dirs.data_dir().to_path_buf());
    }

    pub fn for_data(filename: &str, description: &'static str) -> Self {
        if let Some(dir) = Self::dir_for_data() {
            let full_filename = dir.join(filename);
            return Self {
                description,
                paths: Some(ProjectFilePaths { dir, full_filename }),
            };
        }
        return Self {
            description,
            paths: None,
        };
    }

    fn paths(&self) -> Result<&ProjectFilePaths> {
        if let Some(paths) = &self.paths {
            return Ok(paths);
        }
        bail!(format!(
            "unable to determine the file location for {}",
            self.description
        ))
    }

    pub fn load(&self) -> Result<String> {
        let paths = self.paths()?;
        return fs::read_to_string(&paths.full_filename).with_context(|| {
            format!(
                "cannot read {}: {}",
                self.description,
                paths.full_filename.to_string_lossy()
            )
        });
    }

    pub fn save(&self, contents: &str) -> Result<()> {
        let paths = self.paths()?;
        fs::create_dir_all(&paths.dir).with_context(|| {
            format!(
                "cannot create directory for {}: {}",
                self.description,
                paths.full_filename.to_string_lossy()
            )
        })?;
        fs::write(&paths.full_filename, contents).with_context(|| {
            format!(
                "cannot write to {}: {}",
                self.description,
                paths.full_filename.to_string_lossy()
            )
        })?;
        return Ok(());
    }

    pub fn filename(&self) -> Result<&PathBuf> {
        let paths = self.paths()?;
        return Ok(&paths.full_filename);
    }
}

impl ProjectFileJson {
    pub fn for_data(filename: &str, description: &'static str) -> Self {
        return Self {
            file: ProjectFileString::for_data(filename, description),
        };
    }

    pub fn load<T>(&self) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let json = self.file.load()?;
        let result = serde_json::from_str(&json)
            .with_context(|| format!("cannot parse {}", self.file.description))?;
        return Ok(result);
    }

    pub fn save<T>(&self, obj: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let json = serde_json::to_string(obj)
            .with_context(|| format!("cannot serialize {}", self.file.description))?;
        self.file.save(&json)?;
        return Ok(());
    }
}
