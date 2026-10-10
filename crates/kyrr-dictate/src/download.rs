//! Downloads a model file by file, each hash-checked before it is moved
//! into place, so an interrupted download never leaves a broken model.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::model::{hex, ModelFile, ModelSpec};

fn agent() -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::config::Config::builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .build()
        .new_agent()
}

/// Progress of a running download, in bytes over all files.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

#[derive(Debug)]
pub enum DownloadError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => f.write_str("cancelled"),
            DownloadError::Failed(e) => f.write_str(e),
        }
    }
}

/// Downloads every missing file of `spec` below `root`. Files that are
/// already present with the right size are kept. `progress` is called often;
/// setting `cancel` stops at the next chunk.
pub fn download(
    spec: &ModelSpec,
    root: &Path,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let dir = spec.dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| DownloadError::Failed(format!("{}: {e}", dir.display())))?;
    let total = spec.total_size();
    let mut done = 0;
    for file in spec.files {
        let dest = dir.join(file.name);
        if std::fs::metadata(&dest).is_ok_and(|m| m.len() == file.size) {
            done += file.size;
            progress(Progress { done, total });
            continue;
        }
        fetch_file(
            file,
            &dest,
            &mut |n| progress(Progress { done: done + n, total }),
            cancel,
        )?;
        done += file.size;
    }
    Ok(())
}

fn fetch_file(
    file: &ModelFile,
    dest: &Path,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let fail = |e: String| DownloadError::Failed(format!("{}: {e}", file.name));
    let part = dest.with_extension("part");
    let response = agent().get(file.url).call().map_err(|e| fail(e.to_string()))?;
    let mut reader = response.into_body().into_reader();
    let mut out = std::fs::File::create(&part).map_err(|e| fail(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(DownloadError::Cancelled);
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(fail(e.to_string())),
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            break Err(fail(e.to_string()));
        }
        hasher.update(&buf[..n]);
        got += n as u64;
        if got > file.size {
            break Err(fail("larger than expected".into()));
        }
        progress(got);
    };
    drop(out);
    let result = result.and_then(|()| {
        if got != file.size || hex(&hasher.finalize()) != file.sha256 {
            Err(fail("checksum mismatch".into()))
        } else {
            std::fs::rename(&part, dest).map_err(|e| fail(e.to_string()))
        }
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}
