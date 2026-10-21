//! "Check for updates" and "Update now": asks GitHub for the newest release
//! (once a day unless switched off, or when the user clicks), and on request
//! downloads its installer, checks it against the release's SHA256 file and
//! starts it.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use kyrr_dictate::Language;
use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/heast13/kyrr/releases/latest";

/// The address for feedback. Someone without a GitHub account can
/// write there too.
const FEEDBACK_ADDRESS: &str = "stueckig.habicht_2o@icloud.com";

/// A ready e-mail for "Feedback": the subject, a place to write and
/// the version and system below. Nothing is sent by kyrr; the mail program
/// opens and the user decides.
pub fn feedback_mailto(language: Language, version: &str, system: &str) -> String {
    let (subject, ask, language_name) = match language {
        Language::German => ("kyrr: Feedback", "Dein Feedback:", "Deutsch"),
        Language::English => ("kyrr: Feedback", "Your feedback:", "English"),
    };
    let body = format!("{ask}\n\n\n\n---\nkyrr {version}\n{system}\n{language_name}");
    format!(
        "mailto:{FEEDBACK_ADDRESS}?subject={}&body={}",
        percent_encode(subject),
        percent_encode(&body)
    )
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Only answers that point at kyrr's own release pages are trusted.
const RELEASES: &str = "https://github.com/heast13/kyrr/releases/";

/// Only installers from kyrr's own releases are downloaded.
const DOWNLOADS: &str = "https://github.com/heast13/kyrr/releases/download/";

/// The download page of the website, the way out when kyrr cannot install the
/// update itself. The German page for German, the English one otherwise.
pub fn download_page(language: Language) -> &'static str {
    match language {
        Language::German => "https://heast13.github.io/kyrr/de/#download",
        Language::English => "https://heast13.github.io/kyrr/#download",
    }
}

/// The installer of a release and the file with its SHA256.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installer {
    pub url: String,
    pub sha256_url: String,
}

/// What is new in a release, in plain words for each language, as the
/// release writes it into its description (see `scripts/release-notes.py`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct News {
    pub en: String,
    pub de: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    Available {
        version: String,
        installer: Option<Installer>,
        /// None for releases without a summary.
        news: Option<News>,
    },
    /// Offline, no release yet, or an answer that made no sense.
    Failed,
}

/// HTTPS only, redirects included: GitHub sends downloads on to another
/// host, and a redirect to plain HTTP must not be followed.
fn agent(timeout: Duration) -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::config::Config::builder()
        .https_only(true)
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
        Ok(body) => parse(&body, env!("CARGO_PKG_VERSION"), cfg!(target_os = "linux")),
        Err(_) => Outcome::Failed,
    }
}

/// Reads GitHub's answer about the latest release and compares it with
/// `current`. On `linux` only a release with an AppImage counts: one that
/// brings a new Windows version only has nothing to update here.
fn parse(body: &str, current: &str, linux: bool) -> Outcome {
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
    if linux && asset(&format!("kyrr-{version}-x86_64.AppImage")).is_none() {
        return Outcome::UpToDate;
    }
    let exe = format!("kyrr-setup-{version}.exe");
    let installer = asset(&exe)
        .zip(asset(&format!("{exe}.sha256")))
        .map(|(url, sha256_url)| Installer { url, sha256_url });
    Outcome::Available {
        version,
        installer,
        news: json["body"].as_str().and_then(|body| news(body, SYSTEM)),
    }
}

/// The most the summary shows, per language.
const NEWS_LIMIT: usize = 2000;

/// The system this build runs on, as NEWS.md marks items for one system.
const SYSTEM: &str = if cfg!(windows) { "Windows" } else { "Linux" };

/// The first version for Linux: the ones before it are Windows' history.
const FIRST_LINUX: (u32, u32, u32) = (0, 5, 3);

/// The name of a system's summary in a release description: Windows keeps
/// the one older versions read.
fn news_tag(system: &str) -> &'static str {
    if system == "Linux" {
        "kyrr-linux-news"
    } else {
        "kyrr-news"
    }
}

/// `block` without the items NEWS.md marks for the other system: a bullet
/// that starts with "(Windows)" or "(Linux)" is only for that system, and
/// the mark goes where the item stays. Lines that continue a bullet go with it.
fn for_system(block: &str, system: &str) -> String {
    let mut out = String::new();
    let mut keep = true;
    for line in block.lines() {
        let mut line = line;
        if let Some(item) = line.strip_prefix("- ") {
            let mark = ["Windows", "Linux"]
                .into_iter()
                .find(|m| item.starts_with(&format!("({m}) ")));
            keep = mark.is_none_or(|m| m == system);
            if let Some(m) = mark {
                out.push_str(if keep { "- " } else { "" });
                line = &item[m.len() + 3..];
            }
        } else if line.trim().is_empty() {
            keep = true;
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The summary hidden in a release description as `<!-- kyrr-news:en … -->`
/// and `<!-- kyrr-news:de … -->` (`kyrr-linux-news` for Linux, see
/// `scripts/release-notes.py`). A language that is missing falls back to
/// the other one; none at all is no summary.
fn news(body: &str, system: &str) -> Option<News> {
    let (en, de) = (news_block(body, system, "en"), news_block(body, system, "de"));
    let (en, de) = match (en, de) {
        (None, None) => return None,
        (Some(en), None) => (en.clone(), en),
        (None, Some(de)) => (de.clone(), de),
        (Some(en), Some(de)) => (en, de),
    };
    Some(News { en, de })
}

/// One language's block as plain text: bullets become "•", wrapped lines
/// are joined, blank lines separate paragraphs.
fn news_block(body: &str, system: &str, lang: &str) -> Option<String> {
    let start = format!("<!-- {}:{lang}", news_tag(system));
    let from = body.find(&start)? + start.len();
    plain(&body[from..from + body[from..].find("-->")?], NEWS_LIMIT)
}

/// A block of bullets and paragraphs as plain text: bullets become "•",
/// wrapped lines are joined, blank lines separate paragraphs.
fn plain(block: &str, limit: usize) -> Option<String> {
    let mut out = String::new();
    for line in block.lines() {
        let text = line.trim();
        if text.is_empty() {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push_str("\n\n");
            }
        } else if let Some(item) = line.strip_prefix("- ") {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str("• ");
            out.push_str(item.trim());
        } else {
            if !out.is_empty() && !out.ends_with("\n\n") && !out.ends_with('\n') {
                out.push(' ');
            }
            out.push_str(text);
        }
    }
    let out: String = out.trim().chars().take(limit).collect();
    (!out.is_empty()).then_some(out)
}

/// NEWS.md, the file the release description's summary is made from, built
/// into the app: what a version brought can be read again after the update,
/// however it was installed.
const NEWS_MD: &str = include_str!("../../../NEWS.md");

/// The summary of `version` from the built-in NEWS.md.
pub fn bundled_news(version: &str) -> Option<News> {
    section(NEWS_MD, version, SYSTEM, NEWS_LIMIT)
}

/// Every version's summary from the built-in NEWS.md, the newest first (the
/// order of the file), each under a line "kyrr <version>". On Linux the list
/// starts with the first version for Linux.
pub fn all_news() -> Option<News> {
    all_news_of(NEWS_MD, SYSTEM)
}

fn all_news_of(markdown: &str, system: &str) -> Option<News> {
    let (mut en, mut de) = (String::new(), String::new());
    for v in markdown.lines().filter_map(|l| l.strip_prefix("## ")).map(str::trim) {
        // "Unreleased" collects what the next version will say; not shown yet.
        if version(v).is_none() || (system == "Linux" && version(v).is_some_and(|v| v < FIRST_LINUX)) {
            continue;
        }
        let Some(news) = section(markdown, v, system, usize::MAX) else {
            continue;
        };
        for (text, part) in [(&mut en, news.en), (&mut de, news.de)] {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&format!("kyrr {v}\n{part}"));
        }
    }
    (!en.is_empty()).then_some(News { en, de })
}

/// The `## <version>` section of a NEWS.md for `system`: English first, then
/// the part after the `### Deutsch` line.
fn section(markdown: &str, version: &str, system: &str, limit: usize) -> Option<News> {
    let mut lines = markdown.lines().skip_while(|l| l.trim_end() != format!("## {version}"));
    lines.next()?;
    let (mut en, mut de, mut german) = (String::new(), String::new(), false);
    for line in lines {
        if line.starts_with("## ") {
            break;
        }
        if line.trim_end() == "### Deutsch" {
            german = true;
            continue;
        }
        let target = if german { &mut de } else { &mut en };
        target.push_str(line);
        target.push('\n');
    }
    let (en, de) = (
        plain(&for_system(&en, system), limit),
        plain(&for_system(&de, system), limit),
    );
    match (en, de) {
        (None, None) => None,
        (Some(en), None) => Some(News { de: en.clone(), en }),
        (None, Some(de)) => Some(News { en: de.clone(), de }),
        (Some(en), Some(de)) => Some(News { en, de }),
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
    // Written next to it and moved into place once checked, so a broken
    // download never leaves a file that looks like an installer.
    let part = dir.join(format!("{name}.part"));
    let result = fetch_checked(&agent, &installer.url, &part, &expected, progress)
        .and_then(|()| std::fs::rename(&part, &dest).map_err(|e| e.to_string()));
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.map(|()| dest)
}

/// Downloads `url` into `dest`; an error if its SHA256 is not `expected`.
fn fetch_checked(
    agent: &ureq::Agent,
    url: &str,
    dest: &std::path::Path,
    expected: &str,
    progress: &mut dyn FnMut(f32),
) -> Result<(), String> {
    let response = agent.get(url).call().map_err(|e| e.to_string())?;
    let total = response.body().content_length().unwrap_or(0);
    let mut reader = response.into_body().into_reader();
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
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
    out.sync_all().map_err(|e| e.to_string())?;
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != expected {
        return Err("the download is damaged (checksum mismatch)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_mail_is_ready_to_send() {
        let m = feedback_mailto(Language::German, "0.6.0", "Windows");
        assert!(m.starts_with("mailto:stueckig.habicht_2o@icloud.com?subject=kyrr%3A%20Feedback&body="));
        assert!(m.contains("kyrr%200.6.0%0AWindows%0ADeutsch"));
        assert!(!m.contains(' ') && !m.contains('\n'));
        assert!(feedback_mailto(Language::English, "0.6.0", "Ubuntu 26.10, GNOME").contains("Your%20feedback%3A"));
    }

    fn answer(tag: &str, url: &str) -> String {
        serde_json::json!({ "tag_name": tag, "html_url": url, "draft": false }).to_string()
    }

    #[test]
    fn newer_release_is_offered() {
        let url = "https://github.com/heast13/kyrr/releases/tag/v0.2.0";
        assert_eq!(
            parse(&answer("v0.2.0", url), "0.1.0", false),
            Outcome::Available {
                version: "0.2.0".into(),
                installer: None,
                news: None,
            }
        );
        assert!(matches!(
            parse(&answer("v0.10.0", url), "0.9.3", false),
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
        let Outcome::Available { installer, .. } = parse(&body, "0.3.0", false) else {
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
            parse(&foreign, "0.3.0", false),
            Outcome::Available { installer: None, .. }
        ));
    }

    #[test]
    fn linux_only_sees_releases_for_linux() {
        let dl = "https://github.com/heast13/kyrr/releases/download/v0.5.1/";
        let windows = serde_json::json!({
            "tag_name": "v0.5.1",
            "html_url": "https://github.com/heast13/kyrr/releases/tag/v0.5.1",
            "assets": [
                { "name": "kyrr-setup-0.5.1.exe", "browser_download_url": format!("{dl}kyrr-setup-0.5.1.exe") },
            ]
        });
        assert_eq!(parse(&windows.to_string(), "0.5.0", true), Outcome::UpToDate);
        assert!(matches!(
            parse(&windows.to_string(), "0.5.0", false),
            Outcome::Available { .. }
        ));
        let mut both = windows;
        both["assets"].as_array_mut().unwrap().push(serde_json::json!({
            "name": "kyrr-0.5.1-x86_64.AppImage",
            "browser_download_url": format!("{dl}kyrr-0.5.1-x86_64.AppImage"),
        }));
        assert!(matches!(
            parse(&both.to_string(), "0.5.0", true),
            Outcome::Available { .. }
        ));
    }

    #[test]
    fn news_comes_from_the_release_description() {
        let body = "- tech notes\n\nWindows 10.\n\n<!-- kyrr-news:en\n- A new button.\n  It is red.\n- Fewer bugs.\n\nSecond paragraph.\n-->\n<!-- kyrr-news:de\n- Ein neuer Knopf.\n-->\n";
        let n = news(body, "Windows").expect("news");
        assert_eq!(n.en, "• A new button. It is red.\n• Fewer bugs.\n\nSecond paragraph.");
        assert_eq!(n.de, "• Ein neuer Knopf.");
        // One language only: the other one falls back to it.
        let only = news("<!-- kyrr-news:de\n- Nur Deutsch.\n-->", "Windows").expect("news");
        assert_eq!(only.en, only.de);
        // No block, an empty one or one never closed: no summary.
        assert_eq!(news("- just notes", "Windows"), None);
        assert_eq!(news("<!-- kyrr-news:en\n\n-->", "Windows"), None);
        assert_eq!(news("<!-- kyrr-news:en\n- open end", "Windows"), None);
        // Long text is cut.
        let long = format!("<!-- kyrr-news:en\n- {}\n-->", "x".repeat(5000));
        assert_eq!(news(&long, "Windows").expect("news").en.chars().count(), NEWS_LIMIT);
        // Linux reads its own blocks, Windows (and older versions) the plain ones.
        let both = "<!-- kyrr-news:en\n- Windows.\n-->\n<!-- kyrr-linux-news:en\n- Linux.\n-->\n";
        assert_eq!(news(both, "Windows").expect("news").en, "• Windows.");
        assert_eq!(news(both, "Linux").expect("news").en, "• Linux.");
        assert_eq!(news(body, "Linux"), None);
    }

    #[test]
    fn same_or_older_is_up_to_date() {
        let url = "https://github.com/heast13/kyrr/releases/tag/v0.1.0";
        assert_eq!(parse(&answer("v0.1.0", url), "0.1.0", false), Outcome::UpToDate);
        assert_eq!(parse(&answer("0.0.9", url), "0.1.0", false), Outcome::UpToDate);
    }

    #[test]
    fn odd_answers_fail() {
        let url = "https://github.com/heast13/kyrr/releases/tag/v9.0.0";
        assert_eq!(parse("", "0.1.0", false), Outcome::Failed);
        assert_eq!(parse(r#"{"message":"Not Found"}"#, "0.1.0", false), Outcome::Failed);
        assert_eq!(parse(&answer("nightly", url), "0.1.0", false), Outcome::Failed);
        assert_eq!(parse(&answer("v9.0.0-rc1", url), "0.1.0", false), Outcome::Failed);
        assert_eq!(
            parse(&answer("v9.0.0", "https://example.com/kyrr.exe"), "0.1.0", false),
            Outcome::Failed
        );
    }

    /// Refused before any connection, so no network is needed.
    #[test]
    fn plain_http_is_refused() {
        let result = agent(Duration::from_secs(5))
            .get("http://github.com/heast13/kyrr")
            .call();
        assert!(matches!(result, Err(ureq::Error::RequireHttpsOnly(_))), "{result:?}");
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
        let Outcome::Available { installer, .. } = parse(&body, "0.0.1", false) else {
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

    #[test]
    fn news_of_a_version_comes_from_news_md() {
        let md = "# What's new

intro

## 0.6.0

- New thing.
- Another one.

### Deutsch

- Neues.

## 0.5.0

- Old.
";
        let n = section(md, "0.6.0", "Windows", NEWS_LIMIT).expect("news");
        assert_eq!(
            n.en,
            "• New thing.
• Another one."
        );
        assert_eq!(n.de, "• Neues.");
        assert_eq!(section(md, "0.5.0", "Windows", NEWS_LIMIT).expect("news").de, "• Old.");
        assert_eq!(section(md, "9.9.9", "Windows", NEWS_LIMIT), None);
    }

    /// Items marked for one system show only there, without the mark; a
    /// version with nothing left for a system is left out of its list.
    #[test]
    fn news_items_can_be_for_one_system() {
        let md = "## 0.7.0

- (Linux) Only Linux,
  over two lines.
- (Windows) Only Windows.
- Both.

### Deutsch

- (Linux) Nur Linux.
- Beide.

## 0.6.0

- (Windows) Only Windows again.

## 0.5.0

- Before Linux.
";
        let linux = section(md, "0.7.0", "Linux", NEWS_LIMIT).expect("news");
        assert_eq!(linux.en, "• Only Linux, over two lines.\n• Both.");
        assert_eq!(linux.de, "• Nur Linux.\n• Beide.");
        let windows = section(md, "0.7.0", "Windows", NEWS_LIMIT).expect("news");
        assert_eq!(windows.en, "• Only Windows.\n• Both.");
        assert_eq!(windows.de, "• Beide.");
        assert_eq!(section(md, "0.6.0", "Linux", NEWS_LIMIT), None);
        let all = all_news_of(md, "Linux").expect("news");
        assert_eq!(all.en, "kyrr 0.7.0\n• Only Linux, over two lines.\n• Both.");
        let all = all_news_of(md, "Windows").expect("news");
        assert!(all
            .en
            .contains("kyrr 0.6.0\n• Only Windows again.\n\nkyrr 0.5.0\n• Before Linux."));
    }

    /// What Linux shows from NEWS.md never talks about Windows: such items
    /// need the "(Windows)" mark.
    #[test]
    fn linux_news_is_about_linux() {
        let n = all_news_of(NEWS_MD, "Linux").expect("news");
        for text in [&n.en, &n.de] {
            assert!(!text.contains("Windows"), "{text}");
        }
    }

    /// A release without its NEWS.md section would show an empty window.
    #[test]
    fn this_version_has_news() {
        let n = bundled_news(env!("CARGO_PKG_VERSION")).expect("NEWS.md needs a section for this version");
        assert!(!n.en.is_empty() && !n.de.is_empty());
    }

    #[test]
    fn all_news_lists_every_version_newest_first() {
        let n = all_news().expect("news");
        let this = format!("kyrr {}", env!("CARGO_PKG_VERSION"));
        assert!(n.en.starts_with(&this), "{}", &n.en[..40.min(n.en.len())]);
        let n = all_news_of(NEWS_MD, "Windows").expect("news");
        assert!(n.en.contains("kyrr 0.2.0") && n.de.contains("kyrr 0.2.0"));
        assert!(n.de.find("kyrr 0.5.0") < n.de.find("kyrr 0.4.0"));
        let n = all_news_of(NEWS_MD, "Linux").expect("news");
        assert!(n.en.starts_with(&this) && !n.en.contains("kyrr 0.5.2"));
        let n = all_news_of("## Unreleased\n\n- soon\n\n## 0.1.0\n\n- first\n", "Windows").expect("news");
        assert!(n.en.starts_with("kyrr 0.1.0") && !n.en.contains("Unreleased"));
    }
}
