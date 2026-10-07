//! Unit tests for `card.rs` — split into a child module file so the parent
//! stays under the 1000-line gate (same layout as `snapshot/tests.rs`).

use super::*;
use std::collections::BTreeMap;

#[test]
fn archive_file_name_validation_rejects_escapes() {
    assert!(validate_archive_file_name("eva-1234.agent.png").is_ok());
    for bad in [
        "../escape.agent.png",
        "sub/dir.agent.png",
        "sub\\dir.agent.png",
        "not-a-card.png",
        "plain.json",
        "",
    ] {
        assert!(
            validate_archive_file_name(bad).is_err(),
            "expected rejection: {bad:?}"
        );
    }
}

#[test]
fn card_template_decodes_with_expected_shape() {
    // The embedded template is generation input only, but a corrupt or
    // accidentally swapped asset should fail the build's test gate, not a
    // user's first mint.
    let img = image::load_from_memory(CARD_TEMPLATE_PNG).expect("template must decode");
    // 2:3-ish portrait frame.
    assert!(img.height() > img.width(), "template must be portrait");
    assert!(img.width() >= 512, "template unexpectedly small");
}

#[test]
fn key_resolution_layering_record_wins() {
    let mut global = BTreeMap::new();
    global.insert("OPENAI_API_KEY".to_string(), "global".to_string());
    let mut persona = BTreeMap::new();
    persona.insert("OPENAI_API_KEY".to_string(), "persona".to_string());
    let mut record = BTreeMap::new();
    record.insert("OPENAI_API_KEY".to_string(), "record".to_string());

    assert_eq!(
        resolve_env_from_layers("OPENAI_API_KEY", &global, &persona, &record, None).as_deref(),
        Some("record")
    );
    record.clear();
    assert_eq!(
        resolve_env_from_layers("OPENAI_API_KEY", &global, &persona, &record, None).as_deref(),
        Some("persona")
    );
    persona.clear();
    assert_eq!(
        resolve_env_from_layers("OPENAI_API_KEY", &global, &persona, &record, None).as_deref(),
        Some("global")
    );
    global.clear();
    assert_eq!(
        resolve_env_from_layers(
            "OPENAI_API_KEY",
            &global,
            &persona,
            &record,
            Some("process".to_string())
        )
        .as_deref(),
        Some("process")
    );
    assert!(resolve_env_from_layers("OPENAI_API_KEY", &global, &persona, &record, None).is_none());
}

/// Prove that `resolve_key_layer` classifies layers in the same precedence
/// order that `mint_agent_card`/`resolve_env_from_layers` uses, so the dialog
/// update path is only offered when writing global will actually win.
#[test]
fn key_status_layer_matches_mint_resolution_priority() {
    let key = "OPENAI_API_KEY";
    let mut global = BTreeMap::new();
    let mut persona = BTreeMap::new();
    let mut record = BTreeMap::new();

    // No key anywhere → "none"
    assert_eq!(resolve_key_layer(&global, &persona, &record, None), "none");

    // Only global → "global" (the only writable layer)
    global.insert(key.to_string(), "sk-global".to_string());
    assert_eq!(
        resolve_key_layer(&global, &persona, &record, None),
        "global"
    );
    // mint resolution also picks global when record and persona are empty
    assert_eq!(
        resolve_env_from_layers(key, &global, &persona, &record, None).as_deref(),
        Some("sk-global")
    );

    // Persona overrides global → status must report "persona", NOT "global"
    persona.insert(key.to_string(), "sk-persona".to_string());
    assert_eq!(
        resolve_key_layer(&global, &persona, &record, None),
        "persona"
    );
    // mint would use the persona key
    assert_eq!(
        resolve_env_from_layers(key, &global, &persona, &record, None).as_deref(),
        Some("sk-persona")
    );
    // Writing to global would NOT change what mint resolves — status correctly
    // returns "persona" so the dialog shows a read-only redirect instead.
    let mut global_updated = global.clone();
    global_updated.insert(key.to_string(), "sk-new-global".to_string());
    assert_eq!(
        resolve_env_from_layers(key, &global_updated, &persona, &record, None).as_deref(),
        Some("sk-persona"),
        "writing global must not change resolution when persona key exists"
    );

    // Agent record overrides both → status must report "agent"
    record.insert(key.to_string(), "sk-agent".to_string());
    assert_eq!(resolve_key_layer(&global, &persona, &record, None), "agent");
    assert_eq!(
        resolve_env_from_layers(key, &global, &persona, &record, None).as_deref(),
        Some("sk-agent")
    );

    // Process env is last resort (only when all map layers are empty)
    let empty = BTreeMap::new();
    assert_eq!(
        resolve_key_layer(&empty, &empty, &empty, Some("sk-process".to_string())),
        "process"
    );

    // Blank values are skipped — process wins over a whitespace global
    let mut blank_global = BTreeMap::new();
    blank_global.insert(key.to_string(), "   ".to_string());
    assert_eq!(
        resolve_key_layer(
            &blank_global,
            &empty,
            &empty,
            Some("sk-process".to_string())
        ),
        "process"
    );
}

#[test]
fn key_resolution_skips_blank_values() {
    let mut record = BTreeMap::new();
    record.insert("OPENAI_API_KEY".to_string(), "   ".to_string());
    let mut persona = BTreeMap::new();
    persona.insert("OPENAI_API_KEY".to_string(), "persona".to_string());
    assert_eq!(
        resolve_env_from_layers("OPENAI_API_KEY", &BTreeMap::new(), &persona, &record, None)
            .as_deref(),
        Some("persona")
    );
}

#[test]
fn responses_url_default_and_override() {
    assert_eq!(responses_url(None), "https://api.openai.com/v1/responses");
    // Trailing slashes must not produce a double-slash path.
    assert_eq!(
        responses_url(Some("https://proxy.example/v1/".to_string())),
        "https://proxy.example/v1/responses"
    );
    assert_eq!(
        responses_url(Some("https://proxy.example/v1".to_string())),
        "https://proxy.example/v1/responses"
    );
}

#[test]
fn instructions_pin_style_match_default_and_owner_primacy() {
    let base = build_card_instructions("Eva", "leads the team", "");
    assert!(base.contains("match input image 2's art style EXACTLY"));
    assert!(base.contains("\"Eva\""));
    assert!(!base.contains("OWNER'S DIRECTIONS"));

    let directed = build_card_instructions("Eva", "leads the team", "make it stormy");
    // Owner directions take primacy over style defaults...
    assert!(directed.contains("OWNER'S DIRECTIONS"));
    assert!(directed.contains("make it stormy"));
    assert!(directed.contains("override the default art-style and copy guidance"));
    // ...but the fixed contract survives: frame, style anchor (as an
    // overridable default), and text-fidelity requirements stay present.
    assert!(directed.contains("match input image 2's art style EXACTLY"));
    assert!(directed.contains("cannot change the frame, layout, or"));
    assert!(directed.contains("Render all text with perfect fidelity"));
    // Card-text direction is an explicitly named capability, and the
    // owner-wording rule acknowledges the fixed 220-char text-box limit
    // (no mutually impossible "verbatim" vs "under 220 chars" pair).
    assert!(directed.contains("card text"));
    assert!(directed.contains("use their wording within the 220-character text-box limit"));
}

#[test]
fn extract_card_output_happy_path_and_missing_image() {
    let ok = serde_json::json!({
        "output": [
            {"type": "reasoning"},
            {"type": "image_generation_call", "result": "aW1n"},
            {"type": "message", "content": [
                {"type": "output_text", "text": "notes here"}
            ]}
        ]
    });
    let (img, notes) = extract_card_output(&ok).unwrap();
    assert_eq!(img, "aW1n");
    assert_eq!(notes, "notes here");

    let missing = serde_json::json!({"output": [{"type": "message", "content": []}]});
    let err = extract_card_output(&missing).unwrap_err();
    assert!(err.contains("No image"), "{err}");

    let no_output = serde_json::json!({});
    assert!(extract_card_output(&no_output).is_err());
}

#[test]
fn kind0_picture_wins_over_record_avatar_unless_blank() {
    let some = |s: &str| Some(s.to_string());
    // Published picture wins.
    assert_eq!(
        preferred_avatar_url(some("https://relay/media/k0.png"), some("data:image/png;x")),
        some("https://relay/media/k0.png")
    );
    // No profile / no picture: record avatar survives.
    assert_eq!(
        preferred_avatar_url(None, some("data:image/png;x")),
        some("data:image/png;x")
    );
    // Blank or whitespace picture must not shadow a real avatar.
    assert_eq!(
        preferred_avatar_url(some(""), some("data:image/png;x")),
        some("data:image/png;x")
    );
    assert_eq!(
        preferred_avatar_url(some("  "), some("data:image/png;x")),
        some("data:image/png;x")
    );
    // Nothing anywhere: None (caller surfaces the "no avatar" error).
    assert_eq!(preferred_avatar_url(None, None), None);
}

#[test]
fn unlocked_manifest_inlines_real_avatar_bytes_downscaled() {
    // 700px source (over MANIFEST_AVATAR_MAX_DIM) in a solid color.
    let avatar = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        700,
        700,
        image::Rgba([9, 120, 33, 255]),
    ));
    let mut avatar_png = std::io::Cursor::new(Vec::new());
    avatar
        .write_to(&mut avatar_png, image::ImageFormat::Png)
        .unwrap();

    let inlined = manifest_avatar_bytes(false, avatar_png.get_ref(), None)
        .unwrap()
        .expect("unlocked mints must inline the real avatar");
    let img = image::load_from_memory(&inlined).unwrap();
    assert_eq!(
        (img.width(), img.height()),
        (MANIFEST_AVATAR_MAX_DIM, MANIFEST_AVATAR_MAX_DIM)
    );
    assert_eq!(img.to_rgba8().get_pixel(0, 0).0, [9, 120, 33, 255]);

    // Undecodable avatar bytes fail the mint (pre-spend), never silently
    // produce a card whose import would wear the artwork as a face.
    assert!(manifest_avatar_bytes(false, b"not a png", None).is_err());
}

#[test]
fn locked_manifest_keeps_data_url_only_avatar() {
    // Locked mints must not inline fetched bytes (NIP-44 cap): only a record
    // data URL carries over, exactly as before.
    let unused = [0u8; 4];
    assert_eq!(
        manifest_avatar_bytes(true, &unused, Some("data:image/png;base64,aGk="))
            .unwrap()
            .as_deref(),
        Some(b"hi".as_slice())
    );
    assert_eq!(
        manifest_avatar_bytes(true, &unused, Some("https://relay/media/a.png")).unwrap(),
        None
    );
    assert_eq!(manifest_avatar_bytes(true, &unused, None).unwrap(), None);
}

#[test]
fn media_get_auth_gate_is_same_origin_only() {
    // The minted Blossom get-auth header may only travel to the relay's own
    // origin — scheme, host, and port all count (same contract as
    // `validate_download_url` in `media_download.rs`).
    let relay = "https://relay.example.com";
    assert!(is_same_origin(
        "https://relay.example.com/media/abc.png",
        relay
    ));
    // Different host, scheme, or port: no auth.
    assert!(!is_same_origin(
        "https://evil.example.com/media/abc.png",
        relay
    ));
    assert!(!is_same_origin(
        "http://relay.example.com/media/abc.png",
        relay
    ));
    assert!(!is_same_origin(
        "https://relay.example.com:8443/media/abc.png",
        relay
    ));
    // Unparseable inputs fail closed.
    assert!(!is_same_origin("not a url", relay));
    assert!(!is_same_origin(
        "https://relay.example.com/x",
        "also not a url"
    ));
    // Explicit port on both sides matches.
    assert!(is_same_origin(
        "http://localhost:3100/media/abc.png",
        "http://localhost:3100"
    ));
}

#[test]
fn avatar_cap_rejects_before_appending_crossing_chunk() {
    // The streaming accumulator must reject a chunk that would cross the
    // cap BEFORE buffering it — this is what bounds memory when
    // Content-Length is missing or dishonest.
    let mut buf = vec![0u8; MAX_AVATAR_FETCH_BYTES - 1];
    assert!(append_within_avatar_cap(&mut buf, &[0u8]).is_ok());
    assert_eq!(buf.len(), MAX_AVATAR_FETCH_BYTES);
    // Exactly at the cap: one more byte must fail and not grow the buffer.
    assert!(append_within_avatar_cap(&mut buf, &[0u8]).is_err());
    assert_eq!(buf.len(), MAX_AVATAR_FETCH_BYTES);

    // A single oversized chunk is rejected outright.
    let mut fresh = Vec::new();
    let oversized = vec![0u8; MAX_AVATAR_FETCH_BYTES + 1];
    assert!(append_within_avatar_cap(&mut fresh, &oversized).is_err());
    assert!(fresh.is_empty());
}

#[test]
fn save_rejects_plain_png_without_snapshot_chunk() {
    // A plain PNG (no buzz_agent_snapshot chunk) must not be saveable as
    // a card. Exercise the same validation the command runs.
    let img = image::DynamicImage::new_rgba8(4, 4);
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    assert!(decode_snapshot_png(&png).is_err());
}

#[test]
fn archived_sidecar_without_memory_level_defaults_to_none() {
    // Every mint before the memory option existed embedded MemoryLevel::None
    // structurally, so old sidecars (no memoryLevel field) must deserialize
    // to None — the gallery's disclosure depends on this being honest.
    let legacy = r#"{
        "storedFileName": "eva-1234.agent.png",
        "fileName": "eva.agent.png",
        "agentId": "abc",
        "agentName": "Eva",
        "designerNotes": "",
        "locked": false,
        "mintedAt": "2026-07-28T00:00:00Z"
    }"#;
    let meta: ArchivedCardMeta = serde_json::from_str(legacy).unwrap();
    assert_eq!(meta.memory_level, MemoryLevel::None);

    let with_level = legacy.replace(
        "\"locked\": false,",
        "\"locked\": false, \"memoryLevel\": \"everything\",",
    );
    let meta: ArchivedCardMeta = serde_json::from_str(&with_level).unwrap();
    assert_eq!(meta.memory_level, MemoryLevel::Everything);
}

#[test]
fn minted_card_serializes_memory_level_snake_case_value() {
    // The TS layer narrows on the exact wire strings "none"/"core"/
    // "everything" — pin the serde representation the frontend will see.
    let minted = MintedCard {
        card_png_base64: String::new(),
        file_name: "eva.agent.png".to_string(),
        designer_notes: String::new(),
        locked: false,
        memory_level: MemoryLevel::Core,
    };
    let json = serde_json::to_value(&minted).unwrap();
    assert_eq!(json["memoryLevel"], "core");
}
