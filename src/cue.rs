// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use cuna::{track::Track, Cuna};
use regex::Regex;

use crate::{
    err_util::{eprintln_with_date, LogErr},
    stream_base::TrackMeta,
};

const SOURCE_EXTS: [&str; 1] = ["flac"];

struct CueTrack {
    index: usize,
    start: Duration,
    duration: Option<Duration>,
    meta: TrackMeta,
}

pub struct CueSheet {
    tracks: Vec<CueTrack>,
    pub source_filename: String,
}

impl CueSheet {
    fn is_supported_file(filename: &str) -> bool {
        let len = filename.len();
        if len < 4 {
            return false;
        }
        let last_chars = &filename[len - 4..];
        let eq = last_chars.eq_ignore_ascii_case(".cue");
        return eq;
    }

    fn find_source(cue_filename: &str) -> Option<String> {
        let cue_path = Path::new(cue_filename);
        if let Some(cue_dir) = cue_path.parent() {
            match fs::read_dir(cue_dir) {
                Ok(items) => {
                    let items = items
                        .filter_map(|item| match item {
                            Ok(item) => match item.metadata() {
                                Ok(metadata) => {
                                    if metadata.is_file() {
                                        let filename = item.file_name();
                                        let p: &Path = filename.as_ref();
                                        Some(p.to_path_buf())
                                    } else {
                                        None
                                    }
                                }
                                Err(e) => {
                                    e.log();
                                    None
                                }
                            },
                            Err(e) => {
                                e.log();
                                None
                            }
                        })
                        .filter(|filename| {
                            filename
                                .extension()
                                .and_then(|src_ext| {
                                    let src_ext = src_ext.to_string_lossy();
                                    SOURCE_EXTS
                                        .iter()
                                        .find(|ext| ext.eq_ignore_ascii_case(&src_ext))
                                })
                                .is_some()
                        })
                        .collect::<Vec<PathBuf>>();

                    if let Some(full_filename) = items.iter().find_map(|filename| {
                        let mut full_filename = cue_dir.to_path_buf();
                        full_filename.push(filename);
                        if full_filename
                            .with_extension("cue")
                            .to_string_lossy()
                            .eq_ignore_ascii_case(cue_filename)
                        {
                            return Some(full_filename);
                        }
                        if (full_filename.to_string_lossy() + ".cue")
                            .eq_ignore_ascii_case(cue_filename)
                        {
                            return Some(full_filename);
                        }
                        return None;
                    }) {
                        return full_filename.to_str().map(|s| s.to_string());
                    }
                }
                Err(e) => {
                    e.log_context(format!("reading dir failed {}", cue_dir.to_string_lossy()));
                }
            }
        }
        return None;
    }

    fn new(filename: &str) -> Result<Self> {
        let s = fs::read_to_string(filename).with_context(|| format!("cannot read: {filename}"))?;
        let cue = Cuna::new(&s).with_context(|| format!("cannot parse CUE: {filename}"))?;

        let source_filename = Self::find_source(filename)
            .with_context(|| format!("no source file found for {filename}"))?;

        let mut tracks: Vec<CueTrack> = Vec::new();
        if let Some(file) = cue.first_file() {
            let tracks_count = file.tracks.len();
            for track in file.tracks.iter().rev() {
                let index = track.id() as usize;
                let start = Self::extract_track_start(track)
                    .with_context(|| format!("cannot extract track {index} start"))?;
                let duration = if tracks.is_empty() {
                    None
                } else {
                    let start_next = &tracks[tracks.len() - 1].start;
                    let duration = start_next.saturating_sub(start);
                    if duration.is_zero() {
                        bail!("track {} has zero length", index);
                    }
                    Some(duration)
                };
                let meta = Self::extract_track_meta(&cue, track, tracks_count);

                tracks.push(CueTrack {
                    index,
                    start,
                    duration,
                    meta,
                });
            }
        }

        if tracks.is_empty() {
            bail!("no tracks found in CUE file: {}", filename);
        }

        tracks.reverse();

        return Ok(Self {
            tracks,
            source_filename,
        });
    }

    pub fn track_ids(&self) -> Vec<usize> {
        return self.tracks.iter().map(|t| t.index).collect();
    }

    fn extract_track_start(track: &Track) -> Result<Duration> {
        for i in &track.index {
            if i.id() == 1 {
                let dur = i.begin_time.into();
                return Ok(dur);
            }
        }
        bail!("cannot detect the start of track {}", track.id());
    }

    fn track(&self, index: usize) -> Result<&CueTrack> {
        for track in &self.tracks {
            if track.index == index {
                return Ok(track);
            }
        }
        bail!("trying to get out-of-bounds track {}", index);
    }

    fn opt_str(s: &[String]) -> Option<String> {
        if let Some(s) = s.first() {
            return Some(s.trim().to_string());
        }
        return None;
    }

    fn opt_str2(s1: &[String], s2: &[String]) -> Option<String> {
        if let Some(s) = s1.first() {
            return Some(s.trim().to_string());
        }
        if let Some(s) = s2.first() {
            return Some(s.trim().to_string());
        }
        return None;
    }

    fn extract_comment(cue: &Cuna, tag: &str) -> Option<String> {
        let rx_str = String::from(r"(?i)^") + &regex::escape(tag) + r#"\s+(.+)"?$"#;
        let rx = Regex::new(&rx_str).unwrap();
        for comment in &cue.comments.0 {
            if let Some(m) = rx.captures(comment) {
                if let Some(m) = m.get(1) {
                    let s = m.as_str();
                    if s.starts_with('"') && s.ends_with('"') && s.len() > 1 {
                        return Some(s[1..s.len() - 1].trim().to_string());
                    }
                    return Some(s.trim().to_string());
                }
            }
        }
        return None;
    }

    fn extract_comment_num<T>(cue: &Cuna, tag: &str) -> Option<T>
    where
        T: FromStr + Clone,
    {
        if let Some(comment) = Self::extract_comment(cue, tag) {
            return if let Ok(num) = comment.parse() {
                Some(num)
            } else {
                eprintln_with_date("cannot parse \"{tag}\" as number");
                None
            };
        }
        return None;
    }

    fn extract_track_meta(cue: &Cuna, track: &Track, tracks_count: usize) -> TrackMeta {
        return TrackMeta {
            duration: Duration::ZERO,
            album: Self::opt_str(cue.title()),
            title: Self::opt_str(track.title()),
            artist: Self::opt_str2(track.performer(), cue.performer()),
            disc: Self::extract_comment_num(cue, "DISCNUMBER"),
            disc_total: Self::extract_comment_num(cue, "TOTALDISCS"),
            track: Some(track.id() as usize),
            track_total: Some(tracks_count),
            year: Self::extract_comment_num(cue, "DATE"),
        };
    }

    fn opt_def<T>(opt1: &Option<T>, opt2: &Option<T>) -> Option<T>
    where
        T: Clone,
    {
        if opt1.is_some() {
            return opt1.clone();
        }
        return opt2.clone();
    }

    pub fn track_index_by_position(&self, position: Duration) -> usize {
        for track in self.tracks.iter().rev() {
            if position >= track.start {
                return track.index;
            }
        }
        return self
            .tracks
            .first()
            .expect("CUE file with zero tracks")
            .index;
    }

    pub fn track_start(&self, index: usize) -> Result<Duration> {
        let track = self
            .track(index)
            .context("cannot get track for start info")?;
        return Ok(track.start);
    }

    pub fn track_meta(&self, index: usize, file_meta: &TrackMeta) -> Result<TrackMeta> {
        let track = self
            .track(index)
            .context("cannot get track for meta info")?;
        let meta = &track.meta;
        let duration = track
            .duration
            .unwrap_or_else(|| file_meta.duration.saturating_sub(track.start));

        return Ok(TrackMeta {
            duration,
            album: Self::opt_def(&meta.album, &file_meta.album),
            title: Self::opt_def(&meta.title, &file_meta.title),
            artist: Self::opt_def(&meta.artist, &file_meta.artist),
            disc: meta.disc.or(file_meta.disc),
            disc_total: meta.disc_total.or(file_meta.disc_total),
            track: meta.track,
            track_total: meta.track_total,
            year: meta.year.or(file_meta.year),
        });
    }
}

pub struct CueFactory {
    sheets: HashMap<String, Option<Arc<CueSheet>>>,
}

impl CueFactory {
    pub fn new() -> Self {
        return Self {
            sheets: HashMap::new(),
        };
    }

    pub fn get_or_new(&mut self, filename: &str) -> Result<Option<Arc<CueSheet>>> {
        let filename = filename.to_string();
        if let Some(cue) = self.sheets.get(&filename) {
            return Ok(cue.clone());
        }

        if !CueSheet::is_supported_file(&filename) {
            return Ok(None);
        }

        let sheet = match CueSheet::new(&filename) {
            Ok(sheet) => Some(Arc::new(sheet)),
            Err(e) => bail!("reading CUE sheet {}: {}", filename, e),
        };
        self.sheets.insert(filename, sheet.clone());
        return Ok(sheet);
    }

    pub fn clear(&mut self) {
        self.sheets.clear();
    }

    pub fn sheets(&self) -> Vec<Arc<CueSheet>> {
        return self.sheets.values().filter_map(|v| v.clone()).collect();
    }
}
