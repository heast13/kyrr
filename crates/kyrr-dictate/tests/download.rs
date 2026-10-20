//! Network test of the model download with the English model's small
//! tokens file, from each of its sources. Run with `cargo test -- --ignored`.

use std::sync::atomic::AtomicBool;

use kyrr_dictate::download::{download, DownloadError};
use kyrr_dictate::{ModelFile, ModelKind, ModelSpec};

const TOKENS_URL: &str =
    "https://huggingface.co/heast13/kyrr-models/resolve/fcb7ff4db50300b95f9301fb5f8c62c56c4e69ea/en-unified/tokens.txt";

static GOOD: ModelSpec = ModelSpec {
    kind: ModelKind::Transducer,
    id: "good",
    attribution: "",
    files: &[ModelFile {
        name: "tokens.txt",
        urls: &[TOKENS_URL],
        size: 8_952,
        sha256: "dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff",
    }],
    replaces: &[],
};

static TAMPERED: ModelSpec = ModelSpec {
    kind: ModelKind::Transducer,
    id: "tampered",
    attribution: "",
    files: &[ModelFile {
        name: "tokens.txt",
        urls: &[TOKENS_URL],
        size: 8_952,
        sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    }],
    replaces: &[],
};

#[test]
#[ignore = "network"]
fn downloads_verifies_and_cancels() {
    let root = std::env::temp_dir().join(format!("kyrr-download-test-{}", std::process::id()));
    let never = AtomicBool::new(false);

    let mut last = 0;
    download(&GOOD, &root, &mut |p| last = p.done, &never).expect("download");
    assert_eq!(last, 8_952);
    assert!(GOOD.is_installed(&root));
    GOOD.verify(&root).expect("hash ok");

    let err = download(&TAMPERED, &root, &mut |_| {}, &never).unwrap_err();
    assert!(
        matches!(err, DownloadError::Corrupt(ref e) if e.contains("checksum")),
        "{err}"
    );
    assert!(!TAMPERED.is_installed(&root));
    assert!(
        !TAMPERED.dir(&root).join("tokens.part").exists(),
        "partial file removed"
    );

    let cancel = AtomicBool::new(true);
    let err = download(&TAMPERED, &root, &mut |_| {}, &cancel).unwrap_err();
    assert!(matches!(err, DownloadError::Cancelled));

    std::fs::remove_dir_all(&root).unwrap();
}

/// A source that is gone (here: a path that does not exist) passes on to
/// the next one.
static FALLBACK: ModelSpec = ModelSpec {
    kind: ModelKind::Transducer,
    id: "fallback",
    attribution: "",
    files: &[ModelFile {
        name: "tokens.txt",
        urls: &[
            "https://huggingface.co/heast13/kyrr-models/resolve/main/does-not-exist/tokens.txt",
            TOKENS_URL,
        ],
        size: 8_952,
        sha256: "dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff",
    }],
    replaces: &[],
};

#[test]
#[ignore = "network"]
fn falls_back_to_the_next_source() {
    let root = std::env::temp_dir().join(format!("kyrr-fallback-test-{}", std::process::id()));
    download(&FALLBACK, &root, &mut |_| {}, &AtomicBool::new(false)).expect("second source");
    FALLBACK.verify(&root).expect("hash ok");
    std::fs::remove_dir_all(&root).unwrap();
}

/// Every source of every model file is there, with the right size. Asks
/// for the size only; the files themselves are big.
#[test]
#[ignore = "network"]
fn every_source_answers() {
    let agent: ureq::Agent = ureq::config::Config::builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .new_agent();
    for speech in kyrr_dictate::Speech::ALL {
        for file in speech.model().files {
            for url in file.urls {
                let response = agent.head(*url).call().unwrap_or_else(|e| panic!("{url}: {e}"));
                let size: u64 = response
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| panic!("{url}: no size"));
                assert_eq!(size, file.size, "{url}");
            }
        }
    }
}
