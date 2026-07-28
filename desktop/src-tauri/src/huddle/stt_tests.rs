//! Tests for huddle/stt.rs — split into a sibling file to keep stt.rs
//! focused. Unit tests for the decode guards (collapse, prefix stitch,
//! lone-filler noise gate, sticky punctuation) plus the #[ignore]d decode
//! experiments that document measured model behavior.

#[cfg(test)]
mod stitch_prefix_collapse_tests {
    use super::super::stitch_prefix_collapse;

    // Strings from chl's 2026-07-24 trace (/tmp/buzz-dev-dictation.log).
    #[test]
    fn splices_at_the_overlap_anchor() {
        assert_eq!(
            stitch_prefix_collapse(
                "Big questions are coming out, I need to start asking.",
                "I need to start asking whether it's a little bit more."
            )
            .as_deref(),
            Some("Big questions are coming out, I need to start asking whether it's a little bit more.")
        );
    }

    #[test]
    fn appends_when_no_overlap_survived() {
        assert_eq!(
            stitch_prefix_collapse(
                "Big questions are coming out.",
                "I need to start asking whether it's"
            )
            .as_deref(),
            Some("Big questions are coming out. I need to start asking whether it's")
        );
    }

    #[test]
    fn tail_revision_wins_from_the_anchor_on() {
        assert_eq!(
            stitch_prefix_collapse(
                "Big questions are coming out, I need to start asking whether it's a little bit more.",
                "I need to start asking whether or not that's the same"
            )
            .as_deref(),
            Some("Big questions are coming out, I need to start asking whether or not that's the same")
        );
    }

    #[test]
    fn honest_revisions_pass_through() {
        // Front agrees → not a drop.
        assert!(
            stitch_prefix_collapse("Big question to come.", "Big questions are coming up.")
                .is_none()
        );
        // Short phrases rewrite themselves legitimately.
        assert!(stitch_prefix_collapse("Yeah.", "Big question.").is_none());
        assert!(stitch_prefix_collapse("Costing.", "Calls passing.").is_none());
    }
}

#[cfg(test)]
mod lone_filler_tests {
    use super::super::lone_filler;

    #[test]
    fn trace_strings() {
        // The noise phrases from the 2026-07-24 ramble trace.
        assert!(lone_filler("Yeah."));
        assert!(lone_filler("Mm."));
        assert!(lone_filler("Uh-huh."));
        // Real speech that merely starts with or resembles filler survives.
        assert!(!lone_filler("Yeah, so"));
        assert!(!lone_filler("The problem."));
        assert!(!lone_filler("Testing."));
        assert!(!lone_filler(""));
    }
}

#[cfg(test)]
mod decode_collapsed_tests {
    use super::super::decode_collapsed;

    #[test]
    fn collapse_detection() {
        let best = "caused by some other things";
        assert!(decode_collapsed("", best));
        assert!(decode_collapsed("Yeah.", best));
        assert!(decode_collapsed("Cause personal things.", best));
        // Honest revisions and growth keep the new decode.
        assert!(!decode_collapsed("Caused by some other things.", best));
        assert!(!decode_collapsed("caused by some other things too", best));
        // No baseline words → nothing to protect.
        assert!(!decode_collapsed("", ""));
    }
}

/// The tokio channel's `blocking_send` is safe to call from sync contexts.
#[cfg(test)]
mod prefer_punctuated_tests {
    use super::super::prefer_punctuated;

    #[test]
    fn keeps_dropped_terminal_punctuation() {
        let hint = Some("Is there anything we can do about that?");
        assert_eq!(
            prefer_punctuated("is there anything we can do about that".into(), hint),
            "Is there anything we can do about that?"
        );
    }

    #[test]
    fn internal_punctuation_flicker_still_matches() {
        // Observed in live_partial_sequence: the final decode swapped an
        // internal "." for "," — same words, so the punctuated hint wins.
        let hint = Some("I'm going to ask a question. Do you know why that happened?");
        assert_eq!(
            prefer_punctuated(
                "I'm going to ask a question, Do you know why that happened".into(),
                hint
            ),
            "I'm going to ask a question. Do you know why that happened?"
        );
    }

    #[test]
    fn grafts_terminal_mark_when_only_last_word_agrees() {
        assert_eq!(
            prefer_punctuated(
                "do you know er why that happened,".into(),
                Some("Do you know why that happened?")
            ),
            "do you know er why that happened?"
        );
    }

    #[test]
    fn comma_hint_is_never_grafted() {
        assert_eq!(
            prefer_punctuated("hello there friend".into(), Some("hello there,")),
            "hello there friend"
        );
    }

    #[test]
    fn ignores_hint_when_text_differs() {
        assert_eq!(
            prefer_punctuated("hello world again".into(), Some("hello world.")),
            "hello world again"
        );
    }

    #[test]
    fn never_replaces_empty_final() {
        assert_eq!(prefer_punctuated(String::new(), Some("hello.")), "");
    }

    #[test]
    fn no_hint_passes_through() {
        assert_eq!(prefer_punctuated("hello".into(), None), "hello");
    }
}

#[cfg(test)]
mod silence_experiment {
    use super::super::{decode_speech, prefer_punctuated, process_16k_samples};

    /// Manual experiment: does Parakeet v3 (0.6B transducer) produce stable
    /// punctuation under noisy pauses where the 110M CTC model flickers?
    /// Also times each decode — the live mode re-decodes the whole phrase
    /// every ~300 ms, so per-decode latency bounds the streaming cadence.
    /// Run: cargo test v3_punctuation -- --ignored --nocapture
    /// Needs ~/.buzz/models/parakeet-tdt-0.6b-v3 downloaded (see models.rs).
    #[test]
    #[ignore]
    fn v3_punctuation() {
        for threads in [1i32, 2, 4] {
            let recognizer = v3_recognizer(threads);
            println!("--- num_threads = {threads} ---");
            run_decodes(&recognizer);
        }
    }

    fn v3_recognizer(threads: i32) -> sherpa_onnx::OfflineRecognizer {
        let model_dir = dirs::home_dir()
            .unwrap()
            .join(".buzz/models/parakeet-tdt-0.6b-v3");
        let mut cfg = sherpa_onnx::OfflineRecognizerConfig::default();
        cfg.model_config.transducer.encoder = Some(
            model_dir
                .join("encoder.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.transducer.decoder = Some(
            model_dir
                .join("decoder.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.transducer.joiner = Some(
            model_dir
                .join("joiner.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.tokens = Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
        cfg.model_config.debug = false;
        cfg.model_config.num_threads = threads;
        sherpa_onnx::OfflineRecognizer::create(&cfg).unwrap()
    }

    fn read_wav_16k(path: &str) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap();
        let data_pos = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("no data chunk")
            + 8;
        bytes[data_pos..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    /// Manual experiment: chl reported that during a long continuous ramble
    /// "a whole bunch of text just got removed from the end". Two suspects:
    /// (a) v3 transducer output degrades/truncates as the phrase buffer grows
    ///     toward the 30 s MAX_SPEECH_SAMPLES cap, so a later partial decode
    ///     yields LESS text than an earlier one and the frontend diff deletes
    ///     the tail;
    /// (b) per-decode latency on long buffers (~70 ms/s of audio) blows past
    ///     the 300 ms partial cadence, the worker falls behind and the 5 s
    ///     audio queue overflows.
    /// This decodes growing prefixes of a 35 s ramble, timing each decode and
    /// flagging any shrinkage between consecutive prefixes.
    /// Run: cargo test v3_long_buffer -- --ignored --nocapture
    /// Setup: say -v Samantha "<long ~35s text>" -o /tmp/dict_long.wav
    ///   --data-format=LEF32@16000
    #[test]
    #[ignore]
    fn v3_long_buffer() {
        let recognizer = v3_recognizer(super::super::LIVE_STT_NUM_THREADS);
        let samples = read_wav_16k("/tmp/dict_long.wav");
        println!("total: {:.1}s", samples.len() as f32 / 16_000.0);
        let mut prev = String::new();
        for secs in [4, 8, 12, 16, 20, 22, 24, 26, 28, 30] {
            let end = (16_000 * secs).min(samples.len());
            let t0 = std::time::Instant::now();
            let text = decode_speech(&samples[..end], &recognizer);
            let shrink = if text.len() < prev.len() {
                "  <<< SHRANK"
            } else {
                ""
            };
            println!(
                "@{secs:>2}s [{:>6.0?}ms] {} chars{shrink}: {text:?}",
                t0.elapsed().as_millis(),
                text.len()
            );
            prev = text;
            if end == samples.len() {
                break;
            }
        }

        // Simulate the worker's adaptive partial cadence over the same audio:
        // next partial waits max(PARTIAL_DECODE_STEP, last decode duration).
        // Real-time safe iff cumulative decode time stays under audio time.
        println!("--- adaptive cadence simulation ---");
        let mut last_len = 0usize;
        let mut step = super::super::PARTIAL_DECODE_STEP;
        let mut decode_total = std::time::Duration::ZERO;
        let mut n = 0u32;
        while last_len + step <= samples.len() {
            last_len += step;
            let t0 = std::time::Instant::now();
            let text = decode_speech(&samples[..last_len], &recognizer);
            let dt = t0.elapsed();
            decode_total += dt;
            step = super::super::PARTIAL_DECODE_STEP.max(32 * dt.as_millis() as usize);
            n += 1;
            println!(
                "partial {n} @ audio {:>4.1}s: decode {:>4.0?}ms, next step {:.1}s, tail {:?}",
                last_len as f32 / 16_000.0,
                dt.as_millis(),
                step as f32 / 16_000.0,
                &text[text.len().saturating_sub(40)..]
            );
        }
        let audio_secs = last_len as f32 / 16_000.0;
        println!(
            "{n} partials over {audio_secs:.1}s audio, total decode {:.1}s → real-time safe: {}",
            decode_total.as_secs_f32(),
            decode_total.as_secs_f32() < audio_secs
        );
    }

    fn run_decodes(recognizer: &sherpa_onnx::OfflineRecognizer) {
        for path in ["/tmp/dict_multi.wav", "/tmp/dict_q.wav"] {
            let bytes = std::fs::read(path).unwrap();
            let data_pos = bytes
                .windows(4)
                .position(|w| w == b"data")
                .expect("no data chunk")
                + 8;
            let samples: Vec<f32> = bytes[data_pos..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            println!(
                "=== {path} ({:.1}s speech) ===",
                samples.len() as f32 / 16_000.0
            );
            for (label, amp, tail_ms) in [
                ("bare", 0.0f32, 0usize),
                ("300ms noise", 0.01, 300),
                ("608ms noise", 0.01, 608),
                ("608ms loud noise", 0.03, 608),
            ] {
                let mut buf = samples.clone();
                let mut state = 0x2545_f491u32;
                for _ in 0..(16 * tail_ms) {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    buf.push((state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp);
                }
                let t0 = std::time::Instant::now();
                let text = decode_speech(&buf, recognizer);
                println!("  +{label}: [{:?}] {text:?}", t0.elapsed());
            }
        }
    }

    /// Manual experiment: simulate the live worker's exact cadence over real
    /// (synthesized) speech with a noisy tail — decode the growing buffer
    /// every PARTIAL_DECODE_STEP samples, track `punct_partial` like the
    /// worker does, then run the final flush through `prefer_punctuated`.
    /// Shows whether the sticky-punctuation hint actually matches on
    /// multi-sentence phrases.
    /// Run: cargo test live_partial_sequence -- --ignored --nocapture
    /// Setup: say -v Samantha "It missed the full stop on the first
    ///   sentence. I'm now going to try and ask a question. Do you know why
    ///   that happened" -o /tmp/dict_multi.wav --data-format=LEF32@16000
    #[test]
    #[ignore]
    fn live_partial_sequence() {
        let model_dir = dirs::home_dir()
            .unwrap()
            .join(".buzz/models/parakeet-tdt-ctc-110m-en");
        let mut cfg = sherpa_onnx::OfflineRecognizerConfig::default();
        cfg.model_config.nemo_ctc.model = Some(
            model_dir
                .join("model.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.tokens = Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
        cfg.model_config.num_threads = 1;
        cfg.model_config.debug = false;
        let recognizer = sherpa_onnx::OfflineRecognizer::create(&cfg).unwrap();

        for (path, amp) in [
            ("/tmp/dict_multi.wav", 0.01f32),
            ("/tmp/dict_q.wav", 0.01f32),
        ] {
            let bytes = std::fs::read(path).unwrap();
            let data_pos = bytes
                .windows(4)
                .position(|w| w == b"data")
                .expect("no data chunk")
                + 8;
            let mut samples: Vec<f32> = bytes[data_pos..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            // Noisy silence tail up to the live flush window (~608 ms).
            let mut state = 0x2545_f491u32;
            for _ in 0..(16 * 608) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                samples.push((state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp);
            }
            println!("=== {path} (noise amp {amp}) ===");
            let mut punct_partial: Option<String> = None;
            let mut last_len = 0usize;
            while last_len + super::super::PARTIAL_DECODE_STEP <= samples.len() {
                last_len += super::super::PARTIAL_DECODE_STEP;
                let text = decode_speech(&samples[..last_len], &recognizer);
                if text.ends_with(['.', '?', '!', ',']) {
                    punct_partial = Some(text.clone());
                }
                println!("  partial@{last_len}: {text:?}");
            }
            let final_text = decode_speech(&samples, &recognizer);
            println!("  FINAL:  {final_text:?}");
            println!("  HINT:   {punct_partial:?}");
            println!(
                "  KEPT:   {:?}",
                prefer_punctuated(final_text, punct_partial.as_deref())
            );
            // Rejected candidate fix (measured 2026-07-24): replacing the
            // noisy VAD hangover with digital zeros at flush time sometimes
            // REMOVES punctuation the noisy final had — tail composition is
            // non-monotonic, so don't retry audio-level recipes.
            let speech_end = samples.len() - 16 * 608;
            let mut zeroed = samples[..speech_end].to_vec();
            zeroed.extend(std::iter::repeat_n(0.0f32, 16 * 600));
            println!("  ZEROED-TAIL: {:?}", decode_speech(&zeroed, &recognizer));
            // Same but with noise overlaid on the speech itself (real mics
            // pick up room noise under the voice too, not just in pauses).
            let mut state = 0x1234_5678u32;
            let mut noisy: Vec<f32> = samples[..speech_end]
                .iter()
                .map(|s| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    s + (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                })
                .collect();
            noisy.extend(std::iter::repeat_n(0.0f32, 16 * 600));
            println!(
                "  NOISY-SPEECH+ZEROED-TAIL: {:?}",
                decode_speech(&noisy, &recognizer)
            );
        }
    }

    /// Manual experiment: reproduce chl's "captured it, then deleted it and
    /// wrote Yeah" report (2026-07-24, Enhanced/v3 model) at the worker level.
    /// Drives `process_16k_samples` exactly like the live worker — real VAD,
    /// onset debounce, silence flush, partial cadence — over composed audio:
    /// room noise, the spoken sentence (noise overlaid), a post-speech tail,
    /// then the 1 s stop-flush zeros. A frontend simulator applies the events
    /// the way useDictation.ts does and prints what the composer would show.
    /// Run: cargo test v3_live_worker_sim -- --ignored --nocapture
    /// Setup: say -v Samantha "just testing to see if this is working any
    ///   better" -o /tmp/dict_better.wav --data-format=LEF32@16000
    #[test]
    #[ignore]
    fn v3_live_worker_sim() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let recognizer = v3_recognizer(super::super::LIVE_STT_NUM_THREADS);
        let speech = read_wav_16k("/tmp/dict_better.wav");

        let noise = |len: usize, amp: f32, seed: u32| {
            let mut state = seed;
            (0..len)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                })
                .collect::<Vec<f32>>()
        };
        let overlay = |samples: &[f32], amp: f32| {
            let mut state = 0x1234_5678u32;
            samples
                .iter()
                .map(|s| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    s + (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                })
                .collect::<Vec<f32>>()
        };

        // (label, pre-speech, post-speech tail before the stop flush)
        let scenarios: Vec<(&str, Vec<f32>, Vec<f32>)> = vec![
            (
                "release right after speaking",
                noise(16 * 500, 0.01, 0xA1),
                noise(16 * 400, 0.01, 0xB2),
            ),
            (
                "pause >608ms then breath then release",
                noise(16 * 500, 0.01, 0xA1),
                [
                    noise(16 * 900, 0.01, 0xB2),
                    noise(16 * 250, 0.08, 0xC3), // breath/exhale burst
                    noise(16 * 300, 0.01, 0xD4),
                ]
                .concat(),
            ),
            (
                "louder room noise",
                noise(16 * 500, 0.03, 0xA1),
                noise(16 * 700, 0.03, 0xB2),
            ),
        ];

        for (label, pre, tail) in scenarios {
            println!("=== scenario: {label} ===");
            let mut audio = pre;
            audio.extend(overlay(&speech, 0.01));
            audio.extend(tail);
            audio.extend(std::iter::repeat_n(0.0f32, 16_000)); // stop-flush 1s zeros

            let (live_tx, mut live_rx) = tokio::sync::mpsc::channel::<super::super::LiveEvent>(1024);
            let (text_tx, _text_rx) = tokio::sync::mpsc::channel::<String>(64);
            let tts_active = Arc::new(AtomicBool::new(false));

            let mut vad = earshot::Detector::new(earshot::DefaultPredictor::new());
            let mut leftover = Vec::new();
            let mut speech_buf = Vec::new();
            let mut silence_frames = 0usize;
            let mut in_speech = false;
            let mut barge_in_frames = 0usize;
            let mut tts_stopped_at = None;
            let mut last_partial_len = 0usize;
            let mut decode_hold_until = std::time::Instant::now();
            let mut onset_buf = Vec::new();
            let mut punct_partial: Option<String> = None;
            let mut best_partial: Option<String> = None;

            for chunk in audio.chunks(320) {
                process_16k_samples(
                    chunk,
                    &mut leftover,
                    &mut vad,
                    &mut speech_buf,
                    &mut silence_frames,
                    &mut in_speech,
                    &mut barge_in_frames,
                    &recognizer,
                    &text_tx,
                    &tts_active,
                    None,
                    &mut tts_stopped_at,
                    None,
                    Some(&live_tx),
                    &mut last_partial_len,
                    &mut decode_hold_until,
                    &mut onset_buf,
                    &mut punct_partial,
                    &mut best_partial,
                );
            }
            drop(live_tx);

            // Frontend simulator — the transcriptDiff mechanics reduce to this.
            let mut committed = String::new();
            let mut partial = String::new();
            while let Ok(event) = live_rx.try_recv() {
                match event {
                    super::super::LiveEvent::Transcript { text, is_final } => {
                        let trimmed = text.trim();
                        println!(
                            "  {} {trimmed:?}",
                            if is_final { "FINAL  " } else { "partial" }
                        );
                        if is_final {
                            if !trimmed.is_empty() {
                                committed.push_str(trimmed);
                                committed.push(' ');
                            } else if !partial.is_empty() {
                                println!("  !!! empty final wiped shown partial {partial:?}");
                            }
                            partial.clear();
                        } else if !trimmed.is_empty() {
                            if trimmed.len() + 10 < partial.len() {
                                println!("  !!! partial shrank {partial:?} -> {trimmed:?}");
                            }
                            partial = trimmed.to_string();
                        }
                    }
                    super::super::LiveEvent::Flushed => println!("  FLUSHED"),
                }
            }
            println!("  composer: {:?}", format!("{committed}{partial}"));
        }
    }

    /// Manual experiment: how much trailing silence does the model need before
    /// it commits sentence-final punctuation? (Answer when tuned: ≥600 ms —
    /// see LIVE_SILENCE_FLUSH_FRAMES.)
    /// Run: cargo test silence_vs_punctuation -- --ignored --nocapture
    /// Setup: say -v Samantha "is there anything we can do about that?" \
    ///          -o /tmp/dict_q.wav --data-format=LEF32@16000   (same for dict_s)
    #[test]
    #[ignore]
    fn silence_vs_punctuation() {
        let model_dir = dirs::home_dir()
            .unwrap()
            .join(".buzz/models/parakeet-tdt-ctc-110m-en");
        let mut cfg = sherpa_onnx::OfflineRecognizerConfig::default();
        cfg.model_config.nemo_ctc.model = Some(
            model_dir
                .join("model.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.tokens = Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
        cfg.model_config.num_threads = 1;
        cfg.model_config.debug = false;
        let recognizer = sherpa_onnx::OfflineRecognizer::create(&cfg).unwrap();

        for path in ["/tmp/dict_q.wav", "/tmp/dict_s.wav"] {
            let bytes = std::fs::read(path).unwrap();
            // Naive WAV parse: find the "data" chunk, samples are f32 LE.
            let data_pos = bytes
                .windows(4)
                .position(|w| w == b"data")
                .expect("no data chunk")
                + 8;
            let samples: Vec<f32> = bytes[data_pos..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            println!("=== {path} ({} samples) ===", samples.len());
            for silence_ms in [0usize, 100, 300, 600] {
                let mut buf = samples.clone();
                buf.extend(std::iter::repeat_n(0.0f32, 16 * silence_ms));
                println!(
                    "  +{silence_ms}ms zeros: {:?}",
                    decode_speech(&buf, &recognizer)
                );
            }
            // Real mic "silence" is room noise, not zeros — simulate with
            // low-level deterministic pseudo-noise (LCG; no rand dep) at
            // amplitudes well below the VAD speech threshold.
            for amp in [0.002f32, 0.01, 0.03] {
                let noise = |len: usize| {
                    let mut state = 0x2545_f491u32;
                    (0..len)
                        .map(|_| {
                            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                        })
                        .collect::<Vec<f32>>()
                };
                let mut buf = samples.clone();
                buf.extend(noise(16 * 608));
                println!(
                    "  +608ms noise(amp {amp}): {:?}",
                    decode_speech(&buf, &recognizer)
                );
                buf.extend(std::iter::repeat_n(0.0f32, 16 * 600));
                println!(
                    "  +608ms noise(amp {amp}) + 600ms zeros: {:?}",
                    decode_speech(&buf, &recognizer)
                );
            }
        }
    }
}
