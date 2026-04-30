//! `kb-parse-md` — Markdown parsing for the KB pipeline (§3.7b).
//!
//! P1-2 implements the **frontmatter** submodule only. P1-3 will add a
//! sibling `blocks` submodule for block parsing using `pulldown-cmark`.
//!
//! Public surface for P1-2 is intentionally narrow:
//!
//! * [`parse_frontmatter`] — pure function from Markdown bytes to
//!   `(Metadata, Option<FrontmatterSpan>, Vec<Warning>)`.
//! * [`BodyHints`] — caller-supplied fallbacks that feed the §0 Q9 derive
//!   table when frontmatter is missing or partial.
//! * [`FrontmatterSpan`] — byte offsets of the frontmatter region in the
//!   input slice (returned by [`parse_frontmatter`]).
//!
//! Anything else in this crate is `pub(crate)` and may change without notice.

pub mod frontmatter;

pub use frontmatter::{BodyHints, FrontmatterSpan, parse_frontmatter};
