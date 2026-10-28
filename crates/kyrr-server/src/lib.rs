//! Talking to the user's own servers: a language model that reworks the
//! dictated text (Ollama, LM Studio, llama.cpp) and a speech server that
//! recognises it (speaches, whisper.cpp, LocalAI), both through the
//! OpenAI-style interface they share. Only servers on the own computer or in
//! the home network may be reached without encryption; nothing goes to a
//! cloud unless the user typed an https address.

use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

pub mod guard;
pub mod rework;

/// A server the user set up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    /// The address as [`address`] made it: scheme, host, port, no "/v1".
    pub url: String,
    /// Sent as a bearer token when not empty; never logged.
    pub key: String,
}

/// Why a server did not help, in words kyrr can say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Plain http to somewhere outside the own computer and home network.
    NotLocal,
    /// No answer at all: wrong address, server off, network gone.
    Offline,
    /// It took longer than allowed.
    Timeout,
    /// The server said no (wrong model, wrong key: 401, 404, …).
    Refused(u16),
    /// An answer kyrr could not read.
    Answer,
    /// The model thought aloud and gave no answer: its thinking is on.
    Thinks,
}

/// The address as typed, made usable: "192.168.1.5:1234/v1/" becomes
/// "http://192.168.1.5:1234". None for nothing typed.
pub fn address(typed: &str) -> Option<String> {
    let mut s = typed.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return None;
    }
    if !s.contains("://") {
        s = format!("http://{s}");
    }
    // A name and password in the address would end up in the settings:
    // keys go in their own field.
    let authority = s
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    if authority.contains('@') {
        return None;
    }
    // As a server log names it ("listening on 0.0.0.0:1234"): reachable
    // here as the own computer, which Windows does not take 0.0.0.0 for.
    for (any, own) in [("://0.0.0.0", "://127.0.0.1"), ("://[::]", "://[::1]")] {
        if s.contains(any) {
            s = s.replacen(any, own, 1);
        }
    }
    // Pasted with the path of one of the calls.
    for tail in ["/v1/chat/completions", "/v1/audio/transcriptions", "/v1/models", "/v1"] {
        if let Some(base) = s.strip_suffix(tail) {
            s = base.to_string();
            break;
        }
    }
    Some(s.trim_end_matches('/').to_string())
}

/// Host and port of an address, and whether it is encrypted.
fn host_port(url: &str) -> Option<(String, u16, bool)> {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else {
        (false, url.strip_prefix("http://")?)
    };
    let authority = rest.split('/').next()?;
    // Credentials in the address: not supported, and not to be sent around.
    if authority.contains('@') {
        return None;
    }
    let standard = if secure { 443 } else { 80 };
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => standard,
            None => return None,
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse().ok()?),
            None => (authority.to_string(), standard),
        }
    };
    (!host.is_empty()).then_some((host, port, secure))
}

/// The own computer, a home or company network, or a Tailscale network.
fn local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return local(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// Whether kyrr may talk to this address: https anywhere, http only where
/// the name leads to the own computer or the home network (the address it
/// resolves to counts, so "server.tailnet.ts.net" works).
pub fn allowed(url: &str) -> Result<(), Error> {
    let Some((host, port, secure)) = host_port(url) else {
        return Err(Error::Offline);
    };
    if secure {
        return Ok(());
    }
    let ips: Vec<IpAddr> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|_| Error::Offline)?
        .map(|a| a.ip())
        .collect();
    if !ips.is_empty() && ips.iter().all(|ip| local(*ip)) {
        Ok(())
    } else {
        Err(Error::NotLocal)
    }
}

/// Finds where a name leads like ureq itself, but plain http only ever
/// goes to the addresses of the own computer and home network among them:
/// a name that leads elsewhere by the time of the request (after
/// [`allowed`] looked) finds nothing.
#[derive(Debug)]
struct HomeOnly;

impl ureq::unversioned::resolver::Resolver for HomeOnly {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let all = ureq::unversioned::resolver::DefaultResolver::default().resolve(uri, config, timeout)?;
        if uri.scheme_str() == Some("https") {
            return Ok(all);
        }
        let mut home = self.empty();
        for addr in all.iter().filter(|a| local(a.ip())) {
            home.push(*addr);
        }
        if home.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(home)
    }
}

/// No proxy (a server in the home network is not behind it), no redirects
/// (they could lead out of it), and an end to waiting.
fn agent(timeout: Duration) -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    let config = ureq::config::Config::builder()
        .proxy(None)
        .max_redirects(0)
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .timeout_connect(Some(timeout.min(Duration::from_secs(5))))
        .timeout_global(Some(timeout))
        .user_agent(concat!("kyrr/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::default(),
        HomeOnly,
    )
}

fn failed(e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(code) => Error::Refused(code),
        ureq::Error::Timeout(_) => Error::Timeout,
        _ => Error::Offline,
    }
}

fn read(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<serde_json::Value, Error> {
    let body = response
        .and_then(|mut r| r.body_mut().with_config().limit(8 << 20).read_to_string())
        .map_err(failed)?;
    serde_json::from_str(&body).map_err(|_| Error::Answer)
}

fn with_key<B>(request: ureq::RequestBuilder<B>, server: &Server) -> ureq::RequestBuilder<B> {
    if server.key.is_empty() {
        request
    } else {
        request.header("Authorization", format!("Bearer {}", server.key))
    }
}

/// The models the server offers, by name.
pub fn models(server: &Server, timeout: Duration) -> Result<Vec<String>, Error> {
    allowed(&server.url)?;
    let request = with_key(agent(timeout).get(format!("{}/v1/models", server.url)), server);
    let json = read(request.call())?;
    let list = json["data"].as_array().ok_or(Error::Answer)?;
    Ok(list
        .iter()
        .filter_map(|m| m["id"].as_str())
        .map(str::to_string)
        .collect())
}

/// One chat request; the reply's text.
pub fn chat(server: &Server, body: &serde_json::Value, timeout: Duration) -> Result<String, Error> {
    allowed(&server.url)?;
    let request = with_key(
        agent(timeout).post(format!("{}/v1/chat/completions", server.url)),
        server,
    )
    .content_type("application/json");
    let json = read(request.send(body.to_string().as_bytes()))?;
    let message = &json["choices"][0]["message"];
    let content = message["content"].as_str().unwrap_or_default();
    // Thinking that a server hands back on its own (llama.cpp, vLLM,
    // LM Studio, Ollama each name it differently).
    let thought = ["reasoning_content", "reasoning", "thinking"]
        .iter()
        .any(|k| message[k].as_str().is_some_and(|t| !t.trim().is_empty()));
    if content.trim().is_empty() && thought {
        return Err(Error::Thinks);
    }
    // No answer at all, or one in a form kyrr does not read (parts).
    if !message["content"].is_string() && !thought {
        return Err(Error::Answer);
    }
    Ok(content.to_string())
}

/// A mono recording at 16 kHz as a WAV file, 16 bits.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    const RATE: u32 = 16_000;
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Sends a recording (a WAV file) to a speech server; what it heard. The
/// model may be empty (whisper.cpp ignores it, but wants the field), the
/// language a code or None to let the server tell.
pub fn transcribe(
    server: &Server,
    model: &str,
    wav: &[u8],
    language: Option<&str>,
    timeout: Duration,
) -> Result<String, Error> {
    allowed(&server.url)?;
    let boundary = format!(
        "kyrr{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let mut body = Vec::with_capacity(wav.len() + 1024);
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes(),
        );
    };
    field("model", if model.is_empty() { "whisper-1" } else { model });
    field("response_format", "json");
    if let Some(language) = language {
        field("language", language);
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"dictation.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let request = with_key(
        agent(timeout).post(format!("{}/v1/audio/transcriptions", server.url)),
        server,
    )
    .content_type(format!("multipart/form-data; boundary={boundary}"));
    let json = read(request.send(&body[..]))?;
    json["text"].as_str().map(|t| t.trim().to_string()).ok_or(Error::Answer)
}

/// Servers running on this computer at their usual ports: Ollama, LM
/// Studio, llama.cpp, speaches. Takes a moment at most.
pub fn find_local() -> Vec<(&'static str, String)> {
    const KNOWN: [(&str, u16); 4] = [
        ("Ollama", 11434),
        ("LM Studio", 1234),
        ("llama.cpp", 8080),
        ("speaches", 8000),
    ];
    let threads: Vec<_> = KNOWN
        .into_iter()
        .map(|(name, port)| {
            std::thread::spawn(move || {
                let server = Server {
                    url: format!("http://127.0.0.1:{port}"),
                    key: String::new(),
                };
                models(&server, Duration::from_millis(800))
                    .ok()
                    .map(|_| (name, server.url))
            })
        })
        .collect();
    threads.into_iter().filter_map(|t| t.join().ok().flatten()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_as_typed() {
        assert_eq!(
            address(" 192.168.1.5:1234/v1/ ").as_deref(),
            Some("http://192.168.1.5:1234")
        );
        assert_eq!(
            address("http://localhost:11434/v1/chat/completions").as_deref(),
            Some("http://localhost:11434")
        );
        assert_eq!(
            address("https://ai.example.org").as_deref(),
            Some("https://ai.example.org")
        );
        assert_eq!(address("0.0.0.0:1234").as_deref(), Some("http://127.0.0.1:1234"));
        assert_eq!(address("http://[::]:8080").as_deref(), Some("http://[::1]:8080"));
        assert_eq!(address("  "), None);
        assert_eq!(address("http://me:secret@192.168.1.5:8080"), None);
    }

    #[test]
    fn plain_http_stays_at_home() {
        assert_eq!(allowed("http://127.0.0.1:8080"), Ok(()));
        assert_eq!(allowed("http://192.168.1.10:1234"), Ok(()));
        assert_eq!(allowed("http://10.0.0.2"), Ok(()));
        assert_eq!(allowed("http://100.101.102.103:8080"), Ok(()));
        assert_eq!(allowed("http://[::1]:11434"), Ok(()));
        assert_eq!(allowed("http://[fd7a:115c:a1e0::1]:8080"), Ok(()));
        assert_eq!(allowed("http://8.8.8.8"), Err(Error::NotLocal));
        assert_eq!(allowed("http://100.128.0.1"), Err(Error::NotLocal));
        assert_eq!(allowed("http://[2001:db8::1]:80"), Err(Error::NotLocal));
        assert_eq!(allowed("https://8.8.8.8"), Ok(()));
        assert_eq!(allowed("http://user:pw@127.0.0.1"), Err(Error::Offline));
        assert_eq!(allowed("ftp://127.0.0.1"), Err(Error::Offline));
    }

    #[test]
    fn a_wav_file_has_its_header() {
        let w = wav(&[0.0, 0.5, -1.0]);
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[44..], &[0, 0, 0xff, 0x3f, 0x01, 0x80]);
    }
}
