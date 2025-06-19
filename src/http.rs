// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2025, Alexey Parfenov <zxed@alkatrazstudio.net>

use crate::project_info;
use anyhow::{Context, Result};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use ureq::config::Config;
use ureq::http::Response;
use ureq::tls::{TlsConfig, TlsProvider};
use ureq::{Agent, Body};

pub struct HttpResponse {
    pub status_code: u16,
    pub is_success: bool,
    pub body: String,
}

pub fn new_agent() -> Agent {
    static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| {
        Mutex::new(
            Config::builder()
                .tls_config(
                    TlsConfig::builder()
                        .provider(TlsProvider::NativeTls)
                        .build(),
                )
                .timeout_global(Some(Duration::from_secs(10)))
                .http_status_as_error(false)
                .build(),
        )
    });
    let agent = CONFIG.lock().unwrap().new_agent();
    return agent;
}

fn user_agent() -> String {
    let user_agent = format!("{}/{}", project_info::title(), project_info::version());
    return user_agent;
}

fn response_to_result(mut response: Response<Body>) -> Result<HttpResponse> {
    let status = response.status();
    let status_code = status.as_u16();
    let body = response.body_mut().read_to_string().with_context(|| {
        format!("cannot read error status HTTP response as string (status: {status_code})")
    })?;
    let is_success = status.is_success();
    return Ok(HttpResponse {
        status_code,
        is_success,
        body,
    });
}

pub fn post(
    url: &str,
    content_type: &str,
    payload: &str,
    authorization: &str,
) -> Result<HttpResponse> {
    let mut builder = new_agent().post(url);
    if !authorization.is_empty() {
        builder = builder.header("Authorization", authorization);
    }
    let response = builder
        .header("User-Agent", user_agent())
        .header("Content-Type", content_type)
        .header("Content-Length", payload.len().to_string())
        .send(payload)
        .context("HTTP error")?;
    let result = response_to_result(response);
    return result;
}

pub fn get(url: &str, authorization: &str) -> Result<HttpResponse> {
    let mut builder = new_agent().get(url);
    if !authorization.is_empty() {
        builder = builder.header("Authorization", authorization);
    }
    let response = builder
        .call()
        .context("HTTP error")?;
    let result = response_to_result(response);
    return result;
}
