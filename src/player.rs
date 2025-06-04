// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender, channel},
};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use cpal::traits::StreamTrait;

use crate::{
    cue::CueFactory,
    decoder::{Decoder, DecoderReadResult},
    err_util::{IgnoreErr, LogErr, eprintln_with_date},
    stream_base::{Track, TrackMeta},
    thread_util,
};

const DECODER_THREAD_SLEEP: Duration = Duration::from_millis(100);
const READ_PACKETS_PER_CYCLE: u8 = 5;

pub enum PlayerCmd {
    SetPlaylist {
        tracks: Vec<Track>,
        cue_factory: Option<CueFactory>,
    },

    LoadMeta {
        index: usize,
    },

    Play {
        index: Option<usize>,
    },
    Pause,
    UnPause,
    Stop,
    RequestPosition,

    Next,
    Prev,
    NextDir,
    PrevDir,

    SeekBy {
        forward: bool,
        length: Duration,
    },
    SeekTo {
        position: Duration,
    },

    SetVolume {
        volume: f32,
    },

    Exit,
}

pub enum PlayerResponse {
    NewPlaylistIndex {
        playlist_index: usize,
        track: Track,
        user_navigation: bool,
    },
    NewMeta {
        meta: TrackMeta,
        user_navigation: bool,
    },
    PlaybackStateChanged {
        state: PlaybackState,
        position: Duration,
    },
    PositionRequested {
        position: Duration,
    },
    PositionCallback {
        callback: PositionCallback,
    },
    PlaylistEnded,
    Seeked {
        position: Duration,
    },
    VolumeSet {
        volume: f32,
    },
    Exited,
}

#[derive(Clone, Copy)]
enum MoveTo {
    Next,
    Prev,
    NextDir,
    PrevDir,
}

#[derive(Debug, Default, Clone)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

pub type PositionCallbackId = u32;

#[derive(Clone)]
pub enum PositionCallbackMarker {
    SecsFromStart(Duration),
    SecsFromEnd(Duration),
}

#[derive(Clone)]
pub struct PositionCallback {
    pub id: PositionCallbackId,
    pub marker: PositionCallbackMarker,
}

pub type PositionCallbacks = Vec<PositionCallback>;

struct PlayerThread {
    decoder: Decoder,
    playlist: Vec<Track>,
    playlist_index: usize,
    sent_playlist_index: Option<usize>,
    rx: Receiver<PlayerCmd>,
    tx: Sender<PlayerResponse>,
    position_callbacks: Option<PositionCallbacks>,
    triggered_callbacks: Vec<PositionCallbackId>,
    user_navigation_for_next_meta: bool,
    need_fast_read: bool,
    output: Option<cpal::Stream>,
    output_is_paused: bool,
}

impl PositionCallback {
    pub fn from_start(id: PositionCallbackId, secs: f64) -> Self {
        return Self {
            id,
            marker: PositionCallbackMarker::SecsFromStart(Duration::from_secs_f64(secs)),
        };
    }

    pub fn from_end(id: PositionCallbackId, secs: f64) -> Self {
        return Self {
            id,
            marker: PositionCallbackMarker::SecsFromEnd(Duration::from_secs_f64(secs)),
        };
    }
}

impl PlayerThread {
    fn new(
        tx: Sender<PlayerResponse>,
        rx: Receiver<PlayerCmd>,
        position_callbacks: Option<PositionCallbacks>,
    ) -> Self {
        return Self {
            decoder: Decoder::new(),
            playlist: Vec::new(),
            playlist_index: 0,
            sent_playlist_index: None,
            rx,
            tx,
            position_callbacks,
            triggered_callbacks: Vec::new(),
            user_navigation_for_next_meta: false,
            need_fast_read: true,
            output: None,
            output_is_paused: false,
        };
    }

    fn stop(&mut self) {
        self.decoder.stop();
        self.output = None;
        self.sent_playlist_index = None;
        self.tx
            .send(PlayerResponse::PlaybackStateChanged {
                state: PlaybackState::Stopped,
                position: Duration::ZERO,
            })
            .unwrap();
    }

    fn set_playlist(&mut self, files: Vec<Track>, cue_factory: Option<CueFactory>) {
        self.stop();
        if let Some(cue_factory) = cue_factory {
            self.decoder.set_cue_factory(cue_factory);
        } else {
            self.decoder.clear_cue_factory();
        }
        self.playlist = files;
        self.playlist_index = 0;
    }

    fn load_meta(&mut self, index: usize) -> Result<()> {
        let track = &self.playlist[index];
        self.decoder.load_meta(track).context("cannot load meta")?;
        self.playlist_index = index;

        self.tx
            .send(PlayerResponse::NewPlaylistIndex {
                playlist_index: index,
                track: track.clone(),
                user_navigation: false,
            })
            .unwrap();

        if let Some(meta) = self.decoder.track_meta.clone() {
            self.tx
                .send(PlayerResponse::NewMeta {
                    meta,
                    user_navigation: false,
                })
                .unwrap();
        }

        return Ok(());
    }

    fn play(&mut self, index: Option<usize>, user_navigation: bool) -> Result<()> {
        let index = index.unwrap_or(self.playlist_index);
        if index >= self.playlist.len() {
            bail!("index {} is not in the playlist", index);
        }
        let track = &self.playlist[index];
        self.playlist_index = index;
        self.decoder.play(track).context("cannot play")?;
        self.need_fast_read = true;
        self.triggered_callbacks.clear();
        self.send_playlist_index(user_navigation);
        self.user_navigation_for_next_meta = user_navigation;
        self.tx
            .send(PlayerResponse::PlaybackStateChanged {
                state: PlaybackState::Playing,
                position: Duration::ZERO,
            })
            .unwrap();
        return Ok(());
    }

    fn playlist_index_dir(&self, index: usize) -> PathBuf {
        let track = &self.playlist[index];
        let path = Path::new(&track.filename)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        return path;
    }

    fn fetch_next_playlist_index(
        &self,
        cur_index: usize,
        wrap: bool,
        emit_ended: bool,
    ) -> Result<usize> {
        if cur_index < self.playlist.len() - 1 {
            return Ok(cur_index + 1);
        }
        if wrap {
            return Ok(0);
        }

        if emit_ended {
            self.tx.send(PlayerResponse::PlaylistEnded).unwrap();
        }
        bail!("playlist end reached");
    }

    fn fetch_prev_playlist_index(&self, cur_index: usize, wrap: bool) -> Result<usize> {
        if cur_index > 0 {
            return Ok(cur_index - 1);
        }

        if wrap {
            return Ok(self.playlist.len() - 1);
        }

        bail!("playlist start reached");
    }

    fn fetch_first_playlist_index_in_dir(
        &self,
        cur_index: usize,
        stop_index: usize,
        wrap: bool,
        files_left: &mut usize,
    ) -> Result<usize> {
        let mut cur_dir = self.playlist_index_dir(cur_index);
        let mut index = self.fetch_prev_playlist_index(cur_index, wrap)?;
        if index != 0 && index != stop_index && self.playlist_index_dir(index) != cur_dir {
            cur_dir = self.playlist_index_dir(index);
        }
        while index != 0 && index != stop_index && self.playlist_index_dir(index - 1) == *cur_dir {
            Self::dec_valid_files(files_left).context("no valid left")?;
            index = self
                .fetch_prev_playlist_index(index, wrap)
                .context("cannot fetch previous playlist index")?;
        }
        return Ok(index);
    }

    fn dec_valid_files(x: &mut usize) -> Result<()> {
        if *x == 0 {
            bail!("no valid files in the playlist");
        }
        *x -= 1;
        return Ok(());
    }

    fn move_and_play(&mut self, step: MoveTo, wrap: bool, user_navigation: bool) -> Result<()> {
        let mut files_left = self.playlist.len();
        if files_left == 0 {
            bail!("no files in the playlist");
        }
        let start_index = self.playlist_index;
        let mut cur_index = self.playlist_index;
        let mut index_after_dir_skip: Option<usize> = None;
        loop {
            Self::dec_valid_files(&mut files_left)?;

            let new_playlist_index = match step {
                MoveTo::Next => self.fetch_next_playlist_index(cur_index, wrap, true)?,
                MoveTo::Prev => self.fetch_prev_playlist_index(cur_index, wrap)?,
                MoveTo::NextDir => {
                    let mut index = self.fetch_next_playlist_index(cur_index, wrap, true)?;
                    if index_after_dir_skip.is_none() {
                        let cur_dir = self.playlist_index_dir(cur_index);
                        while index != 0 && self.playlist_index_dir(index) == cur_dir {
                            Self::dec_valid_files(&mut files_left)?;
                            index = self.fetch_next_playlist_index(index, wrap, true)?;
                        }
                        index_after_dir_skip = Some(index);
                    }
                    index
                }
                MoveTo::PrevDir => {
                    if let Some(found_index) = index_after_dir_skip {
                        if let Ok(next_index) =
                            self.fetch_next_playlist_index(cur_index, wrap, false)
                        {
                            if start_index != next_index
                                && self.playlist_index_dir(next_index)
                                    == self.playlist_index_dir(cur_index)
                            {
                                next_index
                            } else {
                                let index = self.fetch_first_playlist_index_in_dir(
                                    found_index,
                                    start_index,
                                    wrap,
                                    &mut files_left,
                                )?;
                                index_after_dir_skip = Some(index);
                                index
                            }
                        } else {
                            let index = self.fetch_first_playlist_index_in_dir(
                                found_index,
                                start_index,
                                wrap,
                                &mut files_left,
                            )?;
                            index_after_dir_skip = Some(index);
                            index
                        }
                    } else {
                        let index = self.fetch_first_playlist_index_in_dir(
                            cur_index,
                            start_index,
                            wrap,
                            &mut files_left,
                        )?;
                        index_after_dir_skip = Some(index);
                        index
                    }
                }
            };

            if self
                .play(Some(new_playlist_index), user_navigation)
                .to_bool()
            {
                return Ok(());
            }
            cur_index = self.playlist_index;
        }
    }

    fn next(&mut self, wrap: bool, user_navigation: bool) -> Result<()> {
        return self.move_and_play(MoveTo::Next, wrap, user_navigation);
    }

    fn prev(&mut self) -> Result<()> {
        return self.move_and_play(MoveTo::Prev, true, true);
    }

    fn next_dir(&mut self) -> Result<()> {
        return self.move_and_play(MoveTo::NextDir, true, true);
    }

    fn prev_dir(&mut self) -> Result<()> {
        return self.move_and_play(MoveTo::PrevDir, true, true);
    }

    fn send_playlist_index(&mut self, user_navigation: bool) {
        if let Some(index) = self.sent_playlist_index {
            if index == self.playlist_index {
                return;
            }
        }

        if self.playlist_index >= self.playlist.len() {
            return;
        }

        self.tx
            .send(PlayerResponse::NewPlaylistIndex {
                playlist_index: self.playlist_index,
                track: self.playlist[self.playlist_index].clone(),
                user_navigation,
            })
            .ignore_err();

        self.sent_playlist_index = Some(self.playlist_index);
    }

    fn pause(&mut self) -> Result<()> {
        if let Some(output) = &self.output {
            output.pause()?;
            self.output_is_paused = true;
            self.tx
                .send(PlayerResponse::PlaybackStateChanged {
                    state: PlaybackState::Paused,
                    position: self.decoder.playback_position(),
                })
                .unwrap();
            return Ok(());
        }
        bail!("no output created");
    }

    fn unpause(&mut self) -> Result<()> {
        if let Some(output) = &self.output {
            output.play()?;
            self.output_is_paused = false;
            self.tx
                .send(PlayerResponse::PlaybackStateChanged {
                    state: PlaybackState::Playing,
                    position: self.decoder.playback_position(),
                })
                .unwrap();
            return Ok(());
        }
        bail!("no output created");
    }

    fn seek_to(&mut self, pos: Duration) -> Result<()> {
        let seeked_to = self.decoder.seek_to(pos)?;
        self.tx
            .send(PlayerResponse::Seeked {
                position: seeked_to,
            })
            .unwrap();
        return Ok(());
    }

    fn send_position(&self) {
        let position = self.decoder.playback_position();
        self.tx
            .send(PlayerResponse::PositionRequested { position })
            .unwrap();
    }

    fn process_client_cmd(&mut self) -> Result<bool> {
        let recv_timeout = if self.need_fast_read {
            Duration::ZERO
        } else {
            DECODER_THREAD_SLEEP
        };
        if let Ok(cmd) = self.rx.recv_timeout(recv_timeout) {
            match cmd {
                PlayerCmd::SetPlaylist {
                    tracks,
                    cue_factory,
                } => {
                    self.set_playlist(tracks, cue_factory);
                }
                PlayerCmd::LoadMeta { index } => {
                    self.stop();
                    let mut index = index;
                    let playlist_len = self.playlist.len();
                    let mut is_loaded = false;
                    while index < playlist_len {
                        if self.load_meta(index).to_bool() {
                            is_loaded = true;
                            break;
                        }
                        index += 1;
                    }
                    if !is_loaded {
                        eprintln_with_date("the current file is not valid");
                    }
                }
                PlayerCmd::Play { index } => {
                    self.stop();
                    if !self
                        .play(index, true)
                        .with_context(|| format!("cannot play track {index:?}"))
                        .to_bool()
                    {
                        self.next(false, true).context("cannot play next track")?;
                    }
                }
                PlayerCmd::Stop => {
                    self.stop();
                }
                PlayerCmd::RequestPosition => {
                    self.send_position();
                }
                PlayerCmd::Next => {
                    self.stop();
                    self.next(true, true).context("cannot play next track")?;
                }
                PlayerCmd::Prev => {
                    self.stop();
                    self.prev().context("cannot play previous track")?;
                }
                PlayerCmd::NextDir => {
                    self.stop();
                    self.next_dir().context("cannot jump to next directory")?;
                }
                PlayerCmd::PrevDir => {
                    self.stop();
                    self.prev_dir()
                        .context("cannot jump to previous directory")?;
                }
                PlayerCmd::Pause => {
                    self.pause().context("cannot pause")?;
                }
                PlayerCmd::UnPause => {
                    self.unpause().context("cannot unpause")?;
                }
                PlayerCmd::SeekBy { forward, length } => {
                    let result_pos = if forward {
                        self.decoder.playback_position().saturating_add(length)
                    } else {
                        self.decoder.playback_position().saturating_sub(length)
                    };
                    self.seek_to(result_pos).context("cannot seek")?;
                }
                PlayerCmd::SeekTo { position } => {
                    self.seek_to(position).context("cannot seek")?;
                }
                PlayerCmd::SetVolume { volume } => {
                    let volume = self.decoder.set_volume(volume);
                    self.tx.send(PlayerResponse::VolumeSet { volume })?;
                }
                PlayerCmd::Exit => {
                    self.tx.send(PlayerResponse::Exited)?;
                    return Ok(false);
                }
            }
        }
        return Ok(true);
    }

    fn send_new_meta(&mut self) {
        if let Some(track_meta) = self.decoder.new_track_meta.take() {
            self.tx
                .send(PlayerResponse::NewMeta {
                    meta: track_meta,
                    user_navigation: self.user_navigation_for_next_meta,
                })
                .unwrap();
            self.user_navigation_for_next_meta = false;
        }
    }

    fn process_position_callbacks(&mut self) {
        if let (Some(callbacks), Some(duration)) = (
            &self.position_callbacks,
            &self.decoder.track_meta.as_ref().map(|m| m.duration),
        ) {
            match self.decoder.valid_playback_position() {
                Ok(position) => {
                    for callback in callbacks {
                        if !self.triggered_callbacks.contains(&callback.id) {
                            let must_trigger = match callback.marker {
                                PositionCallbackMarker::SecsFromStart(marker) => position >= marker,
                                PositionCallbackMarker::SecsFromEnd(marker) => {
                                    let pos_from_start = duration.saturating_sub(marker);
                                    position >= pos_from_start
                                }
                            };
                            if must_trigger {
                                self.tx
                                    .send(PlayerResponse::PositionCallback {
                                        callback: callback.clone(),
                                    })
                                    .unwrap();
                                self.triggered_callbacks.push(callback.id);
                            }
                        }
                    }
                }
                Err(e) => e.log(),
            }
        }
    }

    fn read_stream(&mut self) -> bool {
        let mut may_create_output = false;
        let mut need_next_track = false;
        let mut need_read_fast = false;
        match self.decoder.read_stream() {
            DecoderReadResult::BufferNotFull => {
                need_read_fast = true;
            }
            DecoderReadResult::BufferFull => {
                may_create_output = true;
            }
            DecoderReadResult::NeedResetOutput => {
                self.output = None;
            }
            DecoderReadResult::ReadEnd => {
                need_next_track = true;
            }
        }

        self.send_new_meta();
        if self.output.is_some() && !self.output_is_paused {
            self.process_position_callbacks();
        }

        if need_next_track {
            if !self.next(false, false).to_bool() {
                self.stop();
                return false;
            }
            return true;
        }

        if may_create_output && self.output.is_none() {
            self.output = self.decoder.create_output_stream();
            if self.output.is_some() {
                self.output_is_paused = false;
            }
        }
        return need_read_fast;
    }

    fn read_stream_packets_batch(&mut self) -> bool {
        let mut packets_left = READ_PACKETS_PER_CYCLE;
        while packets_left > 0 {
            if !self.read_stream() {
                return false;
            }
            packets_left -= 1;
        }
        return true;
    }

    fn process(&mut self) -> bool {
        match self.process_client_cmd() {
            Ok(res) => {
                if !res {
                    return false;
                }
            }
            Err(e) => e.log(),
        }
        self.need_fast_read = self.read_stream_packets_batch();
        return true;
    }
}

pub struct PlayerTx {
    tx: Arc<Mutex<Sender<PlayerCmd>>>,
    server_thread: Option<JoinHandle<()>>,
}

impl PlayerTx {
    pub fn new(tx: Sender<PlayerCmd>, server_thread: JoinHandle<()>) -> Self {
        return Self {
            tx: Arc::new(Mutex::new(tx)),
            server_thread: Some(server_thread),
        };
    }

    pub fn send(&self, cmd: PlayerCmd) {
        self.tx.lock().unwrap().send(cmd).unwrap();
    }

    pub fn set_playlist(&self, tracks: Vec<Track>, cue_factory: Option<CueFactory>) {
        self.send(PlayerCmd::SetPlaylist {
            tracks,
            cue_factory,
        });
    }

    pub fn play(&self, index: Option<usize>) {
        self.send(PlayerCmd::Play { index });
    }

    pub fn load_meta(&self, index: usize) {
        self.send(PlayerCmd::LoadMeta { index });
    }

    pub fn pause(&self) {
        self.send(PlayerCmd::Pause);
    }

    pub fn unpause(&self) {
        self.send(PlayerCmd::UnPause);
    }

    pub fn stop(&self) {
        self.send(PlayerCmd::Stop);
    }

    pub fn request_position(&self) {
        self.send(PlayerCmd::RequestPosition);
    }

    pub fn next(&self) {
        self.send(PlayerCmd::Next);
    }

    pub fn prev(&self) {
        self.send(PlayerCmd::Prev);
    }

    pub fn next_dir(&self) {
        self.send(PlayerCmd::NextDir);
    }

    pub fn prev_dir(&self) {
        self.send(PlayerCmd::PrevDir);
    }

    pub fn seek_to(&self, position: Duration) {
        self.send(PlayerCmd::SeekTo { position });
    }

    pub fn seek_by(&self, forward: bool, length: Duration) {
        self.send(PlayerCmd::SeekBy { forward, length });
    }

    pub fn set_volume(&self, volume: f32) {
        self.send(PlayerCmd::SetVolume { volume });
    }

    pub fn exit(&self) {
        self.send(PlayerCmd::Exit);
    }

    pub fn wait(&mut self) {
        if let Some(t) = self.server_thread.take() {
            t.join().to_anyhow().ignore_err();
        }
    }
}

pub fn start_thread(
    position_callbacks: Option<PositionCallbacks>,
) -> (PlayerTx, Receiver<PlayerResponse>) {
    let (tx, rx) = channel();
    let (dtx, drx) = channel();

    let server_thread = thread_util::thread("player server", move || {
        let mut decoder = PlayerThread::new(dtx, rx, position_callbacks);
        while decoder.process() {}
    });

    return (PlayerTx::new(tx, server_thread), drx);
}
