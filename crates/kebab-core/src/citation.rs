//! Citation (§3.5) — discriminated 5-variant. Each variant has a canonical
//! W3C Media Fragments URI per design §0 Q3.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::asset::WorkspacePath;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum Citation {
    Line {
        path: WorkspacePath,
        start: u32,
        end: u32,
        section: Option<String>,
    },
    Page {
        path: WorkspacePath,
        page: u32,
        section: Option<String>,
    },
    Region {
        path: WorkspacePath,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
    Caption {
        path: WorkspacePath,
        model: String,
    },
    Time {
        path: WorkspacePath,
        start_ms: u64,
        end_ms: u64,
        speaker: Option<String>,
    },
    Code {
        path: WorkspacePath,
        line_start: u32,
        line_end: u32,
        symbol: Option<String>,
        lang: Option<String>,
    },
}

impl Citation {
    pub fn path(&self) -> &WorkspacePath {
        match self {
            Citation::Line { path, .. }
            | Citation::Page { path, .. }
            | Citation::Region { path, .. }
            | Citation::Caption { path, .. }
            | Citation::Time { path, .. }
            | Citation::Code { path, .. } => path,
        }
    }

    /// Emit a W3C Media Fragments URI per design §0 Q3.
    /// `section` and `speaker` and `caption.model` are NOT part of the URI
    /// fragment; they live in the structured wire object.
    pub fn to_uri(&self) -> String {
        match self {
            Citation::Line {
                path, start, end, ..
            } => {
                if start == end {
                    format!("{}#L{}", path.0, start)
                } else {
                    format!("{}#L{}-L{}", path.0, start, end)
                }
            }
            Citation::Page { path, page, .. } => format!("{}#p={}", path.0, page),
            Citation::Region {
                path, x, y, w, h, ..
            } => format!("{}#xywh={},{},{},{}", path.0, x, y, w, h),
            Citation::Caption { path, .. } => format!("{}#caption", path.0),
            Citation::Time {
                path,
                start_ms,
                end_ms,
                speaker,
            } => {
                let s = format_hms_ms(*start_ms);
                let e = format_hms_ms(*end_ms);
                match speaker {
                    Some(sp) => format!("{}#t={},{}&speaker={}", path.0, s, e, sp),
                    None => format!("{}#t={},{}", path.0, s, e),
                }
            }
            Citation::Code {
                path,
                line_start,
                line_end,
                ..
            } => {
                if line_start == line_end {
                    format!("{}#L{}", path.0, line_start)
                } else {
                    format!("{}#L{}-L{}", path.0, line_start, line_end)
                }
            }
        }
    }

    /// Strict inverse of `to_uri`. The `section` / `caption.model` fields
    /// are not part of the URI grammar, so a parsed Citation will have
    /// `section = None` and `model = ""` for the relevant variants.
    /// Round-trip property holds for citations whose non-URI fields are at
    /// their default values (see test).
    pub fn parse(s: &str) -> Result<Self> {
        let (path_str, frag) = match s.rsplit_once('#') {
            Some(t) => t,
            None => bail!("citation has no '#' fragment: {s:?}"),
        };
        // `WorkspacePath::new` rejects any remaining `#` on the path side
        // (e.g. the input had multiple `#` separators), closing the
        // hash-in-path concern at construction rather than at every reader.
        let path = WorkspacePath::new(path_str.to_owned())?;

        if let Some(rest) = frag.strip_prefix("L") {
            // line range: `L<a>` or `L<a>-L<b>`
            if let Some((a, b)) = rest.split_once("-L") {
                let start: u32 = a
                    .parse()
                    .map_err(|_| anyhow::anyhow!("bad line start in {a:?} (input {s:?})"))?;
                let end: u32 = b
                    .parse()
                    .map_err(|_| anyhow::anyhow!("bad line end in {b:?} (input {s:?})"))?;
                return Ok(Citation::Line {
                    path,
                    start,
                    end,
                    section: None,
                });
            }
            let n: u32 = rest
                .parse()
                .map_err(|_| anyhow::anyhow!("bad line number in {rest:?} (input {s:?})"))?;
            return Ok(Citation::Line {
                path,
                start: n,
                end: n,
                section: None,
            });
        }
        if let Some(rest) = frag.strip_prefix("p=") {
            let page: u32 = rest
                .parse()
                .map_err(|_| anyhow::anyhow!("bad page number in {rest:?} (input {s:?})"))?;
            return Ok(Citation::Page {
                path,
                page,
                section: None,
            });
        }
        if let Some(rest) = frag.strip_prefix("xywh=") {
            let parts: Vec<&str> = rest.split(',').collect();
            if parts.len() != 4 {
                bail!("xywh= expects 4 comma-separated values, got {rest:?} (input {s:?})");
            }
            let x: u32 = parts[0]
                .parse()
                .map_err(|_| anyhow::anyhow!("bad xywh.x in {:?} (input {s:?})", parts[0]))?;
            let y: u32 = parts[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("bad xywh.y in {:?} (input {s:?})", parts[1]))?;
            let w: u32 = parts[2]
                .parse()
                .map_err(|_| anyhow::anyhow!("bad xywh.w in {:?} (input {s:?})", parts[2]))?;
            let h: u32 = parts[3]
                .parse()
                .map_err(|_| anyhow::anyhow!("bad xywh.h in {:?} (input {s:?})", parts[3]))?;
            return Ok(Citation::Region { path, x, y, w, h });
        }
        if frag == "caption" {
            return Ok(Citation::Caption {
                path,
                model: String::new(),
            });
        }
        if let Some(rest) = frag.strip_prefix("t=") {
            // `t=<start>,<end>` optionally followed by `&speaker=<id>`
            let (range, speaker) = match rest.split_once('&') {
                Some((r, kv)) => match kv.strip_prefix("speaker=") {
                    Some(sp) => (r, Some(sp.to_owned())),
                    None => bail!("unknown time-fragment param {kv:?} (input {s:?})"),
                },
                None => (rest, None),
            };
            let (s_str, e_str) = match range.split_once(',') {
                Some(t) => t,
                None => bail!("time fragment expects '<start>,<end>', got {range:?} (input {s:?})"),
            };
            let start_ms = parse_hms_ms(s_str)?;
            let end_ms = parse_hms_ms(e_str)?;
            return Ok(Citation::Time {
                path,
                start_ms,
                end_ms,
                speaker,
            });
        }
        bail!("unrecognised citation fragment {frag:?} (input {s:?})")
    }
}

/// Format milliseconds as `hh:mm:ss.mmm` (W3C Media Fragments NPT-with-ms).
fn format_hms_ms(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1000;
    let millis = ms % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn parse_hms_ms(s: &str) -> Result<u64> {
    // Accept `hh:mm:ss.mmm` (the form we emit). Reject malformed input.
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        bail!("time component expects hh:mm:ss.mmm, got {s:?}");
    }
    let h: u64 = parts[0]
        .parse()
        .map_err(|_| anyhow::anyhow!("bad hours in {:?} (input {s:?})", parts[0]))?;
    let m: u64 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("bad minutes in {:?} (input {s:?})", parts[1]))?;
    let (sec, ms) = if let Some((s_part, ms_part)) = parts[2].split_once('.') {
        let sec: u64 = s_part
            .parse()
            .map_err(|_| anyhow::anyhow!("bad seconds in {s_part:?} (input {s:?})"))?;
        // Pad/truncate to exactly 3 digits.
        let mut ms_str = ms_part.to_owned();
        while ms_str.len() < 3 {
            ms_str.push('0');
        }
        ms_str.truncate(3);
        let ms: u64 = ms_str
            .parse()
            .map_err(|_| anyhow::anyhow!("bad milliseconds in {ms_part:?} (input {s:?})"))?;
        (sec, ms)
    } else {
        let sec: u64 = parts[2]
            .parse()
            .map_err(|_| anyhow::anyhow!("bad seconds in {:?} (input {s:?})", parts[2]))?;
        (sec, 0)
    };
    Ok(h * 3_600_000 + m * 60_000 + sec * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> WorkspacePath {
        WorkspacePath::new(s.to_owned()).expect("test paths must not contain '#'")
    }

    #[test]
    fn line_range_uri_and_roundtrip() {
        let c = Citation::Line {
            path: p("notes/rust/kb.md"),
            start: 12,
            end: 34,
            section: None,
        };
        assert_eq!(c.to_uri(), "notes/rust/kb.md#L12-L34");
        let parsed = Citation::parse(&c.to_uri()).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn line_single_uri_and_roundtrip() {
        let c = Citation::Line {
            path: p("a/b.md"),
            start: 7,
            end: 7,
            section: None,
        };
        assert_eq!(c.to_uri(), "a/b.md#L7");
        let parsed = Citation::parse(&c.to_uri()).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn page_uri_and_roundtrip() {
        let c = Citation::Page {
            path: p("papers/book.pdf"),
            page: 23,
            section: None,
        };
        assert_eq!(c.to_uri(), "papers/book.pdf#p=23");
        let parsed = Citation::parse(&c.to_uri()).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn region_uri_and_roundtrip() {
        let c = Citation::Region {
            path: p("photos/x.png"),
            x: 120,
            y: 40,
            w: 520,
            h: 180,
        };
        assert_eq!(c.to_uri(), "photos/x.png#xywh=120,40,520,180");
        let parsed = Citation::parse(&c.to_uri()).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn caption_uri_and_roundtrip() {
        let c = Citation::Caption {
            path: p("photos/x.png"),
            // `model` is not in the URI grammar; round-trip fills it with "".
            model: String::new(),
        };
        assert_eq!(c.to_uri(), "photos/x.png#caption");
        let parsed = Citation::parse(&c.to_uri()).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn time_uri_and_roundtrip_with_speaker() {
        let c = Citation::Time {
            path: p("recordings/r.m4a"),
            start_ms: 822_000,
            end_ms: 850_000,
            speaker: Some("S1".to_string()),
        };
        assert_eq!(
            c.to_uri(),
            "recordings/r.m4a#t=00:13:42.000,00:14:10.000&speaker=S1"
        );
        let parsed = Citation::parse(&c.to_uri()).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn time_uri_and_roundtrip_without_speaker() {
        let c = Citation::Time {
            path: p("recordings/r.m4a"),
            start_ms: 1_500,
            end_ms: 2_750,
            speaker: None,
        };
        assert_eq!(c.to_uri(), "recordings/r.m4a#t=00:00:01.500,00:00:02.750");
        let parsed = Citation::parse(&c.to_uri()).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn parse_rejects_no_fragment() {
        assert!(Citation::parse("just/path.md").is_err());
    }

    #[test]
    fn parse_rejects_unknown_fragment() {
        assert!(Citation::parse("a.md#mystery=1").is_err());
    }

    /// `rsplit_once('#')` would otherwise leave a `#` on the path side when
    /// the input contains multiple `#` separators (e.g. someone embeds a
    /// fake fragment in the path). The `WorkspacePath::new` constructor
    /// closes that hole at construction time.
    #[test]
    fn parse_path_with_hash_rejected_at_to_posix_layer() {
        // `notes/x#evil.md#L7` — rsplit_once strips `#L7`, leaving
        // `notes/x#evil.md` on the path side. WorkspacePath::new must reject.
        let r = Citation::parse("notes/x#evil.md#L7");
        assert!(r.is_err(), "path with embedded '#' must be rejected");
    }

    #[test]
    fn citation_code_variant_serializes_with_kind_tag() {
        let c = Citation::Code {
            path: WorkspacePath("crates/kebab-chunk/src/md_heading_v1.rs".into()),
            line_start: 142,
            line_end: 168,
            symbol: Some("MdHeadingV1Chunker::chunk_doc".into()),
            lang: Some("rust".into()),
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["kind"], "code");
        assert_eq!(v["line_start"], 142);
        assert_eq!(v["line_end"], 168);
        assert_eq!(v["symbol"], "MdHeadingV1Chunker::chunk_doc");
        assert_eq!(v["lang"], "rust");
        // Existing 5 variants must NOT pick up these fields.
        let line = Citation::Line {
            path: WorkspacePath("notes/foo.md".into()),
            start: 1,
            end: 10,
            section: None,
        };
        let lv = serde_json::to_value(&line).unwrap();
        assert!(lv.get("line_start").is_none());
        assert!(lv.get("symbol").is_none());
    }

    #[test]
    fn citation_code_uri_format() {
        let c = Citation::Code {
            path: WorkspacePath("a/b.rs".into()),
            line_start: 10,
            line_end: 20,
            symbol: None,
            lang: Some("rust".into()),
        };
        assert_eq!(c.to_uri(), "a/b.rs#L10-L20");
        // Single-line uses `#L10`.
        let single = Citation::Code {
            path: WorkspacePath("a/b.rs".into()),
            line_start: 5,
            line_end: 5,
            symbol: None,
            lang: None,
        };
        assert_eq!(single.to_uri(), "a/b.rs#L5");
    }

    #[test]
    fn citation_code_path_accessor() {
        let c = Citation::Code {
            path: WorkspacePath("x.rs".into()),
            line_start: 1,
            line_end: 1,
            symbol: None,
            lang: None,
        };
        assert_eq!(c.path().0, "x.rs");
    }
}
