//! Crash-safe file helpers.

use anyhow::{Context, Result};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Write a file atomically: temp file in the same directory → fsync → rename.
/// Readers never observe a half-written file, even after a crash or power loss.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path without parent")?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().context("path without file name")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    {
        let mut f = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    replace(&tmp, path).with_context(|| format!("rename into {}", path.display()))?;
    sync_dir(dir);
    Ok(())
}

fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    // std::fs::rename replaces atomically on Unix and uses MoveFileEx(REPLACE_EXISTING)
    // on Windows.
    fs::rename(from, to)
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Append one line and flush it to disk (journal, segments.jsonl).
pub fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    let mut buf = line.as_bytes().to_vec();
    buf.push(b'\n');
    f.write_all(&buf)?;
    f.sync_data()?;
    Ok(())
}

/// Read a JSONL file, skipping a torn last line (crash during append).
pub fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(v) => out.push(v),
            Err(e) => log::warn!("{}:{}: skipping unreadable line: {e}", path.display(), i + 1),
        }
    }
    Ok(out)
}

/// Total size of a directory tree in bytes.
pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = fs::read_dir(path) {
        for e in rd.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => total += dir_size(&e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.json");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1, "no temp files left behind");
    }

    #[test]
    fn jsonl_skips_torn_line() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.jsonl");
        append_line(&p, r#"{"a":1}"#).unwrap();
        append_line(&p, r#"{"a":2}"#).unwrap();
        let mut f = OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(br#"{"a":"#).unwrap();
        let v: Vec<serde_json::Value> = read_jsonl(&p).unwrap();
        assert_eq!(v.len(), 2);
    }
}
