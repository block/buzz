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
//! Buzz ships these files unmodified; see the on-disk `MODEL_LICENSE.txt`
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
//!   → `Result<Vec<f32>, String>`
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

/// sherpa-onnx upstream default for `max_frames` (LM steps), in
/// `offline-tts-pocket-impl.h:Generate`. 500 steps ≈ 40 s of audio at the
/// Mimi 12.5 Hz frame rate. Referenced only by the regression test below;
/// production code path never raises (or even reads) this value — we just
/// leave sherpa-onnx's own default in place by not setting the override.
#[cfg(test)]
const SHERPA_ONNX_MAX_FRAMES_DEFAULT: i32 = 500;

/// Tight `max_frames` we ask for on short, padded prompts to bound the
/// original "monster breathing" runaway. 100 LM steps ≈ 8 s of audio —
/// roomy for any one-to-four-word utterance the user is likely to elicit
/// while still well short of the 40 s upstream default. Chosen with slack so
/// we never *truncate* a legitimate short reply.
const SHORT_PROMPT_MAX_FRAMES: i32 = 100;

/// Word-count threshold (inclusive) below which we pad the prompt with
/// leading spaces and cap `max_frames` tighter than the upstream default.
/// Matches upstream `pocket_tts.models.tts_model.prepare_text_prompt`. Above
/// this threshold we leave sherpa-onnx's own defaults in place — overriding
/// them caused the "first 'yep' is just static" regression seen on
/// 2026-05-18, where dropping `frames_after_eos` below the upstream default
/// of 3 clipped the leading audio of multi-clause sentences.
const SHORT_PROMPT_WORD_THRESHOLD: usize = 4;

/// Number of leading spaces prepended to short prompts. The upstream Python
/// uses exactly 8 — keep parity rather than tuning blindly.
const SHORT_PROMPT_PAD_SPACES: usize = 8;

/// Sacrificial cold-start prefix appended *after* the leading space pad for
/// short prompts. Pocket TTS' FlowLM autoregressive generation has a 2–3
/// step "settle" period at the start where the first generated phoneme can
/// be smeared or dropped entirely (see kyutai-labs/pocket-tts #91, #70 and
/// sherpa-onnx #3180). For short utterances like "I'm happy." the first
/// phoneme is most of the first word — losing it produces "m happy".
///
/// Two periods separated by a space act as a "phantom utterance" that the
/// model commits to, absorbing the cold-start. The pair (rather than a
/// single period) was empirically the only variant in our probe — see
/// `examples/prod_probe.rs` — that produced a usable post-sacrificial
/// silence gap on every random seed. The resulting leading sacrificial
/// audio is then stripped from the output by [`trim_leading_cold_start`]
/// before the buffer is returned to the synth pipeline.
///
/// Long prompts (>4 words) don't need this — the first phoneme already has
/// enough downstream context to avoid the smear, and an early natural pause
/// (e.g. the comma in "Hello, how can I help you?") could be misdetected as
/// the trim boundary.
const SACRIFICIAL_PREFIX: &str = ". . ";

// ── Leading cold-start trim (post-synth) ──────────────────────────────────────

/// Skip this many samples at the start of the synth buffer before looking
/// for the sacrificial→main silence gap. The Mimi decoder cold-start
/// produces ~30 ms of low-amplitude noise that we *don't* want to treat as
/// the gap. 30 ms × 24 kHz = 720 samples.
const TRIM_SCAN_START_SAMPLES: usize = (SAMPLE_RATE as usize * 30) / 1000;

/// Amplitude threshold below which a sample is considered "silence" for the
/// purposes of finding the post-sacrificial gap. Tuned empirically against
/// production-config probe data — the engine's own `ScaleSilence` uses 0.01,
/// but our boundary detection wants a looser threshold so that the breath /
/// aspiration of the rendered periods (which sits around 0.005–0.015) is
/// treated as silence too.
const TRIM_SILENCE_THRESHOLD: f32 = 0.02;

/// A silence run must be at least this many samples long to be accepted as
/// the sacrificial→main word boundary. 50 ms is comfortably longer than
/// inter-syllable silence within a normal word at this speed (typically
/// 10–30 ms), so this guards against trimming into the middle of the real
/// utterance. 50 ms × 24 kHz = 1200 samples.
const TRIM_MIN_GAP_SAMPLES: usize = (SAMPLE_RATE as usize * 50) / 1000;

/// Hard cap on how much audio we'll trim from the start. Production probe
/// data (with `silence_scale = 0.0`) shows valid trim boundaries land
/// between 30 ms and ~450 ms; 1.2 s is a wide safety margin. If the
/// detector finds a "gap" past this point it's almost certainly an interior
/// pause inside an unusually long short-prompt utterance — bail out and
/// emit untrimmed audio rather than corrupt it. 1.2 s × 24 kHz = 28800
/// samples.
const TRIM_MAX_DROP_SAMPLES: usize = (SAMPLE_RATE as usize * 1200) / 1000;

/// sherpa-onnx's documented `frames_after_eos` default. We deliberately do
/// *not* override this knob — the previous attempt to bump it for short
/// inputs and lower it for long inputs lowered it below the upstream default
/// of 3, which clipped the leading audio of multi-clause sentences (the
/// "first 'yep' is static" regression). The constant exists only for the
/// regression test below. Source: `offline-tts-pocket-impl.h:Generate`.
#[cfg(test)]
const SHERPA_ONNX_FRAMES_AFTER_EOS_DEFAULT: i32 = 3;

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
/// per-call generation overrides derived from the original text.
///
/// `None` for either override means "leave sherpa-onnx's documented default
/// in place". The pipeline only sets `max_frames` (and only for short
/// padded inputs) so it can bound the original "monster breathing" runaway
/// without disturbing the rest of the LM sampling envelope.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedPrompt {
    /// Text to hand to `OfflineTts::generate_with_config`. Capitalized,
    /// punctuation-terminated, and (for short inputs) left-padded with spaces
    /// plus a sacrificial `". . "` cold-start prefix.
    pub text: String,
    /// Value to pass via `GenerationConfig.extra["max_frames"]`, or `None` to
    /// keep the upstream default of 500 LM steps. We only override on short
    /// padded prompts where we have a tight expectation on output length.
    pub max_frames: Option<i32>,
    /// `true` iff this prompt received the short-input treatment (leading
    /// space pad + sacrificial `". . "` prefix). The synth pipeline uses
    /// this to decide whether to apply [`trim_leading_cold_start`] to the
    /// output: long prompts have no sacrificial audio to strip, and
    /// trimming them risks deleting real speech at a natural early pause
    /// (e.g. the comma in "Hello, how can I help you?").
    pub is_short: bool,
}

/// Mirror of the *text-preparation* half of upstream
/// `pocket_tts.models.tts_model.prepare_text_prompt`. Sherpa-onnx's C++
/// Pocket TTS impl does not run these preparation steps, so short /
/// unpunctuated / lowercase inputs can trigger up to 40 s of runaway
/// generation when the EOS logit never crosses its threshold. We replicate
/// the upstream Python recipe here:
///
/// 1. Collapse interior whitespace (already done by `preprocess_for_tts`, but
///    cheap to re-check after sentence splitting).
/// 2. Capitalize the first letter.
/// 3. Append `.` if the text doesn't end in punctuation.
/// 4. If fewer than five words, prepend `SHORT_PROMPT_PAD_SPACES` spaces
///    followed by [`SACRIFICIAL_PREFIX`] (a `". . "` cold-start absorber —
///    see its docstring for the bug it works around), and return a tight
///    [`SHORT_PROMPT_MAX_FRAMES`] cap so the LM can't run away if EOS
///    still doesn't fire.
///
/// We do **not** override `frames_after_eos` — sherpa-onnx's default of 3
/// is what we want. An earlier version set it to 1 on long inputs, which
/// clipped the leading audio of multi-clause sentences ("first 'yep' is
/// just static" regression). Tests `prepare_prompt_never_lowers_frames_…`
/// lock this in.
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

    let (final_text, max_frames) = if is_short {
        let mut padded = String::with_capacity(
            cleaned.len() + SHORT_PROMPT_PAD_SPACES + SACRIFICIAL_PREFIX.len(),
        );
        for _ in 0..SHORT_PROMPT_PAD_SPACES {
            padded.push(' ');
        }
        padded.push_str(SACRIFICIAL_PREFIX);
        padded.push_str(&cleaned);
        (padded, Some(SHORT_PROMPT_MAX_FRAMES))
    } else {
        // For everything ≥5 words, fall back to upstream defaults. Overriding
        // these is what caused the "first 'yep' is static" regression — the
        // upstream LM has been tuned for `frames_after_eos = 3` and
        // `max_frames = 500`, and there's no clear win in second-guessing.
        (cleaned, None)
    };

    Some(PreparedPrompt {
        text: final_text,
        max_frames,
        is_short,
    })
}

/// Strip the leading "sacrificial" audio produced by the `". . "` cold-start
/// prefix from a short-prompt synthesis result. Only call this when
/// [`PreparedPrompt::is_short`] is `true` — the trim looks for a long
/// silence run at the head of the buffer, and an early natural pause inside
/// a long unsacrificed utterance (e.g. the comma in "Hello, how can I help
/// you?") would be misclassified as the sacrificial gap.
///
/// Algorithm:
///   1. Skip the first [`TRIM_SCAN_START_SAMPLES`] of the buffer (Mimi
///      cold-start noise we shouldn't classify as silence).
///   2. Scan forward for the first run of samples below
///      [`TRIM_SILENCE_THRESHOLD`] that lasts at least
///      [`TRIM_MIN_GAP_SAMPLES`] — that's the post-sacrificial boundary.
///   3. If that boundary lies beyond [`TRIM_MAX_DROP_SAMPLES`], treat it as
///      "almost certainly an interior pause" and *do not trim* — the safe
///      fallback is to play the slightly-degraded raw audio rather than
///      delete real speech.
///   4. Otherwise, drop the leading samples up to the end of the silence
///      run. We don't insert a zero lead-in here — `tts.rs` owns playback
///      cushioning by prepending `SENTENCE_LEAD_IN_SAMPLES` of zeros before
///      each appended sentence chunk.
///
/// If the scan never finds a long-enough gap (≈1% of generations in the
/// production-config probe), the function is a no-op — the model trajectory
/// missed the expected sacrificial→main structure and we'd rather play the
/// raw buffer than emit silence.
fn trim_leading_cold_start(samples: &mut Vec<f32>) {
    if samples.len() <= TRIM_SCAN_START_SAMPLES {
        return;
    }

    let mut silence_run_start: Option<usize> = None;
    let mut gap_end: Option<usize> = None;
    for (i, sample) in samples.iter().enumerate().skip(TRIM_SCAN_START_SAMPLES) {
        if sample.abs() < TRIM_SILENCE_THRESHOLD {
            silence_run_start.get_or_insert(i);
        } else if let Some(start) = silence_run_start {
            if i - start >= TRIM_MIN_GAP_SAMPLES {
                gap_end = Some(i);
                break;
            }
            silence_run_start = None;
        }
    }

    let Some(end) = gap_end else {
        // No gap found — model didn't produce the expected sacrificial→main
        // structure. Bail out and let the caller play the raw buffer.
        return;
    };
    if end > TRIM_MAX_DROP_SAMPLES {
        // Boundary too far into the audio to plausibly be the sacrificial
        // gap. Almost certainly an interior pause in the real utterance —
        // leave the audio alone.
        return;
    }

    samples.drain(..end);
}

/// Build the `GenerationConfig.extra` HashMap from a [`PreparedPrompt`].
///
/// Centralised so the regression test below can assert that we **never**
/// emit a `frames_after_eos` override — the previous attempt to override
/// that knob (setting it to 1 for ≥5-word inputs) clipped the leading
/// audio of multi-clause sentences (the "first 'yep' is static" bug on
/// 2026-05-18). The upstream sherpa-onnx default of 3 is what we want, and
/// the right way to keep it is to not set it at all.
fn build_generation_extra(prepared: &PreparedPrompt) -> Option<HashMap<String, serde_json::Value>> {
    prepared.max_frames.map(|mf| {
        let mut h: HashMap<String, serde_json::Value> = HashMap::with_capacity(1);
        h.insert("max_frames".to_string(), serde_json::Value::from(mf));
        h
    })
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
        // `offline-tts-pocket-impl.h`. We only override `max_frames`, and
        // only for short padded prompts where we have a tight expectation
        // on output length — that bounds the original runaway without
        // disturbing the rest of the LM sampling envelope. See
        // `prepare_pocket_prompt` docs for the regression history.
        let extra = build_generation_extra(&prepared);

        let cfg = GenerationConfig {
            speed,
            num_steps: SYNTH_NUM_STEPS,
            silence_scale: SYNTH_SILENCE_SCALE,
            reference_audio: Some(style.samples.clone()),
            reference_sample_rate: style.sample_rate,
            extra,
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
                "buzz-desktop: Pocket TTS returned unexpected sample rate {sample_rate}Hz \
                 (expected {SAMPLE_RATE}Hz); playback speed may be wrong"
            );
        }

        let mut samples = audio.samples().to_vec();
        // For short prompts the prepared text includes a sacrificial ". . "
        // prefix to absorb FlowLM/Mimi cold-start (see `SACRIFICIAL_PREFIX`).
        // Strip the leading sacrificial audio before returning. Long prompts
        // are never trimmed — they have no sacrificial audio, and an early
        // natural pause could be mis-detected as the trim boundary.
        if prepared.is_short {
            trim_leading_cold_start(&mut samples);
        }
        Ok(samples)
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

    /// Helper: the exact leading sequence prepended to every short prompt —
    /// 8 spaces of padding followed by the sacrificial `". . "` cold-start
    /// absorber. Centralising this keeps the assertions readable.
    fn short_prefix() -> String {
        let mut s = " ".repeat(SHORT_PROMPT_PAD_SPACES);
        s.push_str(SACRIFICIAL_PREFIX);
        s
    }

    #[test]
    fn prepare_prompt_pads_and_capitalizes_one_word() {
        // The "yep" case Tyler hit in production — bare lowercase one-word
        // utterance with no punctuation. Must be padded with the short-prompt
        // prefix (8 spaces + ". . " sacrificial), capitalized, terminated,
        // with a tight `max_frames` cap to bound runaway gen.
        let out = prepare_pocket_prompt("yep").expect("non-empty");
        assert_eq!(out.text, format!("{}Yep.", short_prefix()));
        assert!(out.is_short, "1-word input is short");
        assert_eq!(out.max_frames, Some(SHORT_PROMPT_MAX_FRAMES));
        const {
            assert!(
                SHORT_PROMPT_MAX_FRAMES < SHERPA_ONNX_MAX_FRAMES_DEFAULT,
                "short cap must be tighter than the upstream default"
            );
        }
    }

    #[test]
    fn prepare_prompt_preserves_existing_punctuation() {
        let out = prepare_pocket_prompt("yes!").expect("non-empty");
        assert_eq!(out.text, format!("{}Yes!", short_prefix())); // exclamation kept
        let out = prepare_pocket_prompt("really?").expect("non-empty");
        assert_eq!(out.text, format!("{}Really?", short_prefix()));
    }

    #[test]
    fn prepare_prompt_threshold_is_inclusive_at_four_words() {
        // 4 words = short (padded + sacrificial + tight max_frames); 5 words
        // = long (no padding, no sacrificial, no overrides — upstream
        // defaults stand).
        let four = prepare_pocket_prompt("one two three four").expect("non-empty");
        assert!(four.is_short, "four-word input should be short");
        assert!(
            four.text.starts_with(' '),
            "four-word input should start with the space pad"
        );
        assert!(
            four.text.contains(SACRIFICIAL_PREFIX),
            "four-word input should contain the sacrificial prefix"
        );
        assert_eq!(four.max_frames, Some(SHORT_PROMPT_MAX_FRAMES));

        let five = prepare_pocket_prompt("one two three four five").expect("non-empty");
        assert!(!five.is_short, "five-word input should NOT be short");
        assert!(
            !five.text.starts_with(' '),
            "five-word input should NOT be padded"
        );
        assert!(
            !five.text.contains(SACRIFICIAL_PREFIX),
            "five-word input must not receive the sacrificial prefix"
        );
        assert_eq!(
            five.max_frames, None,
            "long inputs must leave sherpa-onnx's max_frames default in place"
        );
    }

    #[test]
    fn prepare_prompt_does_not_pad_long_text() {
        let long = "This is a longer sentence that the model should handle just fine.";
        let out = prepare_pocket_prompt(long).expect("non-empty");
        assert!(!out.is_short);
        assert!(!out.text.starts_with(' '));
        assert!(!out.text.contains(SACRIFICIAL_PREFIX));
        assert_eq!(out.max_frames, None);
        assert!(out.text.ends_with('.'));
    }

    #[test]
    fn prepare_prompt_collapses_whitespace() {
        let out = prepare_pocket_prompt("Hello    world\n\nfriend").expect("non-empty");
        // 3 words → short → padded + sacrificial. Interior whitespace
        // collapsed.
        assert_eq!(out.text, format!("{}Hello world friend.", short_prefix()));
    }

    #[test]
    fn prepare_prompt_does_not_double_capitalize_already_uppercase() {
        let out = prepare_pocket_prompt("HELLO there").expect("non-empty");
        assert_eq!(out.text, format!("{}HELLO there.", short_prefix()));
    }

    #[test]
    fn prepare_prompt_handles_non_ascii_first_letter() {
        // Cyrillic lowercase 'д' → uppercase 'Д'. Must not panic / produce
        // mojibake.
        let out = prepare_pocket_prompt("дa").expect("non-empty");
        assert!(out.text.contains("Дa."));
    }

    #[test]
    fn prepare_prompt_inserts_sacrificial_prefix_only_for_short() {
        // Pinning the exact ordering: pad, then ". . ", then cleaned text.
        // If this ever flips, the trim algorithm's calibration breaks.
        let out = prepare_pocket_prompt("I'm happy.").expect("non-empty");
        assert!(out.is_short);
        let pad = " ".repeat(SHORT_PROMPT_PAD_SPACES);
        let expected = format!("{pad}. . I'm happy.");
        assert_eq!(out.text, expected);
        assert_eq!(SACRIFICIAL_PREFIX, ". . ");
    }

    // ── build_generation_extra ───────────────────────────────────────────────
    //
    // These tests pin down a behaviour we've now regressed twice on:
    //   1) Not padding/punctuating short inputs → 40 s of "monster breathing"
    //      (pre-773a2a1).
    //   2) Setting `frames_after_eos = 1` on long inputs → clipped leading
    //      audio of multi-clause sentences, e.g. "Yep, I can hear you. …"
    //      came out as a static burst (the 773a2a1 regression Tyler hit on
    //      2026-05-18 ~14:30 UTC).
    //
    // The contract we enforce going forward: we **only** override
    // `max_frames`, and only for ≤4-word inputs. Every other knob is left
    // at sherpa-onnx's documented default (notably `frames_after_eos = 3`).

    #[test]
    fn build_extra_short_prompt_sets_only_max_frames() {
        let prepared = prepare_pocket_prompt("yep").expect("non-empty");
        let extra = build_generation_extra(&prepared).expect("short prompts get extra");
        // Exactly one key — `max_frames` — and nothing else.
        assert_eq!(extra.len(), 1, "extra has unexpected keys: {extra:?}");
        assert_eq!(
            extra.get("max_frames"),
            Some(&serde_json::Value::from(SHORT_PROMPT_MAX_FRAMES))
        );
        assert!(
            !extra.contains_key("frames_after_eos"),
            "frames_after_eos must never be set — upstream default of {SHERPA_ONNX_FRAMES_AFTER_EOS_DEFAULT} is what we want"
        );
    }

    #[test]
    fn build_extra_long_prompt_is_none() {
        // ≥5 words: no extras at all. This is the key fix for the "first
        // 'yep' in 'Yep, I can hear you. …' is static" regression — we
        // were previously forcing `frames_after_eos = 1` on this path.
        let prepared = prepare_pocket_prompt("Yep, I can hear you.").expect("non-empty");
        assert_eq!(
            build_generation_extra(&prepared),
            None,
            "long prompts must not override any LM knob"
        );
    }

    #[test]
    fn build_extra_never_lowers_frames_after_eos_for_any_word_count() {
        // Sweep a range of prompt lengths and assert the `extra` map (when
        // present) never carries a `frames_after_eos` override that's lower
        // than the upstream sherpa-onnx default. Implemented as a structural
        // check — we just never set the key — but worth a property test in
        // case someone reintroduces the override in the future.
        let prompts: &[&str] = &[
            "hi",
            "hi there",
            "yes please",
            "one two three four",
            "one two three four five",
            "a slightly longer reply, hopefully fine",
            "This is a multi-clause sentence. It has two parts.",
            "really really really really really long prompt with lots of words just to be sure",
        ];
        for &p in prompts {
            let prepared = prepare_pocket_prompt(p).expect("non-empty");
            if let Some(extra) = build_generation_extra(&prepared) {
                if let Some(v) = extra.get("frames_after_eos") {
                    let n = v.as_i64().expect("frames_after_eos should be int");
                    assert!(
                        n >= SHERPA_ONNX_FRAMES_AFTER_EOS_DEFAULT as i64,
                        "prompt {p:?} set frames_after_eos={n}, below upstream default of {SHERPA_ONNX_FRAMES_AFTER_EOS_DEFAULT}"
                    );
                }
            }
        }
    }

    #[test]
    fn short_prompt_max_frames_is_below_upstream_default() {
        // Sanity: the override only ever *lowers* the cap, never raises it.
        const {
            assert!(SHORT_PROMPT_MAX_FRAMES < SHERPA_ONNX_MAX_FRAMES_DEFAULT);
        }
        // …and is still large enough for a one-to-four-word reply. At Mimi's
        // 12.5 Hz frame rate, 100 frames = 8 s, which is roomy.
        const {
            assert!(SHORT_PROMPT_MAX_FRAMES >= 50, "would risk truncation");
        }
    }

    // ── trim_leading_cold_start ──────────────────────────────────────────────

    /// Build a synthetic buffer shaped like a sacrificial-prefixed Pocket TTS
    /// output: a bit of "sacrificial" energy at the head (modelled as
    /// alternating ±0.05 — above [`TRIM_SILENCE_THRESHOLD`] so it isn't
    /// classified as silence, matching what real probe WAVs look like in the
    /// 0–50 ms window), then a flat silence of `gap_ms` ms, then `tail_ms`
    /// of "real speech" at peak `tail_peak`.
    fn synth_buffer(sacrificial_ms: u32, gap_ms: u32, tail_ms: u32, tail_peak: f32) -> Vec<f32> {
        let sr = SAMPLE_RATE as usize;
        let mut v = Vec::new();
        for i in 0..(sr * sacrificial_ms as usize / 1000) {
            v.push(if i % 2 == 0 { 0.05 } else { -0.05 });
        }
        // Silence gap (true zeros — below TRIM_SILENCE_THRESHOLD).
        v.extend(std::iter::repeat_n(0.0_f32, sr * gap_ms as usize / 1000));
        // Real speech.
        for i in 0..(sr * tail_ms as usize / 1000) {
            v.push(if i % 2 == 0 { tail_peak } else { -tail_peak });
        }
        v
    }

    #[test]
    fn trim_strips_sacrificial_and_keeps_only_speech() {
        // 60 ms sacrificial + 100 ms gap + 500 ms speech at peak 0.3.
        // After trim, the output is just the speech tail.
        let mut v = synth_buffer(60, 100, 500, 0.3);
        trim_leading_cold_start(&mut v);

        // First sample should be speech (|s| ≥ 0.2). No zero lead-in here
        // because tts.rs's `first_append` lead-in handles the device cushion.
        assert!(
            v[0].abs() > 0.2,
            "first sample after trim should be speech, got {}",
            v[0]
        );
        let actual_ms = (v.len() as f32 / SAMPLE_RATE as f32) * 1000.0;
        assert!(
            (actual_ms - 500.0).abs() < 5.0,
            "expected ~500 ms of trimmed audio, got {actual_ms} ms"
        );
    }

    #[test]
    fn trim_is_noop_when_no_long_silence_gap_exists() {
        // Pure speech: every sample is real (no gap >= 50 ms). Trimmer must
        // leave the buffer untouched so we don't truncate the utterance.
        let mut v = synth_buffer(0, 0, 600, 0.3);
        let before = v.clone();
        trim_leading_cold_start(&mut v);
        assert_eq!(v, before, "no gap → no trim");
    }

    #[test]
    fn trim_is_noop_when_gap_is_shorter_than_threshold() {
        // 40 ms gap is below TRIM_MIN_GAP_SAMPLES (50 ms). Must not trigger.
        let mut v = synth_buffer(60, 40, 600, 0.3);
        let before = v.clone();
        trim_leading_cold_start(&mut v);
        assert_eq!(v, before, "sub-threshold gap → no trim");
    }

    #[test]
    fn trim_is_noop_when_gap_is_beyond_max_drop_bound() {
        // Gap starts at 1500 ms (past TRIM_MAX_DROP_SAMPLES = 1200 ms).
        // This represents an interior pause inside an unusually long
        // utterance that slipped past the short-prompt predicate; we must
        // not chop the first 1.5 s of real audio.
        let mut v = synth_buffer(1500, 200, 400, 0.3);
        let before = v.clone();
        trim_leading_cold_start(&mut v);
        assert_eq!(v, before, "gap past max-drop bound → no trim");
    }

    #[test]
    fn trim_is_noop_on_buffer_smaller_than_scan_start() {
        // 20 ms buffer is smaller than TRIM_SCAN_START_SAMPLES (30 ms).
        // Trimmer must early-return without panicking.
        let mut v = vec![0.5f32; (SAMPLE_RATE as usize * 20) / 1000];
        let before = v.clone();
        trim_leading_cold_start(&mut v);
        assert_eq!(v, before);
    }

    #[test]
    fn trim_constants_use_sane_units() {
        // Pin the constants in milliseconds so anyone tuning later can see
        // at a glance what they're changing.
        assert_eq!(
            TRIM_SCAN_START_SAMPLES,
            (SAMPLE_RATE as usize * 30) / 1000,
            "scan-start should be 30 ms"
        );
        assert_eq!(
            TRIM_MIN_GAP_SAMPLES,
            (SAMPLE_RATE as usize * 50) / 1000,
            "min-gap should be 50 ms"
        );
        assert_eq!(
            TRIM_MAX_DROP_SAMPLES,
            (SAMPLE_RATE as usize * 1200) / 1000,
            "max-drop should be 1.2 s"
        );
    }
}
