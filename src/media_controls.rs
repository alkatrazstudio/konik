// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::time::Duration;

use anyhow::{Context, Result};
use souvlaki::{MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};

use crate::{err_util::IgnoreErr, player::PlaybackState, project_info, stream_base::TrackMeta};

pub struct MediaControls {
    controls: souvlaki::MediaControls,
}

impl MediaControls {
    pub fn new_if_available() -> Option<Self> {
        return souvlaki::MediaControls::new(PlatformConfig {
            display_name: project_info::title(),
            dbus_name: project_info::name(),
            hwnd: None,
        })
        .to_anyhow()
        .context("cannot create media controls")
        .map_to_option(|controls| Some(Self { controls }));
    }

    pub fn attach<F>(&mut self, event_handler: F) -> Result<()>
    where
        F: Fn(MediaControlEvent) + Send + 'static,
    {
        return self
            .controls
            .attach(event_handler)
            .to_anyhow()
            .context("cannot attach");
    }

    pub fn set_state(&mut self, state: &PlaybackState, position: Option<Duration>) -> Result<()> {
        match state {
            PlaybackState::Playing => {
                if let Some(position) = position {
                    self.controls
                        .set_playback(MediaPlayback::Playing {
                            progress: Some(MediaPosition(position)),
                        })
                        .to_anyhow()
                        .context("cannot set playing state")?;
                }
            }
            PlaybackState::Stopped => {
                self.controls
                    .set_playback(MediaPlayback::Stopped)
                    .to_anyhow()
                    .context("cannot set stopped state")?;
            }
            PlaybackState::Paused => {
                if let Some(position) = position {
                    self.controls
                        .set_playback(MediaPlayback::Paused {
                            progress: Some(MediaPosition(position)),
                        })
                        .to_anyhow()
                        .context("cannot set paused state")?;
                }
            }
        }
        return Ok(());
    }

    pub fn set_metadata(&mut self, track_meta: &TrackMeta) -> Result<()> {
        let title = track_meta.title.as_deref();
        let artist = track_meta.artist.as_deref();
        let album = track_meta.album.as_deref();

        self.controls
            .set_metadata(MediaMetadata {
                title,
                artist,
                album,
                duration: Some(track_meta.duration),
                ..Default::default()
            })
            .to_anyhow()
            .context("cannot set metadata")?;
        return Ok(());
    }
}
