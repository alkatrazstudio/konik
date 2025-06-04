// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    // all
    clippy::needless_return
)]

use anyhow::{Context, Result, bail};
use cargo_metadata::{MetadataCommand, Package};
use handlebars::Handlebars;
use std::{collections::HashMap, env, fs, path::Path};

const LASTFM_BYTES_LEN: usize = 16;
const LASTFM_KEY_NAME: &str = "API_KEY";
const LASTFM_SECRET_NAME: &str = "SHARED_SECRET";

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=lastfm.key");
    println!("cargo:rerun-if-changed=readme.template.md");

    let path = env::var("CARGO_MANIFEST_DIR")?;
    let meta = MetadataCommand::new()
        .current_dir(&path)
        .manifest_path("./Cargo.toml")
        .exec()?;
    let root = meta.root_package().context("no root package found")?;

    add_env_from_metadata(root, "title", "PROJECT_TITLE")?;
    add_env_from_metadata(root, "organization", "PROJECT_ORGANIZATION")?;
    add_env_from_metadata(root, "qualifier", "PROJECT_QUALIFIER")?;

    let out_dir_str = env::var_os("OUT_DIR").context("no OUT_DIR is set")?;
    let out_dir = Path::new(&out_dir_str);
    write_lastfm_key_consts(out_dir)?;

    built::write_built_file().context("failed to acquire build-time information")?;

    write_readme(root, out_dir)?;

    return Ok(());
}

fn meta_val<'a>(package: &'a Package, meta_key: &'a str) -> Result<&'a str> {
    let env_val = package.metadata[meta_key]
        .as_str()
        .with_context(|| format!("no \"{meta_key}\" set in the metadata"))?;
    return Ok(env_val);
}

fn add_env_from_metadata(package: &Package, meta_key: &str, env_name: &str) -> Result<()> {
    let env_val = meta_val(package, meta_key)?;
    println!("cargo:rustc-env={env_name}={env_val}");
    return Ok(());
}

fn lastfm_key_to_bytes_str(name: &str, key: &str) -> Result<String> {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() != LASTFM_BYTES_LEN * 2 {
        bail!(
            "LastFM keys ({}) must be {} characters length",
            name,
            LASTFM_BYTES_LEN * 2
        );
    }
    if !chars.iter().all(char::is_ascii_hexdigit) {
        bail!("LastFM keys ({}) must only contain [0-9a-f] symbols", name);
    }
    let byte_strs: Vec<String> = chars
        .chunks_exact(2)
        .map(|a| format!("0x{}{}", a[0], a[1]))
        .collect();
    let joined_byte_str = byte_strs.join(", ");
    return Ok(joined_byte_str);
}

fn write_lastfm_key_consts(out_dir: &Path) -> Result<()> {
    let (key, secret) = match get_lastfm_key_consts() {
        Ok((key, secret)) => (format!("Some([{key}])"), format!("Some([{secret}])")),
        Err(e) => {
            eprintln!("{e:?}");
            ("None".to_string(), "None".to_string())
        }
    };

    let key = format!("const {LASTFM_KEY_NAME}: Option<[u8; {LASTFM_BYTES_LEN}]> = {key};");
    let secret =
        format!("const {LASTFM_SECRET_NAME}: Option<[u8; {LASTFM_BYTES_LEN}]> = {secret};");
    let contents = format!("{key}\n{secret}");

    let filename = out_dir.join("lastfm_keys.rs");
    fs::write(filename, contents).context("writing LastFM consts file")?;

    return Ok(());
}

fn get_lastfm_key_consts() -> Result<(String, String)> {
    let keys = fs::read_to_string("lastfm.key").context("loading LastFM key file")?;
    let keys: Vec<&str> = keys.trim().lines().collect();
    if keys.len() != 2 {
        bail!("LastFM key file must contain only two lines");
    }
    let key = lastfm_key_to_bytes_str(LASTFM_KEY_NAME, keys[0])?;
    let secret = lastfm_key_to_bytes_str(LASTFM_SECRET_NAME, keys[1])?;

    return Ok((key, secret));
}

fn write_readme(package: &Package, out_dir: &Path) -> Result<()> {
    let template = include_str!("readme.template.md");
    let bars = Handlebars::new();
    let md = bars.render_template(
        template,
        &HashMap::from([
            ("name", env!("CARGO_PKG_NAME")),
            ("version", env!("CARGO_PKG_VERSION")),
            ("title", meta_val(package, "title")?),
            ("description", env!("CARGO_PKG_DESCRIPTION")),
            ("homepage", env!("CARGO_PKG_HOMEPAGE")),
        ]),
    )?;
    let md_fmt = termimad::text(&md);
    let str_result = format!("{md_fmt}");
    let filename = out_dir.join("readme.rs");
    let contents = format!("const README: &str = r###\"{str_result}\"###;");
    fs::write(filename, contents).context("writing readme file")?;
    return Ok(());
}
