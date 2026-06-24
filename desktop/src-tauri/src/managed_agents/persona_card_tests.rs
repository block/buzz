use super::*;
use png::{BitDepth, ColorType, Encoder};
use std::io::Write;
use zip::write::{SimpleFileOptions, ZipWriter};

/// Helper: build a minimal valid PNG with a custom tEXt chunk.
fn make_png_with_text(keyword: &str, text: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        enc.add_text_chunk(keyword.to_string(), text.to_string())
            .unwrap();
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0, 0, 0, 255]).unwrap();
    }
    buf
}

/// Helper: build a PNG with a buzz_persona_pkg tEXt chunk for the given name/prompt.
fn make_test_persona_png(name: &str, prompt: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "version": 1,
        "displayName": name,
        "systemPrompt": prompt,
    });
    let b64 = STANDARD.encode(payload.to_string().as_bytes());
    make_png_with_text("buzz_persona_pkg", &b64)
}

/// Helper: build a plain PNG with no metadata.
fn make_plain_png() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0, 0, 0, 255]).unwrap();
    }
    buf
}

/// Helper: create a ZIP from name→data pairs.
fn make_test_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut buf);
    let options = SimpleFileOptions::default();
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
    buf.into_inner()
}

#[test]
fn parse_png_round_trip() {
    let png = make_test_persona_png("George Costanza", "You are George.");
    let result = parse_png_persona(&png).unwrap();
    assert_eq!(result.display_name, "George Costanza");
    assert_eq!(result.system_prompt, "You are George.");
    assert!(result
        .avatar_data_url
        .unwrap()
        .starts_with("data:image/png;base64,"));
}

#[test]
fn parse_png_no_metadata() {
    let png = make_plain_png();
    let err = parse_png_persona(&png).unwrap_err();
    assert!(err.contains("doesn't contain persona data"));
}

#[test]
fn parse_png_unknown_version() {
    let payload = serde_json::json!({"version": 99, "displayName": "X", "systemPrompt": "Y"});
    let b64 = STANDARD.encode(payload.to_string().as_bytes());
    let png = make_png_with_text("buzz_persona_pkg", &b64);
    let err = parse_png_persona(&png).unwrap_err();
    assert!(err.contains("Unsupported persona version"));
}

#[test]
fn parse_png_malformed_base64() {
    let png = make_png_with_text("buzz_persona_pkg", "!!!not-base64!!!");
    let err = parse_png_persona(&png).unwrap_err();
    assert!(err.contains("Invalid base64"));
}

#[test]
fn parse_png_malformed_json() {
    let b64 = STANDARD.encode(b"not json at all");
    let png = make_png_with_text("buzz_persona_pkg", &b64);
    let err = parse_png_persona(&png).unwrap_err();
    assert!(err.contains("Invalid JSON"));
}

#[test]
fn parse_png_empty_fields() {
    let payload = serde_json::json!({"version": 1, "displayName": "", "systemPrompt": "Y"});
    let b64 = STANDARD.encode(payload.to_string().as_bytes());
    let png = make_png_with_text("buzz_persona_pkg", &b64);
    let err = parse_png_persona(&png).unwrap_err();
    assert!(err.contains("displayName is empty"));
}

#[test]
fn parse_png_chara_fallback() {
    let chara = serde_json::json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "Kramer",
            "system_prompt": "You are Kramer.",
            "description": ""
        }
    });
    let b64 = STANDARD.encode(chara.to_string().as_bytes());
    let png = make_png_with_text("chara", &b64);
    let result = parse_png_persona(&png).unwrap();
    assert_eq!(result.display_name, "Kramer");
    assert_eq!(result.system_prompt, "You are Kramer.");
}

#[test]
fn parse_png_chara_ignored_when_buzz_present() {
    // Build a PNG with both buzz_persona_pkg and chara chunks.
    let buzz = serde_json::json!({"version": 1, "displayName": "Buzz Name", "systemPrompt": "Buzz prompt"});
    let chara = serde_json::json!({
        "spec": "chara_card_v2", "spec_version": "2.0",
        "data": {"name": "Chara Name", "system_prompt": "Chara prompt", "description": ""}
    });
    let buzz_b64 = STANDARD.encode(buzz.to_string().as_bytes());
    let chara_b64 = STANDARD.encode(chara.to_string().as_bytes());

    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        enc.add_text_chunk("buzz_persona_pkg".to_string(), buzz_b64)
            .unwrap();
        enc.add_text_chunk("chara".to_string(), chara_b64).unwrap();
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0, 0, 0, 255]).unwrap();
    }

    let result = parse_png_persona(&buf).unwrap();
    assert_eq!(result.display_name, "Buzz Name");
    assert_eq!(result.system_prompt, "Buzz prompt");
}

#[test]
fn parse_zip_valid_pack() {
    let p1 = make_test_persona_png("Alice", "Prompt A");
    let p2 = make_test_persona_png("Bob", "Prompt B");
    let p3 = make_test_persona_png("Carol", "Prompt C");
    let zip = make_test_zip(&[("alice.png", &p1), ("bob.png", &p2), ("carol.png", &p3)]);
    let result = parse_zip_personas(&zip).unwrap();
    assert_eq!(result.personas.len(), 3);
    assert!(result.skipped.is_empty());
    assert_eq!(result.personas[0].source_file, "alice.png");
}

#[test]
fn parse_zip_mixed() {
    let valid1 = make_test_persona_png("Alice", "Prompt A");
    let valid2 = make_test_persona_png("Bob", "Prompt B");
    let bad_png = make_plain_png(); // no metadata
    let zip = make_test_zip(&[
        ("alice.png", &valid1),
        ("bob.png", &valid2),
        ("bad.png", &bad_png),
        ("readme.txt", b"hello"),
    ]);
    let result = parse_zip_personas(&zip).unwrap();
    assert_eq!(result.personas.len(), 2);
    assert_eq!(result.skipped.len(), 2);
}

#[test]
fn parse_zip_no_pngs() {
    let zip = make_test_zip(&[("readme.txt", b"hello"), ("data.csv", b"a,b")]);
    let err = parse_zip_personas(&zip).unwrap_err();
    assert!(err.contains("No persona files found"));
}

#[test]
fn parse_zip_exceeds_entry_limit() {
    let png = make_test_persona_png("X", "Y");
    let entries: Vec<(String, &[u8])> = (0..51)
        .map(|i| (format!("{i}.png"), png.as_slice()))
        .collect();
    let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (n.as_str(), *d)).collect();
    let zip = make_test_zip(&refs);
    let err = parse_zip_personas(&zip).unwrap_err();
    assert!(err.contains("too many entries"));
}

#[test]
fn parse_zip_path_traversal() {
    let valid = make_test_persona_png("Safe", "Prompt");
    let evil = make_test_persona_png("Evil", "Prompt");
    let zip = make_test_zip(&[("safe.png", &valid), ("../evil.png", &evil)]);
    let result = parse_zip_personas(&zip).unwrap();
    assert_eq!(result.personas.len(), 1);
    assert_eq!(result.skipped.len(), 1);
    assert!(result.skipped[0].reason.contains("Path traversal"));
}

#[test]
fn parse_png_duplicate_chunks() {
    // Two buzz_persona_pkg chunks — should use the first and ignore the second.
    let payload1 =
        serde_json::json!({"version": 1, "displayName": "First", "systemPrompt": "Prompt 1"});
    let payload2 =
        serde_json::json!({"version": 1, "displayName": "Second", "systemPrompt": "Prompt 2"});
    let b64_1 = STANDARD.encode(payload1.to_string().as_bytes());
    let b64_2 = STANDARD.encode(payload2.to_string().as_bytes());

    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        enc.add_text_chunk("buzz_persona_pkg".to_string(), b64_1)
            .unwrap();
        enc.add_text_chunk("buzz_persona_pkg".to_string(), b64_2)
            .unwrap();
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0, 0, 0, 255]).unwrap();
    }

    let result = parse_png_persona(&buf).unwrap();
    assert_eq!(result.display_name, "First");
    assert_eq!(result.system_prompt, "Prompt 1");
}

#[test]
fn parse_zip_exceeds_size_limit() {
    // Create a ZIP with entries whose cumulative decompressed size exceeds 100MB.
    let mut zip_buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut zip_buf);
        let options = SimpleFileOptions::default();
        zip.start_file("big.png", options).unwrap();
        let chunk = vec![0u8; 1024 * 1024]; // 1 MB
        for _ in 0..101 {
            zip.write_all(&chunk).unwrap();
        }
        zip.finish().unwrap();
    }
    let zip_bytes = zip_buf.into_inner();
    let err = parse_zip_personas(&zip_bytes).unwrap_err();
    assert!(err.contains("exceeds 100MB"));
}

#[test]
fn parse_json_round_trip() {
    let bytes = encode_persona_json(
        "Ada Lovelace",
        "You are Ada.",
        Some("https://example.com/ada.png"),
        None,
        None,
        None,
        &[],
    )
    .unwrap();
    let result = parse_json_persona(&bytes).unwrap();
    assert_eq!(result.display_name, "Ada Lovelace");
    assert_eq!(result.system_prompt, "You are Ada.");
    assert_eq!(
        result.avatar_data_url.as_deref(),
        Some("https://example.com/ada.png")
    );
    assert!(result.source_file.is_empty());
}

#[test]
fn parse_json_round_trip_no_avatar() {
    let bytes = encode_persona_json("Bob", "You are Bob.", None, None, None, None, &[]).unwrap();
    let result = parse_json_persona(&bytes).unwrap();
    assert_eq!(result.display_name, "Bob");
    assert_eq!(result.system_prompt, "You are Bob.");
    assert!(result.avatar_data_url.is_none());
}

#[test]
fn parse_json_round_trip_data_uri_avatar() {
    let data_uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
    let bytes = encode_persona_json(
        "Carol",
        "You are Carol.",
        Some(data_uri),
        None,
        None,
        None,
        &[],
    )
    .unwrap();
    let result = parse_json_persona(&bytes).unwrap();
    assert_eq!(result.display_name, "Carol");
    assert_eq!(result.avatar_data_url.as_deref(), Some(data_uri));
}

#[test]
fn parse_json_round_trip_with_runtime_and_model() {
    let bytes = encode_persona_json(
        "Agent Smith",
        "You are an agent.",
        None,
        Some("goose"),
        Some("claude-sonnet-4"),
        None,
        &[],
    )
    .unwrap();
    let result = parse_json_persona(&bytes).unwrap();
    assert_eq!(result.display_name, "Agent Smith");
    assert_eq!(result.system_prompt, "You are an agent.");
    assert!(result.avatar_data_url.is_none());
    assert_eq!(result.runtime.as_deref(), Some("goose"));
    assert_eq!(result.model.as_deref(), Some("claude-sonnet-4"));
}

#[test]
fn parse_json_round_trip_without_runtime_and_model() {
    let bytes = encode_persona_json("Bob", "You are Bob.", None, None, None, None, &[]).unwrap();
    let result = parse_json_persona(&bytes).unwrap();
    assert_eq!(result.display_name, "Bob");
    assert!(result.runtime.is_none());
    assert!(result.model.is_none());
}

#[test]
fn parse_json_backward_compat_no_runtime_model_fields() {
    // Simulate a legacy persona JSON without runtime/model fields
    let json = serde_json::json!({
        "version": 1,
        "displayName": "Legacy Persona",
        "systemPrompt": "Old school prompt"
    });
    let bytes = serde_json::to_vec(&json).unwrap();
    let result = parse_json_persona(&bytes).unwrap();
    assert_eq!(result.display_name, "Legacy Persona");
    assert_eq!(result.system_prompt, "Old school prompt");
    assert!(result.runtime.is_none());
    assert!(result.model.is_none());
}

#[test]
fn parse_json_backward_compat_legacy_provider_key() {
    // A JSON card written with the old "provider" key should still parse.
    let json = serde_json::json!({
        "version": 1,
        "displayName": "Legacy Agent",
        "systemPrompt": "Old prompt",
        "provider": "goose"
    });
    let bytes = serde_json::to_vec(&json).unwrap();
    let result = parse_json_persona(&bytes).unwrap();
    assert_eq!(result.runtime.as_deref(), Some("goose"));
}

#[test]
fn parse_json_invalid_version() {
    let json = serde_json::json!({
        "version": 99,
        "displayName": "X",
        "systemPrompt": "Y"
    });
    let bytes = serde_json::to_vec(&json).unwrap();
    let err = parse_json_persona(&bytes).unwrap_err();
    assert!(err.contains("Unsupported persona version"));
}

#[test]
fn parse_json_empty_fields() {
    let json_empty_name = serde_json::json!({
        "version": 1,
        "displayName": "",
        "systemPrompt": "Y"
    });
    let err = parse_json_persona(&serde_json::to_vec(&json_empty_name).unwrap()).unwrap_err();
    assert!(err.contains("displayName is empty"));

    let json_empty_prompt = serde_json::json!({
        "version": 1,
        "displayName": "X",
        "systemPrompt": ""
    });
    let err = parse_json_persona(&serde_json::to_vec(&json_empty_prompt).unwrap()).unwrap_err();
    assert!(err.contains("systemPrompt is empty"));
}

#[test]
fn parse_json_malformed() {
    let err = parse_json_persona(b"not json at all").unwrap_err();
    assert!(err.contains("Invalid JSON"));
}

#[test]
fn parse_md_persona_preserves_avatar_ref() {
    let md = br#"---
name: goosey
display_name: Goosey
description: Goose internal agent.
avatar: https://example.com/avatars/goosey.png
model: anthropic:claude-sonnet-4
runtime: goose
---
You are Goosey.
"#;
    let result = parse_md_persona(md).unwrap();
    assert_eq!(result.display_name, "Goosey");
    assert_eq!(result.avatar_data_url, None);
    assert_eq!(
        result.avatar_ref.as_deref(),
        Some("https://example.com/avatars/goosey.png")
    );
    assert_eq!(result.model.as_deref(), Some("claude-sonnet-4"));
    assert_eq!(result.provider.as_deref(), Some("anthropic"));
    assert_eq!(result.runtime.as_deref(), Some("goose"));
}

#[test]
fn parse_lenient_md_persona_preserves_model_provider_prefix() {
    let md = r#"---
display_name: Lenient Agent
model: databricks:gpt-5
runtime: goose
---
You are lenient.
"#;

    let result = parse_lenient_md_persona(md).unwrap();
    assert_eq!(result.display_name, "Lenient Agent");
    assert_eq!(result.model.as_deref(), Some("gpt-5"));
    assert_eq!(result.provider.as_deref(), Some("databricks"));
    assert_eq!(result.runtime.as_deref(), Some("goose"));
}

#[test]
fn parse_md_persona_accepts_goose_internal_frontmatter() {
    let md = br#"---
name: block.md
description: Opinionated guide to Block's intelligence operating model.
avatar: https://avatars.example.com/block-md.png
metadata:
  gooseInternalBundled: true
---
You are block.md.
"#;
    let result = parse_md_persona(md).unwrap();
    assert_eq!(result.display_name, "block.md");
    assert_eq!(
        result.avatar_ref.as_deref(),
        Some("https://avatars.example.com/block-md.png")
    );
    assert_eq!(result.system_prompt, "You are block.md.\n");
}

#[test]
fn parse_zip_with_json() {
    let j1 = encode_persona_json("Alice", "Prompt A", None, None, None, None, &[]).unwrap();
    let j2 = encode_persona_json("Bob", "Prompt B", None, None, None, None, &[]).unwrap();
    let zip = make_test_zip(&[("alice.persona.json", &j1), ("bob.persona.json", &j2)]);
    let result = parse_zip_personas(&zip).unwrap();
    assert_eq!(result.personas.len(), 2);
    assert!(result.skipped.is_empty());
    assert_eq!(result.personas[0].display_name, "Alice");
    assert_eq!(result.personas[1].display_name, "Bob");
}

#[test]
fn parse_zip_mixed_png_and_json() {
    let png = make_test_persona_png("PngPersona", "PNG prompt");
    let json =
        encode_persona_json("JsonPersona", "JSON prompt", None, None, None, None, &[]).unwrap();
    let zip = make_test_zip(&[
        ("persona.png", &png),
        ("persona.json", &json),
        ("readme.txt", b"hello"),
    ]);
    let result = parse_zip_personas(&zip).unwrap();
    assert_eq!(result.personas.len(), 2);
    // readme.txt should be skipped
    assert_eq!(result.skipped.len(), 1);
    assert!(result.skipped[0]
        .reason
        .contains("Not a .png, .json, or .md file"));
}

#[test]
fn parse_zip_with_plain_md_persona_preserves_avatar_ref() {
    let md = br#"---
name: fizz
display_name: Fizz
description: Engineering agent.
avatar: https://avatars.example.com/fizz.png
runtime: goose
model: anthropic:claude-sonnet-4
---
You are Fizz.
"#;
    let zip = make_test_zip(&[("fizz.md", md)]);
    let result = parse_zip_personas(&zip).unwrap();
    assert_eq!(result.personas.len(), 1);
    assert!(result.skipped.is_empty());
    assert_eq!(result.personas[0].display_name, "Fizz");
    assert_eq!(
        result.personas[0].avatar_ref.as_deref(),
        Some("https://avatars.example.com/fizz.png")
    );
    assert_eq!(result.personas[0].source_file, "fizz.md");
}

#[test]
fn parse_zip_ignores_macos_resource_forks() {
    let j1 = encode_persona_json("Frank", "You are Frank.", None, None, None, None, &[]).unwrap();
    let j2 = encode_persona_json("Jackie", "You are Jackie.", None, None, None, None, &[]).unwrap();
    let zip = make_test_zip(&[
        ("frank-costanza.persona.json", &j1),
        ("jackie-chiles.persona.json", &j2),
        ("__MACOSX/._frank-costanza.persona.json", b"\x00\x05\x16"),
        ("__MACOSX/._jackie-chiles.persona.json", b"\x00\x05\x16"),
    ]);
    let result = parse_zip_personas(&zip).unwrap();
    assert_eq!(result.personas.len(), 2);
    // macOS resource forks should be silently ignored, not skipped with errors
    assert!(result.skipped.is_empty());
}
