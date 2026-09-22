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
