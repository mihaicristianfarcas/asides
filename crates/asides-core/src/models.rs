//! Whisper models (ggml, from huggingface.co/ggerganov/whisper.cpp): English-only ones (the default:
//! smaller, and never mistaken for another language) and multilingual ones. None is downloaded until
//! the user asks (Settings, or the pill's microphone).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    /// Transcribes English only (`.en`): no language detection.
    pub english: bool,
}

/// The model asides suggests.
pub const DEFAULT: &str = "small.en";

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "small.en",
        label: "English (190 MB)",
        file: "ggml-small.en-q5_1.bin",
        sha256: "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        size: 190_098_681,
        english: true,
    },
    ModelSpec {
        id: "medium.en",
        label: "English, more accurate (539 MB)",
        file: "ggml-medium.en-q5_0.bin",
        sha256: "76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0",
        size: 539_225_533,
        english: true,
    },
    ModelSpec {
        id: "large-v3-turbo",
        label: "Any language (574 MB)",
        file: "ggml-large-v3-turbo-q5_0.bin",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        size: 574_041_195,
        english: false,
    },
    ModelSpec {
        id: "large-v3",
        label: "Any language, most accurate (1.1 GB, slower)",
        file: "ggml-large-v3-q5_0.bin",
        sha256: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        size: 1_081_140_203,
        english: false,
    },
    ModelSpec {
        id: "small",
        label: "Any language, fastest (190 MB)",
        file: "ggml-small-q5_1.bin",
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        size: 190_085_487,
        english: false,
    },
];

impl ModelSpec {
    /// Unknown ids fall back to the default model.
    pub fn find(id: &str) -> &'static ModelSpec {
        MODELS.iter().find(|m| m.id == id).or_else(|| MODELS.iter().find(|m| m.id == DEFAULT)).unwrap_or(&MODELS[0])
    }

    /// It's on disk, checked.
    pub fn downloaded(&self) -> bool {
        self.path().is_file()
    }

    /// The model's name as OpenAI publishes it (huggingface.co/openai/whisper-large-v3-turbo):
    /// "whisper-large-v3-turbo".
    pub fn name(&self) -> String {
        format!("whisper-{}", self.id)
    }

    /// "190 MB".
    pub fn size_label(&self) -> String {
        match self.size {
            s if s >= 1_000_000_000 => format!("{:.1} GB", s as f64 / 1e9),
            s => format!("{} MB", s / 1_000_000),
        }
    }

    pub fn url(&self) -> String {
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}", self.file)
    }

    pub fn path(&self) -> PathBuf {
        crate::paths::models_dir().join(self.file)
    }
}
