// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use anyhow::{Context, Result};
use fd_lock::RwLock;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, NameType, Stream, ToFsName,
    ToNsName,
    traits::{ListenerExt, Stream as StreamTrait},
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::File,
    fs::{self, OpenOptions},
    io::Write,
    io::{self, BufRead, BufReader},
    marker::PhantomData,
    path::PathBuf,
    thread::JoinHandle,
};

use crate::err_util::{IgnoreErr, LogErr};
use crate::thread_util;

pub struct Singleton<T>
where
    T: for<'de> Deserialize<'de> + Serialize + Sync + Send,
{
    flock: Option<RwLock<File>>,
    flock_filename: PathBuf,
    name: String,
    phantom_data: PhantomData<T>,
}

impl<T> Singleton<T>
where
    T: for<'de> Deserialize<'de> + Serialize + Sync + Send,
{
    pub fn new<F>(name: &str, pass_func: F) -> Result<Option<Self>>
    where
        F: FnOnce() -> Option<T>,
    {
        let sock_name = Self::sock_name(name).context("cannot get socket name")?;

        if let Ok(conn) = Stream::connect(sock_name) {
            let send_data = pass_func();
            let mut buf = BufReader::new(conn);
            if let Some(send_data) = send_data {
                let json =
                    serde_json::to_string(&send_data).context("cannot serialize singleton data")?;
                writeln!(buf.get_mut(), "{json}").context("socket send failed")?;
            }
            return Ok(None);
        }

        let (flock, flock_filename) =
            Self::create_lock_file(name).context("cannot create lock file")?;

        return Ok(Some(Self {
            flock: Some(flock),
            flock_filename,
            name: name.to_string(),
            phantom_data: PhantomData {},
        }));
    }

    fn sock_name(name: &'_ str) -> Result<Name<'_>> {
        let sock_name = if GenericNamespaced::is_supported() {
            name.to_ns_name::<GenericNamespaced>()?
        } else {
            name.to_fs_name::<GenericFilePath>()?
        };
        return Ok(sock_name);
    }

    fn lock_filename(name: &str) -> PathBuf {
        let mut filename = env::temp_dir();
        filename.push(format!("{name}.lock"));
        return filename;
    }

    fn create_lock_file(name: &str) -> Result<(RwLock<File>, PathBuf)> {
        let filename = Self::lock_filename(name);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&filename)
            .with_context(|| format!("cannot open {}", filename.display()))?;
        let mut file = RwLock::new(file);
        let mut write_file = file.write().with_context(|| {
            format!("cannot open lock file for writing: {}", filename.display())
        })?;
        write!(write_file, "{}", &name)?;
        drop(write_file);
        return Ok((file, filename));
    }

    fn process_connection(stream_result: io::Result<Stream>) -> Result<T> {
        let stream = stream_result.context("failed to get incoming connection")?;
        let mut buf = BufReader::new(stream);
        let mut json = String::default();
        buf.read_line(&mut json)
            .context("cannot read socket buffer")?;
        let data =
            serde_json::from_str::<T>(&json).context("cannot parse incoming socket buffer")?;
        return Ok(data);
    }

    pub fn listen<F>(self, on_data: F) -> Result<JoinHandle<()>>
    where
        F: Fn(T) + Clone + Sync + Send + 'static,
    {
        let sock_name = Self::sock_name(&self.name)?;
        let opts = ListenerOptions::new().name(sock_name);
        let listener = opts.create_sync().context("cannot bind to local socket")?;
        let t = thread_util::thread("singleton server", move || {
            for stream_result in listener.incoming() {
                match Self::process_connection(stream_result) {
                    Ok(data) => on_data(data),
                    Err(e) => e.context("cannot process incoming connection").log(),
                }
            }
        });
        return Ok(t);
    }
}

impl<T> Drop for Singleton<T>
where
    T: for<'de> Deserialize<'de> + Serialize + Sync + Send,
{
    fn drop(&mut self) {
        if let Some(flock) = self.flock.take() {
            drop(flock);
            fs::remove_file(&self.flock_filename)
                .with_context(|| format!("cannot remove file: {}", self.flock_filename.display()))
                .ignore_err();
        }
    }
}
