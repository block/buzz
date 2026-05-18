//! Pocket TTS engine wrapper around sherpa-onnx's `OfflineTts`.
//!
//! Pocket TTS is a small (~189 MB int8 ONNX) zero-shot voice-cloning TTS
//! model from Kyutai. It runs quickly on CPU via sherpa-onnx, replacing the
//! previous Kokoro-82M engine that also required an espeak-free but
//! lexicon-heavy G2P pipeline (Misaki + CMUdict).
//!
//! ## Attribution
//!
//! - **Model**: Kyutai *Pocket TTS* — Charles, Roebel, et al., 2026.
//!   arXiv:2509.06926. Original repository: <https://huggingface.co/kyutai/pocket-tts>.
//!   Licensed CC-BY-4.0.
//! - **Mimi neural codec**: Kyutai, bundled in the same release. CC-BY-4.0.
//! - **ONNX export**: KevinAHM —
//!   <https://huggingface.co/KevinAHM/pocket-tts-onnx>. CC-BY-4.0.
//! - **sherpa-onnx repackage**: csukuangfj / k2-fsa —
//!   <https://huggingface.co/csukuangfj2/sherpa-onnx-pocket-tts-int8-2026-01-26>.
//!   Repackages KevinAHM's export with the file layout sherpa-onnx's
//!   `OfflineTtsPocketModelConfig` expects. CC-BY-4.0.
//! - **Reference voice WAV** (`reference_sample.wav`): the "Mary
//!   (f, conversation)" preset from the Kyutai TTS demo
//!   (<https://kyutai.org/tts>), which maps to `vctk/p333_023_enhanced.wav`
//!   in <https://huggingface.co/kyutai/tts-voices>. CC-BY-4.0, base recording
//!   from the VCTK corpus, enhanced by ai-coustics.
//!
//! Sprout ships these files unmodified; see the on-disk `MODEL_LICENSE.txt`
//! sidecar written by `huddle::models` during install for the canonical
//! CC-BY-4.0 §3(a)(1) attribution block.
//!
//! ## Engine-module contract (see `huddle::tts`)
//!
//! `pocket.rs` exposes a fixed surface used by `tts.rs`. Mirroring this
//! contract is what lets the TTS pipeline stay engine-agnostic:
//!
//! - `SAMPLE_RATE: u32`             — engine output sample rate in Hz.
//! - `DEFAULT_VOICE: &str`          — default voice name (without extension).
//! - `VOICE_FILE_EXT: &str`         — extension for per-voice files on disk.
//! - `load_text_to_speech(model_dir)`              → `Result<Engine, String>`
//! - `load_voice_style(path)`                      → `Result<VoiceStyle, String>`
//! - `Engine::synth_chunk(&self, text, lang, &VoiceStyle, steps, speed)`
//!                                                 → `Result<Vec<f32>, String>`
//!
//! `lang` and `steps` are accepted for API compatibility with the previous
//! Kokoro engine but are unused — Pocket TTS does its own language ID from
//! the input text and is not a diffusion model (consistency LM, one step).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, Wave};

// ── Engine-module contract: public consts ─────────────────────────────────────

/// Pocket TTS emits 24 kHz mono PCM. Matches the previous Kokoro output rate,
/// so the rodio sink and inter-sentence silence buffer in `tts.rs` remain valid.
pub const SAMPLE_RATE: u32 = 24_000;

/// Name (without extension) of the bundled reference voice. The model directory
/// is expected to contain `<DEFAULT_VOICE>.<VOICE_FILE_EXT>` after install.
pub const DEFAULT_VOICE: &str = "reference_sample";

/// Voice files for Pocket TTS are reference audio (WAV). Distinct from the
/// Kokoro `.bin` style vectors — the model conditions on raw waveform samples,
/// not a precomputed embedding, so the extension change is honest.
pub const VOICE_FILE_EXT: &str = "wav";

// ── Tuning ────────────────────────────────────────────────────────────────────

/// Single-threaded ONNX execution for predictable CPU contention with the STT
/// pipeline. Matches `STT_NUM_THREADS` in `stt.rs`; raise only if a benchmark
/// argues for it.
const TTS_NUM_THREADS: i32 = 1;

/// LRU cache size for cloned voice embeddings inside the sherpa-onnx engine.
/// We bind to one voice per pipeline today, but the upstream example uses 16
/// and the cost is negligible — keep room for future multi-voice support.
const VOICE_EMBEDDING_CACHE_CAPACITY: i32 = 16;

/// Pocket TTS is a consistency-based LM. Generation quality saturates at one
/// denoising step — the upstream `GenerationConfig` default of 5 multiplies
/// synthesis time by ~5× with no audible benefit on this model.
const SYNTH_NUM_STEPS: i32 = 1;

/// Disable the upstream default 200 ms of pre/post silence padding. We splice
/// `INTER_SENTENCE_SILENCE` in `tts.rs` ourselves and don't want a double
/// helping of leading silence on every utterance.
const SYNTH_SILENCE_SCALE: f32 = 0.0;

/// Mimi codec frame rate — the LM samples one latent per 80 ms. Used to convert
/// a token-count estimate into a `max_frames` cap, mirroring upstream
/// `pocket_tts.models.tts_model._estimate_max_gen_len`.
const MIMI_FRAME_RATE: f32 = 12.5;

/// Upstream-derived "expected tokens per second of speech" for short inputs.
/// Used by [`estimate_max_frames`] together with `GEN_SECONDS_PADDING` to cap
/// runaway generation when the EOS logit fails to fire. Source:
/// `pocket_tts.models.tts_model.TTSModel._TOKENS_PER_SECOND_ESTIMATE`.
const TOKENS_PER_SECOND_ESTIMATE: f32 = 3.0;

/// Slack added to the token-derived gen-length estimate, in seconds. Source:
/// `pocket_tts.models.tts_model.TTSModel._GEN_SECONDS_PADDING`.
const GEN_SECONDS_PADDING: f32 = 2.0;

/// Hard ceiling on per-chunk generation length, in Mimi frames. Matches the
/// sherpa-onnx upstream default (`offline-tts-pocket-impl.h:max_frames`) and
/// is the worst-case bound we'll ever ask for. 500 frames = 40 s of audio.
const MAX_FRAMES_HARD_CEILING: i32 = 500;

/// Word-count threshold (inclusive) below which we (a) pad the prompt with
/// leading spaces and (b) ask for `frames_after_eos = 3` instead of 1.
/// Matches upstream `pocket_tts.models.tts_model.prepare_text_prompt`.
const SHORT_PROMPT_WORD_THRESHOLD: usize = 4;

/// Number of leading spaces prepended to short prompts. The upstream Python
/// uses exactly 8 — keep parity rather than tuning blindly.
const SHORT_PROMPT_PAD_SPACES: usize = 8;

// ── ONNX file names (five Pocket TTS sessions plus two JSON tables) ───────────

const FILE_LM_MAIN: &str = "lm_main.int8.onnx";
const FILE_LM_FLOW: &str = "lm_flow.int8.onnx";
const FILE_ENCODER: &str = "encoder.onnx";
const FILE_DECODER: &str = "decoder.int8.onnx";
const FILE_TEXT_COND: &str = "text_conditioner.onnx";
const FILE_VOCAB: &str = "vocab.json";
const FILE_TOKEN_SCORES: &str = "token_scores.json";

// ── Voice style ───────────────────────────────────────────────────────────────

/// Loaded reference voice — normalised f32 PCM samples plus their sample rate.
///
/// Pocket TTS takes a reference waveform per generation call (not a
/// precomputed style embedding), so we keep the samples in memory and clone
/// the small `Vec` into each `GenerationConfig` rather than re-reading the
/// WAV from disk on every sentence.
#[derive(Debug, Clone)]
pub struct VoiceStyle {
    samples: Vec<f32>,
    sample_rate: i32,
}

/// Load a reference voice WAV from disk.
///
/// Accepts any sample rate sherpa-onnx's `Wave::read` can decode — Pocket TTS
/// resamples internally using `reference_sample_rate`. The bundled
/// `reference_sample.wav` ("Mary" — VCTK p333, enhanced) is 32 kHz mono.
pub fn load_voice_style(path: &Path) -> Result<VoiceStyle, String> {
    let path_str = path
        .to_str()
        .ok_or_else(|| format!("voice path is not valid UTF-8: {}", path.display()))?;
    let wave = Wave::read(path_str)
        .ok_or_else(|| format!("could not read voice WAV at {}", path.display()))?;
    let samples = wave.samples().to_vec();
    if samples.is_empty() {
        return Err(format!("voice WAV is empty: {}", path.display()));
    }
    Ok(VoiceStyle {
        samples,
        sample_rate: wave.sample_rate(),
    })
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// Pocket TTS engine handle. Cheap to construct (one `OfflineTts::create`
/// call). Owned by the TTS worker thread for the lifetime of a huddle session.
///
/// `OfflineTts` does not implement `Debug`, so we don't derive it here — the
/// pipeline only needs to move the engine into the worker thread and call
/// `synth_chunk` on it, never to print it.
pub struct PocketTts {
    inner: OfflineTts,
}

/// Build the Pocket TTS engine from the model directory installed by
/// `huddle::models`. Returns `Err` if any expected ONNX or JSON file is
/// missing — readiness is normally enforced by `is_tts_ready` upstream, but
/// the check is repeated here so a manually-modified model dir produces a
/// clear error string instead of an opaque sherpa-onnx `None`.
pub fn load_text_to_speech(model_dir: &str) -> Result<PocketTts, String> {
    let dir = PathBuf::from(model_dir);
    for name in [
        FILE_LM_MAIN,
        FILE_LM_FLOW,
        FILE_ENCODER,
        FILE_DECODER,
        FILE_TEXT_COND,
        FILE_VOCAB,
        FILE_TOKEN_SCORES,
    ] {
        let p = dir.join(name);
        if !p.is_file() {
            return Err(format!("missing Pocket TTS file: {}", p.display()));
        }
    }

    let to_str = |name: &str| -> String { dir.join(name).to_string_lossy().into_owned() };

    // Build the config by mutating defaults — mirrors `stt.rs` and stays
    // resilient if sherpa-onnx adds unrelated model-family fields.
    let mut cfg = OfflineTtsConfig::default();
    cfg.model.pocket.lm_main = Some(to_str(FILE_LM_MAIN));
    cfg.model.pocket.lm_flow = Some(to_str(FILE_LM_FLOW));
    cfg.model.pocket.encoder = Some(to_str(FILE_ENCODER));
    cfg.model.pocket.decoder = Some(to_str(FILE_DECODER));
    cfg.model.pocket.text_conditioner = Some(to_str(FILE_TEXT_COND));
    cfg.model.pocket.vocab_json = Some(to_str(FILE_VOCAB));
    cfg.model.pocket.token_scores_json = Some(to_str(FILE_TOKEN_SCORES));
    cfg.model.pocket.voice_embedding_cache_capacity = VOICE_EMBEDDING_CACHE_CAPACITY;
    cfg.model.num_threads = TTS_NUM_THREADS;
    // Explicit — defaults are not part of the API contract, and noisy debug
    // logging in release builds would be expensive on every synthesized chunk.
    cfg.model.debug = false;

    let inner = OfflineTts::create(&cfg)
        .ok_or_else(|| "OfflineTts::create returned None for Pocket TTS".to_string())?;
    Ok(PocketTts { inner })
}

// ── Prompt preparation ────────────────────────────────────────────────────────

/// Result of [`prepare_pocket_prompt`]: a synthesizer-ready prompt plus the
/// per-call generation hints derived from the original text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedPrompt {
    /// Text to hand to `OfflineTts::generate_with_config`. Capitalized,
    /// punctuation-terminated, and (for short inputs) left-padded with spaces.
    pub text: String,
    /// Value to pass via `GenerationConfig.extra["frames_after_eos"]`.
    pub frames_after_eos: i32,
    /// Value to pass via `GenerationConfig.extra["max_frames"]`. Adaptive to
    /// text length — short prompts get a much tighter cap to prevent runaway
    /// "monster breathing" generation when the EOS logit fails to fire.
    pub max_frames: i32,
}

/// Mirror of upstream `pocket_tts.models.tts_model.prepare_text_prompt` plus
/// `_estimate_max_gen_len`. Sherpa-onnx's C++ Pocket TTS impl does not run
/// these preparation steps, so short / unpunctuated / lowercase inputs can
/// trigger up to 40 s of runaway generation when the EOS logit never crosses
/// its threshold. We replicate the upstream Python recipe here:
///
/// 1. Collapse interior whitespace (already done by `preprocess_for_tts`, but
///    cheap to re-check after sentence splitting).
/// 2. Capitalize the first letter.
/// 3. Append `.` if the text doesn't end in punctuation.
/// 4. If fewer than five words, prepend `SHORT_PROMPT_PAD_SPACES` spaces and
///    bump `frames_after_eos` from 1 → 3.
/// 5. Compute an adaptive `max_frames` from the (post-padding) word count.
///
/// Returns `None` only if the input is empty after trimming — caller should
/// skip synthesis in that case.
pub(crate) fn prepare_pocket_prompt(input: &str) -> Option<PreparedPrompt> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Collapse stray double-spaces / embedded newlines that may slip past
    // `preprocess_for_tts` when sentences are spliced back together.
    let mut cleaned = String::with_capacity(trimmed.len());
    let mut last_was_space = false;
    for ch in trimmed.chars() {
        let is_ws = ch.is_whitespace();
        if is_ws {
            if !last_was_space {
                cleaned.push(' ');
            }
            last_was_space = true;
        } else {
            cleaned.push(ch);
            last_was_space = false;
        }
    }

    // Capitalize first character. Uses `to_uppercase` (multi-codepoint safe).
    let first = cleaned.chars().next().expect("cleaned non-empty above");
    if first.is_lowercase() {
        let upper: String = first.to_uppercase().collect();
        let mut iter = cleaned.chars();
        iter.next();
        cleaned = upper + iter.as_str();
    }

    // Ensure terminal punctuation. Anything not in `.!?;:,` gets a period.
    // The upstream Python only checks `isalnum` → period, but for our agent
    // text we already may end in `!` `?` `.` etc. — treat any of those as OK.
    let last = cleaned
        .chars()
        .next_back()
        .expect("cleaned non-empty above");
    if !matches!(last, '.' | '!' | '?' | ';' | ':' | ',') {
        cleaned.push('.');
    }

    // Word count of the *cleaned but not padded* text — padding is whitespace
    // only and would just lie to the threshold check below.
    let word_count = cleaned.split_whitespace().count();
    let is_short = word_count <= SHORT_PROMPT_WORD_THRESHOLD;

    let final_text = if is_short {
        let mut padded = String::with_capacity(cleaned.len() + SHORT_PROMPT_PAD_SPACES);
        for _ in 0..SHORT_PROMPT_PAD_SPACES {
            padded.push(' ');
        }
        padded.push_str(&cleaned);
        padded
    } else {
        cleaned
    };

    let frames_after_eos = if is_short { 3 } else { 1 };
    let max_frames = estimate_max_frames(word_count);

    Some(PreparedPrompt {
        text: final_text,
        frames_after_eos,
        max_frames,
    })
}

/// Convert a word count into a Mimi-frame cap, matching upstream
/// `_estimate_max_gen_len`. We use words as a sentencepiece-token proxy: real
/// SP tokenization runs ~1.2–1.5 tokens/word for English, which the
/// `GEN_SECONDS_PADDING` slack absorbs. Saturates at
/// `MAX_FRAMES_HARD_CEILING` so we never *raise* the upstream default.
fn estimate_max_frames(word_count: usize) -> i32 {
    // Treat each word as ~1.3 tokens — within the slack envelope but a touch
    // generous so we don't truncate genuine short utterances.
    let approx_tokens = word_count as f32 * 1.3;
    let gen_len_sec = approx_tokens / TOKENS_PER_SECOND_ESTIMATE + GEN_SECONDS_PADDING;
    let frames = (gen_len_sec * MIMI_FRAME_RATE).ceil() as i32;
    frames.clamp(1, MAX_FRAMES_HARD_CEILING)
}

impl PocketTts {
    /// Synthesise `text` with the given reference voice.
    ///
    /// `_lang` and `_steps` are accepted for API compatibility with the
    /// previous Kokoro engine. Pocket TTS infers language from the input text
    /// directly and is a one-step consistency model. Returns an empty buffer
    /// for whitespace-only input.
    pub fn synth_chunk(
        &self,
        text: &str,
        _lang: &str,
        style: &VoiceStyle,
        _steps: usize,
        speed: f32,
    ) -> Result<Vec<f32>, String> {
        // Mirror upstream pocket-tts prompt prep — without this short or
        // unpunctuated inputs can cause the LM's EOS logit to never trip,
        // producing up to 40 s of "monster breathing" garbage on the first
        // utterance. See `prepare_pocket_prompt` for the full recipe.
        let prepared = match prepare_pocket_prompt(text) {
            Some(p) => p,
            None => return Ok(Vec::new()),
        };

        // Per-call generation hints sherpa-onnx forwards to
        // `offline-tts-pocket-impl.h`. `frames_after_eos` is bumped for short
        // prompts to give the model trailing room to gracefully decay; the
        // adaptive `max_frames` is the safety net that bounds runaway
        // generation when EOS never fires.
        let mut extra: HashMap<String, serde_json::Value> = HashMap::with_capacity(2);
        extra.insert(
            "frames_after_eos".to_string(),
            serde_json::Value::from(prepared.frames_after_eos),
        );
        extra.insert(
            "max_frames".to_string(),
            serde_json::Value::from(prepared.max_frames),
        );

        let cfg = GenerationConfig {
            speed,
            num_steps: SYNTH_NUM_STEPS,
            silence_scale: SYNTH_SILENCE_SCALE,
            reference_audio: Some(style.samples.clone()),
            reference_sample_rate: style.sample_rate,
            extra: Some(extra),
            ..Default::default()
        };

        // No progress callback — synthesis is fast enough that returning the
        // whole buffer at once keeps the lookahead pipelining in `tts.rs`
        // simple. `None::<fn(...) -> bool>` pins the callback type for the
        // `generate_with_config` generic parameter.
        let audio = self
            .inner
            .generate_with_config(&prepared.text, &cfg, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| {
                format!(
                    "Pocket TTS synthesis failed for text ({} chars)",
                    prepared.text.len()
                )
            })?;

        let sample_rate = audio.sample_rate();
        if sample_rate != SAMPLE_RATE as i32 {
            eprintln!(
                "sprout-desktop: Pocket TTS returned unexpected sample rate {sample_rate}Hz \
                 (expected {SAMPLE_RATE}Hz); playback speed may be wrong"
            );
        }

        Ok(audio.samples().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── prepare_pocket_prompt ────────────────────────────────────────────────

    #[test]
    fn prepare_prompt_returns_none_for_empty_input() {
        assert!(prepare_pocket_prompt("").is_none());
        assert!(prepare_pocket_prompt("   ").is_none());
        assert!(prepare_pocket_prompt("\n\t  ").is_none());
    }

    #[test]
    fn prepare_prompt_pads_and_capitalizes_one_word() {
        // The "yep" case Tyler hit in production — bare lowercase one-word
        // utterance with no punctuation. Must be padded, capitalized, and
        // terminated.
        let out = prepare_pocket_prompt("yep").expect("non-empty");
        let pad = " ".repeat(SHORT_PROMPT_PAD_SPACES);
        assert_eq!(out.text, format!("{pad}Yep."));
        assert_eq!(out.frames_after_eos, 3);
        // 1 word → very low frame cap (well under the 500 hard ceiling).
        assert!(out.max_frames < MAX_FRAMES_HARD_CEILING);
    }

    #[test]
    fn prepare_prompt_preserves_existing_punctuation() {
        let out = prepare_pocket_prompt("yes!").expect("non-empty");
        let pad = " ".repeat(SHORT_PROMPT_PAD_SPACES);
        assert_eq!(out.text, format!("{pad}Yes!")); // exclamation kept
        let out = prepare_pocket_prompt("really?").expect("non-empty");
        assert_eq!(out.text, format!("{pad}Really?"));
    }

    #[test]
    fn prepare_prompt_threshold_is_inclusive_at_four_words() {
        // 4 words = short (padded); 5 words = long (not padded).
        let four = prepare_pocket_prompt("one two three four").expect("non-empty");
        assert!(
            four.text.starts_with(' '),
            "four-word input should be padded"
        );
        assert_eq!(four.frames_after_eos, 3);

        let five = prepare_pocket_prompt("one two three four five").expect("non-empty");
        assert!(
            !five.text.starts_with(' '),
            "five-word input should NOT be padded"
        );
        assert_eq!(five.frames_after_eos, 1);
    }

    #[test]
    fn prepare_prompt_does_not_pad_long_text() {
        let long = "This is a longer sentence that the model should handle just fine.";
        let out = prepare_pocket_prompt(long).expect("non-empty");
        assert!(!out.text.starts_with(' '));
        assert_eq!(out.frames_after_eos, 1);
        assert!(out.text.ends_with('.'));
    }

    #[test]
    fn prepare_prompt_collapses_whitespace() {
        let out = prepare_pocket_prompt("Hello    world\n\nfriend").expect("non-empty");
        // No padding (3 words → short → padded), but interior is collapsed.
        let pad = " ".repeat(SHORT_PROMPT_PAD_SPACES);
        assert_eq!(out.text, format!("{pad}Hello world friend."));
    }

    #[test]
    fn prepare_prompt_does_not_double_capitalize_already_uppercase() {
        let out = prepare_pocket_prompt("HELLO there").expect("non-empty");
        let pad = " ".repeat(SHORT_PROMPT_PAD_SPACES);
        assert_eq!(out.text, format!("{pad}HELLO there."));
    }

    #[test]
    fn prepare_prompt_handles_non_ascii_first_letter() {
        // Cyrillic lowercase 'д' → uppercase 'Д'. Must not panic / produce
        // mojibake.
        let out = prepare_pocket_prompt("дa").expect("non-empty");
        assert!(out.text.contains("Дa."));
    }

    // ── estimate_max_frames ──────────────────────────────────────────────────

    #[test]
    fn estimate_max_frames_is_tight_for_short_input() {
        // 1 word: 1 * 1.3 / 3.0 + 2.0 ≈ 2.43s ≈ 31 frames. Well below 500.
        let frames = estimate_max_frames(1);
        assert!(frames > 0);
        assert!(frames < 50, "got {frames}");
    }

    #[test]
    fn estimate_max_frames_saturates_at_ceiling() {
        // 5000 words ≈ a runaway prompt; must clamp at the hard ceiling.
        assert_eq!(estimate_max_frames(5_000), MAX_FRAMES_HARD_CEILING);
    }

    #[test]
    fn estimate_max_frames_grows_with_word_count() {
        let small = estimate_max_frames(2);
        let medium = estimate_max_frames(20);
        let large = estimate_max_frames(100);
        assert!(small < medium);
        assert!(medium < large);
        assert!(large <= MAX_FRAMES_HARD_CEILING);
    }

    #[test]
    fn estimate_max_frames_never_zero() {
        // Sanity: even a 0-word prompt yields ≥1 frame so we never ask the
        // engine for an impossible cap.
        assert!(estimate_max_frames(0) >= 1);
    }
}
