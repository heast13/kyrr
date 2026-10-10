//! The dictation models, one per language, and where they live on disk.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Dictation language. Each language has its own model; the German one also
/// handles English words and sentences well, the English one knows no German.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    German,
    English,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::German, Language::English];

    /// ISO 639-1 code, also used in settings.
    pub fn code(self) -> &'static str {
        match self {
            Language::German => "de",
            Language::English => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Language> {
        let primary = code.split(['-', '_']).next()?.to_ascii_lowercase();
        Language::ALL.into_iter().find(|l| l.code() == primary)
    }

    pub fn model(self) -> &'static ModelSpec {
        match self {
            Language::German => &GERMAN,
            Language::English => &ENGLISH,
        }
    }
}

/// One downloadable file of a model, pinned by size and SHA256.
#[derive(Debug)]
pub struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// A sherpa-onnx transducer model.
#[derive(Debug)]
pub struct ModelSpec {
    /// Folder name below the models root.
    pub id: &'static str,
    /// Shown in the app's about page, as the model licenses require.
    pub attribution: &'static str,
    pub files: &'static [ModelFile],
}

/// primeline/parakeet-primeline, converted with `tools/export-primeline`.
static GERMAN: ModelSpec = ModelSpec {
    id: "de-primeline",
    attribution: "German: primeline/parakeet-primeline by primeLine AI (CC-BY-4.0), \
                  based on NVIDIA parakeet-tdt-0.6b-v3; converted to ONNX and quantized to int8.",
    files: &[
        ModelFile {
            name: "encoder.int8.onnx",
            url: "https://huggingface.co/heast13/kyrr-models/resolve/main/de-primeline/encoder.int8.onnx",
            size: 881_596_736,
            sha256: "e2d1a42b442e7499ecdc9647a358e75540db9691503d430c7a6bc4263b6e0321",
        },
        ModelFile {
            name: "decoder.int8.onnx",
            url: "https://huggingface.co/heast13/kyrr-models/resolve/main/de-primeline/decoder.int8.onnx",
            size: 11_845_274,
            sha256: "6b32c9339900086afd3d282eb40688ae19f3d106281e7731ee27ead1add8b29d",
        },
        ModelFile {
            name: "joiner.int8.onnx",
            url: "https://huggingface.co/heast13/kyrr-models/resolve/main/de-primeline/joiner.int8.onnx",
            size: 6_355_277,
            sha256: "ff5ec87a02959bcc792f7012931665de36cb2f03963e0d8def0d6193b5c6159f",
        },
        ModelFile {
            name: "tokens.txt",
            url: "https://huggingface.co/heast13/kyrr-models/resolve/main/de-primeline/tokens.txt",
            size: 93_939,
            sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
        },
    ],
};

/// nvidia/parakeet-unified-en-0.6b as packaged by sherpa-onnx (int8,
/// non-streaming), from the sherpa-onnx author's Hugging Face mirror.
static ENGLISH: ModelSpec = ModelSpec {
    id: "en-unified",
    attribution: "English: NVIDIA parakeet-unified-en-0.6b, licensed by NVIDIA Corporation under the \
                  NVIDIA Open Model License; ONNX int8 conversion by the sherpa-onnx project.",
    files: &[
        ModelFile {
            name: "encoder.int8.onnx",
            url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/8c3a10fb13408c7a7054f6898958bf1c64a8d6c7/encoder.int8.onnx",
            size: 654_040_552,
            sha256: "6716910b7a0833997fec7a410494c995d70124001a0e9b66d6370d6aced577e0",
        },
        ModelFile {
            name: "decoder.int8.onnx",
            url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/8c3a10fb13408c7a7054f6898958bf1c64a8d6c7/decoder.int8.onnx",
            size: 7_257_753,
            sha256: "a5e223392c90e75f8144cdb5eb95af7625db389e39edef2bd1a9c872b3298fe6",
        },
        ModelFile {
            name: "joiner.int8.onnx",
            url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/8c3a10fb13408c7a7054f6898958bf1c64a8d6c7/joiner.int8.onnx",
            size: 1_735_860,
            sha256: "869f43f7d24595c55581ad3bf249a935fb8a71389fbdaa7504b9f46f93140f8a",
        },
        ModelFile {
            name: "tokens.txt",
            url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/8c3a10fb13408c7a7054f6898958bf1c64a8d6c7/tokens.txt",
            size: 8_952,
            sha256: "dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff",
        },
    ],
};

impl ModelSpec {
    pub fn dir(&self, root: &Path) -> PathBuf {
        root.join(self.id)
    }

    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Quick check: every file present with the expected size. Downloads are
    /// hash-checked before they are moved into place, so this is enough at
    /// startup; [`ModelSpec::verify`] does the full check.
    pub fn is_installed(&self, root: &Path) -> bool {
        let dir = self.dir(root);
        self.files
            .iter()
            .all(|f| std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.len() == f.size))
    }

    /// Full check of every file's SHA256.
    pub fn verify(&self, root: &Path) -> Result<(), String> {
        let dir = self.dir(root);
        for f in self.files {
            let path = dir.join(f.name);
            let digest = sha256_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if digest != f.sha256 {
                return Err(format!("{}: checksum mismatch", path.display()));
            }
        }
        Ok(())
    }
}

pub(crate) fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
