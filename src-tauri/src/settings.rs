//! Persistent user settings (`settings.json` in the app config directory).

use lc_capture::CaptureTarget;
use lc_core::config::{AudioConfig, DetectorConfig, OutputConfig, TranscriptionConfig};
use lc_core::frame::NormRect;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Default parent folder for new lectures.
    pub lectures_root: PathBuf,
    /// Other folders lectures were saved to (shown in the library).
    pub known_roots: Vec<PathBuf>,
    pub detector: DetectorConfig,
    pub audio: AudioConfig,
    pub transcription: TranscriptionConfig,
    pub output: OutputConfig,
    /// User confirmed they will follow university rules / obtain consent.
    pub consent_acknowledged: bool,
    pub capture_shortcut: String,
    pub keep_awake: bool,
    /// `system`, `light`, `dark`
    pub theme: String,
    pub last_target: Option<CaptureTarget>,
    pub last_crop: Option<NormRect>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lectures_root: dirs::document_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default()).join("LectureCapture"),
            known_roots: Vec::new(),
            detector: DetectorConfig::default(),
            audio: AudioConfig::default(),
            transcription: TranscriptionConfig::default(),
            output: OutputConfig::default(),
            consent_acknowledged: false,
            capture_shortcut: "CmdOrCtrl+Shift+S".into(),
            keep_awake: true,
            theme: "system".into(),
            last_target: None,
            last_crop: None,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        lc_core::session::fsutil::atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        let mut r = vec![self.lectures_root.clone()];
        for k in &self.known_roots {
            if !r.contains(k) {
                r.push(k.clone());
            }
        }
        r
    }
}
