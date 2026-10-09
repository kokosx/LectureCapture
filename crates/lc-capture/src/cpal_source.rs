//! cpal-based audio capture: microphone on all platforms and WASAPI loopback of an
//! output device on Windows. The stream lives on its own thread (cpal streams are not
//! `Send` everywhere) and is re-created automatically when the device disappears
//! (headphones unplugged, default device changed).

use super::{AudioDeviceInfo, AudioDevices};
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;
use lc_core::audio::mixer::SourceKind;
use lc_core::pipeline::{AudioEvent, AudioSource, CaptureHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum DeviceSel {
    /// Input device by name (None = system default).
    Input(Option<String>),
    /// Output device used in loopback mode (Windows WASAPI only).
    Loopback(Option<String>),
}

pub struct CpalSource {
    pub kind: SourceKind,
    pub device: DeviceSel,
}

pub fn microphone_source(name: Option<String>) -> Box<dyn AudioSource> {
    Box::new(CpalSource { kind: SourceKind::Microphone, device: DeviceSel::Input(name) })
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| d.to_string())
}

pub fn list_audio_devices() -> AudioDevices {
    let host = cpal::default_host();
    let def_in = host.default_input_device().map(|d| device_name(&d));
    let def_out = host.default_output_device().map(|d| device_name(&d));
    let inputs = host
        .input_devices()
        .map(|it| {
            it.map(|d| {
                let name = device_name(&d);
                AudioDeviceInfo { is_default: Some(&name) == def_in.as_ref(), name }
            })
            .collect()
        })
        .unwrap_or_default();
    let outputs = host
        .output_devices()
        .map(|it| {
            it.map(|d| {
                let name = device_name(&d);
                AudioDeviceInfo { is_default: Some(&name) == def_out.as_ref(), name }
            })
            .collect()
        })
        .unwrap_or_default();
    AudioDevices { inputs, outputs, system_audio_builtin: cfg!(target_os = "macos") }
}

fn find_device(sel: &DeviceSel) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let (name, list, default) = match sel {
        DeviceSel::Input(n) => (n.clone(), host.input_devices()?.collect::<Vec<_>>(), host.default_input_device()),
        DeviceSel::Loopback(n) => (n.clone(), host.output_devices()?.collect::<Vec<_>>(), host.default_output_device()),
    };
    if let Some(n) = name {
        if let Some(d) = list.into_iter().find(|d| device_name(d) == n) {
            return Ok(d);
        }
        log::warn!("audio device '{n}' not found, using default");
    }
    default.ok_or_else(|| anyhow!("brak domyślnego urządzenia audio"))
}

fn build_stream(
    sel: &DeviceSel,
    kind: SourceKind,
    tx: Sender<AudioEvent>,
    failed: Arc<AtomicBool>,
) -> Result<(cpal::Stream, String)> {
    let dev = find_device(sel)?;
    let name = device_name(&dev);
    let cfg = match sel {
        DeviceSel::Input(_) => dev.default_input_config()?,
        DeviceSel::Loopback(_) => dev.default_output_config()?,
    };
    let channels = cfg.channels() as usize;
    let rate = cfg.sample_rate();
    let stream_cfg = cfg.config();
    let err_flag = failed.clone();
    let on_err = move |e: cpal::Error| {
        log::warn!("audio stream error: {e}");
        err_flag.store(true, Ordering::SeqCst);
    };
    let send = move |mono: Vec<f32>| {
        // never block the realtime audio thread
        let _ = tx.try_send(AudioEvent::Samples { source: kind, rate, data: mono });
    };
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_input_stream(
            stream_cfg,
            move |data: &[f32], _| send(lc_core::audio::downmix_interleaved(data, channels)),
            on_err,
            None,
        )?,
        cpal::SampleFormat::I16 => dev.build_input_stream(
            stream_cfg,
            move |data: &[i16], _| {
                let f: Vec<f32> = data.iter().map(|v| *v as f32 / 32768.0).collect();
                send(lc_core::audio::downmix_interleaved(&f, channels))
            },
            on_err,
            None,
        )?,
        cpal::SampleFormat::I32 => dev.build_input_stream(
            stream_cfg,
            move |data: &[i32], _| {
                let f: Vec<f32> = data.iter().map(|v| *v as f32 / 2_147_483_648.0).collect();
                send(lc_core::audio::downmix_interleaved(&f, channels))
            },
            on_err,
            None,
        )?,
        other => return Err(anyhow!("nieobsługiwany format próbek: {other:?}")),
    };
    stream.play()?;
    Ok((stream, format!("{name} ({rate} Hz, {channels} kan.)")))
}

struct Handle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureHandle for Handle {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl AudioSource for CpalSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn describe(&self) -> String {
        match &self.device {
            DeviceSel::Input(n) => format!("mikrofon: {}", n.as_deref().unwrap_or("domyślny")),
            DeviceSel::Loopback(n) => format!("loopback: {}", n.as_deref().unwrap_or("domyślne wyjście")),
        }
    }

    fn start(&mut self, tx: Sender<AudioEvent>) -> Result<Box<dyn CaptureHandle>> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<String>>(1);
        let (sel, kind, stop2) = (self.device.clone(), self.kind, stop.clone());
        let thread = std::thread::Builder::new().name("lc-cpal".into()).spawn(move || {
            let failed = Arc::new(AtomicBool::new(false));
            let mut current = match build_stream(&sel, kind, tx.clone(), failed.clone()) {
                Ok((s, d)) => {
                    let _ = ready_tx.send(Ok(d));
                    Some(s)
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            while !stop2.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(200));
                if failed.swap(false, Ordering::SeqCst) {
                    drop(current.take());
                    let _ = tx.send(AudioEvent::Lost { source: kind, reason: "urządzenie audio niedostępne".into() });
                    // retry until a device is available again
                    while !stop2.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_secs(2));
                        match build_stream(&sel, kind, tx.clone(), failed.clone()) {
                            Ok((s, d)) => {
                                current = Some(s);
                                let _ = tx.send(AudioEvent::Restored { source: kind, detail: d });
                                break;
                            }
                            Err(e) => log::debug!("audio device retry: {e}"),
                        }
                    }
                }
            }
            drop(current);
        })?;
        let desc = ready_rx.recv_timeout(Duration::from_secs(10)).map_err(|_| anyhow!("audio device timeout"))??;
        log::info!("audio source started: {desc}");
        Ok(Box::new(Handle { stop, thread: Some(thread) }))
    }
}
