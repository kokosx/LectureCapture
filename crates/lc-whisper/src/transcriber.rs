//! `Transcriber` implementation on top of whisper.cpp.

use anyhow::{anyhow, Context, Result};
use lc_core::pipeline::{TranscribeOutput, Transcriber};
use lc_core::transcript::{RawSegment, RawToken};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

#[derive(Clone, Debug)]
pub struct WhisperOptions {
    /// `pl`, `en`, … or `auto` for language detection. Never translates.
    pub language: String,
    pub threads: u32,
    pub beam_size: u32,
    pub use_gpu: bool,
    pub initial_prompt: Option<String>,
}

impl Default for WhisperOptions {
    fn default() -> Self {
        Self { language: "pl".into(), threads: 4, beam_size: 1, use_gpu: true, initial_prompt: None }
    }
}

pub struct WhisperTranscriber {
    ctx: WhisperContext,
    state: WhisperState,
    opts: WhisperOptions,
    model_name: String,
}

impl WhisperTranscriber {
    pub fn load(model_path: &Path, model_name: &str, opts: WhisperOptions) -> Result<Self> {
        crate::init_logging();
        let mut cp = WhisperContextParameters::default();
        cp.use_gpu(opts.use_gpu);
        cp.flash_attn(opts.use_gpu);
        let ctx = WhisperContext::new_with_params(model_path.to_str().context("model path")?, cp)
            .map_err(|e| anyhow!("ładowanie modelu {}: {e}", model_path.display()))?;
        let state = ctx.create_state().map_err(|e| anyhow!("whisper state: {e}"))?;
        Ok(Self { ctx, state, opts, model_name: model_name.to_string() })
    }
}

impl Transcriber for WhisperTranscriber {
    fn model_name(&self) -> String {
        self.model_name.clone()
    }

    fn engine_name(&self) -> String {
        format!("whisper.cpp {}", whisper_rs::get_whisper_version())
    }

    fn transcribe(&mut self, pcm: &[f32], _cancel: &AtomicBool) -> Result<TranscribeOutput> {
        let strategy = if self.opts.beam_size > 1 {
            SamplingStrategy::BeamSearch { beam_size: self.opts.beam_size as i32, patience: -1.0 }
        } else {
            SamplingStrategy::Greedy { best_of: 1 }
        };
        let mut p = FullParams::new(strategy);
        let lang = self.opts.language.clone();
        p.set_language(Some(if lang.is_empty() { "auto" } else { lang.as_str() }));
        p.set_translate(false);
        p.set_n_threads(self.opts.threads.max(1) as i32);
        p.set_token_timestamps(true);
        p.set_no_context(true);
        p.set_suppress_blank(true);
        p.set_suppress_nst(true);
        p.set_no_speech_thold(0.6);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        if let Some(prompt) = &self.opts.initial_prompt {
            if !prompt.trim().is_empty() {
                p.set_initial_prompt(prompt);
            }
        }
        self.state.full(p, pcm).map_err(|e| anyhow!("whisper_full: {e}"))?;

        let eot = self.ctx.token_eot();
        let mut segments = Vec::new();
        for seg in self.state.as_iter() {
            let text = seg.to_str_lossy().map(|c| c.into_owned()).unwrap_or_default();
            let mut tokens = Vec::new();
            // A multi-byte character can be split across tokens: accumulate bytes until
            // they form valid UTF-8.
            let mut pending: Vec<u8> = Vec::new();
            let mut pending_t0: Option<u64> = None;
            for i in 0..seg.n_tokens() {
                let Some(tok) = seg.get_token(i) else { continue };
                let data = tok.token_data();
                if data.id >= eot {
                    continue; // timestamps / special tokens
                }
                let Ok(bytes) = tok.to_bytes() else { continue };
                pending.extend_from_slice(bytes);
                let t0 = *pending_t0.get_or_insert((data.t0.max(0) as u64) * 10);
                if let Ok(text) = std::str::from_utf8(&pending) {
                    tokens.push(RawToken { t0_ms: t0, t1_ms: (data.t1.max(0) as u64) * 10, text: text.to_string() });
                    pending.clear();
                    pending_t0 = None;
                } else if pending.len() > 8 {
                    let text = String::from_utf8_lossy(&pending).into_owned();
                    tokens.push(RawToken { t0_ms: t0, t1_ms: (data.t1.max(0) as u64) * 10, text });
                    pending.clear();
                    pending_t0 = None;
                }
            }
            segments.push(RawSegment {
                t0_ms: (seg.start_timestamp().max(0) as u64) * 10,
                t1_ms: (seg.end_timestamp().max(0) as u64) * 10,
                text,
                tokens,
                no_speech_prob: seg.no_speech_probability(),
            });
        }
        let lang_id = self.state.full_lang_id_from_state();
        let language = whisper_rs::get_lang_str(lang_id).map(|s| s.to_string());
        Ok(TranscribeOutput { segments, language })
    }
}
