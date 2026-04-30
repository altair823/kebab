//! `kb-parse-md` — Markdown parsing for the KB pipeline (§3.7b).
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
//!
//! Anything else in this crate is `pub(crate)` and may change without notice.

pub mod blocks;
pub mod frontmatter;

pub use blocks::parse_blocks;
pub use frontmatter::{BodyHints, FrontmatterSpan, parse_frontmatter};
