//! Downloads a model file by file, each hash-checked before it is moved
//! into place, so an interrupted download never leaves a broken model. A
//! connection that breaks off or goes silent is picked up where it stopped.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::model::{hex, ModelFile, ModelSpec};

/// A dead connection gives up instead of hanging (ureq waits forever by
/// default); the body itself may take long on a slow line. HTTPS only,
/// redirects included (the hosts send downloads on to their storage).
fn agent() -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::config::Config::builder()
        // Except the unit tests' web server on localhost.
        .https_only(!cfg!(test))
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

/// A connection that delivers nothing for this long counts as broken, even
/// when the other side never says so (Wi-Fi gone, a VPN switching over).
const STALL: Duration = Duration::from_secs(60);

/// How often a source whose connection broke off is asked again, each time
/// going on where the last one stopped.
const RESUMES: usize = 3;

/// Tries the file's sources in order until one delivers it intact. A
/// connection that breaks off is picked up again where it stopped (also on
/// the next try, its part stays); a missing file or a damaged copy moves on
/// to the next source; cancelling or a full disk stops at once.
fn fetch_file(
    file: &ModelFile,
    dest: &Path,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    fetch_file_with(file, dest, progress, cancel, STALL)
}

fn fetch_file_with(
    file: &ModelFile,
    dest: &Path,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
    stall: Duration,
) -> Result<(), DownloadError> {
    let mut last = DownloadError::Server(format!("{}: no source", file.name));
    for url in file.urls {
        for _ in 0..=RESUMES {
            match fetch_from(file, url, dest, progress, cancel, stall) {
                Ok(()) => return Ok(()),
                Err(e @ (DownloadError::Cancelled | DownloadError::Disk(_))) => return Err(e),
                Err(e @ DownloadError::Offline(_)) => last = e,
                Err(e) => {
                    last = e;
                    break;
                }
            }
        }
    }
    if !matches!(last, DownloadError::Offline(_)) {
        let _ = std::fs::remove_file(part_of(dest));
    }
    Err(last)
}

/// Where a file grows until it is complete and checked.
fn part_of(dest: &Path) -> std::path::PathBuf {
    dest.with_extension("part")
}

fn fetch_from(
    file: &ModelFile,
    url: &str,
    dest: &Path,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
    stall: Duration,
) -> Result<(), DownloadError> {
    let named = |e: &dyn std::fmt::Display| format!("{}: {e}", file.name);
    let part = part_of(dest);
    // What an earlier, broken-off try left behind, checked again from the
    // start of the file so the hash covers every byte.
    let mut hasher = Sha256::new();
    let mut got = match std::fs::File::open(&part) {
        Ok(mut f) => match hash_into(&mut f, &mut hasher) {
            Ok(n) if n < file.size => n,
            _ => {
                hasher = Sha256::new();
                0
            }
        },
        Err(_) => 0,
    };
    let mut request = agent().get(url);
    if got > 0 {
        request = request.header("Range", format!("bytes={got}-"));
    }
    let response = request.call().map_err(|e| match e {
        ureq::Error::StatusCode(_) => DownloadError::Server(named(&e)),
        _ => DownloadError::Offline(named(&e)),
    })?;
    if got > 0 && !resumes_at(&response, got) {
        // The whole file again (or a part from the wrong place).
        got = 0;
        hasher = Sha256::new();
        if response.status() != 200 {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Offline(named(
                &"the server did not go on where it stopped",
            )));
        }
    }
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(got > 0)
        .truncate(got == 0)
        .open(&part)
        .map_err(|e| DownloadError::Disk(named(&e)))?;
    progress(got);
    let chunks = read_ahead(response.into_body().into_reader());
    let mut quiet = Duration::ZERO;
    let tick = Duration::from_millis(250).min(stall);
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(DownloadError::Cancelled);
        }
        let chunk = match chunks.recv_timeout(tick) {
            Ok(Ok(chunk)) => chunk,
            Ok(Err(e)) => break Err(DownloadError::Offline(named(&e))),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                quiet += tick;
                if quiet >= stall {
                    break Err(DownloadError::Offline(named(&"no data for a while")));
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
        };
        quiet = Duration::ZERO;
        if let Err(e) = out.write_all(&chunk) {
            break Err(DownloadError::Disk(named(&e)));
        }
        hasher.update(&chunk);
        got += chunk.len() as u64;
        if got > file.size {
            break Err(DownloadError::Corrupt(named(&"larger than expected")));
        }
        progress(got);
    };
    let result = result.and_then(|()| {
        if got != file.size {
            // Ended early without saying so: like a broken connection.
            return Err(DownloadError::Offline(named(&"the download ended early")));
        }
        if hex(&hasher.finalize()) != file.sha256 {
            return Err(DownloadError::Corrupt(named(&"checksum mismatch")));
        }
        // On the disk for real before it counts as installed.
        out.sync_all().map_err(|e| DownloadError::Disk(named(&e)))?;
        drop(out);
        rename(&part, dest).map_err(|e| DownloadError::Disk(named(&e)))
    });
    match &result {
        // Kept to go on with, on the next try or the next day.
        Ok(()) | Err(DownloadError::Offline(_)) => {}
        Err(_) => {
            let _ = std::fs::remove_file(&part);
        }
    }
    result
}

/// Whether a ranged answer starts exactly at `offset`.
fn resumes_at(response: &ureq::http::Response<ureq::Body>, offset: u64) -> bool {
    response.status() == 206
        && response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| range_start(v) == Some(offset))
}

/// The first byte of a `Content-Range: bytes 100-199/1000` header.
fn range_start(header: &str) -> Option<u64> {
    let range = header.trim().strip_prefix("bytes")?.trim_start();
    range.split('-').next()?.trim().parse().ok()
}

/// Reads the body on a thread of its own and hands it over in chunks, so
/// the download can notice a connection that went silent and stop at once
/// when cancelled. A dropped receiver makes the thread stop after its
/// current read; one stuck on a dead connection waits for the agent's
/// overall time limit, holding only its socket.
fn read_ahead(mut reader: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::sync_channel(4);
    std::thread::spawn(move || loop {
        let mut buf = vec![0u8; 256 * 1024];
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                buf.truncate(n);
                if tx.send(Ok(buf)).is_err() {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                let _ = tx.send(Err(e));
                break;
            }
        }
    });
    rx
}

fn hash_into(file: &mut std::fs::File, hasher: &mut Sha256) -> std::io::Result<u64> {
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(total);
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
}

/// Moves the finished file into place. Right after writing, a virus scanner
/// may still hold it on Windows; that passes within moments.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(_) if tries < 10 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(300));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// A one-file web server on localhost. `answer` gets the connection's
    /// number and its Range start (if asked for) and writes the reply.
    fn serve(
        answer: impl Fn(usize, Option<u64>, &mut TcpStream) + Send + Sync + 'static,
    ) -> (String, Arc<Mutex<Vec<Option<u64>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/data.bin", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        let answer = Arc::new(answer);
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { return };
                let mut range = None;
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = v.trim().trim_end_matches('-').parse().ok();
                    }
                }
                log.lock().unwrap().push(range);
                let answer = Arc::clone(&answer);
                std::thread::spawn(move || answer(n, range, &mut stream));
            }
        });
        (url, asked)
    }

    fn content() -> Vec<u8> {
        (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn file_for(url: String, data: &[u8]) -> ModelFile {
        ModelFile {
            name: "data.bin",
            urls: Box::leak(vec![&*Box::leak(url.into_boxed_str())].into_boxed_slice()),
            size: data.len() as u64,
            sha256: Box::leak(hex(&Sha256::digest(data)).into_boxed_str()),
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kyrr-dl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn whole(stream: &mut TcpStream, data: &[u8]) {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(data);
    }

    fn from(stream: &mut TcpStream, data: &[u8], start: u64) {
        let total = data.len();
        let head = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{total}\r\nConnection: close\r\n\r\n",
            total - start as usize,
            total - 1
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&data[start as usize..]);
    }

    /// Headers promising the whole file, then only the first `n` bytes.
    fn broken_after(stream: &mut TcpStream, data: &[u8], n: usize) {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&data[..n]);
    }

    #[test]
    fn goes_on_where_the_connection_broke_off() {
        let data = content();
        let served = data.clone();
        let (url, asked) = serve(move |n, range, s| match (n, range) {
            (0, None) => broken_after(s, &served, 100_000),
            (_, Some(start)) => from(s, &served, start),
            _ => whole(s, &served),
        });
        let dir = folder("resume");
        let dest = dir.join("data.bin");
        let file = file_for(url, &data);
        fetch_file_with(&file, &dest, &mut |_| {}, &AtomicBool::new(false), STALL).expect("download");
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!part_of(&dest).exists());
        assert_eq!(*asked.lock().unwrap(), [None, Some(100_000)]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn starts_over_when_the_server_ignores_the_range() {
        let data = content();
        let served = data.clone();
        let (url, asked) = serve(move |_, _, s| whole(s, &served));
        let dir = folder("norange");
        let dest = dir.join("data.bin");
        std::fs::write(part_of(&dest), &data[..5_000]).unwrap();
        let file = file_for(url, &data);
        fetch_file_with(&file, &dest, &mut |_| {}, &AtomicBool::new(false), STALL).expect("download");
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(*asked.lock().unwrap(), [Some(5_000)]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_part_is_not_kept() {
        let data = content();
        let served = data.clone();
        let (url, _) = serve(move |_, range, s| from(s, &served, range.unwrap_or(0)));
        let dir = folder("damaged");
        let dest = dir.join("data.bin");
        // Wrong bytes at the start: the finished file fails its check.
        std::fs::write(part_of(&dest), vec![0u8; 5_000]).unwrap();
        let file = file_for(url, &data);
        let err = fetch_file_with(&file, &dest, &mut |_| {}, &AtomicBool::new(false), STALL).unwrap_err();
        assert!(matches!(err, DownloadError::Corrupt(_)), "{err}");
        assert!(!part_of(&dest).exists() && !dest.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_silent_connection_gives_up() {
        let data = content();
        let served = data.clone();
        let (url, asked) = serve(move |_, _, s| {
            broken_after(s, &served, 1_000);
            std::thread::sleep(Duration::from_secs(5));
        });
        let dir = folder("stall");
        let dest = dir.join("data.bin");
        let file = file_for(url, &data);
        let started = std::time::Instant::now();
        let err = fetch_file_with(
            &file,
            &dest,
            &mut |_| {},
            &AtomicBool::new(false),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Offline(_)), "{err}");
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_eq!(asked.lock().unwrap().len(), RESUMES + 1);
        // Kept for the next try.
        assert_eq!(std::fs::metadata(part_of(&dest)).unwrap().len(), 1_000);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelling_stops_a_silent_download_at_once() {
        let data = content();
        let served = data.clone();
        let (url, _) = serve(move |_, _, s| {
            broken_after(s, &served, 1_000);
            std::thread::sleep(Duration::from_secs(5));
        });
        let dir = folder("cancel");
        let dest = dir.join("data.bin");
        let file = file_for(url, &data);
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let err = fetch_file_with(
            &file,
            &dest,
            &mut |got| {
                if got >= 1_000 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            &cancel,
            STALL,
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!part_of(&dest).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn content_range_start() {
        assert_eq!(range_start("bytes 100-199/1000"), Some(100));
        assert_eq!(range_start(" bytes  0-9/10"), Some(0));
        assert_eq!(range_start("bytes */1000"), None);
        assert_eq!(range_start("items 1-2/3"), None);
    }
}
