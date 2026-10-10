//! "Check for updates": asks GitHub for the newest release, only when the
//! user clicks. Nothing is downloaded or installed; the app shows the
//! version and opens the release page.

use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/heast13/kyrr/releases/latest";

/// Only release pages of kyrr are opened, whatever the answer says.
const RELEASES: &str = "https://github.com/heast13/kyrr/releases/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    Available {
        version: String,
        url: String,
    },
    /// Offline, no release yet, or an answer that made no sense.
    Failed,
}

/// Asks GitHub; blocks for up to a quarter minute.
pub fn check() -> Outcome {
    use ureq::tls::{TlsConfig, TlsProvider};
    let agent = ureq::config::Config::builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .new_agent();
    let body = agent
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
    Outcome::Available {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
    }
}

/// "v1.2.3" or "1.2.3" as numbers; anything else is not a version.
fn version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
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
                url: url.into()
            }
        );
        assert!(matches!(
            parse(&answer("v0.10.0", url), "0.9.3"),
            Outcome::Available { .. }
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
}
