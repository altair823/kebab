//! Lexical (FTS5 + bm25) retriever — design §3.7 / §1.5 / §2.2 / §6.4.
//!
//! Owns the SQL pattern documented in `tasks/p2/p2-2-lexical-retriever.md`
//! and constructs `kebab_core::SearchHit` values directly from the joined
//! `chunks_fts` / `chunks` / `documents` rows. Reads only — never mutates
//! the underlying SQLite file.

use std::sync::Arc;

use anyhow::{Context, Result};
use globset::GlobMatcher;
use kebab_core::{
    ChunkId, ChunkerVersion, DocumentId, IndexVersion, RetrievalDetail, Retriever, ScoreKind,
    SearchFilters, SearchHit, SearchMode, SearchQuery, SourceSpan, TrustLevel, WorkspacePath,
};
use kebab_store_sqlite::SqliteStore;
use rusqlite::{Connection, Row, ToSql, params_from_iter};

use crate::citation_helper::citation_from_first_span;

// ── Tunables ─────────────────────────────────────────────────────────────

/// FTS5 hard limit on the `snippet()` `nToken` argument.
/// See SQLite's FTS5 docs: snippet() rejects nToken > 64.
const FTS5_SNIPPET_MAX_WORDS: usize = 64;

/// Floor for the snippet word budget. `snippet_chars / 4` may yield 0 for
/// pathologically small configs; we always ask FTS5 for at least one word
/// so it can still return something matchable for the test harness.
const FTS5_SNIPPET_MIN_WORDS: usize = 1;

/// Default `k` when `SearchQuery::k == 0`. Mirrors §6.4 default_k=10.
const DEFAULT_K: usize = 10;

/// When `path_glob` is set we have to over-fetch and post-filter in Rust,
/// because SQLite's GLOB operator treats `*` as "any chars including `/`",
/// which contradicts the design rule that `*` must NOT cross path
/// separators. Empirically `+128` is generous for any realistic workspace
/// and bounded enough to keep memory predictable.
const PATH_GLOB_OVERFETCH: usize = 128;

// ── Public surface ───────────────────────────────────────────────────────

/// Lexical retriever backed by SQLite FTS5 + bm25.
pub struct LexicalRetriever {
    store: Arc<SqliteStore>,
    index_version: IndexVersion,
    /// Number of `snippet()` words derived from `kb-config::search.snippet_chars`,
    /// clamped into `[FTS5_SNIPPET_MIN_WORDS, FTS5_SNIPPET_MAX_WORDS]`.
    snippet_words: usize,
    /// Hard cap on the returned snippet's character length per design §6.4.
    snippet_chars: usize,
}

impl LexicalRetriever {
    /// Construct with default settings derived from `kb-config`'s defaults.
    /// Snippet width is computed from `Config::defaults().search.snippet_chars`.
    pub fn new(store: Arc<SqliteStore>, index_version: IndexVersion) -> Self {
        let cfg = kebab_config::Config::defaults();
        Self::with_settings(store, index_version, cfg.search.snippet_chars)
    }

    /// Construct with explicit `snippet_chars`. Used by tests / callers
    /// that have already loaded a `Config`.
    pub fn with_settings(
        store: Arc<SqliteStore>,
        index_version: IndexVersion,
        snippet_chars: usize,
    ) -> Self {
        // Heuristic: 1 token ≈ 4 chars (English-leaning estimate; Korean
        // tokens average shorter, so the cap-by-chars trim below is what
        // actually enforces the contract). The `/4` keeps us well below
        // FTS5's nToken=64 limit for typical snippet_chars=220 budgets.
        let raw = snippet_chars / 4;
        let snippet_words = raw.clamp(FTS5_SNIPPET_MIN_WORDS, FTS5_SNIPPET_MAX_WORDS);
        Self {
            store,
            index_version,
            snippet_words,
            snippet_chars,
        }
    }
}

impl Retriever for LexicalRetriever {
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let match_opt = build_match_string(&query.text);
        let k = if query.k == 0 { DEFAULT_K } else { query.k };
        let filters = &query.filters;
        // One-line summary at request entry. Filter shape only — no
        // tag/lang/path values, which could be PII-sensitive.
        tracing::debug!(
            match_str = match_opt.as_deref().unwrap_or("<empty>"),
            tags_any = filters.tags_any.len(),
            has_lang = filters.lang.is_some(),
            has_trust_min = filters.trust_min.is_some(),
            has_path_glob = filters.path_glob.is_some(),
            k,
            "kb-search lexical: search start"
        );

        // Empty / whitespace-only query → nothing to do. Per spec we
        // succeed with an empty hit list rather than erroring.
        let match_str = match match_opt {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };

        // Pre-compile the path_glob once. The `Glob` produced rejects
        // syntactically invalid patterns at construction time so the
        // caller gets a clear error rather than a silent empty result.
        let path_matcher = match &filters.path_glob {
            Some(g) => Some(compile_glob(g)?),
            None => None,
        };

        // Fetch budget: when post-filtering by glob we need to over-fetch
        // so that the final `take(k)` still has enough rows after culling.
        let fetch_limit = if path_matcher.is_some() {
            k.saturating_add(PATH_GLOB_OVERFETCH)
        } else {
            k
        };

        let conn = self.store.read_conn();
        let raw_rows = run_query(&conn, &match_str, self.snippet_words, filters, fetch_limit)?;

        let mut hits: Vec<SearchHit> = Vec::with_capacity(raw_rows.len().min(k));
        let mut rank: u32 = 0;
        for row in raw_rows {
            // Path glob is the only filter we evaluate in Rust because the
            // semantics differ from SQLite's GLOB (no `/` crossing).
            if let Some(m) = &path_matcher {
                if !m.is_match(&row.workspace_path) {
                    continue;
                }
            }
            rank = rank.saturating_add(1);
            let hit = build_hit(row, rank, &self.index_version, self.snippet_chars)?;
            hits.push(hit);
            if hits.len() >= k {
                break;
            }
        }
        tracing::debug!(rows = hits.len(), "kb-search lexical: search done");
        Ok(hits)
    }

    fn index_version(&self) -> IndexVersion {
        self.index_version.clone()
    }
}

// ── Match-string construction ────────────────────────────────────────────

/// Translate a user-typed query into an FTS5 match string.
///
/// v0.17.0 — trigram-aware redesign (see design §5.5 + plan
/// `docs/superpowers/plans/2026-05-22-korean-trigram-tokenizer.md`
/// Task A5). Originally the FTS5 tokenizer was `trigram` so any term
/// shorter than three Unicode chars had no index entry and would zero
/// out an AND branch. Korean compounds typically split into 2-char
/// eojeols (e.g. `해시 충돌`), so a naive token AND drops the dominant
/// usage pattern.
///
/// V009 (2026-05-28): FTS5 tokenizer 가 trigram → unicode61 + 한국어
/// 형태소 분해 column 로 갱신됨. unicode61 은 trigram 과 달리 최소
/// token 길이 제한이 없어 2자 한국어 morpheme query ('한국', '서울')
/// 가 `tokenized_korean_text` column 경유로 hit 가능. MIN_QUERY_CHARS
/// 를 2 로 낮춰 2자 query 를 통과시킨다 (1자 단독은 여전히 필터).
/// multi-token Korean query 의 OR-combine 분기는 redundant 하나 보존
/// (future 확장성).
///
/// post-v0.17.1 dogfood — `text` column filter (closure of HOTFIXES
/// 2026-05-24 `heading_path_json` 노이즈). The `chunks_fts` virtual
/// table indexes both `heading_path` (the JSON-serialized
/// `chunks.heading_path_json` per V002/V007 triggers) and `text`. The
/// default match expression therefore scopes to the `text` column. The
/// `heading_path` column stays indexed so a user who *wants* heading
/// matching can opt in via raw mode (`'heading_path : foo'`).
///
/// Rules:
///
/// - Raw mode (unchanged): the query is wrapped in a single pair of
///   `'...'` → strip the quotes and pass the inner text through verbatim.
///   The user has explicitly opted into FTS5 syntax (e.g.
///   `'rust AND cargo'`, `'foo*'`, `'heading_path : agent'`). No column
///   scoping is applied — the raw expression is honored as-is.
///
/// - Otherwise build up to two MATCH candidates:
///   1. **whole-phrase**: the entire trimmed input wrapped as one FTS5
///      string literal, *only* if it has ≥2 Unicode chars. FTS5 treats
///      a quoted string with spaces as a phrase match.
///   2. **token AND**: whitespace-split tokens, kept only when each has
///      ≥2 Unicode chars (1-char tokens are dropped).
///
/// - Combine: `(whole) OR (token_and)` when both exist *and differ*;
///   either alone when only one exists; `None` when neither exists
///   (caller short-circuits to `Ok(vec![])`, avoiding an FTS5 syntax
///   error from an empty MATCH).
///
/// - A single-token query (`러스트`, `한국`, `foo`) yields `whole == token_and`
///   → return the bare quoted form so the OR doesn't duplicate.
///
/// - Finally wrap the combined expression in `text : (<expr>)` so the
///   match is scoped to the body column. FTS5's column-filter syntax
///   accepts an arbitrary OR/AND sub-expression inside the parens.
fn build_match_string(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(inner) = strip_single_quotes(trimmed) {
        let inner_trim = inner.trim();
        if inner_trim.is_empty() {
            return None;
        }
        return Some(inner_trim.to_string());
    }

    // V009 unicode61: minimum query token length is 2 Unicode chars.
    // (V007 trigram required ≥3; unicode61 has no built-in minimum but
    // single-char queries are too broad to be useful.)
    const MIN_QUERY_CHARS: usize = 2;

    let whole_candidate: Option<String> =
        (trimmed.chars().count() >= MIN_QUERY_CHARS).then(|| escape_fts5_token(trimmed));

    let token_and_candidate: Option<String> = {
        let toks: Vec<String> = trimmed
            .split_whitespace()
            .filter(|t| t.chars().count() >= MIN_QUERY_CHARS)
            .map(escape_fts5_token)
            .collect();
        (!toks.is_empty()).then(|| toks.join(" "))
    };

    let expression = match (whole_candidate, token_and_candidate) {
        (None, None) => return None,
        (Some(w), None) => w,
        (None, Some(a)) => a,
        (Some(w), Some(a)) if w == a => w,
        (Some(w), Some(a)) => format!("({w}) OR ({a})"),
    };
    Some(format!("text : ({expression})"))
}

/// Return `Some(inner)` if `s` is wrapped in a matching pair of single
/// quotes (`'...'`), otherwise `None`. We require the closing quote to
/// be the last character so `'foo' bar` doesn't accidentally engage
/// raw-FTS5 mode.
fn strip_single_quotes(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'' {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

/// FTS5-escape one token by wrapping it in double quotes (FTS5 string
/// literal). Inner `"` are escaped by doubling per FTS5 grammar. This is
/// the simple-and-safe approach that defangs every special character —
/// `(`, `)`, `*`, `^`, `:`, `"`, etc. — without trying to parse FTS5
/// expressions.
fn escape_fts5_token(tok: &str) -> String {
    let mut out = String::with_capacity(tok.len() + 2);
    out.push('"');
    for ch in tok.chars() {
        if ch == '"' {
            out.push('"');
            out.push('"');
        } else {
            out.push(ch);
        }
    }
    out.push('"');
    out
}

// ── SQL execution ────────────────────────────────────────────────────────

/// Raw row shape mirroring the columns selected by [`run_query`]. Kept
/// internal — every public path constructs `SearchHit` from this.
struct RawRow {
    chunk_id: String,
    doc_id: String,
    bm25_raw: f64,
    snippet: String,
    heading_path_json: String,
    section_label: Option<String>,
    source_spans_json: String,
    chunker_version: String,
    workspace_path: String,
    /// p9-fb-32: documents.updated_at (RFC3339).
    updated_at: String,
}

/// Build + execute the FTS5 query. The SQL pattern is the one documented
/// in `tasks/p2/p2-2-lexical-retriever.md` (§Behavior contract).
fn run_query(
    conn: &Connection,
    match_str: &str,
    snippet_words: usize,
    filters: &SearchFilters,
    fetch_limit: usize,
) -> Result<Vec<RawRow>> {
    // Build the dynamic SQL + positional parameter vector. Positional `?`
    // is used (not named bindings) because the dynamic IN-list for
    // `tags_any` is most natural with `params_from_iter`.
    let mut sql = String::from(
        "SELECT \
            f.chunk_id, f.doc_id, \
            bm25(chunks_fts) AS score, \
            snippet(chunks_fts, 3, '', '', '…', ?) AS snippet, \
            c.heading_path_json, c.section_label, c.source_spans_json, \
            c.chunker_version, \
            d.workspace_path, \
            d.updated_at \
         FROM chunks_fts f \
         JOIN chunks c    ON c.chunk_id = f.chunk_id \
         JOIN documents d ON d.doc_id = f.doc_id",
    );

    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    // 1) snippet word count.
    params.push(Box::new(snippet_words as i64));
    // 2) MATCH expression.
    sql.push_str(" WHERE chunks_fts MATCH ?");
    params.push(Box::new(match_str.to_owned()));

    // tags_any: doc must own at least one of the requested tags.
    if !filters.tags_any.is_empty() {
        sql.push_str(" AND f.doc_id IN (SELECT doc_id FROM document_tags WHERE tag IN (");
        for (i, tag) in filters.tags_any.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            sql.push('?');
            params.push(Box::new(tag.clone()));
        }
        sql.push_str("))");
    }
    if let Some(lang) = &filters.lang {
        sql.push_str(" AND d.lang = ?");
        params.push(Box::new(lang.0.clone()));
    }
    if let Some(trust_min) = &filters.trust_min {
        // Mirror `kebab_store_sqlite::documents::list_documents` ranking:
        // Generated < Secondary < Primary. Doing the rank in SQL
        // (rather than post-filtering) keeps the row stream short
        // when the workspace contains many low-trust docs.
        sql.push_str(
            " AND CASE d.trust_level \
                WHEN 'primary'   THEN 3 \
                WHEN 'secondary' THEN 2 \
                WHEN 'generated' THEN 1 \
                ELSE 0 \
              END >= ?",
        );
        let rank: i64 = match trust_min {
            TrustLevel::Primary => 3,
            TrustLevel::Secondary => 2,
            TrustLevel::Generated => 1,
        };
        params.push(Box::new(rank));
    }
    // p9-fb-36: media_type filter (IN-list).
    // `assets.media_type` JSON has two shapes:
    //   - unit variant (Markdown / Pdf): JSON text, e.g. `"markdown"`
    //   - tuple variant (Image(Png) / Audio(Mp3) / Other(s)): JSON object,
    //     e.g. `{"image": "png"}`
    // Extract a unified "kind" string for both shapes via:
    //   CASE WHEN json_type = 'text' THEN json_extract($)
    //        ELSE (first object key)
    //   END IN (?, ...)
    if !filters.media.is_empty() {
        let placeholders: Vec<&str> = std::iter::repeat_n("?", filters.media.len()).collect();
        let placeholders = placeholders.join(",");
        sql.push_str(&format!(
            " AND f.doc_id IN (\
               SELECT d2.doc_id FROM documents d2 \
               JOIN assets a ON a.asset_id = d2.asset_id \
               WHERE CASE \
                 WHEN json_type(a.media_type) = 'text' THEN json_extract(a.media_type, '$') \
                 ELSE (SELECT key FROM json_each(a.media_type) LIMIT 1) \
               END IN ({placeholders}))"
        ));
        for kind in &filters.media {
            params.push(Box::new(kind.clone()));
        }
    }

    // p10-1A-1 fix (dogfood-discovered 2026-05-20): code_lang filter
    // (IN-list on metadata_json.$.code_lang). Empty Vec = no filter.
    if !filters.code_lang.is_empty() {
        let placeholders = std::iter::repeat_n("?", filters.code_lang.len())
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(
            " AND json_extract(d.metadata_json, '$.code_lang') IN ({placeholders})"
        ));
        for lang in &filters.code_lang {
            params.push(Box::new(lang.clone()));
        }
    }

    // p10-1A-1 fix (dogfood-discovered 2026-05-20): repo filter
    // (IN-list on metadata_json.$.repo). Empty Vec = no filter.
    if !filters.repo.is_empty() {
        let placeholders = std::iter::repeat_n("?", filters.repo.len())
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(
            " AND json_extract(d.metadata_json, '$.repo') IN ({placeholders})"
        ));
        for repo in &filters.repo {
            params.push(Box::new(repo.clone()));
        }
    }

    // p9-fb-36: ingested_after filter.
    // `documents.updated_at` is RFC3339 stored as TEXT (always UTC `Z` per
    // fb-32 ingest path), so lexicographic >= compare is correct — but only
    // when the filter instant is also formatted as UTC `Z`. A non-UTC offset
    // (e.g. `+09:00`) would compare as ASCII after `Z` (0x2B < 0x5A) and
    // produce wrong results. Convert to UTC before formatting.
    if let Some(after) = &filters.ingested_after {
        let formatted = after
            .to_offset(time::UtcOffset::UTC)
            .format(&time::format_description::well_known::Rfc3339)
            .expect("OffsetDateTime (UTC) formats to RFC3339");
        sql.push_str(" AND d.updated_at >= ?");
        params.push(Box::new(formatted));
    }

    // p9-fb-36: doc_id filter — single-doc scoping.
    if let Some(id) = &filters.doc_id {
        sql.push_str(" AND d.doc_id = ?");
        params.push(Box::new(id.0.clone()));
    }

    // path_glob is intentionally NOT applied here — see module comment
    // on PATH_GLOB_OVERFETCH and the post-filter in `LexicalRetriever::search`.

    // Determinism: tie-break on chunk_id so equal bm25 scores produce a
    // stable order across runs. `f.chunk_id` is the FTS row's UNINDEXED
    // copy of the same value as `c.chunk_id`; either side works.
    sql.push_str(" ORDER BY score, f.chunk_id LIMIT ?");
    params.push(Box::new(i64::try_from(fetch_limit).unwrap_or(i64::MAX)));

    let mut stmt = conn
        .prepare(&sql)
        .context("kb-search lexical: prepare FTS5 statement")?;
    let rows = stmt
        .query_map(
            params_from_iter(params.iter().map(std::convert::AsRef::as_ref)),
            row_from_sql,
        )
        .context("kb-search lexical: execute FTS5 query")?;
    let mut out: Vec<RawRow> = Vec::new();
    for r in rows {
        out.push(r.context("kb-search lexical: read row")?);
    }
    Ok(out)
}

fn row_from_sql(row: &Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        chunk_id: row.get(0)?,
        doc_id: row.get(1)?,
        bm25_raw: row.get(2)?,
        snippet: row.get(3)?,
        heading_path_json: row.get(4)?,
        section_label: row.get(5)?,
        source_spans_json: row.get(6)?,
        chunker_version: row.get(7)?,
        workspace_path: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

// ── Hit construction ─────────────────────────────────────────────────────

fn build_hit(
    raw: RawRow,
    rank: u32,
    index_version: &IndexVersion,
    snippet_chars: usize,
) -> Result<SearchHit> {
    let normalized = normalize_bm25(raw.bm25_raw);
    let heading_path: Vec<String> = serde_json::from_str(&raw.heading_path_json)
        .context("kb-search lexical: deserialize heading_path_json")?;
    let source_spans: Vec<SourceSpan> = serde_json::from_str(&raw.source_spans_json)
        .context("kb-search lexical: deserialize source_spans_json")?;

    let workspace_path = WorkspacePath::new(raw.workspace_path)
        .context("kb-search lexical: documents.workspace_path violates WorkspacePath invariant")?;

    let citation = citation_from_first_span(
        &raw.chunk_id,
        workspace_path.clone(),
        raw.section_label.clone(),
        source_spans.first(),
    );

    // FTS5's snippet() respects the word budget but produces a
    // character-length we can't predict precisely (token boundaries vary
    // with the tokenizer). The contract caps at `snippet_chars`; trim
    // defensively if SQLite ever returns a longer string.
    let snippet = trim_snippet(&raw.snippet, snippet_chars);

    // p9-fb-32: documents.updated_at is stored as RFC3339 TEXT (V001
    // migration; written by put_document via OffsetDateTime::now_utc).
    // fb-23 incremental ingest's skip path does not call put_document,
    // so this naturally reflects the last actual re-process.
    let indexed_at = time::OffsetDateTime::parse(
        &raw.updated_at,
        &time::format_description::well_known::Rfc3339,
    )
    .context("kb-search lexical: parse documents.updated_at as RFC3339")?;

    Ok(SearchHit {
        rank,
        chunk_id: ChunkId(raw.chunk_id),
        doc_id: DocumentId(raw.doc_id),
        doc_path: workspace_path,
        heading_path,
        section_label: raw.section_label,
        snippet,
        citation,
        retrieval: RetrievalDetail {
            method: SearchMode::Lexical,
            fusion_score: normalized,
            lexical_score: Some(normalized),
            vector_score: None,
            lexical_rank: Some(rank),
            vector_rank: None,
        },
        index_version: index_version.clone(),
        embedding_model: None,
        chunker_version: ChunkerVersion(raw.chunker_version),
        indexed_at,
        // Placeholder — overwritten by `kebab_app::staleness::mark_stale_in_place`
        // (called from `App::search` / `App::search_uncached`) and the equivalent
        // in `RagPipeline::ask` against the configured threshold.
        stale: false,
        score_kind: ScoreKind::Bm25,
        repo: None,
        code_lang: None,
    })
}

/// Map the raw bm25 score (FTS5 returns a *negative* number; lower is
/// better) into a positive score in `(0, 1]`. The formula
/// `score = -bm25 / (1 + |bm25|)` is monotonic, smooth, and bounded —
/// suitable both for human display and for use as an RRF input.
fn normalize_bm25(bm25_raw: f64) -> f32 {
    let abs = bm25_raw.abs();
    let normalized = -bm25_raw / (1.0_f64 + abs);
    normalized as f32
}

/// Cap the snippet at `max_chars` characters (Unicode scalar values, not
/// bytes — matches the §6.4 setting's "characters" semantics). Returns
/// the input unchanged when already short enough.
fn trim_snippet(s: &str, max_chars: usize) -> String {
    // We slice on Unicode scalar values per §6.4's "characters" semantics; this
    // can orphan a combining mark in extreme cases (Hebrew niqqud, Devanagari)
    // but matches the spec's char-budget definition.
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

// ── path_glob ────────────────────────────────────────────────────────────

/// Compile a `path_glob` pattern. We enable `literal_separator` so `*`
/// does NOT cross `/` — design requires `*` to match within a single
/// path segment, not across them. (`globset`'s default is to let `*`
/// span separators.)
fn compile_glob(pattern: &str) -> Result<GlobMatcher> {
    let g = globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .with_context(|| format!("kb-search lexical: invalid path_glob {pattern:?}"))?;
    Ok(g.compile_matcher())
}

// ── Unit tests for pure helpers ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_match_string_empty_returns_none() {
        assert!(build_match_string("").is_none());
        assert!(build_match_string("   ").is_none());
        assert!(build_match_string("''").is_none());
        assert!(build_match_string("'   '").is_none());
    }

    #[test]
    fn build_match_string_default_emits_or_of_phrase_and_and() {
        // Two long tokens: both whole-phrase and token-AND candidates
        // exist and differ, so the builder combines them with OR
        // inside a `text : (...)` column filter (post-v0.17.1 dogfood:
        // text-only scoping to avoid heading_path_json false positives).
        let s = build_match_string("rust cargo").unwrap();
        assert_eq!(s, r#"text : (("rust cargo") OR ("rust" "cargo"))"#);
    }

    #[test]
    fn build_match_string_escapes_special_chars() {
        // `*`, `(`, `)`, `:`, `^`, `"` should all be wrapped inside
        // FTS5 string-literal quotes so they're treated as literal
        // text rather than FTS5 operators. Every token is ≥3 chars,
        // so both the whole-phrase and token-AND candidates exist,
        // wrapped in the `text : (...)` column filter.
        let s = build_match_string(r#"foo* (bar) baz:qux ^head he"llo"#).unwrap();
        assert_eq!(
            s,
            r#"text : (("foo* (bar) baz:qux ^head he""llo") OR ("foo*" "(bar)" "baz:qux" "^head" "he""llo"))"#
        );
        // The doubled `""` is FTS5's way of embedding a literal quote
        // inside a string literal. Appears in both whole-phrase and
        // token-AND halves.
        assert!(s.contains(r#"he""llo"#));
        // Sanity: outermost wrapper is the column filter.
        assert!(s.starts_with("text : ("));
        assert!(s.ends_with(')'));
    }

    #[test]
    fn build_match_string_passthrough_when_single_quoted() {
        // Raw mode bypasses column scoping — the FTS5 expression is
        // preserved verbatim, including any explicit column filter
        // (e.g. `'heading_path : foo'`) the user opts into.
        let s = build_match_string("'foo OR bar*'").unwrap();
        assert_eq!(s, "foo OR bar*");
    }

    /// Raw mode preserves an explicit `heading_path :` column filter
    /// — opt-in path for users who deliberately want heading matching
    /// (post-v0.17.1 dogfood default scopes to `text` only).
    #[test]
    fn build_match_string_raw_mode_preserves_heading_filter() {
        let s = build_match_string("'heading_path : agent'").unwrap();
        assert_eq!(s, "heading_path : agent");
        assert!(!s.starts_with("text : "));
    }

    // ── v0.17.0 trigram-aware redesign coverage ──────────────────────────

    /// V009 unicode61: 1-char query yields None (too broad); 2-char Korean
    /// query now passes the MIN_QUERY_CHARS=2 filter and returns a valid
    /// match expression.
    #[test]
    fn build_match_string_short_korean_returns_none() {
        // 1-char queries remain filtered (too broad).
        assert!(build_match_string("키").is_none());
        assert!(build_match_string("나").is_none());
        // 2-char Korean queries now produce a valid expression (V009 unicode61).
        assert_eq!(build_match_string("충돌").unwrap(), r#"text : ("충돌")"#);
        assert_eq!(build_match_string(" 충돌 ").unwrap(), r#"text : ("충돌")"#);
    }

    /// V009 unicode61: `해시 충돌` — both tokens are 2 chars and now pass
    /// MIN_QUERY_CHARS=2. Both whole-phrase and token-AND candidates exist
    /// and differ → OR-combined inside `text : (...)`.
    #[test]
    fn build_match_string_whole_phrase_only_when_all_tokens_short() {
        let s = build_match_string("해시 충돌").unwrap();
        assert_eq!(s, r#"text : (("해시 충돌") OR ("해시" "충돌"))"#);
    }

    /// Single long token: whole-phrase and token-AND candidates collapse
    /// to the same string. The builder returns the bare quoted form so
    /// the MATCH expression doesn't carry a redundant `(x) OR (x)`,
    /// wrapped in `text : (...)`.
    #[test]
    fn build_match_string_single_long_token_no_duplicate_or() {
        assert_eq!(
            build_match_string("러스트").unwrap(),
            r#"text : ("러스트")"#
        );
        assert_eq!(build_match_string("rust").unwrap(), r#"text : ("rust")"#);
    }

    /// Mixed Korean+English multi-token query where every token is ≥3
    /// chars: both candidates exist and differ, OR-combined inside
    /// `text : (...)`.
    #[test]
    fn build_match_string_mixed_lang_emits_or_of_phrase_and_and() {
        let s = build_match_string("Rust 충돌은").unwrap();
        assert_eq!(s, r#"text : (("Rust 충돌은") OR ("Rust" "충돌은"))"#);
    }

    /// One ≥3 token + one <3 token: short token is dropped from the
    /// AND, leaving a single long token there; whole-phrase exists
    /// independently. Both candidates differ → OR-combined inside
    /// `text : (...)`.
    #[test]
    fn build_match_string_drops_short_token_in_and_keeps_whole() {
        // "키" (1 char) dropped from AND; "해시테이블" (5 chars) kept.
        // Whole phrase "키 해시테이블" (7 chars) keeps the short token.
        let s = build_match_string("키 해시테이블").unwrap();
        assert_eq!(s, r#"text : (("키 해시테이블") OR ("해시테이블"))"#);
    }

    #[test]
    fn normalize_bm25_top_score_in_unit_interval() {
        // A "perfect" hit is bm25 = -1.0 → normalized 0.5.
        // A high-relevance hit (bm25 = -10.0) → 10/11 ≈ 0.909.
        let high = normalize_bm25(-10.0);
        assert!(high > 0.0 && high <= 1.0, "got {high}");
        let medium = normalize_bm25(-1.0);
        assert!((medium - 0.5).abs() < 1e-6);
    }

    #[test]
    fn normalize_bm25_monotonic() {
        // Lower (more-negative) bm25 must map to a higher normalized score.
        let a = normalize_bm25(-2.0);
        let b = normalize_bm25(-1.0);
        assert!(a > b, "{a} should exceed {b}");
    }

    #[test]
    fn trim_snippet_caps_at_char_count() {
        let s = "a".repeat(300);
        let trimmed = trim_snippet(&s, 220);
        assert_eq!(trimmed.chars().count(), 220);
    }

    #[test]
    fn trim_snippet_passthrough_when_short() {
        let s = "short";
        assert_eq!(trim_snippet(s, 220), "short");
    }

    #[test]
    fn build_citation_line_round_trip() {
        use kebab_core::Citation;
        let p = WorkspacePath::new("a/b.md".to_string()).unwrap();
        let span = SourceSpan::Line { start: 7, end: 12 };
        let c = citation_from_first_span("c1", p.clone(), Some("S1".to_string()), Some(&span));
        match c {
            Citation::Line {
                start,
                end,
                ref section,
                path: ref pp,
            } => {
                assert_eq!(start, 7);
                assert_eq!(end, 12);
                assert_eq!(section.as_deref(), Some("S1"));
                assert_eq!(pp, &p);
            }
            other => panic!("expected Citation::Line, got {other:?}"),
        }
    }

    #[test]
    fn build_citation_page_forwards_section() {
        use kebab_core::Citation;
        let p = WorkspacePath::new("doc.pdf".to_string()).unwrap();
        let span = SourceSpan::Page {
            page: 4,
            char_start: None,
            char_end: None,
        };
        let c = citation_from_first_span("c1", p, Some("Intro".to_string()), Some(&span));
        match c {
            Citation::Page {
                page, ref section, ..
            } => {
                assert_eq!(page, 4);
                assert_eq!(section.as_deref(), Some("Intro"));
            }
            other => panic!("expected Citation::Page, got {other:?}"),
        }
    }

    #[test]
    fn build_citation_none_falls_back_to_line_one() {
        use kebab_core::Citation;
        let p = WorkspacePath::new("x.md".to_string()).unwrap();
        let c = citation_from_first_span("c1", p, None, None);
        match c {
            Citation::Line { start, end, .. } => {
                assert_eq!((start, end), (1, 1));
            }
            other => panic!("expected fallback Citation::Line, got {other:?}"),
        }
    }

    #[test]
    fn compile_glob_rejects_invalid_pattern() {
        // `[` is a character-class opener; an unclosed class is invalid.
        let r = compile_glob("notes/[abc");
        assert!(r.is_err());
    }

    #[test]
    fn compile_glob_star_does_not_cross_slash() {
        // This is the design invariant: `*` must NOT match `/`.
        let m = compile_glob("notes/*.md").unwrap();
        assert!(m.is_match("notes/foo.md"));
        assert!(!m.is_match("notes/sub/foo.md"));
    }
}
