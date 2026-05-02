//! Streaming BLAKE3 over a file path. Per task spec, files MUST NOT be
//! loaded fully into memory: `blake3::Hasher::update_reader` reads through a
//! 64 KiB internal buffer, which keeps memory bounded for any size of file.
//!
//! Returns `(byte_len, full_hex)`:
//!   - `byte_len` is the total bytes hashed (== file size after follow).
//!   - `full_hex` is the canonical lowercase hex (64 chars) of the full
//!     blake3 digest. The `kb-core::Checksum` invariant is "full hex"; the
//!     32-char prefix is reserved for `AssetId` derivation via
//!     `kebab_core::id_for_asset`.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{Context, Result};

const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Stream-hash a file with blake3. Returns `(byte_len, full_hex_64)`.
///
/// `byte_len` is computed during streaming so callers do not need a separate
/// `metadata().len()` call (which can disagree with hashed bytes if the file
/// is rewritten mid-scan, but blake3-of-stream is the source of truth for
/// `RawAsset.checksum`).
pub(crate) fn hash_file(path: &Path) -> Result<(u64, String)> {
    let file = File::open(path)
        .with_context(|| format!("failed to open {} for hashing", path.display()))?;
    hash_reader(file).with_context(|| format!("failed to hash {}", path.display()))
}

fn hash_reader<R: Read>(mut reader: R) -> Result<(u64, String)> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; READ_BUFFER_BYTES];
    let mut total: u64 = 0;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                total = total.saturating_add(n as u64);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok((total, hasher.finalize().to_hex().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// blake3 of the empty input is the well-known "official empty hash"
    /// from the blake3 spec. Pinned so that swapping the hash crate or the
    /// streaming implementation can never silently produce a different
    /// digest for a known input.
    #[test]
    fn empty_blake3_pinned() {
        let (n, hex) = hash_reader(std::io::empty()).unwrap();
        assert_eq!(n, 0);
        assert_eq!(
            hex,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    /// `b"hello world"` blake3 (full 64 hex). Computed independently with
    /// `b3sum`; pinning here detects any drift in the streaming pipeline.
    #[test]
    fn known_bytes_blake3_pinned() {
        let bytes = b"hello world";
        let (n, hex) = hash_reader(&bytes[..]).unwrap();
        assert_eq!(n, 11);
        assert_eq!(
            hex,
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
    }

    /// Streaming a buffer larger than `READ_BUFFER_BYTES` must produce the
    /// same digest as a single-shot blake3 over the same bytes — i.e. the
    /// chunk boundary is invisible.
    #[test]
    fn streaming_matches_oneshot_over_buffer_boundary() {
        let bytes: Vec<u8> = (0u8..=255u8).cycle().take(READ_BUFFER_BYTES * 3 + 17).collect();
        let (n, streamed) = hash_reader(&bytes[..]).unwrap();
        assert_eq!(n, bytes.len() as u64);
        let oneshot = blake3::hash(&bytes).to_hex().to_string();
        assert_eq!(streamed, oneshot);
    }
}
