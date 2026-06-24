---
title: "Embedding cache everywhere + OCR/caption derivation cache"
created: 2026-06-24
status: draft
extends: docs/superpowers/specs/2026-05-31-derivation-cache-design.md
contract_sections: ["§2 RAG (embedding)", "§3 derivation cache", "§6 parse/chunk (OCR/caption)", "§9 versioning"]
---

# Embedding cache everywhere + OCR/caption derivation cache

The derivation cache (V012, `2026-05-31-derivation-cache-design.md` §3) keys expensive
ingest derivations by **content hash + version_key**, so a re-ingest recomputes only the
derivations whose input actually changed. Today only **markdown chunk embedding** uses it.
This spec closes two gaps, as two PRs, **without changing user-visible output** (byte-identical
ingest is the HARD GATE per PR).

## 1. Motivation

**(a) The embedding cache is markdown-only.** Markdown embeds via `embed_with_cache`
(`crates/kebab-app/src/ingest.rs:1010-1063`, wired at `1379-1387`); image/PDF/code handlers
call `emb.embed(&inputs)` **directly** (`ingest.rs:1744`, `2383`, `2714`). For those media a
re-ingest re-embeds every chunk even when chunk text is byte-identical. Wiring them into the
same cache gives **consistency** (one code path + one version_key formula, cross-media reuse —
an image-derived chunk and a markdown chunk with identical text share one entry) and **cost**
(vision/PDF/code corpora skip redundant embedding compute on re-scan).

**(b) Vision work (OCR + caption) rides the embedding/chunker version cascade.** OCR text
(`OcrText.joined`) and captions (`ModelCaption.text`) are computed in ingest, then fed into
chunk text. There is **no cache** for the OCR/caption step itself, so any change that triggers
re-chunk/re-embed (chunker_version / embedding_version bumps — design §9) re-runs the expensive
vision LLM / ONNX OCR even though the **source image bytes and OCR/caption version are
unchanged**. The real value:

- **Version-cascade decoupling.** OCR/caption results are keyed on **source bytes + an
  OCR/caption-specific version key**, *not* on chunker/embedding versions. A chunker bump no
  longer re-runs OCR; only an OCR-asset/engine or caption-prompt change invalidates them.
- **Tames residual nondeterminism.** The Ollama-vision path is not bit-deterministic across
  runs. Caching the first result makes re-ingest reproduce the *exact* prior text — which the
  byte-identical parity gate needs for the vision path.

## 2. (a) Embedding cache everywhere — PR1

### Target call sites (replace direct `emb.embed`)

| Handler | Direct-embed site | Function |
|---|---|---|
| Image chunks | `ingest.rs:1744` (block `1737-1746`) | `ingest_one_image_asset` |
| PDF chunks | `ingest.rs:2383` (block `2376-2383`) | `ingest_one_pdf_asset` |
| Code chunks | `ingest.rs:2714` (block `2710-2715`) | `ingest_one_code_asset` |

Markdown (`ingest.rs:1379-1387`) is the reference implementation and is **not changed**.

### Refactor (identical for all three sites)

Replace `let vectors = emb.embed(&inputs)?;` with the markdown pattern: build
`emb_version_key = format!("doc|{}|{}|{}", model_id.0, model_version.0, dimensions)`
(`ingest.rs:1374-1375`), extract `body_texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str())`,
call `embed_with_cache(emb, &app.sqlite, &body_texts, &emb_version_key, &mut hit, &mut miss,
&mut touch_keys)?`, and after the vector upsert run `app.sqlite.derivation_cache_touch(&touch_keys)?`.
`embed_with_cache` is **unchanged** (it already keys via `derivation_cache_key("embedding", …)`,
treats a corrupt payload as a miss, preserves order, batches only misses).

### Shared version_key — single formula, no per-media variation

The embedder is media-agnostic, so the **same chunk text → same key regardless of handler**.
Do **not** introduce per-media version_key variants (would fragment the cache + break cross-media
reuse). The `doc|` prefix reserves space for a future query-embedding path (`ingest.rs:1370-1373`).

### Byte-identical argument

- **Fresh dir:** every key misses → every chunk embedded exactly as today → identical output.
- **Re-ingest (warm):** unchanged text → hit → returns the **same LE-f32 bytes** originally
  `encode_embedding`'d (`derivation_payload.rs:8-14`); `embedding_id` unchanged.
- The parity gate strips `indexed_at`/`stale` only and never inspects cache internals. Cache
  state is not output-affecting; only the vectors are, and they are identical. **PR1 is
  byte-neutral by construction.**

## 3. (b) OCR/caption derivation cache — PR2

### 3.1 Namespaces

Add `"ocr"` (caches `OcrText` for image OCR + PDF page OCR) and `"caption"` (caches
`ModelCaption` for image caption) to the existing `{embedding, alias, korean_tokens}` set
(`derivation.rs:13`). **No schema migration** — the `derivation_cache` table is namespace-agnostic
(`kind TEXT`, `payload BLOB`, V012).

### 3.2 Cache key — bytes-keyed; add `derivation_cache_key_bytes`

OCR/caption inputs are **image/page bytes**, not text. `derivation_cache_key` NFC-normalizes a
`&str` (`derivation.rs:30-41`) — wrong for binary. **Add a byte variant to
`kebab-core/src/derivation.rs`**, same `kind‖0x00‖blake3(content)‖0x00‖version_key` framing,
hashing raw bytes (no NFC), returning the first 32 hex chars:

```rust
pub fn derivation_cache_key_bytes(kind: &str, bytes: &[u8], version_key: &str) -> String {
    let content_blake3 = blake3::hash(bytes).to_hex().to_string();
    let mut hasher = blake3::Hasher::new();
    hasher.update(kind.as_bytes());        hasher.update(&[0x00]);
    hasher.update(content_blake3.as_bytes()); hasher.update(&[0x00]);
    hasher.update(version_key.as_bytes());
    hasher.finalize().to_hex().to_string()[..32].to_string()
}
```

One canonical helper (no inline per-site pre-hashing — avoids the cross-site divergence risk).
Test it like the text variant (32-hex, determinism, kind/version separation).

### 3.3 Version keys

| Namespace | version_key | Source |
|---|---|---|
| `ocr` | `ocr_engine_version_for_sig(engine, model, det, rec, dict)` **+ `\|st:{score_thresh}\|uc:{unclip_ratio}\|mb:{max_boxes}`** | `ingest.rs:3238-3274`; `paddle_onnx.rs:468-507`, `:66-75` |
| `caption` | `format!("caption\|{}\|{}", llm.model_ref().provider, config.ingest.image.caption.prompt_template_version)` | `caption.rs:133-137`; `CaptionCfg` `kebab-config/src/lib.rs:647-659` |

- **OCR** — `score_thresh`/`unclip_ratio`/`max_boxes` (`paddle_onnx.rs:66-75`) shape OCR output
  but are **not** in `ocr_engine_version_for_sig` today. Fold them into the OCR version_key
  **now** (constant today, zero behavior change) so the cache can't serve stale text the moment
  they become user-configurable.
- **Caption** — the ingest *signature* uses only `prompt_template_version` (`ingest.rs:3328-3330`),
  but `ModelCaption.model_version` composes `provider/prompt` (`caption.rs:133-137`). The cache
  key must include **both**, so a provider swap at the same prompt → miss.

### 3.4 Payload encoding — full struct via serde (byte-identical at block level)

`OcrText { joined, regions, engine, engine_version }` and `ModelCaption { text, model,
model_version }` both derive `Serialize`/`Deserialize` (`kebab-core/src/document.rs:161-181`).
Downstream **rendering** consumes only `.joined`/`.text` (`render_block_text` in
`md_heading_v{1,2}.rs`; PDF mutation `pdf_ocr_apply.rs:231,244,249`), **but the full struct is
also written into `CanonicalDocument.blocks[i].ocr`/`.caption`** and persisted with the block.

**Decision: cache the FULL struct (serde, deterministic field order), not just the string.** A
string-only stub (`engine: ""`, `regions: []`) would render identical *chunk* text — so it would
pass the chunks/search/ask parity gate — but the stored **block** would differ from a fresh run
(`block.ocr.engine = ""` vs `"paddle-onnx"`), a latent DB divergence and a trap for any future
consumer or stricter gate. OCR/caption run **per asset/page**, not per chunk, so serde cost is
negligible. Add to `derivation_payload.rs`:

```rust
pub fn encode_ocr_text(o: &OcrText) -> Vec<u8>            // serde (json or bincode), deterministic
pub fn decode_ocr_text(p: &[u8]) -> Option<OcrText>       // decode err → None → treat as miss
pub fn encode_model_caption(c: &ModelCaption) -> Vec<u8>
pub fn decode_model_caption(p: &[u8]) -> Option<ModelCaption>
```

A decode failure is a cache miss → recompute (same accuracy-first contract as `decode_embedding`).
On a cache hit the reconstructed struct is **bit-identical** to a fresh deterministic OCR run, so
both the chunk text AND the stored block match.

### 3.5 Wrap-point refactor — per handler, always at the `kebab-app` call site

Cache wrapping lives in `kebab-app`, **never** inside `kebab-parse-*` (preserves the
`kebab-parse-* ⊄ kebab-store-*` boundary — CLAUDE.md deps rule).

- **b1 — Image OCR** (`ingest.rs:1604-1610`, `apply_ocr(engine,&bytes,block,…,&mut events)`):
  key = `derivation_cache_key_bytes("ocr", &bytes, &ocr_version_key)`. Hit → decode `OcrText`,
  set on block, **skip `apply_ocr`**. Miss → run `apply_ocr`, then `derivation_cache_put(key,
  "ocr", encode_ocr_text(&block.ocr))`. Accumulate hit keys → batched `derivation_cache_touch`.
- **b2 — Image caption** (`ingest.rs:1634-1641`): namespace `"caption"`, same hit/miss/put/touch
  shape. This is the unit that eliminates the vision-LLM nondeterminism on re-ingest.
- **b3 — PDF per-page OCR** (`ingest.rs:2220` → `apply_ocr_to_pdf_pages`, loop
  `pdf_ocr_apply.rs:92-206`): cache the per-page `engine.recognize(&page_image_bytes,…)`
  (`pdf_ocr_apply.rs:175`), keyed on the per-page DCTDecode image bytes (`:148`). Thread an
  **optional cache handle + `ocr_version_key` via `PdfOcrOpts`** — `apply_ocr_to_pdf_pages` lives
  in `kebab-app`, so this pulls **no** dep into `kebab-parse-pdf`. `None` handle ⇒ today's behavior
  (unit tests / store-less callers).

### 3.6 Provenance on cache hit — do NOT replay (decisive)

`ProvenanceEvent.at = OffsetDateTime::now_utc()` is **wall-clock, nondeterministic every run**
(`ocr.rs:101-111`, `caption.rs:188-193`, `pdf_ocr_apply.rs:155-170,187-192,258-271`). Provenance
is persisted (`documents.provenance_json`, `documents.rs:742`) but appears in **no wire schema**
(`search_hit`/`answer`/`chunk_inspection`) and **no chunks column** (`V001__init.sql:80-92`), and
the parity gate strips only `indexed_at`/`stale` (`gate-ingest.sh:32-34,40-41`).

**Decision: on a hit, skip the OCR/caption function entirely and do NOT fabricate/replay a
provenance event.** Byte-identical output is preserved (provenance is in no compared surface;
chunk text from cache is identical). A direct `provenance_json` inspector sees the original run's
event (observationally correct — the OCR genuinely happened then). **Do NOT** add a
`ProvenanceKind::CachedOcrApplied` in PR2 — it would push a fresh nondeterministic `at` for zero
parity benefit; defer unless an audit requirement appears.

## 4. Verification

### 4.1 Markdown parity gate — unchanged HARD GATE
The existing byte-diff markdown gate (`gate-ingest.sh`) stays the hard gate for both PRs. PR1
must keep it green (byte-neutral, §2). PR2 does not touch the markdown path.

### 4.2 Paddle-ONNX deterministic image/PDF byte-diff gate (NEW, with PR2)
Paddle-ONNX OCR is deterministic (CPU ONNX, no sampling) → anchors a byte-diff gate the
nondeterministic ollama-vision path cannot.

- **Assets already bundled — NO download (and no external source).** All three files are
  committed in-repo at `crates/kebab-parse-image/assets/paddleocr-onnx/`:
  `ppocrv5_mobile_det.onnx` (4.7 MB), `korean_ppocrv5_mobile_rec.onnx` (13 MB), `korean_dict.txt`
  (47 KB) — NOTICE Apache-2.0. `ModelPaths::from_default_dir` resolves them from
  `CARGO_MANIFEST_DIR` (`paddle_onnx.rs:105-115`). Engine version = blake3(det‖rec‖dict)
  (`:468-507`), so any asset change auto-invalidates the OCR cache.
- **Gate config (schema v5):** `[ingest.ocr] engine="paddle-onnx"`, `[ingest.image.ocr]
  enabled=true`, `[ingest.pdf.ocr] enabled=true engine="paddle-onnx"`, `[models.embedding]
  provider="none"` (lexical-only, no Ollama). Defaults `score_thresh=0.3`/`unclip_ratio=1.5`/
  `max_boxes=1000`, now folded into the OCR version_key (§3.3).
- **Fixtures (corpus must be created — see R2):** generate in-tree via `gen_smoke_png.rs`
  (`kebab-parse-image/examples`) / `gen_smoke_pdf.rs` (`kebab-parse-pdf/examples`). The crate
  gate (`paddle_e2e.rs:98-104`) expects text-bearing fixtures + `gt.json` and **skips** when
  `KEBAB_TEST_OCR_FIXTURE_DIR` is absent — PR2 MUST commit/regenerate a small deterministic OCR
  corpus into the machine-local dogfood store so the gate runs instead of silently passing.
- **Gate procedure:** ingest twice (cold → warm cache) under the paddle config; byte-diff
  chunks/search/ask (same jq projection as the markdown gate). Cold≡warm proves the OCR/embedding
  caches are output-neutral; a second fresh-dir run proves paddle determinism.

### 4.3 Mock unit tests (no models)
`derivation_cache_key_bytes` (32-hex, determinism, kind/version separation); `encode/decode_*`
round-trip + invalid-bytes → `None` (miss); wrap-point logic with a mock OCR/caption engine +
in-memory `SqliteStore` (miss → engine invoked + put; hit → engine NOT invoked + same struct;
version_key change → miss; assert no provenance fabricated on hit).

### 4.4 GPU-box dogfood (ollama-vision quality, can't byte-gate)
Per CLAUDE.md §Dogfood: swap lemonade→`ollama-r9700` (`.82`), point OCR/caption at
`http://192.168.0.244:11434`, ingest the dogfood image/PDF corpus, confirm re-ingest cache hit
reproduces prior text and search/ask quality is unchanged. **Restore lemonade unconditionally.**
Evidence → `tasks/HOTFIXES.md` + `docs/release-notes/`.

### 4.5 Per-PR HARD GATE
Each PR: ingest output byte-identical vs baseline (markdown gate always; paddle image/PDF gate
from PR2). No PR merges with a parity diff.

## 5. PR split

**PR1 — (a) embedding cache everywhere.** Convert the three direct-embed sites to
`embed_with_cache` + shared `doc|…` version_key + `derivation_cache_touch`. No new namespace/key/
payload. Byte-neutral; markdown gate stays green. Self-contained.

**PR2 — (b) OCR/caption cache**, three reviewable units:
- **core prep (lands with b1):** `derivation_cache_key_bytes` (`kebab-core`),
  `encode/decode_ocr_text` + `encode/decode_model_caption` (`derivation_payload.rs`), unit tests.
- **b1 image OCR** (`ingest.rs:1604`, `"ocr"`), **b2 image caption** (`:1634`, `"caption"`),
  **b3 PDF per-page OCR** (`PdfOcrOpts` cache handle, `"ocr"`, page-bytes key — no
  `kebab-parse-pdf` dep change).
- Provenance NOT replayed on hit (§3.6); paddle deterministic gate (§4.2) added; GPU-box dogfood.

**Version bump (CLAUDE.md §Versioning):** (a) = observability/perf only, no interface change →
**patch**. (b) = new cache namespaces + identical user-visible output, no CLI/wire/migration →
**patch**. Promoting a `[ingest.ocr]` tuning knob to a user-facing key in the same work → **minor**.
Dogfood evidence required before the bump commit either way.

## 6. Risks / open issues

| # | Risk | Mitigation / decision |
|---|---|---|
| R1 | No external paddle-ONNX download source; assets git-committed only; design forbids `hf-hub`. | Keep bundled; "download" step is a no-op. Treat the committed blobs as canonical, never GC. |
| R2 | **Synthetic OCR gate corpus does not exist** (`paddle_e2e.rs` *skips*) → the "deterministic gate" silently passes. | **PR2 MUST** commit/regenerate text-bearing fixtures via `gen_smoke_png/pdf` + `gt.json` and point `KEBAB_TEST_OCR_FIXTURE_DIR` at them. (`gen_smoke_png` emits a solid-color PNG — needs a text-rendering path or real screenshots; resolve in the plan.) |
| R3 | Paddle output params not in OCR signature today. | §3.3: fold `score_thresh`/`unclip_ratio`/`max_boxes` into the OCR version_key now. |
| R4 | PDF cache crossing the parser boundary. | `apply_ocr_to_pdf_pages` is in `kebab-app`; thread the handle via `PdfOcrOpts` only; `kebab-parse-pdf` gains no dep. |
| R5 | Bytes-key divergence across sites. | One canonical `derivation_cache_key_bytes`; all sites call it; unit-tested. |
| R6 | Caption key must include provider, not just prompt. | §3.3: `caption\|{provider}\|{prompt_template_version}`. |
| R7 | Full-struct cache vs deterministic OCR. | OCR deterministic (paddle) → cached struct == fresh struct. ollama-vision: hit skips fresh run → reproducible. Either way byte-identical. |
| R8 | Provenance nondeterminism vs byte-identical. | §3.6: provenance in no wire/gate surface → non-issue. **Tripwire:** if a future wire/gate change adds provenance, re-evaluate the skip-don't-replay assumption. |
| R9 | Config schema v5 freshness; v4→v5 migration. | Verify v4→v5 round-trip during paddle gate config setup (note: the v4→v5 Option-key leak was fixed in PR #214). |
| R10 | Unicode normalization (text path). | Bytes key (§3.2) does not NFC; OCR output feeds NFC-normalized chunk text downstream; OCR engines emit NFC UTF-8. |

**No open TBDs** beyond R2 (gate-corpus creation, resolved in the implementation plan).
