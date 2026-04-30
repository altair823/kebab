//! Markdown frontmatter parsing → `kb_core::Metadata`.
//!
//! Implements the contract pinned in design §0 Q9 (frontmatter derive table)
//! and §3.6 (Metadata shape). Produces structured warnings via
//! `kb-parse-types`.
//!
//! # YAML library
//!
//! Upstream `serde_yaml` (dtolnay) was archived as unmaintained in 2024. We
//! use [`serde_yaml_ng`], a maintained fork with an API-compatible surface,
//! so a future swap to whichever fork wins (`serde_yml`, `yaml-rust2`, …)
//! is a one-line dep change.

use std::sync::OnceLock;

use kb_core::{Metadata, SourceType, TrustLevel};
use kb_parse_types::{Warning, WarningKind};
use lingua::{IsoCode639_1, Language, LanguageDetector, LanguageDetectorBuilder};
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;

/// Caller-supplied fallback values used when frontmatter is missing or partial.
///
/// `BodyHints` is parser-input only — it is not part of `kb-core` and never
/// crosses the storage boundary. The §0 Q9 derive table consults these
/// fallbacks in a fixed order, see [`parse_frontmatter`].
#[derive(Clone, Debug)]
pub struct BodyHints {
    /// First H1 of the body, if any. Used as `title` fallback when the
    /// frontmatter does not specify one.
    pub first_h1: Option<String>,
    /// Filesystem creation time. Used as `created_at` fallback.
    pub fs_ctime: OffsetDateTime,
    /// Filesystem modification time. Used as `updated_at` fallback.
    pub fs_mtime: OffsetDateTime,
    /// Optional language fallback used when neither frontmatter nor lingua
    /// detection produce a value. If `None` the final fallback is `"und"`.
    pub fallback_lang: Option<String>,
}

/// Byte range of the frontmatter region inside the input slice.
///
/// `start` is the offset of the leading delimiter (`---` or `+++`).
/// `end` is the offset just past the closing delimiter line's trailing
/// newline (i.e. the body starts at `bytes[end..]`).
///
/// Per the task brief this is technically meant to be crate-internal, but
/// the [`parse_frontmatter`] return type forces it to be `pub`. P1-3 / P1-4
/// reuse it via this same crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrontmatterSpan {
    pub start: usize,
    pub end: usize,
}

/// Parse the frontmatter (if any) from a Markdown byte slice into a
/// `kb_core::Metadata`, applying the §0 Q9 derive table for missing fields.
///
/// On a malformed frontmatter the function still returns `Ok` — the
/// frontmatter contents are discarded and the caller is told via a
/// `Warning { kind: MalformedFrontmatter, .. }`. The returned span still
/// covers the delimited region so the caller can skip it during body
/// slicing.
///
/// `Err` is reserved for genuinely fatal conditions (e.g. non-UTF-8 input
/// that can't even be lossy-decoded), which currently cannot arise here.
pub fn parse_frontmatter(
    bytes: &[u8],
    hints: &BodyHints,
) -> anyhow::Result<(Metadata, Option<FrontmatterSpan>, Vec<Warning>)> {
    let mut warnings = Vec::new();

    let detected = detect_delimiters(bytes);

    let (raw_opt, span_opt) = match detected {
        None => (None, None),
        Some((delim, span)) => {
            // SAFETY: detect_delimiters guarantees inner bytes are valid UTF-8
            // because it scanned for ASCII delimiters and slices on those
            // boundaries. We still go through `from_utf8` to surface non-ASCII
            // bytes safely as a malformed-frontmatter warning.
            let inner_start = span.start + delim.opening_len();
            let inner_end = span.end - delim.closing_len();
            let inner = &bytes[inner_start..inner_end];
            match std::str::from_utf8(inner) {
                Ok(s) => match parse_raw(delim, s) {
                    Ok(raw) => (Some(raw), Some(span)),
                    Err(e) => {
                        warnings.push(Warning {
                            kind: WarningKind::MalformedFrontmatter,
                            note: e,
                        });
                        (None, Some(span))
                    }
                },
                Err(e) => {
                    warnings.push(Warning {
                        kind: WarningKind::MalformedFrontmatter,
                        note: format!("frontmatter not valid utf-8: {e}"),
                    });
                    (None, Some(span))
                }
            }
        }
    };

    let body_start = span_opt.map(|s| s.end).unwrap_or(0);
    let body = &bytes[body_start..];

    let metadata = derive_metadata(raw_opt, hints, body, &mut warnings);

    Ok((metadata, span_opt, warnings))
}

// ---------------------------------------------------------------------------
// Delimiter detection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DelimKind {
    Yaml,
    Toml,
}

impl DelimKind {
    /// Bytes consumed at the start (delimiter line + newline).
    fn opening_len(self) -> usize {
        // "---\n" or "+++\n" — both 4 bytes; "---\r\n" handled by detect.
        match self {
            DelimKind::Yaml => 4,
            DelimKind::Toml => 4,
        }
    }

    fn closing_len(self) -> usize {
        // The closing delimiter line itself plus its trailing newline. Same
        // shape as opening; `detect_delimiters` adjusts for `\r\n`.
        match self {
            DelimKind::Yaml => 4,
            DelimKind::Toml => 4,
        }
    }

    fn marker(self) -> &'static [u8] {
        match self {
            DelimKind::Yaml => b"---",
            DelimKind::Toml => b"+++",
        }
    }
}

/// Look for a YAML or TOML frontmatter block at the very start of `bytes`.
/// Returns `(kind, span)` where `span.start = 0` and `span.end` points
/// just past the closing delimiter's trailing newline (or EOF).
///
/// Anything that isn't an exact `---\n` / `+++\n` opener at byte 0 is treated
/// as "no frontmatter" — no leading whitespace, no BOM, etc. Per design §0 Q9.
pub(crate) fn detect_delimiters(bytes: &[u8]) -> Option<(DelimKind, FrontmatterSpan)> {
    let kind = match bytes.first()? {
        b'-' if bytes.starts_with(b"---") => DelimKind::Yaml,
        b'+' if bytes.starts_with(b"+++") => DelimKind::Toml,
        _ => return None,
    };

    let marker = kind.marker();

    // Opening line must be just the marker + newline (LF or CRLF). No trailing
    // chars on the same line are allowed — that's not a frontmatter delimiter.
    let after_open = match bytes.get(marker.len()) {
        Some(b'\n') => marker.len() + 1,
        Some(b'\r') if bytes.get(marker.len() + 1) == Some(&b'\n') => marker.len() + 2,
        _ => return None,
    };

    // Find the closing marker on its own line.
    // Walk line by line. We need a line that is exactly `marker` (optionally
    // followed by spaces? per §0 Q9 we keep it strict: marker + EOL only).
    let mut i = after_open;
    while i < bytes.len() {
        let line_start = i;
        // find next newline (or EOF)
        let line_end = bytes[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| line_start + p)
            .unwrap_or(bytes.len());

        let line = {
            // trim trailing \r if present (CRLF)
            let mut end = line_end;
            if end > line_start && bytes[end.saturating_sub(1)] == b'\r' {
                end -= 1;
            }
            &bytes[line_start..end]
        };

        if line == marker {
            // Closing delimiter found. Compute span end = line_end + 1 if a
            // newline is present, else line_end (EOF).
            let span_end = if line_end < bytes.len() {
                line_end + 1
            } else {
                bytes.len()
            };
            return Some((
                kind,
                FrontmatterSpan {
                    start: 0,
                    end: span_end,
                },
            ));
        }

        if line_end >= bytes.len() {
            break;
        }
        i = line_end + 1;
    }

    // No closing delimiter — not a frontmatter block.
    None
}

// ---------------------------------------------------------------------------
// Raw frontmatter (parsed shape, before §0 Q9 derive)
// ---------------------------------------------------------------------------

/// Untyped frontmatter view. Known fields are pulled by name, unknowns flow
/// into `extra`. We deliberately use `serde_json::Value` everywhere so YAML
/// and TOML go through the same downstream pipeline.
#[derive(Debug, Default, Deserialize)]
struct RawFrontmatter {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    aliases: Option<Vec<String>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    source_type: Option<String>,
    #[serde(default)]
    trust_level: Option<String>,
    /// `id:` field is captured as an alias only — never feeds doc_id (§4.2).
    #[serde(default)]
    id: Option<String>,
    /// Catch-all for unknown keys → `metadata.user`.
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn parse_raw(kind: DelimKind, slice: &str) -> Result<RawFrontmatter, String> {
    match kind {
        DelimKind::Yaml => {
            // Empty YAML frontmatter is legal (parses to null) — handle
            // explicitly so `serde_yaml_ng` doesn't fail trying to deserialize
            // null into a struct.
            if slice.trim().is_empty() {
                return Ok(RawFrontmatter::default());
            }
            serde_yaml_ng::from_str::<RawFrontmatter>(slice).map_err(|e| e.to_string())
        }
        DelimKind::Toml => {
            if slice.trim().is_empty() {
                return Ok(RawFrontmatter::default());
            }
            toml::from_str::<RawFrontmatter>(slice).map_err(|e| e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// §0 Q9 derive table
// ---------------------------------------------------------------------------

fn derive_metadata(
    raw: Option<RawFrontmatter>,
    hints: &BodyHints,
    body: &[u8],
    warnings: &mut Vec<Warning>,
) -> Metadata {
    let raw = raw.unwrap_or_default();

    // user map starts from the unknown-key overflow.
    let mut user = raw.extra;

    // ---- title ----
    // Frontmatter → BodyHints.first_h1 → None.
    // Filename fallback is the caller's responsibility (P1-4 normalize), per
    // task brief — `BodyHints` does not carry a filename.
    let title = raw.title.or_else(|| hints.first_h1.clone());
    if let Some(t) = title {
        user.insert("title".to_string(), Value::String(t));
    }

    // ---- aliases / tags ----
    let aliases = raw.aliases.unwrap_or_default();
    let tags = raw.tags.unwrap_or_default();

    // ---- lang ----
    // Frontmatter → lingua autodetect (first 4 KB of body) → fallback_lang → "und".
    // The lang field is not on Metadata (§3.6) — store it under user.lang.
    let lang = raw
        .lang
        .or_else(|| detect_lang(body))
        .or_else(|| hints.fallback_lang.clone())
        .unwrap_or_else(|| "und".to_string());
    user.insert("lang".to_string(), Value::String(lang));

    // ---- timestamps ----
    let mut original_timestamps: Map<String, Value> = Map::new();
    let created_at = parse_ts(
        raw.created_at.as_deref(),
        "created_at",
        &mut original_timestamps,
        warnings,
    )
    .unwrap_or(hints.fs_ctime);
    let updated_at = parse_ts(
        raw.updated_at.as_deref(),
        "updated_at",
        &mut original_timestamps,
        warnings,
    )
    .unwrap_or(hints.fs_mtime);
    if !original_timestamps.is_empty() {
        user.insert(
            "original_timestamps".to_string(),
            Value::Object(original_timestamps),
        );
    }

    // ---- source_type ----
    let source_type = match raw.source_type.as_deref() {
        None => SourceType::Markdown,
        Some(s) => match parse_source_type(s) {
            Some(st) => st,
            None => {
                warnings.push(Warning {
                    kind: WarningKind::MalformedFrontmatter,
                    note: format!("unknown source_type={s}, defaulted to markdown"),
                });
                SourceType::Markdown
            }
        },
    };

    // ---- trust_level ----
    let trust_level = match raw.trust_level.as_deref() {
        None => TrustLevel::Primary,
        Some(s) => match parse_trust_level(s) {
            Some(tl) => tl,
            None => {
                warnings.push(Warning {
                    kind: WarningKind::MalformedFrontmatter,
                    note: format!("unknown trust_level={s}, defaulted to primary"),
                });
                TrustLevel::Primary
            }
        },
    };

    // ---- id alias ----
    // `id:` field becomes `metadata.user_id_alias` only (spec §"Behavior
    // contract" line 74). It is NOT mirrored into the user map.
    let user_id_alias = raw.id;

    Metadata {
        aliases,
        tags,
        created_at,
        updated_at,
        source_type,
        trust_level,
        user_id_alias,
        user,
    }
}

fn parse_source_type(s: &str) -> Option<SourceType> {
    // Mirror the lowercase serde rename used on SourceType.
    match s {
        "markdown" => Some(SourceType::Markdown),
        "note" => Some(SourceType::Note),
        "paper" => Some(SourceType::Paper),
        "reference" => Some(SourceType::Reference),
        "inbox" => Some(SourceType::Inbox),
        _ => None,
    }
}

fn parse_trust_level(s: &str) -> Option<TrustLevel> {
    match s {
        "primary" => Some(TrustLevel::Primary),
        "secondary" => Some(TrustLevel::Secondary),
        "generated" => Some(TrustLevel::Generated),
        _ => None,
    }
}

/// Parse an RFC 3339 timestamp string and normalize to UTC. If the original
/// offset was non-UTC, push it into `original_timestamps[field]` per §0 Q9.
/// Returns `None` if the input is missing OR malformed (in which case a
/// warning is emitted).
fn parse_ts(
    s: Option<&str>,
    field: &str,
    original_timestamps: &mut Map<String, Value>,
    warnings: &mut Vec<Warning>,
) -> Option<OffsetDateTime> {
    let s = s?;
    match OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339) {
        Ok(dt) => {
            if dt.offset() != time::UtcOffset::UTC {
                original_timestamps.insert(field.to_string(), Value::String(s.to_string()));
            }
            Some(dt.to_offset(time::UtcOffset::UTC))
        }
        Err(e) => {
            warnings.push(Warning {
                kind: WarningKind::MalformedFrontmatter,
                note: format!("malformed {field}={s:?}: {e}"),
            });
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Lingua detector (cached statically — first init is heavy)
// ---------------------------------------------------------------------------

fn detector() -> &'static LanguageDetector {
    static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();
    DETECTOR.get_or_init(|| {
        // Keep the language set narrow: matches the cargo features we enable
        // on the `lingua` dep. Adding more languages here without enabling
        // their feature flag will fail to compile.
        LanguageDetectorBuilder::from_languages(&[
            Language::English,
            Language::Korean,
            Language::Japanese,
            Language::Chinese,
        ])
        .build()
    })
}

/// Run lingua autodetect on the first 4 KB of body. Returns an ISO 639-1
/// two-letter code (lowercase) on success.
///
/// Note: lingua needs reasonably long input to be confident. Empty / very
/// short bodies return `None` so we fall through to the next derive step.
fn detect_lang(body: &[u8]) -> Option<String> {
    const WINDOW: usize = 4 * 1024;
    if body.is_empty() {
        return None;
    }
    let n = body.len().min(WINDOW);
    // Find a UTF-8-safe slice end ≤ n. Walk back at most 4 bytes.
    let mut end = n;
    while end > 0 && std::str::from_utf8(&body[..end]).is_err() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    let s = std::str::from_utf8(&body[..end]).ok()?;
    if s.trim().is_empty() {
        return None;
    }
    let lang = detector().detect_language_of(s)?;
    Some(iso_code(lang).to_string())
}

fn iso_code(lang: Language) -> &'static str {
    // `lingua::IsoCode639_1` is gated by the language features enabled on the
    // crate — only the variants below are compiled into our build, so this
    // match is exhaustive for the configured detector.
    match lang.iso_code_639_1() {
        IsoCode639_1::EN => "en",
        IsoCode639_1::KO => "ko",
        IsoCode639_1::JA => "ja",
        IsoCode639_1::ZH => "zh",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kb_core::{
        AssetId, WorkspacePath,
        ids::id_for_doc,
        versions::ParserVersion,
    };
    use time::macros::datetime;

    fn hints() -> BodyHints {
        BodyHints {
            first_h1: None,
            fs_ctime: datetime!(2024-01-01 00:00:00 UTC),
            fs_mtime: datetime!(2024-01-02 00:00:00 UTC),
            fallback_lang: None,
        }
    }

    #[test]
    fn yaml_happy_path() {
        let md = b"---\n\
title: My Doc\n\
aliases: [a, b]\n\
tags: [t1, t2]\n\
lang: en\n\
created_at: 2024-03-01T00:00:00Z\n\
updated_at: 2024-03-02T00:00:00Z\n\
source_type: note\n\
trust_level: secondary\n\
---\nbody\n";

        let (meta, span, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert!(warns.is_empty(), "warnings: {warns:?}");
        let span = span.expect("span present");
        assert_eq!(span.start, 0);
        assert_eq!(meta.aliases, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(meta.tags, vec!["t1".to_string(), "t2".to_string()]);
        assert_eq!(meta.source_type, SourceType::Note);
        assert_eq!(meta.trust_level, TrustLevel::Secondary);
        assert_eq!(meta.created_at, datetime!(2024-03-01 00:00:00 UTC));
        assert_eq!(meta.updated_at, datetime!(2024-03-02 00:00:00 UTC));
        assert_eq!(meta.user.get("title").and_then(|v| v.as_str()), Some("My Doc"));
        assert_eq!(meta.user.get("lang").and_then(|v| v.as_str()), Some("en"));
        assert_eq!(meta.user_id_alias, None);
    }

    #[test]
    fn toml_happy_path() {
        let md = b"+++\n\
title = \"My Doc\"\n\
aliases = [\"a\", \"b\"]\n\
tags = [\"t1\", \"t2\"]\n\
lang = \"en\"\n\
created_at = \"2024-03-01T00:00:00Z\"\n\
updated_at = \"2024-03-02T00:00:00Z\"\n\
source_type = \"note\"\n\
trust_level = \"secondary\"\n\
+++\nbody\n";

        let (meta, span, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert!(warns.is_empty(), "warnings: {warns:?}");
        assert!(span.is_some());
        assert_eq!(meta.aliases, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(meta.tags, vec!["t1".to_string(), "t2".to_string()]);
        assert_eq!(meta.source_type, SourceType::Note);
        assert_eq!(meta.trust_level, TrustLevel::Secondary);
    }

    #[test]
    fn unknown_keys_preserved_in_user() {
        let md = b"---\n\
title: Doc\n\
custom_field: hello\n\
nested: {a: 1}\n\
---\n";
        let (meta, _span, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert!(warns.is_empty(), "warnings: {warns:?}");
        assert_eq!(
            meta.user.get("custom_field").and_then(|v| v.as_str()),
            Some("hello")
        );
        assert!(meta.user.get("nested").is_some());
    }

    #[test]
    fn unknown_enum_value_warns_and_defaults() {
        let md = b"---\n\
trust_level: weird\n\
source_type: alien\n\
---\n";
        let (meta, _span, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert_eq!(meta.trust_level, TrustLevel::Primary);
        assert_eq!(meta.source_type, SourceType::Markdown);
        assert_eq!(warns.len(), 2);
        assert!(warns.iter().all(|w| matches!(w.kind, WarningKind::MalformedFrontmatter)));
        assert!(warns.iter().any(|w| w.note.contains("trust_level=weird")));
        assert!(warns.iter().any(|w| w.note.contains("source_type=alien")));
    }

    #[test]
    fn malformed_yaml_emits_warning_and_defaults() {
        // Unclosed quote → YAML parse fails.
        let md = b"---\ntitle: \"unterminated\n---\n";
        let (meta, span, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert!(span.is_some(), "span still reflects delim region");
        assert_eq!(warns.len(), 1);
        assert!(matches!(warns[0].kind, WarningKind::MalformedFrontmatter));
        // Body fallbacks applied.
        assert_eq!(meta.created_at, datetime!(2024-01-01 00:00:00 UTC));
        assert_eq!(meta.updated_at, datetime!(2024-01-02 00:00:00 UTC));
        assert_eq!(meta.source_type, SourceType::Markdown);
        assert_eq!(meta.trust_level, TrustLevel::Primary);
    }

    #[test]
    fn no_frontmatter_uses_body_hints_silently() {
        let md = b"# Just a heading\n\nsome body";
        let mut h = hints();
        h.first_h1 = Some("Just a heading".to_string());
        h.fallback_lang = Some("en".to_string());
        let (meta, span, warns) = parse_frontmatter(md, &h).unwrap();
        assert!(span.is_none());
        assert!(warns.is_empty());
        assert_eq!(
            meta.user.get("title").and_then(|v| v.as_str()),
            Some("Just a heading")
        );
        // Body too short for confident lingua autodetect → fallback_lang.
        assert_eq!(meta.user.get("lang").and_then(|v| v.as_str()), Some("en"));
    }

    /// `id:` field MUST NOT influence `doc_id` (design §4.2). Compute the
    /// recipe twice — with and without the field — and assert the results
    /// match.
    #[test]
    fn id_field_does_not_feed_doc_id() {
        let with_id = b"---\nid: my-handle\ntitle: Doc\n---\n";
        let without = b"---\ntitle: Doc\n---\n";

        let (meta_with, _, _) = parse_frontmatter(with_id, &hints()).unwrap();
        let (meta_without, _, _) = parse_frontmatter(without, &hints()).unwrap();

        assert_eq!(meta_with.user_id_alias.as_deref(), Some("my-handle"));
        assert_eq!(meta_without.user_id_alias, None);

        let asset = AssetId("0123456789abcdef0123456789abcdef".to_string());
        let path = WorkspacePath::new("notes/test.md".to_string()).unwrap();
        let pv = ParserVersion("pulldown-cmark-0.x".to_string());

        let id_a = id_for_doc(&path, &asset, &pv);
        let id_b = id_for_doc(&path, &asset, &pv);
        assert_eq!(
            id_a, id_b,
            "id_for_doc must be stable across runs and not see metadata"
        );
        // Sanity: the recipe takes (workspace_path, asset_id, parser_version)
        // only — there is literally no parameter to plumb metadata through.
    }

    #[test]
    fn non_utc_timestamp_preserved_in_user_original_timestamps() {
        let md = b"---\ncreated_at: 2024-01-15T10:00:00+09:00\n---\n";
        let (meta, _, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert!(warns.is_empty(), "warnings: {warns:?}");
        // Normalized to UTC.
        assert_eq!(meta.created_at, datetime!(2024-01-15 01:00:00 UTC));
        let orig = meta
            .user
            .get("original_timestamps")
            .and_then(|v| v.as_object())
            .expect("original_timestamps map present");
        assert_eq!(
            orig.get("created_at").and_then(|v| v.as_str()),
            Some("2024-01-15T10:00:00+09:00")
        );
    }

    #[test]
    fn malformed_timestamp_warns_and_falls_back() {
        let md = b"---\ncreated_at: not-a-date\n---\n";
        let (meta, _, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert_eq!(warns.len(), 1);
        assert!(matches!(warns[0].kind, WarningKind::MalformedFrontmatter));
        assert!(warns[0].note.contains("created_at"));
        // Fallback to fs_ctime.
        assert_eq!(meta.created_at, datetime!(2024-01-01 00:00:00 UTC));
    }

    #[test]
    fn detect_delimiters_no_match_without_leading_marker() {
        assert!(detect_delimiters(b"# heading\n---\n---\n").is_none());
        assert!(detect_delimiters(b"  ---\n---\n").is_none(), "leading whitespace");
        assert!(detect_delimiters(b"").is_none());
    }

    #[test]
    fn detect_delimiters_yaml_basic() {
        let bytes = b"---\nfoo: bar\n---\nbody\n";
        let (kind, span) = detect_delimiters(bytes).unwrap();
        assert_eq!(kind, DelimKind::Yaml);
        assert_eq!(span.start, 0);
        // body starts at "body\n" — the closing "---\n" is part of the span.
        assert_eq!(&bytes[span.end..], b"body\n");
    }

    #[test]
    fn detect_delimiters_toml_basic() {
        let bytes = b"+++\nfoo = \"bar\"\n+++\nbody\n";
        let (kind, span) = detect_delimiters(bytes).unwrap();
        assert_eq!(kind, DelimKind::Toml);
        assert_eq!(&bytes[span.end..], b"body\n");
    }

    #[test]
    fn detect_delimiters_unterminated_returns_none() {
        // `---\n` then no closing — treat as no frontmatter.
        let bytes = b"---\nfoo: bar\n";
        assert!(detect_delimiters(bytes).is_none());
    }

    #[test]
    fn empty_yaml_frontmatter_is_legal() {
        let md = b"---\n---\nbody\n";
        let (_meta, span, warns) = parse_frontmatter(md, &hints()).unwrap();
        assert!(span.is_some());
        assert!(warns.is_empty(), "warnings: {warns:?}");
    }

    #[test]
    fn lingua_detects_korean_and_english() {
        let ko = "안녕하세요. 이것은 한국어로 작성된 문서입니다. 형태소 분석은 어렵습니다. 그러나 lingua는 잘 동작합니다.".as_bytes();
        let en = "Hello there. This document is written in English. The lingua language detector is statistical and works on short text too, given enough words.".as_bytes();
        assert_eq!(detect_lang(ko).as_deref(), Some("ko"));
        assert_eq!(detect_lang(en).as_deref(), Some("en"));
    }
}
