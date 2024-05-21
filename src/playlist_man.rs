// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use path_absolutize::Absolutize;
use url::Url;
use walkdir::WalkDir;

use crate::{
    cue::CueFactory,
    err_util::{IgnoreErr, LogErr},
    project_file::ProjectFileJson,
    stream_base::Track,
    stream_man,
};

fn file() -> ProjectFileJson {
    return ProjectFileJson::for_data("playlist.json", "playlist");
}

pub fn save_playlist(tracks: &[Track]) -> Result<()> {
    return file().save(tracks);
}

pub fn load_playlist() -> Result<Vec<Track>> {
    return file().load();
}

fn uri_to_str(uri_str: &String) -> PathBuf {
    if uri_str.starts_with("file://") {
        match Url::parse(uri_str) {
            Ok(url) => match url.to_file_path() {
                Ok(path) => {
                    return path;
                }
                Err(()) => {
                    anyhow!("cannot get filesystem path from URL: {uri_str}").log();
                }
            },
            Err(e) => e.log_context(format!("invalid URL: {uri_str}")),
        }
    }
    return uri_str.into();
}

pub fn collect_tracks(paths: &[String]) -> (Vec<Track>, CueFactory) {
    let mut cue_factory = CueFactory::new();

    #[allow(clippy::needless_collect)] // not actually "needless"
    let tracks: Vec<Track> = paths
        .iter()
        .map(uri_to_str)
        .flat_map(WalkDir::new)
        .filter_map(|entry| entry.to_option())
        .filter_map(|entry| {
            if entry.file_type().is_file() {
                return entry
                    .path()
                    .absolutize()
                    .to_option()
                    .and_then(|s| s.to_str().map(|s| s.to_string()));
            }
            return None;
        })
        .filter_map(|path| {
            if stream_man::is_path_supported(&path) {
                return Some(vec![Track {
                    filename: path,
                    index: None,
                }]);
            }

            return cue_factory.get_or_new(&path).map_to_option(move |sheet| {
                sheet.map(|sheet| {
                    sheet
                        .track_ids()
                        .iter()
                        .map(|id| Track {
                            filename: path.clone(),
                            index: Some(*id),
                        })
                        .collect()
                })
            });
        })
        .flatten()
        .collect();

    let cue_source_filenames = cue_factory
        .sheets()
        .iter()
        .map(|sheet| sheet.source_filename.clone())
        .collect::<Vec<String>>();
    let mut tracks = tracks
        .into_iter()
        .filter(|track| !cue_source_filenames.contains(&track.filename))
        .collect::<Vec<Track>>();

    tracks.sort_by(|a, b| {
        alphanumeric_sort::compare_str(a.filename.to_uppercase(), b.filename.to_uppercase())
            .then_with(|| a.index.cmp(&b.index))
    });

    return (tracks, cue_factory);
}
