//! `kb-parse-md` — Markdown parsing + canonical-document lift for the KB pipeline (§3.7b).
//!
//! v0.19.0 부터 `types` + `normalize` module 은 in-crate 흡수
//! (`kebab-parse-types` + `kebab-normalize` 의 historical crate 가 본 crate 로
//! collapse — see HOTFIXES.md 2026-05-26).
//!
//! Public surface:
//!
//! * [`parse_frontmatter`] — pure function from Markdown bytes to
//!   `(Metadata, Option<FrontmatterSpan>, Vec<Warning>)` (P1-2).
//! * [`BodyHints`] — caller-supplied fallbacks that feed the §0 Q9 derive
//!   table when frontmatter is missing or partial (P1-2).
//! * [`FrontmatterSpan`] — byte offsets of the frontmatter region in the
//!   input slice (returned by [`parse_frontmatter`]) (P1-2).
//! * [`parse_blocks`] — pure function from Markdown body bytes to
//!   `(Vec<ParsedBlock>, Vec<Warning>)` with heading paths and 1-indexed
//!   `SourceSpan::Line` ranges relative to the original file (P1-3).
//! * [`build_canonical_document`] / [`derive_title`] — lift a parsed
//!   markdown document into a `kebab_core::CanonicalDocument` (absorbed
//!   from `kebab-normalize` — P1-4 / p9-fb-07 frozen API).
//! * Parser intermediate types ([`ParsedBlock`], [`ParsedBlockKind`],
//!   [`ParsedPayload`], [`Warning`], [`WarningKind`]) and 3 forward-declared
//!   structs ([`ParsedImageRegion`], [`ParsedPdfPage`], [`ParsedAudioSegment`]) —
//!   absorbed from `kebab-parse-types`.
//!
//! Anything else in this crate is `pub(crate)` and may change without notice.

pub mod blocks;
pub mod frontmatter;
mod normalize;
mod types;

pub use blocks::parse_blocks;
pub use frontmatter::{BodyHints, FrontmatterSpan, parse_frontmatter};

// Spec §3.3 의 surface 보존 정책 — explicit (NOT glob) 으로 future addition leak 방지.
pub use crate::normalize::{build_canonical_document, derive_title};
pub use crate::types::{
    ParsedAudioSegment,
    // 5 사용 type
    ParsedBlock,
    ParsedBlockKind,
    // 3 forward-declared struct (보존 — spec §3.3 + §11.5 future surface)
    ParsedImageRegion,
    ParsedPayload,
    ParsedPdfPage,
    Warning,
    WarningKind,
};

/// Parser-version label for Markdown files ingested through this crate.
/// Re-exported so `kebab-app::schema_with_config` can embed it in
/// `SchemaV1.models.parser_version` without duplicating the literal.
///
/// Kept in sync with `KEBAB_PARSE_MD_VERSION` in `kebab-app/src/lib.rs`.
pub const PARSER_VERSION: &str = "md-frontmatter-v2";
