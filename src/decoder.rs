// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait},
    Sample, SizedSample,
};
use num_traits::ToPrimitive;
use symphonia::core::{
    audio::RawSample,
    conv::{ConvertibleSample, IntoSample},
};

use crate::{
    cue::{CueFactory, CueSheet},
    err_util::{eprintln_with_date, IgnoreErr, LogErr},
    stream_base::{Stream, StreamPacketMeta, Track, TrackMeta},
    stream_man,
};

const BUFFER_CAPACITY: usize = 65535;
const BUFFER_SOFT_STOP: usize = 60000;

trait AudioOutputSample:
    Sample + SizedSample + ConvertibleSample + RawSample + ToPrimitive + Send + 'static
{
}
impl AudioOutputSample for f32 {}

pub struct Decoder {
    stream: Option<Box<dyn Stream>>,
    track: Option<Track>,
    packet_meta: Option<StreamPacketMeta>,
    previous_packet_meta: Option<StreamPacketMeta>,
    file_meta: Option<TrackMeta>,
    pub track_meta: Option<TrackMeta>,
    pub new_track_meta: Option<TrackMeta>,
    buf: Arc<Mutex<VecDeque<f32>>>,
    position: Duration,
    at_end: bool,
    wait_empty_buf: bool,
    cue_factory: CueFactory,
    cue_sheet: Option<Arc<CueSheet>>,
    volume: Arc<Mutex<f32>>,
}

pub enum DecoderReadResult {
    BufferNotFull,
    BufferFull,
    NeedResetOutput,
    ReadEnd,
}

impl Decoder {
    pub fn new() -> Self {
        let mut buf = VecDeque::<f32>::new();
        buf.reserve(BUFFER_CAPACITY);
        let buf = Arc::new(Mutex::new(buf));

        return Self {
            stream: None,
            track: None,
            packet_meta: None,
            previous_packet_meta: None,
            file_meta: None,
            track_meta: None,
            new_track_meta: None,
            buf,
            position: Duration::ZERO,
            at_end: false,
            wait_empty_buf: false,
            cue_factory: CueFactory::new(),
            cue_sheet: None,
            volume: Arc::new(Mutex::new(1.0)),
        };
    }

    pub fn stop(&mut self) {
        self.stream = None;
        self.track = None;
        self.packet_meta = None;
        self.previous_packet_meta = None;
        self.file_meta = None;
        self.track_meta = None;
        self.new_track_meta = None;
        self.cue_sheet = None;
        self.position = Duration::default();
        self.buf.lock().unwrap().clear();
    }

    pub fn clear_cue_factory(&mut self) {
        self.cue_factory.clear();
    }

    pub fn set_cue_factory(&mut self, cue_factory: CueFactory) {
        self.cue_factory = cue_factory;
    }

    fn sheet_for_track(&mut self, track: &Track) -> Result<Option<Arc<CueSheet>>> {
        if track.index.is_some() {
            let sheet = self
                .cue_factory
                .get_or_new(&track.filename)?
                .with_context(|| format!("file is not CUE: {}", &track.filename))?;
            return Ok(Some(sheet));
        }
        return Ok(None);
    }

    #[allow(clippy::type_complexity)]
    fn open(&mut self, track: &Track) -> Result<(Box<dyn Stream>, Option<Arc<CueSheet>>)> {
        let sheet = self.sheet_for_track(track).with_context(|| {
            format!(
                "cannot load CUE for track {}:{}",
                &track.filename,
                track.index.unwrap_or_default()
            )
        })?;
        let filename = sheet
            .as_ref()
            .map_or(&track.filename, |sheet| &sheet.source_filename);
        let stream =
            stream_man::open(filename).with_context(|| format!("error opening {filename}"))?;
        return Ok((stream, sheet));
    }

    pub fn load_meta(&mut self, track: &Track) -> Result<()> {
        let (mut stream, sheet) = self.open(track).context("cannot open track")?;
        let packet = stream.read_packet().context("cannot read packet")?;
        if let Some(meta) = &packet.track_meta {
            let file_meta = meta.clone();
            if let (Some(sheet), Some(index)) = (&sheet, track.index) {
                self.track_meta = Some(
                    sheet
                        .track_meta(index, &file_meta)
                        .context("cannot get track meta")?,
                );
            } else {
                self.track_meta = Some(meta.clone());
            }
            self.file_meta = Some(meta.clone());
            self.packet_meta = Some(packet);
            self.cue_sheet = sheet;
        } else {
            bail!("no meta data found: {}", &track.filename);
        }
        return Ok(());
    }

    pub fn play(&mut self, track: &Track) -> Result<()> {
        let new_sheet = self.sheet_for_track(track).with_context(|| {
            format!(
                "cannot load CUE for track {}:{}",
                &track.filename,
                track.index.unwrap_or_default()
            )
        })?;
        if let (Some(new_sheet), Some(new_index)) = (new_sheet, track.index) {
            if let (Some(_), Some(cur_sheet)) = (&mut self.stream, &self.cue_sheet) {
                if new_sheet.source_filename == cur_sheet.source_filename {
                    if let Some(cur_track) = &self.track {
                        if let Some(cur_index) = cur_track.index {
                            if new_index == cur_index + 1 {
                                if let Some(file_meta) = &self.file_meta {
                                    self.new_track_meta =
                                        new_sheet.track_meta(new_index, file_meta).to_option();
                                }
                                self.track = Some(track.clone());
                                self.at_end = false;
                                return Ok(());
                            }
                        }
                    }
                    self.track_meta = None;
                    self.track = Some(track.clone());
                    self.seek_to(Duration::ZERO)
                        .context("cannot seek to the start")?;
                    if let Some(file_meta) = &self.file_meta {
                        self.new_track_meta =
                            new_sheet.track_meta(new_index, file_meta).to_option();
                    }
                    self.at_end = false;
                    return Ok(());
                }
            }
            let new_stream = stream_man::open(&new_sheet.source_filename)
                .with_context(|| format!("error opening {}", &new_sheet.source_filename))?;
            self.stream = Some(new_stream);
            self.track_meta = None;
            self.file_meta = None;
            self.track = Some(track.clone());
            self.cue_sheet = Some(new_sheet);
            self.seek_to(Duration::ZERO)
                .context("cannot seek to the start")?;
            self.at_end = false;
            return Ok(());
        }

        if let Some(meta) = self.packet_meta.take() {
            self.previous_packet_meta = Some(meta);
        }

        self.track_meta = None;
        self.file_meta = None;
        match stream_man::open(&track.filename) {
            Ok(stream) => {
                self.stream = Some(stream);
            }
            Err(e) => {
                bail!("error opening {}: {}", &track.filename, e);
            }
        }
        self.at_end = false;
        self.track = Some(track.clone());
        return Ok(());
    }

    fn buffer_len(&self) -> usize {
        let buf_size = self.buf.lock().unwrap().len();
        return buf_size;
    }

    fn can_read_more(&self) -> bool {
        let buf_len = self.buffer_len();
        return buf_len < BUFFER_SOFT_STOP;
    }

    fn buf_items_per_sec(&self) -> Result<usize> {
        let meta = self.packet_meta.as_ref().context("no current packet")?;
        let n = meta.channels_count * meta.sample_rate;
        return Ok(n);
    }

    fn buffer_duration(&self) -> Result<Duration> {
        let per_sec = self
            .buf_items_per_sec()
            .context("cannot get samples per second")?;
        let buf_secs = self.buffer_len() as f64 / per_sec as f64;
        return Ok(Duration::from_secs_f64(buf_secs));
    }

    pub fn playback_position(&self) -> Duration {
        let buf_dur = self.buffer_duration().ok_or_default();
        let mut pos = self.position.saturating_sub(buf_dur);
        if let Some((sheet, index)) = self.sheet_and_index() {
            let start = sheet.track_start(index).unwrap_or_default();
            pos = pos.saturating_sub(start);
        }
        return pos;
    }

    pub fn valid_playback_position(&self) -> Result<Duration> {
        let buf_dur = self.buffer_duration()?;
        let mut pos = self.position.saturating_sub(buf_dur);
        if let Some((sheet, index)) = self.sheet_and_index() {
            let start = sheet.track_start(index)?;
            pos = pos.saturating_sub(start);
        }
        return Ok(pos);
    }

    fn sheet_and_index(&self) -> Option<(&Arc<CueSheet>, usize)> {
        if let (Some(sheet), Some(index)) =
            (&self.cue_sheet, self.track.as_ref().and_then(|t| t.index))
        {
            return Some((sheet, index));
        }
        return None;
    }

    pub fn seek_to(&mut self, pos: Duration) -> Result<Duration> {
        let start = if let Some((sheet, index)) = self.sheet_and_index() {
            sheet
                .track_start(index)
                .with_context(|| format!("can't get the start of track {index}"))?
        } else {
            Duration::ZERO
        };
        let pos = pos.saturating_add(start);

        if let Some(stream) = &mut self.stream {
            let seeked_to = stream.seek(pos).context("cannot seek")?;
            self.buf.lock().unwrap().clear();
            self.at_end = false;
            return Ok(seeked_to.saturating_sub(start));
        }
        bail!("the stream is not ready for seeking");
    }

    pub fn set_volume(&self, volume: f32) -> f32 {
        let volume = volume.clamp(0.0, 1.0);
        *self.volume.lock().unwrap() = volume;
        return volume;
    }

    fn is_format_change(cur_meta: &Option<StreamPacketMeta>, new_meta: &StreamPacketMeta) -> bool {
        if let Some(cur_meta) = &cur_meta {
            return cur_meta.channels_count != new_meta.channels_count
                || cur_meta.sample_rate != new_meta.sample_rate;
        }
        return false;
    }

    fn set_track_meta(&mut self, track_meta: &Option<TrackMeta>) {
        if let Some(track_meta) = &track_meta {
            self.track_meta = if let Some((sheet, index)) = self.sheet_and_index() {
                sheet.track_meta(index, track_meta).to_option()
            } else {
                Some(track_meta.clone())
            };
            self.file_meta = Some(track_meta.clone());
            self.new_track_meta.clone_from(&self.track_meta);
        }
    }

    pub fn read_stream(&mut self) -> DecoderReadResult {
        if self.at_end || !self.can_read_more() {
            return DecoderReadResult::BufferFull;
        }

        if let Some(stream) = &mut self.stream {
            if self.wait_empty_buf {
                if self.buffer_len() != 0 {
                    return DecoderReadResult::BufferFull;
                }
                self.wait_empty_buf = false;
                return DecoderReadResult::NeedResetOutput;
            }

            let prev_meta = self.previous_packet_meta.take();
            if let Ok(mut packet_meta) = stream.read_packet() {
                let format_changed = Self::is_format_change(&prev_meta, &packet_meta);

                let track_meta = packet_meta.track_meta.take();
                if format_changed {
                    self.wait_empty_buf = true;
                    self.set_track_meta(&track_meta);
                    return DecoderReadResult::BufferFull;
                }

                let res = stream.write(&mut self.buf.lock().unwrap());
                if res.to_bool() {
                    self.packet_meta = Some(packet_meta);
                    self.set_track_meta(&track_meta);
                }

                if let Some(position) = self.packet_meta.as_ref().and_then(|m| m.position) {
                    self.position = position;
                    if let Some((sheet, index)) = self.sheet_and_index() {
                        let pos_index = sheet.track_index_by_position(position);
                        if pos_index > index {
                            self.at_end = true;
                            return DecoderReadResult::ReadEnd;
                        }
                    }
                }
            } else {
                self.at_end = true;
                return DecoderReadResult::ReadEnd;
            }
            return DecoderReadResult::BufferNotFull;
        }
        return DecoderReadResult::BufferFull;
    }

    pub fn create_output_stream(&self) -> Option<cpal::Stream> {
        if self.stream.is_some() {
            if let Some(meta) = &self.packet_meta {
                return Some(
                    create_output_stream(meta, &self.buf, &self.volume)
                        .expect("cannot create output stream"),
                );
            }
        }
        return None;
    }
}

fn copy_with_volume<T: AudioOutputSample>(src: &[T], dest: &mut [T], volume: f32) {
    let n = src.len();

    // avoiding bounds checking - https://godbolt.org/z/cWjz4e1eM
    let src_iter = src.iter().take(n);
    let dest_iter = dest.iter_mut().take(n);
    let zip_iter = src_iter.zip(dest_iter);

    #[allow(clippy::float_cmp)]
    if volume == 1.0 {
        for (src_sample, dst_sample) in zip_iter {
            *dst_sample = *src_sample;
        }
    } else if volume == 0.0 {
        for (_, dst_sample) in zip_iter {
            *dst_sample = T::MID;
        }
    } else {
        for (src_sample, dst_sample) in zip_iter {
            let mul_val = src_sample.to_f32().unwrap_or_default() * volume;
            *dst_sample = mul_val.into_sample();
        }
    }
}

fn create_output_stream<T: AudioOutputSample>(
    meta: &StreamPacketMeta,
    buf: &Arc<Mutex<VecDeque<T>>>,
    volume: &Arc<Mutex<f32>>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no output device available");

    let config = cpal::StreamConfig {
        channels: meta.channels_count as cpal::ChannelCount,
        sample_rate: cpal::SampleRate(meta.sample_rate as u32),
        buffer_size: cpal::BufferSize::Default,
    };

    let buf = buf.clone();
    let volume = volume.clone();
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [T], _| {
                let buf = &mut buf.lock().unwrap();

                let (s1, s2) = buf.as_slices();
                let mut len = s1.len().min(data.len());
                //data[0..len].clone_from_slice(&s1[0..len]);
                let volume = volume.lock().unwrap();
                copy_with_volume(&s1[0..len], &mut data[0..len], *volume);
                if len < data.len() {
                    let len1 = len;
                    len = (len + s2.len()).min(data.len());
                    //data[len1..len].clone_from_slice(&s2[0..len - len1]);
                    copy_with_volume(&s2[0..len - len1], &mut data[len1..len], *volume);
                    drop(volume);
                    if len < data.len() {
                        eprintln_with_date(format!("underrun: {} samples", data.len() - len));
                        data[len..].iter_mut().for_each(|x| *x = T::MID);
                    }
                }
                buf.drain(0..len);
            },
            move |e| e.log(),
            None,
        )
        .context("cannot create output stream")?;
    return Ok(stream);
}
