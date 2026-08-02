//! Opt-in functional tests for the pinned Portuguese speech models.

use super::*;

#[test]
#[ignore = "requires BUZZ_PIPER_PT_BR_MODEL_DIR"]
fn piper_pt_br_emits_finite_non_silent_audio() {
    let model_dir = std::env::var("BUZZ_PIPER_PT_BR_MODEL_DIR")
        .expect("set BUZZ_PIPER_PT_BR_MODEL_DIR to the extracted Piper model");
    let engine =
        LocalTtsEngine::load(std::path::Path::new(&model_dir)).expect("load Piper pt-BR engine");
    assert_eq!(engine.language(), "pt-BR");
    let samples = engine
        .synth_chunk("Olá! O Buzz agora fala português brasileiro.", None)
        .expect("synthesize Portuguese speech");
    assert!(!samples.is_empty());
    assert!(samples.iter().all(|sample| sample.is_finite()));
    assert!(samples.iter().any(|sample| sample.abs() > 1.0e-6));
}

#[test]
#[ignore = "requires BUZZ_PIPER_PT_BR_MODEL_DIR and BUZZ_WHISPER_PT_BR_MODEL_DIR"]
fn portuguese_tts_to_stt_round_trip_produces_text() {
    let piper_dir =
        std::env::var("BUZZ_PIPER_PT_BR_MODEL_DIR").expect("set BUZZ_PIPER_PT_BR_MODEL_DIR");
    let whisper_dir =
        std::env::var("BUZZ_WHISPER_PT_BR_MODEL_DIR").expect("set BUZZ_WHISPER_PT_BR_MODEL_DIR");
    let engine =
        LocalTtsEngine::load(std::path::Path::new(&piper_dir)).expect("load Piper pt-BR engine");
    let sample_rate = i32::try_from(engine.sample_rate()).expect("valid sample rate");
    let samples = engine
        .synth_chunk("O Buzz fala português brasileiro.", None)
        .expect("synthesize Portuguese speech");

    let model_dir = std::path::Path::new(&whisper_dir);
    let mut config = sherpa_onnx::OfflineRecognizerConfig::default();
    config.model_config.whisper.encoder = Some(
        model_dir
            .join("tiny-encoder.int8.onnx")
            .to_string_lossy()
            .into_owned(),
    );
    config.model_config.whisper.decoder = Some(
        model_dir
            .join("tiny-decoder.int8.onnx")
            .to_string_lossy()
            .into_owned(),
    );
    config.model_config.whisper.language = Some("pt".to_string());
    config.model_config.whisper.task = Some("transcribe".to_string());
    config.model_config.tokens = Some(
        model_dir
            .join("tiny-tokens.txt")
            .to_string_lossy()
            .into_owned(),
    );
    config.model_config.num_threads = 1;
    let recognizer =
        sherpa_onnx::OfflineRecognizer::create(&config).expect("load Whisper Tiny multilingual");
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, &samples);
    recognizer.decode(&stream);
    let result = stream.get_result().expect("recognition result");
    assert!(
        !result.text.trim().is_empty(),
        "Whisper returned empty text"
    );
}
