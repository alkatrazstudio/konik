// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::{collections::VecDeque, fs::File, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use lofty::{
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::{Accessor, ItemKey, ItemValue, Tag},
};
use symphonia::core::{
    audio::{AudioBufferRef, SampleBuffer},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    formats::{FormatOptions, SeekMode, SeekTo, Track},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::{Hint, ProbeResult},
    units::{Time, TimeStamp},
};

use crate::{
    err_util::{LogErr, eprintln_with_date},
    stream_base::{Stream, StreamHelper, StreamPacketMeta, TrackMeta},
};

pub struct SymphoniaStream {
    path: String,
    probe: ProbeResult,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    buffer: Option<SampleBuffer<f32>>,
    metadata_sent: bool,
}

const EXTS: [&str; 3] = ["flac", "ogg", "mp3"];

impl Stream for SymphoniaStream {
    fn open(path: &str) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("cannot open file: {path}"))?;

        let stream_opts = MediaSourceStreamOptions::default();
        let stream = MediaSourceStream::new(Box::new(file), stream_opts);

        let mut hint = Hint::new();
        if let Some(ext) = Path::new(path).extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }

        let metadata_opts: MetadataOptions = MetadataOptions::default();
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };

        let probe = symphonia::default::get_probe()
            .format(&hint, stream, &format_opts, &metadata_opts)
            .context("unsupported format")?;

        let (track, decoder) = Self::track_and_decoder_by_probe(&probe)?;
        let track_id = track.id;

        return Ok(Self {
            path: path.to_string(),
            probe,
            decoder,
            track_id,
            buffer: None,
            metadata_sent: false,
        });
    }

    fn is_path_supported(path: &str) -> bool {
        return Self::is_extension_supported(path, &EXTS);
    }

    fn read_packet(&mut self) -> Result<StreamPacketMeta> {
        let decoder = &mut self.decoder;

        loop {
            let packet = self
                .probe
                .format
                .next_packet()
                .context("cannot read packet")?;
            if packet.track_id() != self.track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(buffer) => {
                    let spec = *buffer.spec();

                    macro_rules! to_buffer {
                        ($packet_buf: ident) => {
                            if let Some(sample_buf) = &mut self.buffer {
                                sample_buf.copy_interleaved_typed($packet_buf);
                            } else {
                                let mut sample_buf = SampleBuffer::<f32>::new(
                                    buffer.capacity() as symphonia::core::units::Duration,
                                    spec,
                                );
                                sample_buf.copy_interleaved_typed($packet_buf);
                                self.buffer = Some(sample_buf);
                            }
                        };
                    }

                    match &buffer {
                        AudioBufferRef::U8(buf) => to_buffer!(buf),
                        AudioBufferRef::U16(buf) => to_buffer!(buf),
                        AudioBufferRef::U24(buf) => to_buffer!(buf),
                        AudioBufferRef::U32(buf) => to_buffer!(buf),
                        AudioBufferRef::S8(buf) => to_buffer!(buf),
                        AudioBufferRef::S16(buf) => to_buffer!(buf),
                        AudioBufferRef::S24(buf) => to_buffer!(buf),
                        AudioBufferRef::S32(buf) => to_buffer!(buf),
                        AudioBufferRef::F32(buf) => to_buffer!(buf),
                        AudioBufferRef::F64(buf) => to_buffer!(buf),
                    }

                    let position = self.timestamp_to_duration(packet.ts());

                    return Ok(StreamPacketMeta {
                        channels_count: spec.channels.bits().count_ones() as usize,
                        sample_rate: spec.rate as usize,
                        track_meta: self.pull_track_info(),
                        position,
                    });
                }
                Err(symphonia::core::errors::Error::DecodeError(e)) => {
                    eprintln_with_date(format!("decode error: {e}"));
                }
                Err(e) => bail!(e),
            }
        }
    }

    fn write(&mut self, data: &mut VecDeque<f32>) -> Result<usize> {
        if let Some(buf) = &self.buffer {
            let samples = buf.samples();
            data.extend(samples);
            return Ok(samples.len());
        }
        bail!("sample buffer is not created yet");
    }

    fn seek(&mut self, pos: Duration) -> Result<Duration> {
        let time = Time::new(pos.as_secs(), pos.subsec_nanos() as f64 / 1_000_000_000_f64);
        let ts = match self.probe.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: None,
            },
        ) {
            Ok(seeked_pos) => seeked_pos.actual_ts,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                let (track, decoder) = Self::track_and_decoder_by_probe(&self.probe)?;
                self.track_id = track.id;
                self.decoder = decoder;
                0
            }
            Err(e) => {
                e.log_context("error while seeking");
                0
            }
        };

        self.buffer = None;
        let seek_to = self
            .timestamp_to_duration(ts)
            .context("cannot get time base from decoder")?;
        return Ok(seek_to);
    }
}

impl SymphoniaStream {
    fn timestamp_to_duration(&self, ts: TimeStamp) -> Option<Duration> {
        if let Some(time_base) = self.decoder.codec_params().time_base {
            let time = time_base.calc_time(ts);
            let duration = Duration::from_secs_f64(time.seconds as f64 + time.frac);
            return Some(duration);
        }
        return None;
    }

    fn track_and_decoder_by_probe(probe: &ProbeResult) -> Result<(&Track, Box<dyn Decoder>)> {
        let track = probe
            .format
            .tracks()
            .iter()
            .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .find_map(|t| {
                let decoder_opts = DecoderOptions::default();
                match symphonia::default::get_codecs().make(&t.codec_params, &decoder_opts) {
                    Ok(decoder) => Some((t, decoder)),
                    Err(e) => {
                        e.log_context(format!("unsupported codec for track {}", t.id));
                        None
                    }
                }
            })
            .context("no supported tracks in file")?;
        return Ok(track);
    }

    fn pull_track_info(&mut self) -> Option<TrackMeta> {
        if self.metadata_sent {
            return None;
        }
        self.metadata_sent = true;
        let meta = Self::get_lofty_meta(&self.path).unwrap_or_default();
        return Some(meta);
    }

    fn valid_lofty_tag_string(tag: &Tag, key: &ItemKey) -> Option<String> {
        if let Some(tag_item) = tag.get(key) {
            return match tag_item.value() {
                ItemValue::Text(s) => {
                    for c in s.chars() {
                        if c.is_ascii_control() {
                            return None;
                        }
                    }
                    return Some(s.clone());
                }
                _ => None,
            };
        }
        return None;
    }

    fn fill_lofty_tag(tag: &Tag, info: &mut TrackMeta) {
        if info.artist.is_none() {
            info.artist = Self::valid_lofty_tag_string(tag, &ItemKey::TrackArtist);
        }
        if info.album.is_none() {
            info.album = Self::valid_lofty_tag_string(tag, &ItemKey::AlbumTitle);
        }
        if info.title.is_none() {
            info.title = Self::valid_lofty_tag_string(tag, &ItemKey::TrackTitle);
        }
        if info.track.is_none() {
            info.track = tag.track().map(|x| x as usize);
        }
        if info.track_total.is_none() {
            info.track_total = tag.track_total().map(|x| x as usize);
        }
        if info.disc.is_none() {
            info.disc = tag.disk().map(|x| x as usize);
        }
        if info.disc_total.is_none() {
            info.disc_total = tag.disk_total().map(|x| x as usize);
        }
        if info.year.is_none() {
            info.year = tag.year().map(|x| x as usize);
        }
    }

    fn get_lofty_meta(path: &str) -> Option<TrackMeta> {
        match Probe::open(path) {
            Ok(probe) => match probe.read() {
                Ok(file) => {
                    let mut info = TrackMeta::default();
                    let properties = file.properties();
                    info.duration = properties.duration();

                    if file.tags().is_empty() {
                        eprintln_with_date(format!("not tags found: {path}"));
                        return Some(info);
                    }

                    match file.primary_tag() {
                        Some(primary_tag) => {
                            Self::fill_lofty_tag(primary_tag, &mut info);
                            for tag in file.tags() {
                                if tag.tag_type() != primary_tag.tag_type() {
                                    Self::fill_lofty_tag(tag, &mut info);
                                }
                            }
                        }
                        None => {
                            for tag in file.tags() {
                                Self::fill_lofty_tag(tag, &mut info);
                            }
                        }
                    }
                    return Some(info);
                }
                Err(e) => {
                    e.log_context(format!("can't read tags: {}", &path));
                    return None;
                }
            },
            Err(e) => {
                e.log_context(format!("can't open a file to read tags: {}", &path));
                return None;
            }
        }
    }
}
