//! The dictation models and where they live on disk.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Language of the window and of what kyrr says. It also picks the model
/// for German and English dictation, see [`Speech`].
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

    /// What is dictated when the user chose nothing else.
    pub fn speech(self) -> Speech {
        match self {
            Language::German => Speech::German,
            Language::English => Speech::English,
        }
    }
}

/// The language that is dictated. German and English each have a model of
/// their own (the German one also handles English words and sentences well,
/// the English one knows no German); Chinese, Japanese and Korean share one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speech {
    German,
    English,
    Chinese,
    Japanese,
    Korean,
}

impl Speech {
    pub const ALL: [Speech; 5] = [
        Speech::German,
        Speech::English,
        Speech::Chinese,
        Speech::Japanese,
        Speech::Korean,
    ];

    /// ISO 639-1 code, also used in settings and as the hint for the model.
    pub fn code(self) -> &'static str {
        match self {
            Speech::German => "de",
            Speech::English => "en",
            Speech::Chinese => "zh",
            Speech::Japanese => "ja",
            Speech::Korean => "ko",
        }
    }

    pub fn from_code(code: &str) -> Option<Speech> {
        let primary = code.split(['-', '_']).next()?.to_ascii_lowercase();
        Speech::ALL.into_iter().find(|s| s.code() == primary)
    }

    pub fn model(self) -> &'static ModelSpec {
        match self {
            Speech::German => &GERMAN,
            Speech::English => &ENGLISH,
            Speech::Chinese | Speech::Japanese | Speech::Korean => &SENSE_VOICE,
        }
    }

    /// Chinese and Japanese are written without spaces between words, and a
    /// sentence is followed by the next without one.
    pub fn spaceless(self) -> bool {
        matches!(self, Speech::Chinese | Speech::Japanese)
    }
}

/// One downloadable file of a model, pinned by size and SHA256.
#[derive(Debug)]
pub struct ModelFile {
    pub name: &'static str,
    /// Where to get it, tried in order: kyrr's own mirrors first, so a
    /// download never depends on one host alone.
    pub urls: &'static [&'static str],
    pub size: u64,
    pub sha256: &'static str,
}

/// How sherpa-onnx runs a model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelKind {
    /// Parakeet: encoder, decoder, joiner and tokens.
    Transducer,
    /// SenseVoice: one model file and tokens.
    SenseVoice,
}

/// A sherpa-onnx speech model.
#[derive(Debug)]
pub struct ModelSpec {
    pub kind: ModelKind,
    /// Folder name below the models root.
    pub id: &'static str,
    /// Shown in the app's about page, as the model licenses require.
    pub attribution: &'static str,
    pub files: &'static [ModelFile],
    /// Folders of earlier versions of this model. One of them on disk means
    /// the user already chose this model: the new version is downloaded
    /// without asking, and the old folder deleted once it is in place.
    pub replaces: &'static [&'static str],
}

/// A file in kyrr's model repository on Hugging Face, at a pinned revision.
macro_rules! hf {
    ($path:literal) => {
        concat!(
            "https://huggingface.co/heast13/kyrr-models/resolve/fcb7ff4db50300b95f9301fb5f8c62c56c4e69ea/",
            $path
        )
    };
}

/// A file added to kyrr's model repository after the revision above. Not
/// pinned: every download is checked against its SHA256 anyway.
macro_rules! hf_latest {
    ($path:literal) => {
        concat!("https://huggingface.co/heast13/kyrr-models/resolve/main/", $path)
    };
}

/// The same files in a release of kyrr's mirror repository on GitHub.
macro_rules! github {
    ($name:literal) => {
        concat!(
            "https://github.com/heast13/kyrr-assets/releases/download/models-1/",
            $name
        )
    };
}

/// The English model where the sherpa-onnx project published it.
macro_rules! sherpa {
    ($name:literal) => {
        concat!(
            "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/8c3a10fb13408c7a7054f6898958bf1c64a8d6c7/",
            $name
        )
    };
}

/// The SenseVoice package where the sherpa-onnx project published it.
macro_rules! sense_voice {
    ($name:literal) => {
        concat!(
            "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07/",
            $name
        )
    };
}

/// primeline/parakeet-primeline, converted with `tools/export-primeline`.
/// `de-primeline` had signed int8 weights, which ONNX Runtime gets wrong on
/// x86 CPUs without VNNI (see the tool's README).
static GERMAN: ModelSpec = ModelSpec {
    kind: ModelKind::Transducer,
    id: "de-primeline-u8",
    attribution: "German: primeline/parakeet-primeline by primeLine AI (CC-BY-4.0), \
                  based on NVIDIA parakeet-tdt-0.6b-v3; converted to ONNX and quantized to int8.",
    files: &[
        ModelFile {
            name: "encoder.int8.onnx",
            urls: &[
                hf!("de-primeline-u8/encoder.int8.onnx"),
                github!("de-primeline-u8.encoder.int8.onnx"),
            ],
            size: 881_596_736,
            sha256: "49596a2c9c20a224fd087b2c1dc8cdc5dfe3fb8d3ddd4eefb070d81e492c93d8",
        },
        ModelFile {
            name: "decoder.int8.onnx",
            urls: &[
                hf!("de-primeline-u8/decoder.int8.onnx"),
                github!("de-primeline-u8.decoder.int8.onnx"),
            ],
            size: 11_845_274,
            sha256: "6b32c9339900086afd3d282eb40688ae19f3d106281e7731ee27ead1add8b29d",
        },
        ModelFile {
            name: "joiner.int8.onnx",
            urls: &[
                hf!("de-primeline-u8/joiner.int8.onnx"),
                github!("de-primeline-u8.joiner.int8.onnx"),
            ],
            size: 6_355_277,
            sha256: "c84fcf848108531a7309ef65308dc5dc610091ce0fbde134f76bb0f6c42ca530",
        },
        ModelFile {
            name: "tokens.txt",
            urls: &[hf!("de-primeline-u8/tokens.txt"), github!("de-primeline-u8.tokens.txt")],
            size: 93_939,
            sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
        },
    ],
    replaces: &["de-primeline"],
};

/// nvidia/parakeet-unified-en-0.6b as packaged by sherpa-onnx (int8,
/// non-streaming), mirrored unchanged in kyrr's repositories.
static ENGLISH: ModelSpec = ModelSpec {
    kind: ModelKind::Transducer,
    id: "en-unified",
    attribution: "English: NVIDIA parakeet-unified-en-0.6b, licensed by NVIDIA Corporation under the \
                  NVIDIA Open Model License; ONNX int8 conversion by the sherpa-onnx project.",
    files: &[
        ModelFile {
            name: "encoder.int8.onnx",
            urls: &[
                hf!("en-unified/encoder.int8.onnx"),
                github!("en-unified.encoder.int8.onnx"),
                sherpa!("encoder.int8.onnx"),
            ],
            size: 654_040_552,
            sha256: "6716910b7a0833997fec7a410494c995d70124001a0e9b66d6370d6aced577e0",
        },
        ModelFile {
            name: "decoder.int8.onnx",
            urls: &[
                hf!("en-unified/decoder.int8.onnx"),
                github!("en-unified.decoder.int8.onnx"),
                sherpa!("decoder.int8.onnx"),
            ],
            size: 7_257_753,
            sha256: "a5e223392c90e75f8144cdb5eb95af7625db389e39edef2bd1a9c872b3298fe6",
        },
        ModelFile {
            name: "joiner.int8.onnx",
            urls: &[
                hf!("en-unified/joiner.int8.onnx"),
                github!("en-unified.joiner.int8.onnx"),
                sherpa!("joiner.int8.onnx"),
            ],
            size: 1_735_860,
            sha256: "869f43f7d24595c55581ad3bf249a935fb8a71389fbdaa7504b9f46f93140f8a",
        },
        ModelFile {
            name: "tokens.txt",
            urls: &[
                hf!("en-unified/tokens.txt"),
                github!("en-unified.tokens.txt"),
                sherpa!("tokens.txt"),
            ],
            size: 8_952,
            sha256: "dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff",
        },
    ],
    replaces: &[],
};

/// FunAudioLLM SenseVoice-Small (Chinese, Japanese, Korean, English and
/// Cantonese) as packaged by sherpa-onnx (int8), mirrored unchanged. Not the
/// `2025-09-09` package: that one is a Cantonese fine-tune which writes
/// Chinese characters for Japanese and Korean speech.
static SENSE_VOICE: ModelSpec = ModelSpec {
    kind: ModelKind::SenseVoice,
    id: "zh-ja-ko-sensevoice",
    attribution: "Chinese, Japanese, Korean: SenseVoice-Small by FunAudioLLM / Alibaba Group (FunASR Model \
                  Open Source License v1.1); ONNX int8 conversion by the sherpa-onnx project.",
    files: &[
        ModelFile {
            name: "model.int8.onnx",
            urls: &[
                github!("zh-ja-ko-sensevoice.model.int8.onnx"),
                hf_latest!("zh-ja-ko-sensevoice/model.int8.onnx"),
                sense_voice!("model.int8.onnx"),
            ],
            size: 239_233_841,
            sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
        },
        ModelFile {
            name: "tokens.txt",
            urls: &[
                github!("zh-ja-ko-sensevoice.tokens.txt"),
                hf_latest!("zh-ja-ko-sensevoice/tokens.txt"),
                sense_voice!("tokens.txt"),
            ],
            size: 315_894,
            sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
        },
    ],
    replaces: &[],
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

    /// Whether an earlier version of this model is on disk.
    pub fn replaces_installed(&self, root: &Path) -> bool {
        self.replaces.iter().any(|id| root.join(id).is_dir())
    }

    /// Deletes the earlier versions; a folder that will not go is left for
    /// the next time.
    pub fn remove_replaced(&self, root: &Path) {
        for id in self.replaces {
            let _ = std::fs::remove_dir_all(root.join(id));
        }
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
