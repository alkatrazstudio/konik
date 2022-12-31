// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::Path, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
pub struct Track {
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

#[derive(Default, Clone)]
pub struct TrackMeta {
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub track: Option<usize>,
    pub track_total: Option<usize>,
    pub disc: Option<usize>,
    pub disc_total: Option<usize>,
    pub year: Option<usize>,
    pub duration: Duration,
}

pub struct StreamPacketMeta {
    pub channels_count: usize,
    pub sample_rate: usize,
    pub track_meta: Option<TrackMeta>,
    pub position: Option<Duration>,
}

pub trait Stream: Sync + Send {
    fn open(path: &str) -> Result<Self>
    where
        Self: Sized;
    fn is_path_supported(path: &str) -> bool
    where
        Self: Sized;
    fn read_packet(&mut self) -> Result<StreamPacketMeta>;
    fn write(&mut self, data: &mut VecDeque<f32>) -> Result<usize>;
    fn seek(&mut self, pos: Duration) -> Result<Duration>;
}

pub trait StreamHelper {
    fn is_extension_supported(path: &str, supported_exts: &[&str]) -> bool;
}

impl<T> StreamHelper for T
where
    T: Stream,
{
    fn is_extension_supported(path: &str, supported_exts: &[&str]) -> bool {
        if let Some(path_ext) = Path::new(path).extension() {
            for ext in supported_exts {
                if path_ext.eq_ignore_ascii_case(ext) {
                    return true;
                }
            }
        }
        return false;
    }
}
