//! Lecture folder on disk: layout, manifest, journal, crash-safe writes, recovery
//! and post-recording edits.

pub mod fsutil;
pub mod layout;
pub mod manifest;
pub mod recovery;
pub mod store;

pub use layout::LectureDir;
pub use manifest::Manifest;
pub use store::LectureSession;
