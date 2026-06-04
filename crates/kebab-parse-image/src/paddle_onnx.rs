//! PP-OCRv5 ONNX OCR engine — in-process detection + recognition on the
//! workspace-pinned `ort` (=2.0.0-rc.9), no Python runtime, no oar-ocr
//! production dependency (see crate-level rationale + `assets/paddleocr-onnx/NOTICE`).
//!
//! Pipeline (`recognize`):
//! 1. decode (RGB) + downscale long edge to `max_pixels`
//! 2. det: ImageNet-normalized NCHW → DBNet prob map `[1,1,H,W]` → threshold
//!    0.3 → contours → min-area rect (rotating calipers, pure Rust) →
//!    unclip(ratio 1.5, pure Rust) → boxes
//! 3. crop+rectify: perspective warp each rotated box to a horizontal strip
//! 4. rec: 48×W normalized `(x-0.5)/0.5` → `[1,T,11947]` → CTC greedy decode
//! 5. assemble reading-order `OcrText`
//!
//! ## Confirmed CTC facts (empirically derived in T0a, see
//! `tests/golden/ctc_rec_golden.json` — do NOT re-derive):
//!   * rec classes = 11947 = dict(11945) + blank + space
//!   * index 0       = CTC blank
//!   * index 1..=11945 = `korean_dict.txt` line N → class N (i.e. `dict[N-1]`)
//!   * index 11946   = space ' '
//!
//! ## rc.9 API notes (differ from rc.12):
//!   * `try_extract_tensor::<f32>()` → `ArrayViewD<f32>` (`.shape()` / indexing).
//!   * `Session::run` is called through a `Mutex` guard so the engine is
//!     `Send + Sync` regardless of `Session`'s own auto-trait status (ingest
//!     is serial today; the lock is uncontended).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use kebab_core::{Lang, OcrRegion, OcrText};
use ndarray::Array4;
use ort::session::Session;
use ort::value::Value;

use crate::ocr::OcrEngine;

/// Engine name written into `OcrText.engine`.
pub const PADDLE_ONNX_ENGINE: &str = "paddle-onnx";

/// CTC blank class index (confirmed in T0a).
const CTC_BLANK: usize = 0;
/// Space class index (confirmed in T0a). `1..=DICT_LINES` map to dict entries.
const CTC_SPACE: usize = 11946;
/// `korean_dict.txt` line count (confirmed in T0a).
const DICT_LINES: usize = 11945;
/// rec output class count = dict + blank + space (confirmed in T0a).
const REC_CLASSES: usize = 11947;

/// det long-edge cap before rounding to a multiple of 32 (PaddleOCR default).
const DET_LIMIT_SIDE_LEN: u32 = 960;
/// rec input height (PP-OCRv5 mobile).
const REC_HEIGHT: u32 = 48;

/// ImageNet normalization (det preprocessing — RGB).
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// PP-OCRv5 ONNX engine. Holds the two ONNX sessions (loaded once) and the
/// dict. `engine_version` is computed once at construction (blake3 over the
/// three model assets) and cached — `ingest_config_signature` calls
/// `engine_version()` per asset, so re-hashing there would be O(assets).
pub struct OnnxPaddleOcr {
    det: Mutex<Session>,
    rec: Mutex<Session>,
    det_input_name: String,
    rec_input_name: String,
    dict: Vec<String>,
    engine_version: String,
    score_thresh: f32,
    unclip_ratio: f32,
    max_boxes: usize,
    max_pixels: u32,
}

impl std::fmt::Debug for OnnxPaddleOcr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxPaddleOcr")
            .field("engine_version", &self.engine_version)
            .field("dict_lines", &self.dict.len())
            .field("score_thresh", &self.score_thresh)
            .field("unclip_ratio", &self.unclip_ratio)
            .field("max_boxes", &self.max_boxes)
            .field("max_pixels", &self.max_pixels)
            .finish_non_exhaustive()
    }
}

/// Resolved model-asset paths. Construction is decoupled from `kebab-config`
/// (T7 adds the `det_model`/`rec_model`/`dict` overrides) so the engine can be
/// built directly in tests.
#[derive(Clone, Debug)]
pub struct ModelPaths {
    pub det: PathBuf,
    pub rec: PathBuf,
    pub dict: PathBuf,
}

impl ModelPaths {
    /// Default bundled-asset directory: `KEBAB_IMAGE_OCR_MODEL_DIR` if set,
    /// else the crate's `assets/paddleocr-onnx/`.
    pub fn from_default_dir() -> Self {
        let dir = std::env::var("KEBAB_IMAGE_OCR_MODEL_DIR").map_or_else(
            |_| Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/paddleocr-onnx"),
            PathBuf::from,
        );
        Self {
            det: dir.join("ppocrv5_mobile_det.onnx"),
            rec: dir.join("korean_ppocrv5_mobile_rec.onnx"),
            dict: dir.join("korean_dict.txt"),
        }
    }

    /// Resolve model paths from the `image.ocr` config (T7). Each of
    /// `det_model` / `rec_model` / `dict` overrides the corresponding bundled
    /// path when set; unset fields fall back to [`from_default_dir`], so a
    /// caller can override just one asset.
    ///
    /// [`from_default_dir`]: ModelPaths::from_default_dir
    pub fn from_config(config: &kebab_config::Config) -> Self {
        let defaults = Self::from_default_dir();
        let ocr = &config.image.ocr;
        Self {
            det: ocr.det_model.as_ref().map(PathBuf::from).unwrap_or(defaults.det),
            rec: ocr.rec_model.as_ref().map(PathBuf::from).unwrap_or(defaults.rec),
            dict: ocr.dict.as_ref().map(PathBuf::from).unwrap_or(defaults.dict),
        }
    }
}

impl OnnxPaddleOcr {
    /// Build from a workspace [`kebab_config::Config`]. Resolves model paths
    /// from the default bundled directory (T7 will thread config overrides).
    /// Construction loads both ONNX sessions and hashes the assets — failures
    /// here are fail-fast (matches the Ollama adapter's construction contract).
    pub fn new(config: &kebab_config::Config) -> Result<Self> {
        let paths = ModelPaths::from_config(config);
        let ocr = &config.image.ocr;
        Self::from_paths(
            &paths,
            ocr.score_thresh,
            ocr.unclip_ratio,
            ocr.max_boxes,
            ocr.max_pixels,
        )
    }

    /// Build from explicit asset paths + tuning knobs. Used by tests and by
    /// `new` after path resolution.
    pub fn from_paths(
        paths: &ModelPaths,
        score_thresh: f32,
        unclip_ratio: f32,
        max_boxes: usize,
        max_pixels: u32,
    ) -> Result<Self> {
        let dict = load_dict(&paths.dict)
            .with_context(|| format!("loading OCR dict from {}", paths.dict.display()))?;
        // bounds-check: dict length must match the rec class layout
        // (dict + blank + space). A mismatch means a wrong dict file —
        // fail at construction rather than mis-decoding silently.
        if dict.len() != DICT_LINES {
            anyhow::bail!(
                "OnnxPaddleOcr: dict has {} lines, expected {DICT_LINES} \
                 (rec classes {REC_CLASSES} = dict + blank + space)",
                dict.len()
            );
        }

        let engine_version = compute_engine_version(paths)
            .context("hashing OCR model assets for engine_version")?;

        let det = Session::builder()
            .context("ort Session::builder (det)")?
            .commit_from_file(&paths.det)
            .with_context(|| format!("loading det model {}", paths.det.display()))?;
        let rec = Session::builder()
            .context("ort Session::builder (rec)")?
            .commit_from_file(&paths.rec)
            .with_context(|| format!("loading rec model {}", paths.rec.display()))?;

        let det_input_name = det
            .inputs
            .first()
            .map(|i| i.name.clone())
            .context("det model has no inputs")?;
        let rec_input_name = rec
            .inputs
            .first()
            .map(|i| i.name.clone())
            .context("rec model has no inputs")?;

        Ok(Self {
            det: Mutex::new(det),
            rec: Mutex::new(rec),
            det_input_name,
            rec_input_name,
            dict,
            engine_version,
            score_thresh,
            unclip_ratio,
            max_boxes,
            max_pixels: max_pixels.clamp(256, 4096),
        })
    }

    /// Map a CTC class index to its output string. `None` for blank.
    /// `index 0 = blank`, `1..=11945 = dict[index-1]`, `11946 = space`.
    fn class_to_str(&self, idx: usize) -> Option<&str> {
        match idx {
            CTC_BLANK => None,
            CTC_SPACE => Some(" "),
            i if (1..=DICT_LINES).contains(&i) => Some(self.dict[i - 1].as_str()),
            _ => None, // out-of-range guard (should not happen for 11947 classes)
        }
    }
}

impl OcrEngine for OnnxPaddleOcr {
    fn engine_name(&self) -> &'static str {
        PADDLE_ONNX_ENGINE
    }

    fn engine_version(&self) -> String {
        self.engine_version.clone()
    }

    // The trait method's elided lifetime ties the return to `&self`; the body
    // returns a literal, but the signature must match the trait, so allow the
    // `'static`-narrowing lint here.
    #[allow(clippy::unnecessary_literal_bound)]
    fn model(&self) -> &str {
        // Static label for the progress display; the per-asset hash lives
        // in `engine_version`.
        "ppocrv5-mobile-kor"
    }

    fn recognize(&self, image_bytes: &[u8], _lang_hint: Option<&Lang>) -> Result<OcrText> {
        let img = image::load_from_memory(image_bytes)
            .context("decoding image for OCR")?
            .to_rgb8();
        let (orig_w, orig_h) = (img.width(), img.height());
        if orig_w == 0 || orig_h == 0 {
            return Ok(empty_ocr(self));
        }

        // ── det ────────────────────────────────────────────────────────
        let (det_w, det_h) = det_target_dims(orig_w, orig_h, self.max_pixels);
        let det_img = image::imageops::resize(
            &img,
            det_w,
            det_h,
            image::imageops::FilterType::Triangle,
        );
        let prob = self.run_det(&det_img)?; // (det_h, det_w) prob map
        let scale_x = orig_w as f32 / det_w as f32;
        let scale_y = orig_h as f32 / det_h as f32;
        let mut boxes = det_postprocess(
            &prob,
            prob.w,
            prob.h,
            self.score_thresh,
            self.unclip_ratio,
        );
        if boxes.len() > self.max_boxes {
            tracing::warn!(
                target: "kebab-parse-image",
                "paddle-onnx: {} boxes exceeds max_boxes {} — truncating",
                boxes.len(),
                self.max_boxes
            );
            boxes.truncate(self.max_boxes);
        }
        // scale box corners back to original image coordinates
        for b in &mut boxes {
            for p in &mut b.corners {
                p.0 *= scale_x;
                p.1 *= scale_y;
            }
        }

        if boxes.is_empty() {
            return Ok(empty_ocr(self));
        }

        // ── rec per box (reading order: top→bottom, left→right) ─────────
        boxes.sort_by(|a, b| {
            let ay = a.center_y();
            let by = b.center_y();
            // group into rough rows by 0.5*box height tolerance via y then x
            ay.partial_cmp(&by)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.center_x()
                        .partial_cmp(&b.center_x())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let mut regions: Vec<OcrRegion> = Vec::with_capacity(boxes.len());
        for b in &boxes {
            let crop = rectify_crop(&img, &b.corners);
            if crop.width() == 0 || crop.height() == 0 {
                continue;
            }
            let (text, conf) = self.run_rec(&crop)?;
            if text.is_empty() {
                continue; // rec empty → skip this box, keep the rest
            }
            let (x, y, w, h) = b.aabb();
            regions.push(OcrRegion {
                bbox: (x, y, w, h),
                text,
                confidence: conf,
            });
        }

        let joined = regions
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(OcrText {
            joined,
            regions,
            engine: PADDLE_ONNX_ENGINE.to_string(),
            engine_version: self.engine_version.clone(),
        })
    }
}

impl OnnxPaddleOcr {
    /// Run det session → `(det_h, det_w)` probability map as a row-major Vec.
    fn run_det(&self, det_img: &image::RgbImage) -> Result<ProbMap> {
        let (w, h) = (det_img.width() as usize, det_img.height() as usize);
        let mut arr = Array4::<f32>::zeros((1, 3, h, w));
        for (x, y, px) in det_img.enumerate_pixels() {
            let (xi, yi) = (x as usize, y as usize);
            for c in 0..3 {
                let v = f32::from(px[c]) / 255.0;
                arr[[0, c, yi, xi]] = (v - IMAGENET_MEAN[c]) / IMAGENET_STD[c];
            }
        }
        let input = Value::from_array(arr).context("det Value::from_array")?;
        let sess = self.det.lock().expect("det session mutex poisoned");
        let outputs = sess
            .run(ort::inputs![self.det_input_name.as_str() => input]?)
            .context("det session run")?;
        let out_name = sess.outputs[0].name.clone();
        let view = outputs[out_name.as_str()]
            .try_extract_tensor::<f32>()
            .context("det output extract")?;
        // shape [1,1,H,W]
        let shape = view.shape();
        let (oh, ow) = (shape[shape.len() - 2], shape[shape.len() - 1]);
        let data: Vec<f32> = view.iter().copied().collect();
        Ok(ProbMap { w: ow, h: oh, data })
    }

    /// Run rec session on a rectified crop → (decoded string, mean confidence).
    fn run_rec(&self, crop: &image::RgbImage) -> Result<(String, f32)> {
        // resize keep-aspect to height 48, then this single crop is its own batch
        let (cw, ch) = (crop.width().max(1), crop.height().max(1));
        let new_w = ((REC_HEIGHT as f32 / ch as f32) * cw as f32).round().max(1.0) as u32;
        let resized = image::imageops::resize(
            crop,
            new_w,
            REC_HEIGHT,
            image::imageops::FilterType::Triangle,
        );
        let w = new_w as usize;
        let h = REC_HEIGHT as usize;
        let mut arr = Array4::<f32>::zeros((1, 3, h, w));
        for (x, y, px) in resized.enumerate_pixels() {
            let (xi, yi) = (x as usize, y as usize);
            for c in 0..3 {
                let v = f32::from(px[c]) / 255.0;
                arr[[0, c, yi, xi]] = (v - 0.5) / 0.5; // [-1, 1]
            }
        }
        let input = Value::from_array(arr).context("rec Value::from_array")?;
        let sess = self.rec.lock().expect("rec session mutex poisoned");
        let outputs = sess
            .run(ort::inputs![self.rec_input_name.as_str() => input]?)
            .context("rec session run")?;
        let out_name = sess.outputs[0].name.clone();
        let view = outputs[out_name.as_str()]
            .try_extract_tensor::<f32>()
            .context("rec output extract")?;
        // shape [1, T, C]
        let shape = view.shape();
        let (t, c) = (shape[shape.len() - 2], shape[shape.len() - 1]);
        if c != REC_CLASSES {
            anyhow::bail!(
                "rec output has {c} classes, expected {REC_CLASSES} \
                 (dict {DICT_LINES} + blank + space)"
            );
        }
        let data: Vec<f32> = view.iter().copied().collect();
        Ok(self.ctc_greedy_decode(&data, t, c))
    }

    /// CTC greedy decode over `[T, C]` logits/probs (row-major). Per timestep
    /// argmax → collapse consecutive duplicates → drop blank → map class→str.
    fn ctc_greedy_decode(&self, data: &[f32], t: usize, c: usize) -> (String, f32) {
        let mut out = String::new();
        let mut confs: Vec<f32> = Vec::new();
        let mut prev = usize::MAX;
        for ti in 0..t {
            let row = &data[ti * c..(ti + 1) * c];
            let mut best = 0usize;
            let mut best_v = f32::MIN;
            for (i, &v) in row.iter().enumerate() {
                if v > best_v {
                    best_v = v;
                    best = i;
                }
            }
            if best != prev && best != CTC_BLANK {
                if let Some(s) = self.class_to_str(best) {
                    out.push_str(s);
                    confs.push(best_v);
                }
            }
            prev = best;
        }
        let conf = if confs.is_empty() {
            0.0
        } else {
            confs.iter().sum::<f32>() / confs.len() as f32
        };
        (out, conf)
    }
}

fn empty_ocr(e: &OnnxPaddleOcr) -> OcrText {
    OcrText {
        joined: String::new(),
        regions: Vec::new(),
        engine: PADDLE_ONNX_ENGINE.to_string(),
        engine_version: e.engine_version.clone(),
    }
}

/// Load the dict file: one token per line, trailing newline tolerated.
/// Empty lines are preserved as empty tokens (PaddleOCR dicts may carry a
/// blank-looking line; index integrity matters more than trimming).
fn load_dict(path: &Path) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path)?;
    // split on '\n'; drop a single trailing empty element from the final newline
    let mut lines: Vec<String> = raw.split('\n').map(|s| s.trim_end_matches('\r').to_string()).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    Ok(lines)
}

/// Resolve the paddle-onnx `engine_version` for `config` without loading the
/// ONNX sessions (T9). This is the same blake3-over-assets string that a
/// constructed [`OnnxPaddleOcr`] exposes via [`OcrEngine::engine_version`], so
/// the ingest config signature can include it. Reads ~17 MB of model bytes —
/// callers MUST memoize per (det,rec,dict) triple (m3: never re-hash per asset).
pub fn engine_version_for_config(config: &kebab_config::Config) -> Result<String> {
    compute_engine_version(&ModelPaths::from_config(config))
}

/// blake3 over det + rec + dict bytes → stable `engine_version`.
fn compute_engine_version(paths: &ModelPaths) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    for p in [&paths.det, &paths.rec, &paths.dict] {
        let bytes = std::fs::read(p).with_context(|| format!("reading {}", p.display()))?;
        hasher.update(&bytes);
    }
    let hash = hasher.finalize();
    let hex = hash.to_hex();
    Ok(format!("ppocrv5-mobile-kor-{}", &hex.as_str()[..12]))
}

/// det resize target: keep aspect, cap long edge at `min(max_pixels, 960)`,
/// then round each dim to a multiple of 32 (DBNet stride). Reproduces the T0a
/// golden (192×900 → 192×896).
fn det_target_dims(w: u32, h: u32, max_pixels: u32) -> (u32, u32) {
    let limit = DET_LIMIT_SIDE_LEN.min(max_pixels.max(32));
    let long = w.max(h);
    let ratio = if long > limit {
        limit as f32 / long as f32
    } else {
        1.0
    };
    let rw = (w as f32 * ratio).round().max(1.0);
    let rh = (h as f32 * ratio).round().max(1.0);
    let round32 = |v: f32| -> u32 {
        let r = (v / 32.0).round() as u32 * 32;
        r.max(32)
    };
    (round32(rw), round32(rh))
}

// ── det postprocessing ──────────────────────────────────────────────────────

struct ProbMap {
    w: usize,
    h: usize,
    data: Vec<f32>,
}

impl ProbMap {
    #[inline]
    fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }
}

/// A detected text box: 4 corners (clockwise from top-left) in det-image
/// coordinates (later scaled to original).
#[derive(Clone, Debug)]
struct DetBox {
    corners: [(f32, f32); 4],
    #[allow(dead_code)]
    score: f32,
}

impl DetBox {
    fn center_x(&self) -> f32 {
        self.corners.iter().map(|p| p.0).sum::<f32>() / 4.0
    }
    fn center_y(&self) -> f32 {
        self.corners.iter().map(|p| p.1).sum::<f32>() / 4.0
    }
    /// Axis-aligned bounding box (x, y, w, h) clamped to non-negative.
    fn aabb(&self) -> (u32, u32, u32, u32) {
        let xs = self.corners.iter().map(|p| p.0);
        let ys = self.corners.iter().map(|p| p.1);
        let minx = xs.clone().fold(f32::MAX, f32::min).max(0.0);
        let maxx = xs.fold(f32::MIN, f32::max).max(0.0);
        let miny = ys.clone().fold(f32::MAX, f32::min).max(0.0);
        let maxy = ys.fold(f32::MIN, f32::max).max(0.0);
        (
            minx.round() as u32,
            miny.round() as u32,
            (maxx - minx).round().max(0.0) as u32,
            (maxy - miny).round().max(0.0) as u32,
        )
    }
}

/// DBNet-style postprocess: threshold → connected components → contour →
/// min-area rect (rotating calipers) → box-score filter → unclip → boxes.
/// Pinned by `tests/golden/det_boxes_clean_paragraph.json` (3 boxes).
fn det_postprocess(
    prob: &ProbMap,
    w: usize,
    h: usize,
    score_thresh: f32,
    unclip_ratio: f32,
) -> Vec<DetBox> {
    use image::{GrayImage, Luma};

    // binarize at the detection threshold
    let mut bin = GrayImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let v = if prob.at(x, y) > 0.3 { 255u8 } else { 0u8 };
            bin.put_pixel(x as u32, y as u32, Luma([v]));
        }
    }

    let contours = imageproc::contours::find_contours::<u32>(&bin);
    let mut boxes = Vec::new();
    for contour in &contours {
        if contour.points.len() < 4 {
            continue;
        }
        let pts: Vec<(f32, f32)> = contour
            .points
            .iter()
            .map(|p| (p.x as f32, p.y as f32))
            .collect();
        let Some(rect) = min_area_rect(&pts) else {
            continue;
        };
        // mean-prob box score over the AABB of the rotated rect
        let score = box_score(prob, &rect.corners);
        if score < score_thresh {
            continue;
        }
        let unclipped = unclip_rect(&rect, unclip_ratio);
        boxes.push(DetBox {
            corners: unclipped,
            score,
        });
    }
    boxes
}

/// Mean probability inside the axis-aligned bbox of the rect — the
/// `box_thresh` mean-prob filter used by the golden harness.
fn box_score(prob: &ProbMap, corners: &[(f32, f32); 4]) -> f32 {
    let minx = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min).max(0.0) as usize;
    let maxx = (corners.iter().map(|p| p.0).fold(f32::MIN, f32::max).max(0.0) as usize)
        .min(prob.w.saturating_sub(1));
    let miny = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min).max(0.0) as usize;
    let maxy = (corners.iter().map(|p| p.1).fold(f32::MIN, f32::max).max(0.0) as usize)
        .min(prob.h.saturating_sub(1));
    if maxx <= minx || maxy <= miny {
        return 0.0;
    }
    let mut sum = 0.0f32;
    let mut n = 0usize;
    for y in miny..=maxy {
        for x in minx..=maxx {
            sum += prob.at(x, y);
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f32 }
}

/// Rotated rect described by its 4 corners + box dims.
#[derive(Clone, Debug)]
struct RotRect {
    corners: [(f32, f32); 4],
    width: f32,
    height: f32,
}

/// Minimum-area enclosing rectangle of a point set via rotating calipers on
/// the convex hull (pure Rust — no OpenCV / clipper2).
fn min_area_rect(points: &[(f32, f32)]) -> Option<RotRect> {
    let hull = convex_hull(points);
    if hull.len() < 3 {
        return None;
    }
    let n = hull.len();
    let mut best_area = f32::MAX;
    let mut best: Option<RotRect> = None;
    for i in 0..n {
        let p0 = hull[i];
        let p1 = hull[(i + 1) % n];
        let edge = (p1.0 - p0.0, p1.1 - p0.1);
        let len = (edge.0 * edge.0 + edge.1 * edge.1).sqrt();
        if len < 1e-6 {
            continue;
        }
        let ux = (edge.0 / len, edge.1 / len); // edge direction
        let uy = (-ux.1, ux.0); // normal
        let (mut min_u, mut max_u) = (f32::MAX, f32::MIN);
        let (mut min_v, mut max_v) = (f32::MAX, f32::MIN);
        for &p in &hull {
            let du = p.0 * ux.0 + p.1 * ux.1;
            let dv = p.0 * uy.0 + p.1 * uy.1;
            min_u = min_u.min(du);
            max_u = max_u.max(du);
            min_v = min_v.min(dv);
            max_v = max_v.max(dv);
        }
        let area = (max_u - min_u) * (max_v - min_v);
        if area < best_area {
            best_area = area;
            // reconstruct corners in (u,v) basis → world
            let to_world = |u: f32, v: f32| (u * ux.0 + v * uy.0, u * ux.1 + v * uy.1);
            let corners = [
                to_world(min_u, min_v),
                to_world(max_u, min_v),
                to_world(max_u, max_v),
                to_world(min_u, max_v),
            ];
            best = Some(RotRect {
                corners,
                width: max_u - min_u,
                height: max_v - min_v,
            });
        }
    }
    best
}

/// Andrew's monotone chain convex hull. Returns CCW hull without duplicates.
fn convex_hull(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut pts: Vec<(f32, f32)> = points.to_vec();
    pts.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut lower: Vec<(f32, f32)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(f32, f32)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Unclip a rotated rect by `ratio` (PaddleOCR `distance = area*ratio/perimeter`),
/// expanding width + height by `2*distance`. For a rectangle this matches the
/// general polygon offset PaddleOCR uses (pyclipper) — pure Rust here.
fn unclip_rect(rect: &RotRect, ratio: f32) -> [(f32, f32); 4] {
    let area = rect.width * rect.height;
    let perimeter = 2.0 * (rect.width + rect.height);
    if perimeter < 1e-6 {
        return rect.corners;
    }
    let distance = area * ratio / perimeter;
    // Offset every EDGE outward by `distance` (PaddleOCR pyclipper polygon
    // offset): width and height each grow by 2*distance. A naive radial
    // push-from-centroid is WRONG for text boxes — a wide/short box has an
    // almost-horizontal diagonal, so radial expansion barely grows the height
    // and clips character tops/bottoms (ㄷ→ㄴ, ascenders lost). We instead
    // expand along the rect's own (u, v) axes recovered from its ordered
    // corners (c0=min_u,min_v; c1=max_u,min_v; c2=max_u,max_v; c3=min_u,max_v).
    let c = &rect.corners;
    let unit = |dx: f32, dy: f32| -> (f32, f32) {
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-6 { (dx / len, dy / len) } else { (0.0, 0.0) }
    };
    let u = unit(c[1].0 - c[0].0, c[1].1 - c[0].1); // +u (along width)
    let v = unit(c[3].0 - c[0].0, c[3].1 - c[0].1); // +v (along height)
    let off = |p: (f32, f32), su: f32, sv: f32| -> (f32, f32) {
        (
            p.0 + su * distance * u.0 + sv * distance * v.0,
            p.1 + su * distance * u.1 + sv * distance * v.1,
        )
    };
    [
        off(c[0], -1.0, -1.0),
        off(c[1], 1.0, -1.0),
        off(c[2], 1.0, 1.0),
        off(c[3], -1.0, 1.0),
    ]
}

// ── crop + rectify ───────────────────────────────────────────────────────────

/// Perspective-warp the quadrilateral `corners` (clockwise from top-left) into
/// a horizontal strip. Output size derives from the box edge lengths.
fn rectify_crop(img: &image::RgbImage, corners: &[(f32, f32); 4]) -> image::RgbImage {
    // order corners: top-left, top-right, bottom-right, bottom-left
    let ordered = order_corners(corners);
    let dist = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let w = dist(ordered[0], ordered[1]).max(dist(ordered[3], ordered[2]));
    let h = dist(ordered[0], ordered[3]).max(dist(ordered[1], ordered[2]));
    let out_w = w.round().max(1.0) as u32;
    let out_h = h.round().max(1.0) as u32;
    let mut out = image::RgbImage::new(out_w, out_h);
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    // bilinear map from output grid back to the source quad (inverse via
    // bilinear interpolation of the four corners — adequate for near-affine
    // text boxes).
    for oy in 0..out_h {
        let fy = oy as f32 / (out_h.max(1) as f32 - 1.0).max(1.0);
        for ox in 0..out_w {
            let fx = ox as f32 / (out_w.max(1) as f32 - 1.0).max(1.0);
            // bilinear blend of the four source corners
            let top = (
                ordered[0].0 + (ordered[1].0 - ordered[0].0) * fx,
                ordered[0].1 + (ordered[1].1 - ordered[0].1) * fx,
            );
            let bot = (
                ordered[3].0 + (ordered[2].0 - ordered[3].0) * fx,
                ordered[3].1 + (ordered[2].1 - ordered[3].1) * fx,
            );
            let sx = (top.0 + (bot.0 - top.0) * fy).clamp(0.0, iw - 1.0);
            let sy = (top.1 + (bot.1 - top.1) * fy).clamp(0.0, ih - 1.0);
            let px = img.get_pixel(sx.round() as u32, sy.round() as u32);
            out.put_pixel(ox, oy, *px);
        }
    }
    out
}

/// Order 4 corners as [top-left, top-right, bottom-right, bottom-left] using
/// coordinate sums/diffs (standard PaddleOCR ordering).
fn order_corners(corners: &[(f32, f32); 4]) -> [(f32, f32); 4] {
    // top-left has smallest x+y, bottom-right largest x+y;
    // top-right smallest y-x, bottom-left largest y-x.
    let mut tl = corners[0];
    let mut br = corners[0];
    let mut tr = corners[0];
    let mut bl = corners[0];
    let (mut min_sum, mut max_sum) = (f32::MAX, f32::MIN);
    let (mut min_diff, mut max_diff) = (f32::MAX, f32::MIN);
    for &p in corners {
        let sum = p.0 + p.1;
        let diff = p.1 - p.0;
        if sum < min_sum {
            min_sum = sum;
            tl = p;
        }
        if sum > max_sum {
            max_sum = sum;
            br = p;
        }
        if diff < min_diff {
            min_diff = diff;
            tr = p;
        }
        if diff > max_diff {
            max_diff = diff;
            bl = p;
        }
    }
    [tl, tr, br, bl]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn det_target_dims_matches_golden() {
        // T0a golden: clean_paragraph 192×900 → det input 192×896.
        assert_eq!(det_target_dims(900, 192, 1600), (896, 192));
    }

    #[test]
    fn convex_hull_square() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (5.0, 5.0)];
        let hull = convex_hull(&pts);
        assert_eq!(hull.len(), 4);
    }

    #[test]
    fn min_area_rect_axis_aligned() {
        let pts = vec![(0.0, 0.0), (20.0, 0.0), (20.0, 5.0), (0.0, 5.0)];
        let r = min_area_rect(&pts).expect("rect");
        let (lo, hi) = (r.width.min(r.height), r.width.max(r.height));
        assert!((lo - 5.0).abs() < 1e-3, "short side {lo}");
        assert!((hi - 20.0).abs() < 1e-3, "long side {hi}");
    }

    #[test]
    fn dict_length_mismatch_is_construction_error() {
        // T10: a dict whose line count != DICT_LINES must fail at construction
        // (before loading the ONNX sessions) rather than mis-decoding silently.
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let dict_path = dir.path().join("bad_dict.txt");
        let mut f = std::fs::File::create(&dict_path).unwrap();
        writeln!(f, "a\nb\nc").unwrap(); // 3 lines, not DICT_LINES
        let paths = ModelPaths {
            det: dir.path().join("unused_det.onnx"),
            rec: dir.path().join("unused_rec.onnx"),
            dict: dict_path,
        };
        let err = OnnxPaddleOcr::from_paths(&paths, 0.3, 1.5, 1000, 1600)
            .expect_err("dict mismatch must error");
        let msg = format!("{err:#}");
        assert!(msg.contains("dict has 3 lines"), "unexpected error: {msg}");
    }

    #[test]
    fn model_paths_from_config_uses_overrides() {
        // T7: unset overrides → bundled default asset paths.
        let mut cfg = kebab_config::Config::defaults();
        let def = ModelPaths::from_config(&cfg);
        assert!(def.det.ends_with("ppocrv5_mobile_det.onnx"), "{:?}", def.det);
        assert!(def.rec.ends_with("korean_ppocrv5_mobile_rec.onnx"), "{:?}", def.rec);
        assert!(def.dict.ends_with("korean_dict.txt"), "{:?}", def.dict);

        // Override det + dict; rec stays bundled (partial override allowed).
        cfg.image.ocr.det_model = Some("/custom/det.onnx".to_string());
        cfg.image.ocr.dict = Some("/custom/dict.txt".to_string());
        let ov = ModelPaths::from_config(&cfg);
        assert_eq!(ov.det, PathBuf::from("/custom/det.onnx"));
        assert_eq!(ov.dict, PathBuf::from("/custom/dict.txt"));
        assert!(ov.rec.ends_with("korean_ppocrv5_mobile_rec.onnx"), "{:?}", ov.rec);
    }

    #[test]
    fn unclip_expands_box() {
        let rect = RotRect {
            corners: [(0.0, 0.0), (20.0, 0.0), (20.0, 5.0), (0.0, 5.0)],
            width: 20.0,
            height: 5.0,
        };
        let out = unclip_rect(&rect, 1.5);
        // unclipped box must be strictly larger than the original
        let orig_minx = 0.0;
        let new_minx = out.iter().map(|p| p.0).fold(f32::MAX, f32::min);
        assert!(new_minx < orig_minx, "expected expansion, got {new_minx}");
    }
}
