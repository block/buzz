use std::{
    fs,
    path::{Path, PathBuf},
};

use buzz_voice::{
    imported::{write_pcm16_wav, PcmStats, PocketVoiceLibrary},
    pocket::{load_text_to_speech, load_voice_style, DEFAULT_VOICE, SAMPLE_RATE, VOICE_FILE_EXT},
};

const PREVIEW_TEXT: &str = "This is an objective Pocket voice preview.";

fn required_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to the required local test path"))
}

fn checked_in_voice() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../desktop/src-tauri/resources/pocket-voices/eve.wav")
}

fn evidence_dir() -> PathBuf {
    std::env::var_os("BUZZ_VOICE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/buzz-voice-evidence")
        })
}

fn synthesize(model_dir: &Path, voice_path: &Path, text: &str) -> (Vec<f32>, PcmStats) {
    let engine = load_text_to_speech(
        model_dir
            .to_str()
            .expect("Pocket model path must be valid UTF-8"),
    )
    .expect("load Pocket model");
    let style = load_voice_style(voice_path).expect("load selected voice");
    let samples = engine
        .synth_chunk(text, "en", &style, 1)
        .expect("synthesize preview");
    let stats = PcmStats::analyze(&samples, SAMPLE_RATE);
    assert!(
        stats.is_non_silent(),
        "generated PCM must be non-silent: {stats:?}"
    );
    assert!(
        stats.duration_seconds > 0.2,
        "generated PCM is unexpectedly short: {stats:?}"
    );
    (samples, stats)
}

#[test]
#[ignore = "requires BUZZ_POCKET_MODEL_DIR and runs the installed Pocket ONNX model"]
fn objective_import_synthesis_delete_and_mary_fallback() {
    let model_dir = required_path("BUZZ_POCKET_MODEL_DIR");
    let temp = tempfile::tempdir().expect("temporary voice workspace");
    let source = temp.path().join("Imported Eve.wav");
    fs::copy(checked_in_voice(), &source).expect("copy checked-in voice fixture");

    let library_root = temp.path().join("library");
    let library = PocketVoiceLibrary::new(&library_root);
    let imported = library.import_path(&source).expect("import valid WAV");
    assert_eq!(
        library.find(&imported.key).expect("read selection"),
        Some(imported.clone())
    );

    drop(library);
    let relaunched = PocketVoiceLibrary::new(&library_root);
    let selected = relaunched
        .find(&imported.key)
        .expect("reload persisted selection")
        .expect("selected imported voice survived relaunch");
    let imported_path = relaunched
        .resolve_file(&selected)
        .expect("resolve persisted imported voice");
    let (imported_pcm, imported_stats) = synthesize(&model_dir, &imported_path, PREVIEW_TEXT);

    let evidence = evidence_dir();
    fs::create_dir_all(&evidence).expect("create evidence directory");
    let imported_wav = evidence.join("imported-preview.wav");
    write_pcm16_wav(&imported_wav, &imported_pcm, SAMPLE_RATE)
        .expect("write imported preview evidence");

    relaunched
        .delete(&imported.key)
        .expect("delete imported voice");
    assert_eq!(
        relaunched.find(&imported.key).expect("reload after delete"),
        None
    );

    let mary_path = model_dir.join(format!("{DEFAULT_VOICE}.{VOICE_FILE_EXT}"));
    assert_eq!(
        mary_path.file_name().and_then(|name| name.to_str()),
        Some("reference_sample.wav"),
        "fallback must remain the deterministic Mary reference"
    );
    let (mary_pcm, mary_stats) = synthesize(&model_dir, &mary_path, PREVIEW_TEXT);
    let mary_wav = evidence.join("mary-fallback-preview.wav");
    write_pcm16_wav(&mary_wav, &mary_pcm, SAMPLE_RATE)
        .expect("write Mary fallback preview evidence");

    println!(
        "{}",
        serde_json::json!({
            "importedKey": imported.key,
            "persistence": "reloaded",
            "afterDelete": "pocket:mary",
            "importedPreview": {
                "path": imported_wav,
                "samples": imported_stats.sample_count,
                "sampleRate": imported_stats.sample_rate,
                "durationSeconds": imported_stats.duration_seconds,
                "peak": imported_stats.peak,
                "rms": imported_stats.rms,
                "nonSilentSamples": imported_stats.non_silent_samples,
            },
            "maryFallbackPreview": {
                "path": mary_wav,
                "samples": mary_stats.sample_count,
                "sampleRate": mary_stats.sample_rate,
                "durationSeconds": mary_stats.duration_seconds,
                "peak": mary_stats.peak,
                "rms": mary_stats.rms,
                "nonSilentSamples": mary_stats.non_silent_samples,
            }
        })
    );
}
