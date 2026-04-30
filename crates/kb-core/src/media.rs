//! Media / file-type primitives (§3.3 + §3.7a).

use serde::{Deserialize, Serialize};

/// Full blake3 hex (64 chars) per §3.7a. Stored as `String` for serde
/// simplicity.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Checksum(pub String);

/// BCP-47 / ISO-639 language tag (e.g. "ko", "en"). §3.7a.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Lang(pub String);

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageType {
    Png,
    Jpeg,
    Webp,
    Gif,
    Tiff,
    Other(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioType {
    M4a,
    Mp3,
    Wav,
    Flac,
    Ogg,
    Other(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Markdown,
    Pdf,
    Image(ImageType),
    Audio(AudioType),
    Other(String),
}
