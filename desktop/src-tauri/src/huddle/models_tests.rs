use super::*;

#[test]
fn voice_models_follow_the_selected_build_nest() {
    let home = PathBuf::from("/Users/example");
    for nest_name in [
        ".buzz",
        ".buzz-demo-workstream-board",
        ".buzz-demo-second-demo",
    ] {
        let nest = home.join(nest_name);
        assert_eq!(models_dir(nest.clone()), nest.join("models"));
    }
}

fn create_ready_model_dir(root: &Path) -> PathBuf {
    let model_dir = root.join(TTS_MODEL_DIR_NAME);
    std::fs::create_dir_all(&model_dir).expect("create model dir");
    for file in TTS_EXPECTED_FILES {
        let path = model_dir.join(file);
        let handle = std::fs::File::create(path).expect("create expected file");
        if let Some(size) = tts_expected_size(file) {
            handle.set_len(size).expect("size expected file");
        } else {
            std::fs::write(model_dir.join(file), b"test").expect("write expected file");
        }
    }
    std::fs::write(model_dir.join(MANIFEST_FILENAME), TTS_MODEL_VERSION).expect("manifest");
    model_dir
}

#[test]
fn expected_files_match_april_int8_metadata() {
    let mut expected = april_model_info()
        .artifacts
        .iter()
        .map(|artifact| artifact.filename)
        .chain([TTS_LICENSE_ARTIFACT.filename, TTS_LICENSE_FILE_NAME])
        .chain(POCKET_VOICES.iter().map(|voice| voice.reference_file))
        .collect::<Vec<_>>();
    expected.sort_unstable();
    let mut actual = TTS_EXPECTED_FILES.to_vec();
    actual.sort_unstable();

    assert_eq!(actual, expected);
    assert!(!actual.contains(&"flow_lm_main.onnx"));
    assert!(!actual.contains(&"flow_lm_flow.onnx"));
    assert!(!actual.contains(&"mimi_decoder.onnx"));
    assert!(!actual.contains(&"marius.wav"));
}

#[test]
fn tts_readiness_requires_license_sidecar() {
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = tts_model_slot();
    let model_dir = create_ready_model_dir(temp.path());

    assert!(slot.is_ready(temp.path()));

    std::fs::remove_file(model_dir.join(TTS_LICENSE_FILE_NAME)).expect("remove sidecar");
    assert!(!slot.is_ready(temp.path()));
}

#[test]
fn tts_readiness_rejects_truncated_pinned_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = tts_model_slot();
    let model_dir = create_ready_model_dir(temp.path());
    let artifact = april_model_info().artifacts[0];

    std::fs::OpenOptions::new()
        .write(true)
        .open(model_dir.join(artifact.filename))
        .expect("open artifact")
        .set_len(artifact.size_bytes - 1)
        .expect("truncate artifact");

    assert!(!slot.is_ready(temp.path()));
}

#[test]
fn january_cache_is_not_ready_for_april_int8() {
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = tts_model_slot();
    let model_dir = temp.path().join(TTS_MODEL_DIR_NAME);
    std::fs::create_dir_all(&model_dir).expect("create model dir");
    for file in [
        "decoder.onnx",
        "encoder.onnx",
        "lm_flow.onnx",
        "lm_main.onnx",
        "text_conditioner.onnx",
        "vocab.json",
        "token_scores.json",
        "LICENSE",
        "reference_sample.wav",
        TTS_LICENSE_FILE_NAME,
    ] {
        std::fs::write(model_dir.join(file), b"january").expect("write January file");
    }
    std::fs::write(model_dir.join(MANIFEST_FILENAME), "3").expect("manifest");

    assert!(!slot.is_ready(temp.path()));
}

#[test]
fn interrupted_install_restores_backup_when_destination_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = tts_model_slot();
    let backup_dir = temp.path().join("pocket-tts.old");
    std::fs::create_dir_all(&backup_dir).expect("create backup");
    std::fs::write(backup_dir.join("sentinel"), b"previous").expect("write sentinel");

    slot.recover_interrupted_install(temp.path());

    assert_eq!(
        std::fs::read(temp.path().join(TTS_MODEL_DIR_NAME).join("sentinel"))
            .expect("restored sentinel"),
        b"previous"
    );
    assert!(!backup_dir.exists());
}

#[test]
fn interrupted_install_replaces_incomplete_destination_with_backup() {
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = tts_model_slot();
    let model_dir = temp.path().join(TTS_MODEL_DIR_NAME);
    let backup_dir = temp.path().join("pocket-tts.old");
    std::fs::create_dir_all(&model_dir).expect("create incomplete destination");
    std::fs::write(model_dir.join("incomplete"), b"april").expect("write incomplete file");
    std::fs::create_dir_all(&backup_dir).expect("create backup");
    std::fs::write(backup_dir.join("sentinel"), b"previous").expect("write sentinel");

    slot.recover_interrupted_install(temp.path());

    assert_eq!(
        std::fs::read(model_dir.join("sentinel")).expect("restored sentinel"),
        b"previous"
    );
    assert!(!model_dir.join("incomplete").exists());
    assert!(!backup_dir.exists());
}

#[test]
fn ready_destination_removes_stale_backup() {
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = tts_model_slot();
    let model_dir = create_ready_model_dir(temp.path());
    let backup_dir = temp.path().join("pocket-tts.old");
    std::fs::create_dir_all(&backup_dir).expect("create backup");
    std::fs::write(backup_dir.join("sentinel"), b"previous").expect("write sentinel");

    slot.recover_interrupted_install(temp.path());

    assert!(slot.is_ready(temp.path()));
    assert!(model_dir.exists());
    assert!(!backup_dir.exists());
}

// ── STT model selection (issue #2478) ─────────────────────────────────────────

#[test]
fn defaults_to_english_without_override_or_locale() {
    assert_eq!(select_stt_model(None, None).id, "parakeet-en");
    assert_eq!(select_stt_model(None, Some("en-US")).id, "parakeet-en");
    assert_eq!(select_stt_model(None, Some("en")).id, "parakeet-en");
    assert_eq!(select_stt_model(None, None).auto_select_languages, &["en"]);
}

#[test]
fn supported_european_locale_selects_parakeet_v3() {
    for locale in ["de-DE", "uk_UA", "fr", "es-ES", "pl_PL.UTF-8"] {
        let model = select_stt_model(None, Some(locale));
        assert_eq!(model.id, "parakeet-v3", "locale {locale}");
    }
}

#[test]
fn cjk_locales_select_sensevoice() {
    for locale in ["zh-CN", "ja_JP", "ko-KR", "yue_HK.UTF-8"] {
        assert_eq!(
            select_stt_model(None, Some(locale)).id,
            "sensevoice",
            "locale {locale}"
        );
    }
}

#[test]
fn unsupported_locale_does_not_select_an_incompatible_multilingual_model() {
    assert_eq!(select_stt_model(None, Some("ar-SA")).id, "parakeet-en");
}

#[test]
fn explicit_override_wins_over_locale() {
    assert_eq!(
        select_stt_model(Some("parakeet-v3"), Some("en-US")).id,
        "parakeet-v3"
    );
    assert_eq!(
        select_stt_model(Some("parakeet-en"), Some("de-DE")).id,
        "parakeet-en"
    );
    assert_eq!(
        select_stt_model(Some("PARAKEET-V3"), None).id,
        "parakeet-v3"
    );
    assert_eq!(
        select_stt_model(Some("SENSEVOICE"), Some("de-DE")).id,
        "sensevoice"
    );
}

#[test]
fn unknown_or_empty_override_falls_back() {
    assert_eq!(
        select_stt_model(Some("does-not-exist"), Some("en-US")).id,
        "parakeet-en"
    );
    assert_eq!(
        select_stt_model(Some("does-not-exist"), Some("fr-FR")).id,
        "parakeet-v3"
    );
    assert_eq!(select_stt_model(Some("   "), None).id, "parakeet-en");
}

#[test]
fn registry_invariants_hold() {
    assert!(!STT_MODELS.is_empty());
    assert_eq!(default_stt_model().id, "parakeet-en");
    assert!(
        default_stt_model().archive_sha256.is_some(),
        "English default must ship a pinned SHA-256"
    );
    assert!(STT_MODELS
        .iter()
        .any(|model| model.auto_select_languages.len() > 1));
    for (index, model) in STT_MODELS.iter().enumerate() {
        assert!(!model.model_files.is_empty(), "{} has no files", model.id);
        assert!(model.max_download_bytes > 0, "{} has no size cap", model.id);
        for other in &STT_MODELS[index + 1..] {
            assert!(
                !model.id.eq_ignore_ascii_case(other.id),
                "duplicate model id {}",
                model.id
            );
            for language in model.auto_select_languages {
                assert!(
                    !other.auto_select_languages.contains(language),
                    "locale {language} is auto-selected by both {} and {}",
                    model.id,
                    other.id
                );
            }
        }
    }
}

#[test]
fn expected_files_always_include_license_sidecar() {
    for model in STT_MODELS {
        let files = stt_expected_files(model);
        assert!(
            files.contains(&STT_LICENSE_FILE_NAME),
            "{} missing license sidecar in expected files",
            model.id
        );
        for file in model.model_files {
            assert!(files.contains(file), "{} missing {file}", model.id);
        }
    }
}

#[test]
fn readiness_uses_per_model_expected_files() {
    let model = stt_model_by_id("parakeet-v3").expect("v3 registered");
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = ModelSlot::new(model.dir_name, stt_expected_files(model), model.version);
    let dir = temp.path().join(model.dir_name);
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(dir.join(MANIFEST_FILENAME), model.version).expect("manifest");

    std::fs::write(dir.join("encoder.int8.onnx"), b"x").expect("write");
    std::fs::write(dir.join("tokens.txt"), b"x").expect("write");
    assert!(!slot.is_ready(temp.path()));

    for file in stt_expected_files(model) {
        std::fs::write(dir.join(file), b"x").expect("write");
    }
    assert!(slot.is_ready(temp.path()));
}

#[test]
fn sensevoice_readiness_uses_single_model_file() {
    let model = stt_model_by_id("sensevoice").expect("SenseVoice registered");
    let temp = tempfile::tempdir().expect("tempdir");
    let slot = ModelSlot::new(model.dir_name, stt_expected_files(model), model.version);
    let dir = temp.path().join(model.dir_name);
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(dir.join(MANIFEST_FILENAME), model.version).expect("manifest");
    std::fs::write(dir.join("model.int8.onnx"), b"x").expect("model");
    std::fs::write(dir.join("tokens.txt"), b"x").expect("tokens");
    assert!(!slot.is_ready(temp.path()));

    std::fs::write(dir.join(STT_LICENSE_FILE_NAME), b"x").expect("license");
    assert!(slot.is_ready(temp.path()));
}
