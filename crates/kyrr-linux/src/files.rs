//! Opening files and folders for the user (the voice memos). The desktop's own
//! way first (`xdg-open`, the file manager's D-Bus call), then the desktop
//! portal, as not every desktop has a portal that opens files.

use std::fs::File;
use std::path::Path;

use ashpd::desktop::open_uri::{OpenDirectoryRequest, OpenFileRequest};
use futures_lite::future::block_on;

/// Opens `path` (a file or a folder) with the app the desktop has for it.
/// Blocks for a moment: call it beside the window.
pub fn open(path: &Path) -> bool {
    if xdg_open(path) {
        return true;
    }
    let portal = File::open(path).is_ok_and(|file| block_on(OpenFileRequest::default().send_file(&file)).is_ok());
    portal || xdg_open(path)
}

/// Opens the folder that holds `path` in the file manager, with `path`
/// picked out where the file manager can. Blocks for a moment.
pub fn show(path: &Path) -> bool {
    let portal = || File::open(path).is_ok_and(|file| block_on(OpenDirectoryRequest::default().send(&file)).is_ok());
    let manager = || block_on(show_items(path)).is_ok();
    let ways: [&dyn Fn() -> bool; 2] = [&manager, &portal];
    let shown = ways.iter().any(|way| way());
    shown || path.parent().is_some_and(xdg_open)
}

/// The file managers' own call (Nautilus, Dolphin, Nemo, Thunar and others).
async fn show_items(path: &Path) -> zbus::Result<()> {
    let uri = format!("file://{}", encode(&path.to_string_lossy()));
    let connection = zbus::Connection::session().await?;
    connection
        .call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![uri], ""),
        )
        .await
        .map(|_| ())
}

/// Waits for `xdg-open` (it hands the file on and ends): no process is left
/// behind, and one that found no app for the file says so, so the portal
/// can try.
fn xdg_open(path: &Path) -> bool {
    std::process::Command::new("xdg-open")
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

/// A path as it goes into a `file://` URI: everything but plain characters
/// and slashes percent-encoded, byte by byte.
fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_take_spaces_and_umlauts() {
        assert_eq!(
            encode("/home/flo/Dokumente/kyrr/2026-10-21 18-05 Größe.opus"),
            "/home/flo/Dokumente/kyrr/2026-10-21%2018-05%20Gr%C3%B6%C3%9Fe.opus"
        );
    }
}
