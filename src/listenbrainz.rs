// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{
    sync::{Arc, Mutex, MutexGuard},
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    cli,
    err_util::{eprintln_with_date, IgnoreErr, LogErr},
    project_file::{ProjectFileJson, ProjectFileString},
    project_info, thread_util,
};

const SUBMIT_ENDPOINT: &str = "https://api.listenbrainz.org/1/submit-listens";
const VALIDATE_ENDPOINT: &str = "https://api.listenbrainz.org/1/validate-token";
const MAX_IMPORT: usize = 25; // https://listenbrainz.readthedocs.io/en/production/dev/api/#listenbrainz.webserver.views.api_tools.MAX_LISTEN_SIZE

fn skip_if_none_or_empty(x: &Option<String>) -> bool {
    if let Some(val) = x {
        if !val.is_empty() {
            return false;
        }
    }
    return true;
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
enum ListenType {
    PlayingNow,
    Import,
}

#[derive(Serialize)]
struct AdditionalInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    tracknumber: Option<usize>,
    listening_from: &'static str,
}

#[derive(Serialize)]
struct TrackMetaData {
    artist_name: String,
    track_name: String,
    #[serde(skip_serializing_if = "skip_if_none_or_empty")]
    release_name: Option<String>,
    additional_info: AdditionalInfo,
}

#[derive(Serialize)]
struct Payload {
    #[serde(skip_serializing_if = "Option::is_none")]
    listened_at: Option<u64>,
    track_metadata: TrackMetaData,
}

#[derive(Serialize)]
struct Request {
    listen_type: ListenType,
    payload: Vec<Payload>,
}

#[derive(Deserialize)]
struct TokenValidationResponse {
    code: u16,
    message: String,
    valid: bool,
    user_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ListenItem {
    artist: String,
    track: String,
    album: Option<String>,
    number: Option<usize>,
    timestamp: u64,
}

pub struct ListenBrainz {
    token: Option<String>,
    not_submitted: Arc<Mutex<Vec<ListenItem>>>,
    api_thread: Option<JoinHandle<()>>,
}

impl ListenBrainz {
    pub fn useable_or_none() -> Option<Self> {
        return match Self::token_file().load() {
            Ok(token) => Some(Self::new(Some(token))),
            Err(e) => {
                e.context("no authorization for ListenBrainz").log();
                None
            }
        };
    }

    fn new(token: Option<String>) -> Self {
        let not_submitted = Self::not_submitted_file().load().ok_or(Vec::new);
        return Self {
            token,
            not_submitted: Arc::new(Mutex::new(not_submitted)),
            api_thread: None,
        };
    }

    fn token_file() -> ProjectFileString {
        return ProjectFileString::for_data("listenbrainz_token", "ListenBrainz token file");
    }

    fn not_submitted_file() -> ProjectFileJson {
        return ProjectFileJson::for_data(
            "listenbrainz_not_submitted.json",
            "ListenBrainz not-submitted listens list",
        );
    }

    pub fn playing_now(
        &mut self,
        artist: &str,
        album: &Option<String>,
        track: &str,
        number: Option<usize>,
    ) -> Result<()> {
        let release_name = album.clone();

        let payload = Payload {
            listened_at: None,
            track_metadata: TrackMetaData {
                artist_name: artist.to_string(),
                track_name: track.to_string(),
                release_name,
                additional_info: AdditionalInfo::new(number),
            },
        };

        let request = Request {
            listen_type: ListenType::PlayingNow,
            payload: vec![payload],
        };

        self.send(
            request,
            |_| {},
            |json| {
                eprintln_with_date(json);
            },
        )
        .context("cannot perform ListenBrainz playing_now API call")?;

        return Ok(());
    }

    pub fn submit(
        &mut self,
        artist: &str,
        album: &Option<String>,
        track: &str,
        number: Option<usize>,
    ) -> Result<()> {
        let start = SystemTime::now();
        let timestamp = start
            .duration_since(UNIX_EPOCH)
            .context("cannot get current timestamp")?
            .as_secs();
        let release_name = album.clone();

        let listen = ListenItem {
            artist: artist.to_string(),
            album: release_name,
            track: track.to_string(),
            number,
            timestamp,
        };

        let items_arc = self.not_submitted.clone();
        let mut items = items_arc.lock().unwrap();
        let was_empty = items.is_empty();
        items.push(listen);
        let items_len = items.len();
        let first_item_index = if items_len >= MAX_IMPORT {
            items_len - MAX_IMPORT
        } else {
            0
        };
        let batch = &items[first_item_index..items_len];
        let timestamps: Vec<u64> = batch.iter().map(|i| i.timestamp).collect();

        let request = Request {
            listen_type: ListenType::Import,
            payload: batch.iter().map(Payload::from_listen).collect(),
        };
        drop(items);

        self.send(
            request,
            {
                let items_arc = self.not_submitted.clone();
                move |_| {
                    let mut items = items_arc.lock().unwrap();
                    items.retain(|i| !timestamps.contains(&i.timestamp));
                    if !was_empty || !items.is_empty() {
                        Self::save_not_submitted_guarded(&items);
                    }
                    drop(items);
                }
            },
            move |json| {
                eprintln_with_date(json);
                let items = items_arc.lock().unwrap();
                Self::save_not_submitted_guarded(&items);
            },
        )
        .context("cannot perform ListenBrainz import API call")?;

        return Ok(());
    }

    fn save_not_submitted_guarded(items: &MutexGuard<Vec<ListenItem>>) {
        Self::not_submitted_file()
            .save::<Vec<ListenItem>>(items)
            .ignore_err();
    }

    fn wait_for_api_thread(&mut self) {
        if let Some(t) = self.api_thread.take() {
            t.join().to_anyhow().ignore_err();
        }
    }

    fn send<S, E>(&mut self, request: Request, on_succ: S, on_err: E) -> Result<()>
    where
        S: FnOnce(String) + Send + 'static,
        E: FnOnce(String) + Send + 'static,
    {
        if let Some(token) = &self.token {
            let auth = format!("Token {}", &token);
            let json = serde_json::to_string(&request).context("cannot serialize payload")?;

            self.wait_for_api_thread();
            let handle = thread_util::thread("ListenBrainz submit API call", move || {
                let result = ureq::post(SUBMIT_ENDPOINT)
                    .set("Authorization", &auth)
                    .set("Content-Type", "application/json")
                    .send_string(&json);

                match result {
                    Ok(resp) => {
                        let json = resp.into_string().unwrap_or_default();
                        on_succ(json.trim().to_string());
                    }
                    Err(e) => {
                        let json = match e.into_response() {
                            Some(resp) => resp.into_string().unwrap_or_default(),
                            None => String::new(),
                        };
                        on_err(json.trim().to_string());
                        eprintln_with_date(format!(
                            "cannot perform ListenBrainz API call: {:?}",
                            &request.listen_type
                        ));
                    }
                }
            });
            self.api_thread = Some(handle);

            return Ok(());
        }
        bail!("no token is set");
    }

    fn validate_token(token: &str) -> Result<String> {
        let auth = format!("Token {}", &token);
        let resp = ureq::get(VALIDATE_ENDPOINT)
            .set("Authorization", &auth)
            .set("Content-Type", "application/json")
            .call();
        let json = match resp {
            Ok(resp) => resp
                .into_string()
                .context("cannot read HTTP response as string")?,
            Err(e) => match e {
                ureq::Error::Status(e, resp) => {
                    eprintln_with_date(format!("HTTP Code: {e}"));
                    resp.into_string()
                        .context("cannot read HTTP response as string")?
                }
                ureq::Error::Transport(e) => bail!(e),
            },
        };
        let msg: TokenValidationResponse =
            serde_json::from_str(&json).context("cannot parse token response")?;
        if msg.valid {
            return msg
                .user_name
                .context("user_name field is missing fron the response");
        }
        bail!("[{}] {}", msg.code, msg.message);
    }

    pub fn cli_auth() -> Result<()> {
        let brainz = Self::useable_or_none();
        if brainz.is_some() {
            let session_key = Self::token_file();
            bail!(
                "there is already a stored token at {:?}. Remove this file to authenticate again.",
                session_key.filename().context("no token filename")?
            );
        }
        let token = cli::read_line("ListenBrainz token: ").context("cannot read token")?;
        if token.is_empty() {
            bail!("the token can't be empty");
        }
        let user_id = Self::validate_token(&token).context("cannot validate token")?;
        Self::token_file()
            .save(&token)
            .context("cannot save token")?;
        println!("Authenticated: {}", &user_id);

        return Ok(());
    }
}

impl Payload {
    fn from_listen(listen: &ListenItem) -> Self {
        return Self {
            listened_at: Some(listen.timestamp),
            track_metadata: TrackMetaData {
                artist_name: listen.artist.clone(),
                track_name: listen.track.clone(),
                release_name: listen.album.clone(),
                additional_info: AdditionalInfo::new(listen.number),
            },
        };
    }
}

impl AdditionalInfo {
    fn new(number: Option<usize>) -> Self {
        return Self {
            tracknumber: number,
            listening_from: project_info::title(),
        };
    }
}

impl Drop for ListenBrainz {
    fn drop(&mut self) {
        self.wait_for_api_thread();
    }
}
