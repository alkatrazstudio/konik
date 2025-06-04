// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::fmt::{Debug, Display};

use anyhow::{Error, Result, anyhow, bail};

fn get_now_str() -> String {
    let now = chrono::Local::now();
    let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
    return now_str;
}

pub fn println_with_date<T>(s: T)
where
    T: Display,
{
    let now_str = get_now_str();
    println!("[{now_str}] {s}");
}

pub fn eprintln_with_date<T>(s: T)
where
    T: Display,
{
    let now_str = get_now_str();
    eprintln!("[{now_str}] {s}");
}

pub trait LogErr
where
    Self: Into<Error>,
{
    fn log(self);
    fn log_context<C>(self, context: C)
    where
        C: Display + Send + Sync + 'static;
}

impl<T> LogErr for T
where
    T: Into<Error>,
{
    fn log(self) {
        eprintln_with_date(format!("{:?}", anyhow!(self)));
    }

    fn log_context<'a, C>(self, context: C)
    where
        C: Display + Send + Sync + 'static,
    {
        anyhow!(self).context(context).log();
    }
}

pub trait IgnoreErr<T, E> {
    fn ignore_err(self)
    where
        E: Into<Error>;
    fn to_option(self) -> Option<T>
    where
        E: Into<Error>;
    fn map_to_option<F, R>(self, f: F) -> Option<R>
    where
        E: Into<Error>,
        F: FnOnce(T) -> Option<R> + 'static;
    fn to_bool(self) -> bool
    where
        E: Into<Error>;
    fn ok_or<F>(self, f: F) -> T
    where
        E: Into<Error>,
        F: FnOnce() -> T + 'static;
    fn ok_or_default(self) -> T
    where
        E: Into<Error>,
        T: Default;
    fn to_anyhow(self) -> Result<T>
    where
        E: Debug;
}

impl<T, E> IgnoreErr<T, E> for core::result::Result<T, E> {
    fn ignore_err(self)
    where
        E: Into<Error>,
    {
        if let Err(e) = self {
            e.log();
        }
    }

    fn to_option(self) -> Option<T>
    where
        E: Into<Error>,
    {
        return match self {
            Ok(val) => Some(val),
            Err(e) => {
                e.log();
                None
            }
        };
    }

    fn map_to_option<F, R>(self, f: F) -> Option<R>
    where
        E: Into<Error>,
        F: FnOnce(T) -> Option<R> + 'static,
    {
        return match self {
            Ok(val) => f(val),
            Err(e) => {
                e.log();
                None
            }
        };
    }

    fn to_bool(self) -> bool
    where
        E: Into<Error>,
    {
        if let Err(e) = self {
            e.log();
            return false;
        }
        return true;
    }

    fn ok_or<F>(self, f: F) -> T
    where
        E: Into<Error>,
        F: FnOnce() -> T + 'static,
    {
        return match self {
            Ok(val) => val,
            Err(e) => {
                e.log();
                f()
            }
        };
    }

    fn ok_or_default(self) -> T
    where
        E: Into<Error>,
        T: Default,
    {
        return match self {
            Ok(val) => val,
            Err(e) => {
                e.log();
                Default::default()
            }
        };
    }

    fn to_anyhow(self) -> Result<T>
    where
        E: Debug,
    {
        return match self {
            Ok(val) => Ok(val),
            Err(e) => bail!("{:?}", e),
        };
    }
}

pub trait OptionAnd<T> {
    fn ref_map<R, F>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R;
    fn mut_map<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R;
}

impl<T> OptionAnd<T> for Option<T> {
    fn ref_map<R, F>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        if let Some(val) = self {
            return Some(f(val));
        }
        return None;
    }

    fn mut_map<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        if let Some(val) = self {
            return Some(f(val));
        }
        return None;
    }
}
