//! ZIP export of a lecture folder (internal `.lc/` data excluded).

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

pub fn export_zip(lecture_root: &Path, dest: &Path, include_audio: bool) -> Result<u64> {
    let folder = lecture_root.file_name().context("bad lecture folder")?.to_string_lossy().into_owned();
    let tmp = dest.with_extension("zip.part");
    let file = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let mut zip = zip::ZipWriter::new(BufWriter::new(file));
    let mut count = 0u64;
    let mut stack = vec![lecture_root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir)?.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue; // .lc internal state, .DS_Store, temp files
            }
            if p.is_dir() {
                if !include_audio && p == lecture_root.join("audio") {
                    continue;
                }
                stack.push(p);
            } else {
                files.push(p);
            }
        }
    }
    files.sort();
    for p in files {
        let rel = p.strip_prefix(lecture_root)?;
        let rel: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let name = format!("{folder}/{}", rel.join("/"));
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        // already-compressed formats are stored as-is
        let method = match ext.as_str() {
            "png" | "webp" | "ogg" | "opus" | "zip" => CompressionMethod::Stored,
            _ => CompressionMethod::Deflated,
        };
        zip.start_file(name, SimpleFileOptions::default().compression_method(method).large_file(true))?;
        let mut f = File::open(&p)?;
        std::io::copy(&mut f, &mut zip)?;
        count += 1;
    }
    let mut w = zip.finish()?;
    w.flush()?;
    drop(w);
    std::fs::rename(&tmp, dest)?;
    Ok(count)
}
