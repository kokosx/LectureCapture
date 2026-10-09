//! whisper.cpp transcription (via whisper-rs) and a small model manager.

pub mod models;
pub mod transcriber;

pub use models::{ModelInfo, ModelManager, ModelStatus, CATALOG};
pub use transcriber::{WhisperOptions, WhisperTranscriber};

/// Route whisper.cpp / ggml logging into the `log` crate (once).
pub fn init_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(whisper_rs::install_logging_hooks);
}

pub fn system_info() -> String {
    whisper_rs::print_system_info().to_string()
}
