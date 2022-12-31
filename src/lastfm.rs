// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    cli,
    err_util::{eprintln_with_date, IgnoreErr, LogErr},
    project_file::{ProjectFileJson, ProjectFileString},
    project_info, thread_util,
};

include!(concat!(env!("OUT_DIR"), "/lastfm_keys.rs"));

const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
const MAX_SCROBBLES: usize = 50;

pub struct LastFM {
    api_key: String,
    shared_secret: String,
    session_key: Option<String>,
    not_scrobbled: Arc<Mutex<Vec<ScrobbleItem>>>,
    api_thread: Option<JoinHandle<()>>,
}

#[derive(Deserialize)]
struct AuthResponse {
    session: AuthSession,
}

#[derive(Deserialize)]
struct AuthSession {
    name: String,
    key: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    message: String,
    error: usize,
}

#[derive(Deserialize)]
struct NowPlayingResponse {
    #[serde(rename = "nowplaying")]
    now_playing: TrackResult,
}

#[derive(Deserialize, Debug)]
struct TrackResult {
    #[serde(rename = "ignoredMessage")]
    ignored_message: IgnoredMessage,
    artist: TrackField,
    album: TrackField,
    track: TrackField,
}

#[derive(Deserialize, Debug)]
struct TrackField {
    #[serde(rename = "#text")]
    text: Option<String>,
}

#[derive(Deserialize, Debug)]
struct IgnoredMessage {
    code: String,
    #[serde(rename = "#text")]
    text: String,
}

#[derive(Deserialize, Debug)]
struct ScrobbleResponse {
    scrobbles: ScrobbleResponseRoot,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum ScrobbleResponseRoot {
    Many { scrobble: Vec<TrackResult> },
    Single { scrobble: TrackResult },
}

#[derive(Serialize, Deserialize)]
struct ScrobbleItem {
    artist: String,
    track: String,
    album: Option<String>,
    number: Option<usize>,
    duration: Option<u64>,
    timestamp: u64,
}

impl LastFM {
    fn new_or_none() -> Option<Self> {
        if let (Some(key), Some(secret)) = (API_KEY, SHARED_SECRET) {
            let session_key = Self::session_key_file().load().to_option();
            let not_scrobbled = if session_key.is_some() {
                Self::not_scrobbled_file().load().ok_or(Vec::new)
            } else {
                Vec::new()
            };
            return Some(Self {
                api_key: Self::key_arr_to_string(&key),
                shared_secret: Self::key_arr_to_string(&secret),
                session_key,
                not_scrobbled: Arc::new(Mutex::new(not_scrobbled)),
                api_thread: None,
            });
        }
        return None;
    }

    pub fn useable_or_none() -> Option<Self> {
        let lfm = Self::new_or_none();
        if let Some(lfm) = lfm {
            if lfm.is_useable() {
                return Some(lfm);
            }
            eprintln_with_date("no authorization for Last.fm");
            return None;
        }
        eprintln_with_date("Last.fm is not supported in this build");
        return None;
    }

    fn is_useable(&self) -> bool {
        return self.session_key.is_some();
    }

    fn wait_for_api_thread(&mut self) {
        if let Some(t) = self.api_thread.take() {
            t.join().to_anyhow().ignore_err();
        }
    }

    pub fn playing_now(
        &mut self,
        artist: &str,
        album: &Option<String>,
        track: &str,
        number: Option<usize>,
        duration: Option<Duration>,
    ) -> Result<()> {
        let mut params = vec![
            ("artist".to_string(), artist.to_string()),
            ("track".to_string(), track.to_string()),
        ];
        if let Some(session_key) = &self.session_key {
            params.push(("sk".to_string(), session_key.to_string()));
        } else {
            bail!("Last.fm session key is not set");
        }
        if let Some(album) = album {
            params.push(("album".to_string(), album.to_string()));
        }
        if let Some(number) = number {
            params.push(("trackNumber".to_string(), number.to_string()));
        }
        if let Some(duration) = duration {
            params.push(("duration".to_string(), duration.as_secs().to_string()));
        }

        let url = self
            .get_method_url("track.updateNowPlaying", &params)
            .context("cannot get URL for playing_now")?;

        self.wait_for_api_thread();
        thread_util::thread(
            "Last.fm now playing API call",
            move || match Self::api_call::<NowPlayingResponse>(&url) {
                Ok(response) => {
                    response.now_playing.warn_if_ignored();
                }
                Err(e) => e
                    .context("cannot perform Last.fm API playing_now call")
                    .log(),
            },
        );

        return Ok(());
    }

    pub fn scrobble(
        &mut self,
        artist: &str,
        album: &Option<String>,
        track: &str,
        number: Option<usize>,
        duration: Option<Duration>,
    ) -> Result<()> {
        let mut params = vec![];
        if let Some(session_key) = &self.session_key {
            params.push(("sk".to_string(), session_key.clone()));
        } else {
            bail!("Last.fm session key is not set");
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("cannot get current timestamp")?
            .as_secs();
        let item = ScrobbleItem {
            artist: artist.to_string(),
            album: album.clone(),
            track: track.to_string(),
            number,
            duration: duration.map(|d| d.as_secs()),
            timestamp,
        };

        let items_arc = self.not_scrobbled.clone();
        let mut items = items_arc.lock().unwrap();
        let was_empty = items.is_empty();
        items.push(item);
        let items_len = items.len();
        let first_item_index = if items_len >= MAX_SCROBBLES {
            items_len - MAX_SCROBBLES
        } else {
            0
        };
        let batch = &items[first_item_index..items_len];
        let mut timestamps = Vec::new();
        for (i, item) in batch.iter().enumerate() {
            timestamps.push(item.timestamp);
            params.push((format!("artist[{i}]"), item.artist.clone()));
            params.push((format!("track[{i}]"), item.track.clone()));
            params.push((format!("timestamp[{i}]"), item.timestamp.to_string()));

            if let Some(album) = album {
                params.push((format!("album[{i}]"), album.clone()));
            }
            if let Some(number) = number {
                params.push((format!("trackNumber[{i}]"), number.to_string()));
            }
            if let Some(duration) = duration {
                params.push((format!("duration[{i}]"), duration.as_secs().to_string()));
            }
        }

        let url = self
            .get_method_url("track.scrobble", &params)
            .context("cannot get URL for scrobble")?;

        let items_arc = self.not_scrobbled.clone();
        self.wait_for_api_thread();
        self.api_thread = Some(thread_util::thread(
            "Last.fm scrobble API call",
            move || {
                match Self::api_call::<ScrobbleResponse>(&url) {
                    Ok(response) => {
                        let infos = match response.scrobbles {
                            ScrobbleResponseRoot::Many { scrobble } => scrobble,
                            ScrobbleResponseRoot::Single { scrobble } => vec![scrobble],
                        };

                        for info in &infos {
                            info.warn_if_ignored();
                        }

                        let mut items = items_arc.lock().unwrap();
                        items.retain(|i| !timestamps.contains(&i.timestamp));
                    }
                    Err(e) => {
                        e.context("Last.fm API scrobble call failed").log();
                    }
                }
                let items = items_arc.lock().unwrap();
                if !items.is_empty() || !was_empty {
                    Self::not_scrobbled_file()
                        .save::<Vec<ScrobbleItem>>(&items)
                        .ignore_err();
                }
            },
        ));

        return Ok(());
    }

    fn not_scrobbled_file() -> ProjectFileJson {
        return ProjectFileJson::for_data("lastfm_not_scrobbled.json", "not-scrobbled tracks file");
    }

    fn key_arr_to_string(key: &[u8]) -> String {
        let hex_arr: Vec<String> = key.iter().map(|b| format!("{:01$x}", b, 2)).collect();
        let key_str = hex_arr.join("");
        return key_str;
    }

    fn session_key_file() -> ProjectFileString {
        return ProjectFileString::for_data("lastfm_session_key", "Last.fm session key file");
    }

    fn calc_sig(&self, params: &[(String, String)]) -> String {
        let mut params = params.to_owned();
        params.sort_by(|(a, _), (b, _)| a.cmp(b));
        let comb_params: Vec<String> = params
            .iter()
            .map(|(key, val)| format!("{key}{val}"))
            .collect();
        let params_str = comb_params.join("");
        let payload = format!("{params_str}{}", &self.shared_secret);
        let digest = md5::compute(payload);
        let digest_hex = format!("{digest:x}");
        return digest_hex;
    }

    fn get_method_url(&self, method: &str, method_params: &[(String, String)]) -> Result<String> {
        let mut params = vec![
            ("method".to_string(), method.to_string()),
            ("api_key".to_string(), self.api_key.clone()),
        ];
        params.extend(method_params.to_owned());
        let signature = self.calc_sig(&params);
        params.push(("api_sig".to_string(), signature));
        params.push(("format".to_string(), "json".to_string()));
        let url = Url::parse_with_params(API_URL, &params)
            .with_context(|| format!("cannot build URL for method {method}"))?;
        let full_url = url.as_str();
        return Ok(full_url.to_string());
    }

    fn api_call<T>(url: &str) -> Result<T>
    where
        for<'de> T: Deserialize<'de>,
    {
        let user_agent = format!("{}/{}", project_info::title(), project_info::version());

        let result = ureq::post(url)
            .set("User-Agent", &user_agent)
            .set("Content-Type", "application/json")
            .set("Content-Length", "0")
            .call();

        let result = match result {
            Ok(result) => result,
            Err(e) => match e {
                ureq::Error::Status(status, response) => {
                    let json = response
                        .into_string()
                        .context("cannot read error status HTTP response as string")?;
                    let err: ErrorResponse = serde_json::from_str(&json)
                        .context("cannot parse error status HTTP response ")?;
                    bail!(
                        "{}, Error Code = {}, HTTP status = {}",
                        &err.message,
                        err.error,
                        status
                    );
                }
                ureq::Error::Transport(e) => {
                    let msg = e.message().unwrap_or_default();
                    let kind = e.kind();
                    bail!("HTTP error [{kind}]: {msg}")
                }
            },
        };
        let json = result
            .into_string()
            .context("cannot get HTTP response as string")?;
        let result = serde_json::from_str(&json).context("cannot parse HTTP response")?;
        return Ok(result);
    }

    pub fn cli_auth() -> Result<()> {
        let lastfm = Self::new_or_none().context("Last.fm support was not enabled")?;
        if lastfm.session_key.is_some() {
            let session_key = Self::session_key_file();
            bail!("there is already a stored session key at {:?}. Remove this file to authenticate again.", session_key.filename()?);
        }

        let username = cli::read_line("Last.fm username: ").context("cannot read username")?;
        if username.is_empty() {
            bail!("the username can't be empty");
        }
        let password =
            rpassword::prompt_password("Last.fm password: ").context("cannot read password")?;
        if username.is_empty() {
            bail!("the password can't be empty");
        }

        let url = lastfm
            .get_method_url(
                "auth.getMobileSession",
                &[
                    ("username".to_string(), username),
                    ("password".to_string(), password),
                ],
            )
            .context("cannot get auth URL")?;
        let result =
            Self::api_call::<AuthResponse>(&url).context("cannot perform auth API call")?;

        Self::session_key_file()
            .save(&result.session.key)
            .context("cannot save session key")?;
        println!("Authenticated: {}", &result.session.name);

        return Ok(());
    }
}

impl Drop for LastFM {
    fn drop(&mut self) {
        self.wait_for_api_thread();
    }
}

impl TrackResult {
    fn warn_if_ignored(&self) -> bool {
        if self.ignored_message.code == "0" {
            return false;
        }

        eprintln_with_date(format!(
            "track ignored [{}/{}/{}]: {}, Code = {}",
            self.artist.text.as_deref().unwrap_or_default(),
            self.album.text.as_deref().unwrap_or_default(),
            self.track.text.as_deref().unwrap_or_default(),
            &self.ignored_message.text,
            &self.ignored_message.code
        ));
        return true;
    }
}
