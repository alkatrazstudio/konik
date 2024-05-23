// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use crate::{
    app_state::AppState,
    cli::Args,
    err_util::{eprintln_with_date, println_with_date, IgnoreErr, LogErr, OptionAnd},
    hotkeys::{HotKeyAction, HotKeys},
    lastfm::LastFM,
    listenbrainz::ListenBrainz,
    media_controls::MediaControls,
    player::{self, PlaybackState, PlayerResponse, PlayerTx, PositionCallback, PositionCallbackId},
    playlist_man,
    popup::Popup,
    show_file::show_file,
    stream_base::{Track, TrackMeta},
    sys_vol::SysVol,
    thread_util,
    tray_icon::{TrayIcon, TrayIconImageType, TrayMenuItem},
};
use anyhow::{Context, Result};
use souvlaki::{MediaControlEvent, SeekDirection};
use std::{
    path::Path,
    sync::{mpsc::Receiver, Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

pub struct App {
    player: PlayerTx,
    playback_state: PlaybackState,
    playlist_index: usize,
    cur_track: Option<Track>,
    meta: TrackMeta,
    hotkeys: HotKeys,
    tray: TrayIcon,
    listenbrainz: Option<ListenBrainz>,
    lastfm: Option<LastFM>,
    state: AppState,
    popup: Popup,
    media_controls: Option<MediaControls>,
    last_seek_position: Option<Duration>,
}

const VOL_STEP: f64 = 0.01;
const POS_CALLBACK_NOW_PLAYING: PositionCallbackId = 0;
const POS_NOW_PLAYING_SECS: f64 = 5.0;
const POS_CALLBACK_SCROBBLE: PositionCallbackId = 1;
const POS_SCROBBLE_SECS: f64 = 5.0;
const POS_CALLBACK_HL_END: PositionCallbackId = 2;
const POS_HL_END_SECS: f64 = 0.5;
const POS_MIN_DURATION_TO_SCROBBLE: Duration = Duration::from_secs(30);
const DEFAULT_SEEK_LENGTH: Duration = Duration::from_secs(5);

impl App {
    pub fn new_args(&self, args: &Args) {
        self.play_paths(&args.paths);
    }

    fn play_paths(&self, paths: &[String]) {
        let (tracks, cue_factory) = playlist_man::collect_tracks(paths);
        if tracks.is_empty() {
            return;
        }

        playlist_man::save_playlist(&tracks).ignore_err();
        self.player.stop();
        self.player.set_playlist(tracks, Some(cue_factory));
        self.player.play(Some(0));
    }

    pub fn quit(&self) {
        self.user_action_quit();
    }

    fn init_playlist(&self, paths: &[String]) {
        let tracks;
        let auto_play;
        let playlist_index;
        let cue_factory;
        if paths.is_empty() {
            match playlist_man::load_playlist() {
                Ok(loaded_tracks) => tracks = loaded_tracks,
                Err(e) => {
                    e.log();
                    tracks = vec![];
                }
            }
            auto_play = false;
            playlist_index = if tracks.is_empty() {
                None
            } else {
                Some(self.state.playlist_index.unwrap_or(0))
            };
            cue_factory = None;
        } else {
            (tracks, cue_factory) = {
                let (tracks, cue_factory) = playlist_man::collect_tracks(paths);
                (tracks, Some(cue_factory))
            };
            auto_play = true;
            playlist_index = if tracks.is_empty() { None } else { Some(0) };
            if !tracks.is_empty() {
                playlist_man::save_playlist(&tracks).ignore_err();
            }
        }
        if tracks.is_empty() {
            eprintln_with_date("the track list is empty");
        }

        self.player.set_playlist(tracks, cue_factory);
        if let Some(playlist_index) = playlist_index {
            if auto_play {
                self.player.play(Some(playlist_index));
            } else {
                self.player.load_meta(playlist_index);
            }
        }
    }

    fn set_playback_state(&mut self, state: PlaybackState, position: Option<Duration>) {
        match state {
            PlaybackState::Playing => {
                if !matches!(
                    self.tray.image_type(),
                    TrayIconImageType::Play | TrayIconImageType::PlayHL
                ) {
                    self.tray.play();
                }
            }
            PlaybackState::Stopped => self.tray.stop(),
            PlaybackState::Paused => self.tray.pause(),
        }
        self.media_controls
            .mut_map(|c| c.set_state(&state, position).ignore_err());
        self.playback_state = state;
    }

    fn user_action_toggle_stop(&mut self) {
        match self.playback_state {
            PlaybackState::Stopped => {
                self.player.play(None);
                self.set_playback_state(PlaybackState::Playing, None);
            }
            PlaybackState::Playing => {
                self.player.stop();
                self.set_playback_state(PlaybackState::Stopped, None);
            }
            PlaybackState::Paused => {
                self.player.unpause();
                self.set_playback_state(PlaybackState::Playing, None);
            }
        }
    }

    fn user_action_next(&self) {
        self.player.next();
    }

    fn user_action_prev(&self) {
        self.player.prev();
    }

    fn user_action_next_dir(&self) {
        self.player.next_dir();
    }

    fn user_action_prev_dir(&self) {
        self.player.prev_dir();
    }

    fn user_action_stop(&mut self) {
        self.player.stop();
        self.set_playback_state(PlaybackState::Stopped, None);
    }

    fn user_action_quit(&self) {
        println_with_date("shutting down...");
        self.player.exit();
    }

    fn user_action_play(&mut self) {
        match self.playback_state {
            PlaybackState::Paused => {
                self.player.unpause();
                self.set_playback_state(PlaybackState::Playing, None);
            }
            PlaybackState::Stopped => {
                self.player.play(None);
                self.set_playback_state(PlaybackState::Playing, None);
            }
            PlaybackState::Playing => {}
        }
    }

    fn user_action_pause(&mut self) {
        if matches!(self.playback_state, PlaybackState::Playing) {
            self.player.pause();
            self.set_playback_state(PlaybackState::Paused, None);
        }
    }

    fn user_action_toggle_pause(&mut self) {
        match self.playback_state {
            PlaybackState::Stopped => {
                self.player.play(None);
                self.set_playback_state(PlaybackState::Playing, None);
            }
            PlaybackState::Playing => {
                self.player.pause();
                self.set_playback_state(PlaybackState::Paused, None);
            }
            PlaybackState::Paused => {
                self.player.unpause();
                self.set_playback_state(PlaybackState::Playing, None);
            }
        }
    }

    fn process_sys_vol_result(&mut self, result: Result<f64>) {
        match result {
            Ok(vol) => {
                #[allow(clippy::cast_sign_loss)]
                let vol_percent = (vol * 100.0).round() as u8;
                self.popup.show(&format!("system volume: {vol_percent}%"));
            }
            Err(e) => e.log(),
        }
    }

    fn change_volume(&mut self, step: f64) {
        // re-create SysVol everytime, to always use the current device
        match SysVol::new() {
            Ok(sys_vol) => self.process_sys_vol_result(sys_vol.modify_with_step(step)),
            Err(e) => e.context("cannot create system volume controller").log(),
        };
    }

    fn user_action_sysvol_down(&mut self) {
        self.change_volume(-VOL_STEP);
    }

    fn user_action_sysvol_up(&mut self) {
        self.change_volume(VOL_STEP);
    }

    fn set_vol(&mut self, new_volume: f32, show_popup: bool) {
        let new_volume = new_volume.clamp(0.0, 1.0);
        let steps_count = (new_volume / VOL_STEP as f32).round();
        let new_volume = steps_count * VOL_STEP as f32;
        self.state.volume = new_volume;
        self.player.set_volume(new_volume);
        self.update_tray(show_popup);
        self.state.save().ignore_err();
    }

    fn user_action_vol_down(&mut self) {
        let new_volume = self.state.volume - VOL_STEP as f32;
        self.set_vol(new_volume, true);
    }

    fn user_action_vol_up(&mut self) {
        let new_volume = self.state.volume + VOL_STEP as f32;
        self.set_vol(new_volume, true);
    }

    fn user_action_set_vol(&mut self, new_volume: f32) {
        self.set_vol(new_volume, false);
    }

    fn user_action_seek_by(&self, forward: bool, length: Duration) {
        self.player.seek_by(forward, length);
    }

    fn user_action_seek_to(&self, position: Duration) {
        self.player.seek_to(position);
    }

    fn user_action_open_uri(&self, uri_str: String) {
        self.play_paths(&[uri_str]);
    }

    fn process_hotkey(&mut self, action: HotKeyAction) {
        match action {
            HotKeyAction::StopPlay => self.user_action_toggle_stop(),
            HotKeyAction::Next => self.user_action_next(),
            HotKeyAction::Prev => self.user_action_prev(),
            HotKeyAction::NextDir => self.user_action_next_dir(),
            HotKeyAction::PrevDir => self.user_action_prev_dir(),
            HotKeyAction::PauseToggle => self.user_action_toggle_pause(),
            HotKeyAction::SysVolDown => self.user_action_sysvol_down(),
            HotKeyAction::SysVolUp => self.user_action_sysvol_up(),
            HotKeyAction::VolDown => self.user_action_vol_down(),
            HotKeyAction::VolUp => self.user_action_vol_up(),
        }
    }

    fn update_tray(&mut self, show_popup: bool) {
        #[allow(clippy::cast_sign_loss)]
        let vol_percent = (self.state.volume * 100.0).round() as u8;
        if let Some(track) = &self.cur_track {
            let path = Path::new(&track.filename);
            let dir_part = if let Some(dir) = path.parent() {
                if let Some(dirname) = dir.file_name() {
                    dirname.to_string_lossy().to_string()
                } else {
                    "?".to_string()
                }
            } else {
                "?".to_string()
            };
            let dir_part = format!("[{dir_part}] - {vol_percent}%\n");

            let artist_part = if let Some(artist) = &self.meta.artist {
                format!("{artist} - ")
            } else {
                String::new()
            };

            let title_part = if let Some(title) = &self.meta.title {
                title.clone()
            } else if let Some(basename) = path.file_stem() {
                basename.to_string_lossy().to_string()
            } else {
                String::new()
            };

            let tooltip = format!(
                "{}{}. {}{}",
                dir_part,
                self.playlist_index + 1,
                artist_part,
                title_part
            );
            self.tray.set_tooltip(&tooltip);

            self.media_controls
                .mut_map(|c| c.set_metadata(&self.meta).ignore_err());
            self.media_controls
                .mut_map(|c| c.set_volume(self.state.volume));
            self.player.request_position(); // because set_volume resets the position

            if show_popup {
                self.popup.show(&tooltip);
            }
        } else {
            self.tray
                .set_tooltip(&format!("[no file loaded] - {vol_percent}%"));
        }
    }

    fn process_position_callback(&mut self, callback: &PositionCallback) {
        if self.meta.duration > POS_MIN_DURATION_TO_SCROBBLE {
            let meta = &self.meta;
            if let (Some(artist), Some(title)) = (&meta.artist, &meta.title) {
                match callback.id {
                    POS_CALLBACK_NOW_PLAYING => {
                        if let Some(listenbrainz) = &mut self.listenbrainz {
                            listenbrainz
                                .playing_now(artist, &meta.album, title, meta.track)
                                .context("ListenBrainz playing now call failed")
                                .ignore_err();
                        }

                        if let Some(lastfm) = &mut self.lastfm {
                            lastfm
                                .playing_now(
                                    artist,
                                    &meta.album,
                                    title,
                                    meta.track,
                                    Some(meta.duration),
                                )
                                .context("Last.fm playing now call failed")
                                .ignore_err();
                        }
                    }
                    POS_CALLBACK_SCROBBLE => {
                        if self.last_seek_position.unwrap_or_default().is_zero() {
                            if let Some(listenbrainz) = &mut self.listenbrainz {
                                listenbrainz
                                    .submit(artist, &meta.album, title, meta.track)
                                    .context("ListenBrainz submit failed")
                                    .ignore_err();
                            }

                            if let Some(lastfm) = &mut self.lastfm {
                                lastfm
                                    .scrobble(
                                        artist,
                                        &meta.album,
                                        title,
                                        meta.track,
                                        Some(meta.duration),
                                    )
                                    .context("Last.fm scrobble failed")
                                    .ignore_err();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if callback.id == POS_CALLBACK_HL_END
            && matches!(self.tray.image_type(), TrayIconImageType::PlayHL)
        {
            self.tray.play();
        }
    }

    fn process_player_response(&mut self, resp: PlayerResponse) -> bool {
        match resp {
            PlayerResponse::NewPlaylistIndex {
                playlist_index,
                track,
                user_navigation,
            } => {
                self.playlist_index = playlist_index;
                self.cur_track = Some(track);
                self.meta = TrackMeta::default();
                if self.state.playlist_index != Some(playlist_index) {
                    self.state.playlist_index = Some(playlist_index);
                    self.state.save().ignore_err();
                }
                self.last_seek_position = None;
                if !user_navigation && matches!(self.tray.image_type(), TrayIconImageType::Play) {
                    self.tray.play_hl();
                }
            }
            PlayerResponse::PlaylistEnded => {
                self.popup.show("the playlist has ended");
            }
            PlayerResponse::NewMeta {
                meta,
                user_navigation,
            } => {
                self.meta = meta;
                let state = self.playback_state.clone();
                self.set_playback_state(state, Some(Duration::default()));
                self.update_tray(user_navigation);
            }
            PlayerResponse::PlaybackStateChanged { state, position } => {
                self.set_playback_state(state, Some(position));
            }
            PlayerResponse::PositionRequested { position } => {
                self.set_playback_state(self.playback_state.clone(), Some(position));
            }
            PlayerResponse::Seeked { position } => {
                let state = self.playback_state.clone();
                self.last_seek_position = Some(position);
                self.media_controls
                    .mut_map(|c| c.set_state(&state, Some(position)).ignore_err());
            }
            PlayerResponse::PositionCallback { callback, .. } => {
                self.process_position_callback(&callback);
            }
            PlayerResponse::VolumeSet { .. } => {}
            PlayerResponse::Exited => {
                return false;
            }
        }
        return true;
    }

    #[allow(clippy::needless_pass_by_value)]
    fn process_media_control_event(&mut self, event: MediaControlEvent) {
        match event {
            MediaControlEvent::Play => self.user_action_play(),
            MediaControlEvent::Pause => self.user_action_pause(),
            MediaControlEvent::Toggle => self.user_action_toggle_pause(),
            MediaControlEvent::Next => self.user_action_next(),
            MediaControlEvent::Previous => self.user_action_prev(),
            MediaControlEvent::Stop => self.user_action_stop(),
            MediaControlEvent::Raise => self.update_tray(true),
            MediaControlEvent::Seek(dir) => match dir {
                SeekDirection::Forward => {
                    self.user_action_seek_by(true, DEFAULT_SEEK_LENGTH);
                }
                SeekDirection::Backward => {
                    self.user_action_seek_by(false, DEFAULT_SEEK_LENGTH);
                }
            },
            MediaControlEvent::SeekBy(dir, length) => match dir {
                SeekDirection::Forward => self.user_action_seek_by(true, length),
                SeekDirection::Backward => self.user_action_seek_by(false, length),
            },
            MediaControlEvent::Quit => self.user_action_quit(),
            MediaControlEvent::SetPosition(pos) => self.user_action_seek_to(pos.0),
            MediaControlEvent::OpenUri(uri) => self.user_action_open_uri(uri),
            MediaControlEvent::SetVolume(vol) => self.user_action_set_vol(vol as f32),
        }
    }
}

pub struct AppHandle {
    pub app: Arc<Mutex<App>>,
    player_thread: JoinHandle<()>,
}

impl AppHandle {
    pub fn wait(self) {
        self.player_thread.join().unwrap();
        let mut app = self.app.lock().unwrap();
        app.hotkeys.stop();
        app.player.wait();
        app.lastfm.take();
        app.listenbrainz.take();
        app.tray.shutdown();

        // Unregistering media_controls may take almost 1 second
        // app.media_controls.take();
    }
}

pub fn start(cli_args: &Args) -> Result<AppHandle> {
    let listenbrainz = ListenBrainz::useable_or_none();
    let lastfm = LastFM::useable_or_none();
    let position_callbacks = if listenbrainz.is_some() || lastfm.is_some() {
        Some(vec![
            PositionCallback::from_start(POS_CALLBACK_NOW_PLAYING, POS_NOW_PLAYING_SECS),
            PositionCallback::from_end(POS_CALLBACK_SCROBBLE, POS_SCROBBLE_SECS),
            PositionCallback::from_start(POS_CALLBACK_HL_END, POS_HL_END_SECS),
        ])
    } else {
        None
    };
    let (player, dec_rx) = player::start_thread(position_callbacks);
    let media_controls = MediaControls::new_if_available();

    let state = AppState::load_or_default();
    player.set_volume(state.volume);
    let app = Arc::new(Mutex::new(App {
        player,
        playback_state: PlaybackState::default(),
        playlist_index: 0,
        cur_track: None,
        meta: TrackMeta::default(),
        hotkeys: HotKeys::new(),
        tray: TrayIcon::new().context("cannot create tray icon")?,
        listenbrainz,
        lastfm,
        state,
        popup: Popup::new(),
        media_controls,
        last_seek_position: None,
    }));

    set_tray_menu(&app);
    start_hotkey_thread(&app).context("cannot start hotkey thread")?;
    app.lock().unwrap().init_playlist(&cli_args.paths);
    setup_media_controls(&app).context("cannot setup media controls")?;

    let player_thread = start_player_response_thread(&app, dec_rx);

    return Ok(AppHandle { app, player_thread });
}

fn start_hotkey_thread(app_arc: &Arc<Mutex<App>>) -> Result<()> {
    let app_arc = app_arc.clone();
    app_arc
        .clone()
        .lock()
        .unwrap()
        .hotkeys
        .start(move |action| {
            let mut app = app_arc.lock().unwrap();
            app.process_hotkey(action);
        })
        .context("cannot register hotkeys")?;
    return Ok(());
}

fn start_player_response_thread(
    app_arc: &Arc<Mutex<App>>,
    dec_rx: Receiver<PlayerResponse>,
) -> JoinHandle<()> {
    let app_arc = app_arc.clone();
    let t = thread_util::thread("player client", move || loop {
        let resp = dec_rx.recv();
        match resp {
            Err(e) => {
                e.log();
                return;
            }
            Ok(resp) => {
                let mut app = app_arc.lock().unwrap();
                if !app.process_player_response(resp) {
                    return;
                }
            }
        }
    });
    return t;
}

fn set_tray_menu(app_arc: &Arc<Mutex<App>>) {
    let mut app = app_arc.lock().unwrap();

    app.tray.add_menu_item(|| {
        TrayMenuItem::new("Show current file", {
            let app = app_arc.clone();
            move || {
                let app = app.lock().unwrap();
                app.cur_track
                    .ref_map(|t| show_file(&t.filename).ignore_err());
            }
        })
    });

    app.tray.add_menu_item(|| {
        TrayMenuItem::new("Exit", {
            let app = app_arc.clone();
            move || {
                let app = app.lock().unwrap();
                app.user_action_quit();
            }
        })
    });
}

fn setup_media_controls(app_arc: &Arc<Mutex<App>>) -> Result<()> {
    let controls = &mut app_arc.lock().unwrap().media_controls;
    if let Some(controls) = controls {
        let app_arc = app_arc.clone();
        controls
            .attach(move |event| {
                let mut app = app_arc.lock().unwrap();
                app.process_media_control_event(event);
            })
            .to_anyhow()
            .context("cannot attach media controls")?;
    }
    return Ok(());
}
