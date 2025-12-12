// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{collections::HashMap, ffi::CString};

use alsa::{
    Mixer,
    mixer::{Selem, SelemChannelId, SelemId},
};
use anyhow::{Context, Result, bail};

use crate::err_util::IgnoreErr;

pub struct SysVol {
    mixer: Mixer,
    master_id: SelemId,
}

pub struct Master<'a> {
    selem: Selem<'a>,
    vol_min: f64,
    vol_range: f64,
    has_switch: bool,
}

impl SysVol {
    const CARD_NAME: &'static str = "default";
    const MASTER_NAME: &'static str = "Master";

    pub fn new() -> Result<Self> {
        let mut mixer = Mixer::open(false).context("cannot open ALSA mixer")?;
        let card_name = CString::new(Self::CARD_NAME)?;
        mixer
            .attach(&card_name)
            .context("cannot attach ALSA mixer")?;
        Selem::register(&mut mixer).context("cannot register ALSA mixer")?;
        mixer.load().context("cannot load ALSA mixer")?;

        let mut master_id = SelemId::empty();
        let selem_name = CString::new(Self::MASTER_NAME).context("cannot create c-string")?;
        master_id.set_name(&selem_name);
        master_id.set_index(0);

        return Ok(Self { mixer, master_id });
    }

    fn master(&'_ self) -> Result<Master<'_>> {
        let selem = self
            .mixer
            .find_selem(&self.master_id)
            .context("ALSA master element not found")?;
        let (vol_min, vol_max) = selem.get_playback_volume_range();
        let vol_range = vol_max - vol_min;
        let has_switch = selem.has_playback_switch();
        return Ok(Master {
            selem,
            vol_min: vol_min as f64,
            vol_range: vol_range as f64,
            has_switch,
        });
    }

    pub fn get(&self) -> Result<f64> {
        self.mixer
            .handle_events()
            .context("cannot handle ALSA mixer events")?;
        return self.master()?.get();
    }

    pub fn set(&self, vol: f64) -> Result<()> {
        return self.master()?.set(vol);
    }

    fn real_step(&self, step: f64) -> Result<f64> {
        let min_step = self.master()?.min_step();
        if step > 0.0 {
            if min_step > step {
                return Ok(min_step);
            }
            return Ok(step);
        }
        if -min_step < step {
            return Ok(-min_step);
        }
        return Ok(step);
    }

    pub fn modify_with_step(&self, step: f64) -> Result<f64> {
        let step = self.real_step(step)?;
        let vol = self.get()?;
        let vol = vol + step;
        let step_abs = step.abs();
        let n_steps = (vol / step_abs).round();
        let vol = n_steps * step_abs;
        self.set(vol)?;
        let vol = self.get()?;
        return Ok(vol);
    }
}

impl Master<'_> {
    const DEFAULT_CHANNEL: SelemChannelId = SelemChannelId::FrontLeft;

    pub fn get(&self) -> Result<f64> {
        let vol = self
            .selem
            .get_playback_volume(Self::DEFAULT_CHANNEL)
            .context("cannot get ALSA master volume")?;
        let normalized_vol = (vol as f64 + self.vol_min) / self.vol_range;
        let normalized_vol = normalized_vol.clamp(0.0, 1.0);
        return Ok(normalized_vol);
    }

    pub fn set(&self, vol: f64) -> Result<()> {
        let mut switches = HashMap::new();

        if self.has_switch {
            for channel in SelemChannelId::all() {
                if let Ok(switch) = self.selem.get_playback_switch(*channel) {
                    switches.insert(channel, switch);
                }
            }
        }

        let vol = vol.clamp(0.0, 1.0);
        let selem_vol = self.vol_range.mul_add(vol, self.vol_min).round();

        let result = self.selem.set_playback_volume_all(selem_vol as i64);

        if self.has_switch {
            let mut ok = true;
            for (channel, switch) in switches {
                if !self.selem.set_playback_switch(*channel, switch).to_bool() {
                    ok = false;
                }
            }
            if !ok {
                bail!("cannot set all master channel switches back");
            }
        }

        return result.context("cannot set master volume");
    }

    pub fn min_step(&self) -> f64 {
        return 1.0 / self.vol_range;
    }
}
