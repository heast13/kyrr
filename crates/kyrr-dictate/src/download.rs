//! Downloads a model file by file, each hash-checked before it is moved
//! into place, so an interrupted download never leaves a broken model.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::model::{hex, ModelFile, ModelSpec};

/// A dead connection gives up instead of hanging (ureq waits forever by
/// default); the body itself may take long on a slow line.
fn agent() -> ureq::Agent {
    use std::time::Duration;
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::config::Config::builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_global(Some(Duration::from_secs(6 * 60 * 60)))
        .build()
        .new_agent()
}

/// Progress of a running download, in bytes over all files.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// Why a download stopped, so the app can say it in plain words.
#[derive(Debug)]
pub enum DownloadError {
    Cancelled,
    /// No connection, or it broke off.
    Offline(String),
    /// The server answered with an error (e.g. the file is not there).
    Server(String),
    /// The file arrived damaged (size or checksum wrong).
    Corrupt(String),
    /// Writing to the disk failed (full, no permission).
    Disk(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => f.write_str("cancelled"),
            DownloadError::Offline(e)
            | DownloadError::Server(e)
            | DownloadError::Corrupt(e)
            | DownloadError::Disk(e) => f.write_str(e),
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
    std::fs::create_dir_all(&dir).map_err(|e| DownloadError::Disk(format!("{}: {e}", dir.display())))?;
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
    let named = |e: &dyn std::fmt::Display| format!("{}: {e}", file.name);
    let part = dest.with_extension("part");
    let response = agent().get(file.url).call().map_err(|e| match e {
        ureq::Error::StatusCode(_) => DownloadError::Server(named(&e)),
        _ => DownloadError::Offline(named(&e)),
    })?;
    let mut reader = response.into_body().into_reader();
    let mut out = std::fs::File::create(&part).map_err(|e| DownloadError::Disk(named(&e)))?;
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
            Err(e) => break Err(DownloadError::Offline(named(&e))),
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            break Err(DownloadError::Disk(named(&e)));
        }
        hasher.update(&buf[..n]);
        got += n as u64;
        if got > file.size {
            break Err(DownloadError::Corrupt(named(&"larger than expected")));
        }
        progress(got);
    };
    drop(out);
    let result = result.and_then(|()| {
        if got != file.size || hex(&hasher.finalize()) != file.sha256 {
            Err(DownloadError::Corrupt(named(&"checksum mismatch")))
        } else {
            std::fs::rename(&part, dest).map_err(|e| DownloadError::Disk(named(&e)))
        }
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}
