//! Network test of the model download with the English model's small
//! tokens file. Run with `cargo test -- --ignored`.

use std::sync::atomic::AtomicBool;

use kyrr_dictate::download::{download, DownloadError};
use kyrr_dictate::{ModelFile, ModelSpec};

const TOKENS_URL: &str = "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/8c3a10fb13408c7a7054f6898958bf1c64a8d6c7/tokens.txt";

static GOOD: ModelSpec = ModelSpec {
    id: "good",
    attribution: "",
    files: &[ModelFile {
        name: "tokens.txt",
        url: TOKENS_URL,
        size: 8_952,
        sha256: "dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff",
    }],
    replaces: &[],
};

static TAMPERED: ModelSpec = ModelSpec {
    id: "tampered",
    attribution: "",
    files: &[ModelFile {
        name: "tokens.txt",
        url: TOKENS_URL,
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
