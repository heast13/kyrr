//! "Check for updates" and "Update now": asks GitHub for the newest release,
//! only when the user clicks, and on request downloads its installer, checks
//! it against the release's SHA256 file and starts it.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/heast13/kyrr/releases/latest";

/// Only release pages of kyrr are opened, whatever the answer says.
const RELEASES: &str = "https://github.com/heast13/kyrr/releases/";

/// Only installers from kyrr's own releases are downloaded.
const DOWNLOADS: &str = "https://github.com/heast13/kyrr/releases/download/";

/// The installer of a release and the file with its SHA256.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installer {
    pub url: String,
    pub sha256_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    Available {
        version: String,
        /// The release page, the way out when the installer cannot be used.
        url: String,
        installer: Option<Installer>,
    },
    /// Offline, no release yet, or an answer that made no sense.
    Failed,
}

fn agent(timeout: Duration) -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::config::Config::builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(timeout))
        .build()
        .new_agent()
}

/// Asks GitHub; blocks for up to a quarter minute.
pub fn check() -> Outcome {
    let body = agent(Duration::from_secs(15))
        .get(LATEST)
        .header("User-Agent", concat!("kyrr/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut r| r.body_mut().with_config().limit(1 << 20).read_to_string());
    match body {
        Ok(body) => parse(&body, env!("CARGO_PKG_VERSION")),
        Err(_) => Outcome::Failed,
    }
}

/// Reads GitHub's answer about the latest release and compares it with
/// `current`.
fn parse(body: &str, current: &str) -> Outcome {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return Outcome::Failed;
    };
    let (Some(tag), Some(url)) = (json["tag_name"].as_str(), json["html_url"].as_str()) else {
        return Outcome::Failed;
    };
    let (Some(latest), Some(ours)) = (version(tag), version(current)) else {
        return Outcome::Failed;
    };
    if latest <= ours {
        return Outcome::UpToDate;
    }
    if !url.starts_with(RELEASES) {
        return Outcome::Failed;
    }
    let version = tag.trim_start_matches('v').to_string();
    let asset = |name: &str| {
        json["assets"].as_array()?.iter().find_map(|a| {
            let link = a["browser_download_url"].as_str()?;
            (a["name"].as_str() == Some(name) && link.starts_with(DOWNLOADS)).then(|| link.to_string())
        })
    };
    let exe = format!("kyrr-setup-{version}.exe");
    let installer = asset(&exe)
        .zip(asset(&format!("{exe}.sha256")))
        .map(|(url, sha256_url)| Installer { url, sha256_url });
    Outcome::Available {
        version,
        url: url.to_string(),
        installer,
    }
}

/// "v1.2.3" or "1.2.3" as numbers; anything else is not a version.
fn version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// The first word of a `.sha256` file, if it is a SHA256 in hex.
fn sha256_of(file: &str) -> Option<String> {
    let hash = file.split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

/// Where downloaded installers go; cleared at the next start.
pub fn folder() -> PathBuf {
    std::env::temp_dir().join("kyrr-update")
}

/// Downloads the installer into [`folder`] and checks it against its
/// SHA256 file. `progress` gets 0..1. Blocks; for a thread of its own.
pub fn download(installer: &Installer, progress: &mut dyn FnMut(f32)) -> Result<PathBuf, String> {
    let agent = agent(Duration::from_secs(30 * 60));
    let expected = agent
        .get(&installer.sha256_url)
        .call()
        .and_then(|mut r| r.body_mut().with_config().limit(4096).read_to_string())
        .map_err(|e| e.to_string())
        .and_then(|text| sha256_of(&text).ok_or_else(|| "no checksum in the release".to_string()))?;
    let name = installer.url.rsplit('/').next().unwrap_or("kyrr-setup.exe");
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(name);
    let response = agent.get(&installer.url).call().map_err(|e| e.to_string())?;
    let total = response.body().content_length().unwrap_or(0);
    let mut reader = response.into_body().into_reader();
    let mut out = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if total > 0 {
            progress((done as f64 / total as f64).min(1.0) as f32);
        }
    }
    drop(out);
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != expected {
        let _ = std::fs::remove_file(&dest);
        return Err("the download is damaged (checksum mismatch)".into());
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(tag: &str, url: &str) -> String {
        serde_json::json!({ "tag_name": tag, "html_url": url, "draft": false }).to_string()
    }

    #[test]
    fn newer_release_is_offered() {
        let url = "https://github.com/heast13/kyrr/releases/tag/v0.2.0";
        assert_eq!(
            parse(&answer("v0.2.0", url), "0.1.0"),
            Outcome::Available {
                version: "0.2.0".into(),
                url: url.into(),
                installer: None,
            }
        );
        assert!(matches!(
            parse(&answer("v0.10.0", url), "0.9.3"),
            Outcome::Available { .. }
        ));
    }

    #[test]
    fn installer_comes_from_the_release() {
        let dl = "https://github.com/heast13/kyrr/releases/download/v0.4.0/";
        let body = serde_json::json!({
            "tag_name": "v0.4.0",
            "html_url": "https://github.com/heast13/kyrr/releases/tag/v0.4.0",
            "assets": [
                { "name": "kyrr-setup.exe", "browser_download_url": format!("{dl}kyrr-setup.exe") },
                { "name": "kyrr-setup-0.4.0.exe", "browser_download_url": format!("{dl}kyrr-setup-0.4.0.exe") },
                { "name": "kyrr-setup-0.4.0.exe.sha256", "browser_download_url": format!("{dl}kyrr-setup-0.4.0.exe.sha256") },
            ]
        })
        .to_string();
        let Outcome::Available { installer, .. } = parse(&body, "0.3.0") else {
            panic!("not offered");
        };
        assert_eq!(
            installer,
            Some(Installer {
                url: format!("{dl}kyrr-setup-0.4.0.exe"),
                sha256_url: format!("{dl}kyrr-setup-0.4.0.exe.sha256"),
            })
        );

        // Assets from elsewhere are ignored.
        let foreign = body.replace(
            "https://github.com/heast13/kyrr/releases/download/",
            "https://example.com/",
        );
        assert!(matches!(
            parse(&foreign, "0.3.0"),
            Outcome::Available { installer: None, .. }
        ));
    }

    #[test]
    fn same_or_older_is_up_to_date() {
        let url = "https://github.com/heast13/kyrr/releases/tag/v0.1.0";
        assert_eq!(parse(&answer("v0.1.0", url), "0.1.0"), Outcome::UpToDate);
        assert_eq!(parse(&answer("0.0.9", url), "0.1.0"), Outcome::UpToDate);
    }

    #[test]
    fn odd_answers_fail() {
        let url = "https://github.com/heast13/kyrr/releases/tag/v9.0.0";
        assert_eq!(parse("", "0.1.0"), Outcome::Failed);
        assert_eq!(parse(r#"{"message":"Not Found"}"#, "0.1.0"), Outcome::Failed);
        assert_eq!(parse(&answer("nightly", url), "0.1.0"), Outcome::Failed);
        assert_eq!(parse(&answer("v9.0.0-rc1", url), "0.1.0"), Outcome::Failed);
        assert_eq!(
            parse(&answer("v9.0.0", "https://example.com/kyrr.exe"), "0.1.0"),
            Outcome::Failed
        );
    }

    #[test]
    fn checksum_files() {
        let hash = "dbfe1cf8630478e7944249d3638f79e89f3cdfac7bf06559486632fd3c428316";
        assert_eq!(sha256_of(&format!("{hash}  kyrr-setup-0.3.0.exe\n")), Some(hash.into()));
        assert_eq!(sha256_of(&hash.to_uppercase()), Some(hash.into()));
        assert_eq!(sha256_of("not a hash"), None);
        assert_eq!(sha256_of(""), None);
    }

    /// GitHub's real answer, as if this were kyrr 0.0.1.
    #[test]
    #[ignore = "network"]
    fn the_latest_release_offers_its_installer() {
        let body = agent(Duration::from_secs(15))
            .get(LATEST)
            .header("User-Agent", "kyrr-test")
            .call()
            .and_then(|mut r| r.body_mut().read_to_string())
            .expect("GitHub answers");
        let Outcome::Available { installer, .. } = parse(&body, "0.0.1") else {
            panic!("no release offered");
        };
        let installer = installer.expect("installer and checksum attached");
        assert!(installer.url.starts_with(DOWNLOADS) && installer.url.ends_with(".exe"));
    }

    /// The real 0.3.0 installer, as if this were an older kyrr.
    #[test]
    #[ignore = "network, 60 MB"]
    fn downloads_and_checks_a_real_installer() {
        let dl = "https://github.com/heast13/kyrr/releases/download/v0.3.0/";
        let installer = Installer {
            url: format!("{dl}kyrr-setup-0.3.0.exe"),
            sha256_url: format!("{dl}kyrr-setup-0.3.0.exe.sha256"),
        };
        let mut last = 0.0;
        let path = download(&installer, &mut |p| last = p).expect("download");
        assert!(path.is_file());
        assert!((last - 1.0).abs() < 1e-6);
        std::fs::remove_file(path).unwrap();
    }
}
