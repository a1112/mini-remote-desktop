use crate::{AudioPacket, PlayoutBuffer, CHANNELS, SAMPLES_PER_PACKET, SAMPLE_RATE};
use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use screencapturekit::{cm::CMSampleBuffer, prelude::*};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub struct CapturedPacket {
    pub packet: AudioPacket,
    pub captured_at: Instant,
}
pub struct SystemAudioCapture {
    pub packets: mpsc::Receiver<CapturedPacket>,
    stop: mpsc::Sender<()>,
}
impl SystemAudioCapture {
    pub fn start(stream_id: [u8; 16]) -> Result<Self> {
        let (tx, packets) = mpsc::sync_channel(8);
        let (stop, stopped) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("mrd-system-audio".into())
            .spawn(move || {
                let result = capture_stream(stream_id, tx);
                match result {
                    Ok(stream) => {
                        if ready.send(Ok(())).is_ok() {
                            let _ = stopped.recv();
                        }
                        let _ = stream.stop_capture();
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error.to_string()));
                    }
                }
            })?;
        started
            .recv_timeout(Duration::from_secs(10))
            .context("system audio startup timed out")?
            .map_err(anyhow::Error::msg)?;
        Ok(Self { packets, stop })
    }
}
impl Drop for SystemAudioCapture {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

fn capture_stream(stream_id: [u8; 16], tx: mpsc::SyncSender<CapturedPacket>) -> Result<SCStream> {
    let content = SCShareableContent::get().map_err(|error| {
        anyhow!("Screen Recording permission is required for system audio: {error}")
    })?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .context("no desktop display for audio capture")?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();
    let config = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_fps(1)
        .with_captures_audio(true)
        .with_sample_rate(SAMPLE_RATE as i32)
        .with_channel_count(CHANNELS as i32)
        .with_excludes_current_process_audio(true);
    let mut stream = SCStream::new(&filter, &config);
    let pending = Arc::new(Mutex::new((
        Vec::<i16>::with_capacity(SAMPLES_PER_PACKET),
        0_u64,
    )));
    if stream
        .add_output_handler(
            move |sample: CMSampleBuffer, kind| {
                if kind != SCStreamOutputType::Audio {
                    return;
                }
                let Some(format) = sample.format_description() else {
                    return;
                };
                if format.audio_sample_rate() != Some(SAMPLE_RATE as f64)
                    || format.audio_channel_count() != Some(CHANNELS as u32)
                    || format.audio_bits_per_channel() != Some(32)
                    || !format.audio_is_float()
                    || format.audio_is_big_endian()
                {
                    return;
                }
                let Some(buffers) = sample.audio_buffer_list() else {
                    return;
                };
                let buffers: Vec<_> = buffers.iter().collect();
                let planar = buffers.len() == CHANNELS
                    && buffers.iter().all(|buffer| buffer.number_channels == 1);
                let interleaved =
                    buffers.len() == 1 && buffers[0].number_channels == CHANNELS as u32;
                if !planar && !interleaved {
                    return;
                }
                let frames = sample.num_samples();
                if frames > SAMPLE_RATE as usize
                    || buffers.iter().any(|buffer| {
                        buffer.data().len() < frames * 4 * if planar { 1 } else { CHANNELS }
                    })
                {
                    return;
                }
                let Ok(mut pending) = pending.try_lock() else {
                    return;
                };
                for frame in 0..frames {
                    for channel in 0..CHANNELS {
                        let (buffer, index) = if planar {
                            (buffers[channel].data(), frame)
                        } else {
                            (buffers[0].data(), frame * CHANNELS + channel)
                        };
                        let value = f32::from_le_bytes(
                            buffer[index * 4..index * 4 + 4].try_into().unwrap(),
                        );
                        pending.0.push(if value.is_finite() {
                            (value.clamp(-1.0, 1.0) * 32767.0).round() as i16
                        } else {
                            0
                        });
                    }
                    if pending.0.len() == SAMPLES_PER_PACKET {
                        let packet = AudioPacket {
                            stream_id,
                            sequence: pending.1,
                            samples: std::mem::replace(
                                &mut pending.0,
                                Vec::with_capacity(SAMPLES_PER_PACKET),
                            ),
                        };
                        pending.1 = pending.1.saturating_add(1);
                        let _ = tx.try_send(CapturedPacket {
                            packet,
                            captured_at: Instant::now(),
                        });
                    }
                }
            },
            SCStreamOutputType::Audio,
        )
        .is_none()
    {
        bail!("cannot register system audio callback");
    }
    stream
        .start_capture()
        .map_err(|error| anyhow!("start system audio: {error}"))?;
    Ok(stream)
}

pub struct AudioPlayback {
    pub buffer: Arc<Mutex<PlayoutBuffer>>,
    pub muted: Arc<AtomicBool>,
    pub active: Arc<AtomicBool>,
    pub error: Arc<Mutex<Option<String>>>,
    stop: mpsc::Sender<()>,
}
impl AudioPlayback {
    pub fn start(expires_at: Instant) -> Result<Self> {
        let buffer = Arc::new(Mutex::new(PlayoutBuffer::default()));
        let muted = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(true));
        let error = Arc::new(Mutex::new(None));
        let (stop, stopped) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let (b, m, a, e) = (buffer.clone(), muted.clone(), active.clone(), error.clone());
        thread::Builder::new()
            .name("mrd-audio-playback".into())
            .spawn(move || {
                let result = output_stream(b, m, a, e, expires_at);
                match result {
                    Ok(stream) => {
                        if ready.send(Ok(())).is_ok() {
                            let _ = stopped.recv();
                        }
                        drop(stream);
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error.to_string()));
                    }
                }
            })?;
        started
            .recv_timeout(Duration::from_secs(5))
            .context("audio playback startup timed out")?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            buffer,
            muted,
            active,
            error,
            stop,
        })
    }
}
impl Drop for AudioPlayback {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        let _ = self.stop.send(());
    }
}
fn output_stream(
    buffer: Arc<Mutex<PlayoutBuffer>>,
    muted: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    expires_at: Instant,
) -> Result<cpal::Stream> {
    let device = cpal::default_host()
        .default_output_device()
        .context("no audio output device")?;
    let config = device
        .supported_output_configs()?
        .find(|range| {
            range.channels() == 2
                && range.sample_format() == cpal::SampleFormat::F32
                && range.min_sample_rate().0 <= SAMPLE_RATE
                && range.max_sample_rate().0 >= SAMPLE_RATE
        })
        .context("audio output requires stereo Float32 at 48 kHz")?
        .with_sample_rate(cpal::SampleRate(SAMPLE_RATE));
    let stream = device.build_output_stream(
        &config.config(),
        move |output: &mut [f32], _| {
            output.fill(0.0);
            let Ok(mut buffer) = buffer.try_lock() else {
                return;
            };
            if !active.load(Ordering::Acquire) || Instant::now() >= expires_at {
                buffer.clear();
                return;
            }
            let muted = muted.load(Ordering::Relaxed);
            for sample in output {
                let value = buffer.next_sample();
                if !muted {
                    *sample = value;
                }
            }
        },
        move |failure| {
            if let Ok(mut error) = error.lock() {
                *error = Some(failure.to_string());
            }
        },
        None,
    )?;
    stream.play()?;
    Ok(stream)
}
