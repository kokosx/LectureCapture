//! Platform capture backends implementing the `lc_core::pipeline` traits.
//!
//! * macOS: ScreenCaptureKit for window/display frames and for system audio
//!   (with the app's own audio excluded to prevent feedback loops),
//! * Windows: Windows Graphics Capture for frames, WASAPI loopback for system audio,
//! * both: microphone through cpal.

use serde::{Deserialize, Serialize};

pub mod cpal_source;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    /// Native pixel size.
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub bundle_id: String,
    pub pid: i32,
    /// Size in points.
    pub width: u32,
    pub height: u32,
    pub on_screen: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceList {
    pub displays: Vec<DisplayInfo>,
    pub windows: Vec<WindowInfo>,
}

/// What to capture. A free-hand region is a display/window plus a crop rectangle
/// (`lc_core::frame::NormRect`) handled by the detector.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureTarget {
    Display { id: u32 },
    Window { id: u32, bundle_id: Option<String>, title: Option<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDeviceInfo>,
    /// Output devices usable for loopback (Windows).
    pub outputs: Vec<AudioDeviceInfo>,
    /// Whether system audio is captured without choosing a device (macOS).
    pub system_audio_builtin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Permissions {
    pub screen: bool,
    /// `granted`, `denied`, `undetermined`, `unknown`
    pub microphone: String,
}

pub use platform::*;

#[cfg(target_os = "macos")]
mod platform {
    pub use super::macos::{
        list_sources, permissions, request_screen_permission, snapshot, system_audio_source, video_source,
    };
}

#[cfg(windows)]
mod platform {
    pub use super::windows::{
        list_sources, permissions, request_screen_permission, snapshot, system_audio_source, video_source,
    };
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::*;
    use anyhow::{bail, Result};
    use lc_core::pipeline::{AudioSource, VideoSource};
    pub fn list_sources() -> Result<SourceList> {
        bail!("screen capture is not supported on this platform")
    }
    pub fn snapshot(_t: &CaptureTarget) -> Result<lc_core::frame::Frame> {
        bail!("screen capture is not supported on this platform")
    }
    pub fn video_source(_t: CaptureTarget) -> Result<Box<dyn VideoSource>> {
        bail!("screen capture is not supported on this platform")
    }
    pub fn system_audio_source(_c: &lc_core::config::AudioConfig) -> Result<Box<dyn AudioSource>> {
        bail!("system audio capture is not supported on this platform")
    }
    pub fn permissions() -> Permissions {
        Permissions { screen: false, microphone: "unknown".into() }
    }
    pub fn request_screen_permission() -> bool {
        false
    }
}

pub use cpal_source::{list_audio_devices, microphone_source};
