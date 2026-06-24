//! v3 마이그레이션 무손실 골든 — 사용자 실제 v2 config.
//!
//! 불변식: 사용자가 손본 값·주석·대안(commented) 줄이 [ingest.*] relocation
//! 후에도 전부 보존되고, v3 Config 로 파싱했을 때 같은 값을 내며, 재실행이
//! 멱등이어야 한다.
use kebab_config::migrate::migrate_document;

const USER_V2: &str = include_str!("fixtures/user_v2_config.toml");

#[test]
fn user_v2_migrates_losslessly() {
    let out = migrate_document(USER_V2);
    assert_eq!(out.from_schema_version, 2);
    // v2 → CURRENT(=5): v3 의 [ingest.*] relocation, v4 의
    // [[workspace.sources]] default source 미러링, v5 의 공유 [ingest.ocr]
    // 통합까지 적용된다.
    assert_eq!(
        out.to_schema_version,
        kebab_config::migrate::CURRENT_SCHEMA_VERSION
    );
    let t = &out.new_text;

    // 사용자 값 보존.
    assert!(t.contains("root = \"/Users/user/Obsidian/Default\""), "{t}");
    // v4: workspace.root → [[workspace.sources]] id=default 미러링.
    assert!(t.contains("[[workspace.sources]]"), "v4 sources 누락:\n{t}");
    assert!(t.contains("id = \"default\""), "default source 누락:\n{t}");
    assert!(t.contains("model = \"snowflake-arctic-embed2\""));
    assert!(t.contains("endpoint = \"http://192.168.0.2:11943\""));
    // 사용자 주석/대안 줄 보존.
    assert!(t.contains("# engine = \"ollama-vision\""), "대안 주석 유실:\n{t}");
    assert!(t.contains("# provider = \"candle\""));
    // 새 위치.
    assert!(t.contains("[ingest.image.ocr]"));
    assert!(t.contains("[ingest.pdf.ocr]"));
    assert!(t.contains("[ingest.chunking]"));
    assert!(t.contains("[ingest.image.caption]"));
    // 옛 top-level 위치 제거.
    assert!(!t.contains("\n[chunking]"));
    assert!(!t.contains("\n[image.ocr]"));
    assert!(!t.contains("\n[indexing]"));

    // v5: 공유 [ingest.ocr] 통합 후 image 엔진 키는 공유 블록에 산다.
    assert!(t.contains("[ingest.ocr]"), "공유 OCR 블록 누락:\n{t}");

    // effective 값은 from_file(공유 OCR resolution 포함)로 검증한다 —
    // image 의 engine=paddle-onnx 가 공유 블록으로 끌어올려진 뒤에도 보존.
    let dir = std::env::temp_dir().join(format!("kebab_mv3_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("config.toml");
    std::fs::write(&p, t).unwrap();
    let cfg = kebab_config::Config::from_file(&p).expect("v5 from_file");
    assert!(cfg.image_ocr().enabled);
    assert_eq!(cfg.image_ocr().engine, "paddle-onnx");
    assert_eq!(cfg.models.embedding.model, "snowflake-arctic-embed2");
    assert_eq!(cfg.models.llm.endpoint, "http://192.168.0.2:11943");
    // pdf paddle 값 보존(v2 비대칭 → pdf 대칭 키로 복사). user 의 pdf.ocr 는
    // engine=paddle-onnx 이고 자체 det_model 없으므로 번들(None) 유지.
    assert_eq!(cfg.pdf_ocr().engine, "paddle-onnx");

    // 멱등.
    let again = migrate_document(t);
    assert!(!again.changed(), "재실행 변경: {:?}", again.changes);
    assert_eq!(again.new_text, *t);
}
